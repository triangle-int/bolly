use std::path::PathBuf;

use axum::{Router, middleware};
use tower_http::services::{ServeDir, ServeFile};

use crate::{app::state::AppState, routes};

use super::{auth::auth_middleware, companion_boundary::companion_boundary};

fn api_router(state: &AppState) -> Router<AppState> {
    Router::new()
        .merge(routes::meta::router())
        .merge(routes::instances::router())
        .merge(routes::chat::router())
        .merge(routes::drops::router())
        .merge(routes::config::router())
        .merge(routes::soul::router())
        .merge(routes::thoughts::router())
        .merge(routes::uploads::router())
        .merge(routes::skills::router())
        .merge(routes::heartbeat::router())
        .merge(routes::ws::router())
        .merge(routes::update::router())
        .merge(routes::tts::router())
        .merge(routes::memory_import::router())
        .merge(routes::agents::router())
        .merge(routes::machine_agents::router())
        // Inner: admit only the canonical companion once the caller is authenticated.
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            companion_boundary,
        ))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ))
}

pub fn build_router(state: AppState, static_dir: Option<PathBuf>) -> Router {
    // API routes — protected by auth middleware
    let api = api_router(&state);

    // Public routes — no auth
    let health = routes::health::router();
    // Public routes still address a companion by slug and fail closed on foreign ones.
    let public_files = routes::uploads::public_router().route_layer(
        middleware::from_fn_with_state(state.clone(), companion_boundary),
    );
    let public_memory = routes::instances::public_memory_router().route_layer(
        middleware::from_fn_with_state(state.clone(), companion_boundary),
    );

    let app = Router::new()
        .merge(health)
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

#[cfg(test)]
#[path = "../../test-support/router_security.rs"]
mod tests;

#[cfg(test)]
#[path = "../../test-support/companion_boundary.rs"]
mod companion_boundary_tests;
