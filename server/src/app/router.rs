use std::path::PathBuf;

use axum::{Router, middleware};
use tower_http::services::{ServeDir, ServeFile};

use crate::{app::state::AppState, routes};

use super::auth::auth_middleware;

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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;

    #[tokio::test]
    async fn removed_google_workspace_routes_are_not_in_api_router() {
        let state = AppState::new(crate::config::Config::default()).await;
        let account_routes = format!("/api/instances/moon/google/{}", "accounts");
        let connect_route = format!("/api/instances/moon/google/{}", "connect");
        let disconnect_route = format!("{account_routes}/user@example.com");
        for (method, uri) in [
            ("GET", account_routes),
            ("GET", connect_route),
            ("DELETE", disconnect_route),
        ] {
            let response = api_router(&state)
                .with_state(state.clone())
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(&uri)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                axum::http::StatusCode::NOT_FOUND,
                "{uri}"
            );
        }
    }

    #[tokio::test]
    async fn standalone_api_has_no_managed_status_or_usage_route() {
        let state = AppState::new(crate::config::Config::default()).await;
        let status = api_router(&state)
            .with_state(state.clone())
            .oneshot(
                Request::builder()
                    .uri("/api/config/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(status.status(), axum::http::StatusCode::OK);
        let body = axum::body::to_bytes(status.into_body(), usize::MAX)
            .await
            .unwrap();
        let status: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(status.get("is_managed").is_none());

        let usage = api_router(&state)
            .with_state(state)
            .oneshot(
                Request::builder()
                    .uri("/api/usage")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(usage.status(), axum::http::StatusCode::NOT_FOUND);
    }
}
