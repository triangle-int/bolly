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
        .filter_module("lance", log::LevelFilter::Warn)
        .filter_module("lance_core", log::LevelFilter::Warn)
        .filter_module("lance_io", log::LevelFilter::Warn)
        .filter_module("lance_index", log::LevelFilter::Warn)
        .filter_module("lance_table", log::LevelFilter::Warn)
        .filter_module("lancedb", log::LevelFilter::Warn)
        .init();

    let mut config = config::load_config().unwrap_or_else(|err| {
        panic!(
            "failed to load config from {}: {err}",
            config::config_path().display()
        )
    });

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
             or BOLLY_PUBLIC_URL env var to your public address.",
            config.public_url
        );
    }

    let state = app::state::AppState::new(config).await;

    // Migrate legacy memory (facts.md + episodes.md → library) for all instances
    services::memory::migrate_all_instances(&state.workspace_dir);

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
        );

        // Backfill existing memories into LanceDB (background, non-blocking)
        let vs = state.vector_store.clone();
        let ws = state.workspace_dir.clone();
        let gai = google_ai_key;
        tokio::spawn(async move {
            let marker = ws.join(".vectors_backfilled_lancedb");
            if marker.exists() {
                return;
            }

            // Scan all instances and backfill
            let instances_dir = ws.join("instances");
            let entries = match std::fs::read_dir(&instances_dir) {
                Ok(e) => e,
                Err(_) => return,
            };

            let mut had_errors = false;
            for entry in entries.flatten() {
                if !entry.path().is_dir() {
                    continue;
                }
                let slug = entry.file_name().to_string_lossy().to_string();
                info!("[backfill] starting for instance {slug}");
                match vs.backfill_text_memories(&ws, &slug, &gai).await {
                    Ok(count) => info!("[backfill] {slug}: indexed {count} chunks"),
                    Err(e) => {
                        log::warn!("[backfill] {slug}: failed: {e}");
                        had_errors = true;
                    }
                }
            }

            // Only write marker if all instances succeeded
            if !had_errors {
                let _ = std::fs::write(&marker, "done");
                info!("[backfill] completed");
            } else {
                log::warn!("[backfill] completed with errors — will retry on next restart");
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
