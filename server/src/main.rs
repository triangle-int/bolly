mod app;
mod cli;
mod config;
mod domain;
mod routes;
mod services;

use std::net::SocketAddr;

use clap::Parser;
use log::info;

#[tokio::main]
async fn main() {
    let args = cli::Cli::parse();

    // Handle subcommands (start/stop/restart/status/logs/version)
    if let Some(cmd) = args.command {
        let code = cli::run(cmd);
        std::process::exit(code);
    }

    // No subcommand → run the server
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .filter_module("tracing::span", log::LevelFilter::Warn)
        .format(app::logging::format_record)
        .init();

    let mut config = config::load_config().unwrap_or_else(|err| {
        panic!(
            "failed to load config from {}: {err}",
            config::config_path().display()
        )
    });

    services::tools::register_control_secret(&config.auth_token);
    let host = config.host.clone();
    let port = config.port;
    let static_dir = if config.static_dir.is_empty() {
        None
    } else {
        let path = std::path::PathBuf::from(&config.static_dir);
        if path.is_dir() {
            Some(path)
        } else {
            log::warn!("static_dir {} does not exist, skipping", config.static_dir);
            None
        }
    };

    // Default public_url to localhost if not configured
    if config.public_url.is_empty() {
        config.public_url = format!("http://localhost:{port}");
        log::warn!(
            "public_url not set — defaulting to {}. \
             If running on a remote server, set public_url in config.toml \
             or NOLUNE_PUBLIC_URL env var to your public address.",
            config.public_url
        );
    }

    let state = app::state::AppState::new(config).await;

    // Remove unpublished passive-capture state before any agents start, using
    // the persistent workspace capability opened by the media store.
    let media_store = state.vector_store.media_store();
    if let Err(error) = media_store.cleanup_legacy_observers() {
        log::warn!("legacy observer cleanup was incomplete: {error}");
    }
    if let Err(error) = media_store.cleanup_legacy_screen_capture() {
        log::warn!("legacy passive screen-capture cleanup was incomplete: {error}");
    }

    // One companion per server (#103). Sibling directories from the unpublished
    // multi-instance layout are reported and otherwise ignored.
    let obsolete = services::companion::obsolete_instance_dirs(&state.workspace_dir);
    if !obsolete.is_empty() {
        log::warn!(
            "ignoring {} obsolete multi-instance director{}: {:?}. This server owns one companion \
             ({}); see docs/companion-storage.md",
            obsolete.len(),
            if obsolete.len() == 1 { "y" } else { "ies" },
            obsolete,
            domain::companion::CANONICAL_SLUG,
        );
    }

    // Migrate legacy memory (facts.md + episodes.md → library) for the companion
    services::memory::migrate_companion(&media_store);

    let addr: SocketAddr = format!("{host}:{port}").parse().unwrap_or_else(|_| {
        log::warn!("invalid host:port {host}:{port}, falling back to 0.0.0.0:{port}");
        SocketAddr::from(([0, 0, 0, 0], port))
    });

    // Notify active chats that the server restarted and spawn agent loops
    let restart_chats = services::chat::notify_restart(&state.workspace_dir, &state.events);
    for (slug, chat_id) in restart_chats {
        let cancel = tokio_util::sync::CancellationToken::new();
        let key = format!("{slug}/{chat_id}");
        {
            let mut tasks = state.agent_tasks.lock().await;
            tasks.insert(key, cancel.clone());
        }
        let bg_state = state.clone();
        tokio::spawn(async move {
            routes::chat::run_agent_loop(bg_state, slug, chat_id, cancel, false).await;
        });
    }

    // Start background scheduler for scheduled messages
    services::scheduler::start(state.clone());

    // Start heartbeat — companion's autonomous inner life
    {
        let google_ai_key = state.config.read().await.llm.tokens.google_ai.clone();
        services::heartbeat::start(
            &state.workspace_dir,
            state.llm.clone(),
            state.events.clone(),
            state.vector_store.clone(),
            google_ai_key.clone(),
            state.machine_registry.clone(),
            state.resources.clone(),
        );

        // Backfill missing or invalid local indexes from memory files (background, non-blocking)
        let vs = state.vector_store.clone();
        let ws = state.workspace_dir.clone();
        tokio::spawn(async move {
            let slug = domain::companion::CANONICAL_SLUG;
            match services::companion::read_identity(&ws) {
                Ok(Some(_)) => {}
                Ok(None) => {
                    info!("[backfill] companion not created yet; nothing to index");
                    return;
                }
                Err(error) => {
                    log::warn!("[backfill] companion storage is unusable: {error}");
                    return;
                }
            }
            match vs.needs_backfill(slug).await {
                Ok(false) => {
                    info!("[backfill] completed");
                    return;
                }
                Ok(true) => {}
                Err(e) => {
                    log::warn!(
                        "[backfill] {slug}: cannot prepare index: {e} — will retry on next restart"
                    );
                    return;
                }
            }
            info!("[backfill] starting for companion {slug}");
            match vs.backfill_text_memories(&ws, slug).await {
                Ok(count) => info!("[backfill] {slug}: indexed {count} chunks"),
                Err(e) => log::warn!("[backfill] {slug}: failed: {e} — will retry on next restart"),
            }
        });
    }

    let app = app::router::build_router(state, static_dir);

    info!("Starting server on http://{addr}");

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("failed to bind tcp listener");
    axum::serve(listener, app)
        .await
        .expect("server exited unexpectedly");
}
