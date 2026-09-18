use std::path::PathBuf;

use axum::{Router, middleware};
use tower_http::services::{ServeDir, ServeFile};

use crate::{app::state::AppState, routes};

use super::auth::auth_middleware;

pub fn build_router(state: AppState, static_dir: Option<PathBuf>) -> Router {
    // API routes — protected by auth middleware
    let api = Router::new()
        .merge(routes::meta::router())
        .merge(routes::instances::router())
        .merge(routes::chat::router())
        .merge(routes::drops::router())
        .merge(routes::config::router())
        .merge(routes::soul::router())
        .merge(routes::thoughts::router())
        .merge(routes::uploads::router())
        .merge(routes::skills::router())
        .merge(routes::usage::router())
        .merge(routes::google::router())
        .merge(routes::heartbeat::router())
        .merge(routes::ws::router())
        .merge(routes::update::router())
        .merge(routes::tts::router())
        .merge(routes::memory_import::router())
        .merge(routes::agents::router())
        .merge(routes::machine_agents::router())
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ));

    // Public routes — no auth
    let health = routes::health::router();
    let auth = routes::auth::router();
    let pwa = routes::pwa::router();
    let public_files = routes::uploads::public_router();
    let public_memory = routes::instances::public_memory_router();

    let app = Router::new()
        .merge(health)
        .merge(auth)
        .merge(pwa)
        .merge(public_files)
        .merge(public_memory)
        .merge(api)
        .with_state(state);

    // Serve static client files as fallback (SPA routing)
    // Priority: external static_dir > embedded assets
    if let Some(dir) = static_dir {
        let index = dir.join("index.html");
        let serve = ServeDir::new(dir).not_found_service(ServeFile::new(index));
        app.fallback_service(serve)
    } else {
        app.fallback_service(super::embedded_static::EmbeddedStaticService)
    }
}
