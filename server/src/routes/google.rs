use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{delete, get},
};

use crate::app::state::AppState;
use crate::services::google::GoogleClient;

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/instances/{slug}/google/accounts",
            get(list_google_accounts),
        )
        .route(
            "/api/instances/{slug}/google/connect",
            get(google_connect_url),
        )
        .route(
            "/api/instances/{slug}/google/accounts/{email}",
            delete(disconnect_google_account),
        )
}

/// GET /api/instances/:slug/google/accounts — list connected Google accounts
async fn list_google_accounts(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> impl IntoResponse {
    let config = state.config.read().await;
    let google = GoogleClient::new(&config.landing_url, &config.auth_token);

    let Some(google) = google else {
        return Json(serde_json::json!({ "accounts": [] })).into_response();
    };

    match google.accounts(&slug).await {
        Ok(accounts) => {
            let items: Vec<serde_json::Value> = accounts
                .iter()
                .map(|a| serde_json::json!({ "email": a.email, "scopes": a.scopes }))
                .collect();
            Json(serde_json::json!({ "accounts": items })).into_response()
        }
        Err(e) => {
            log::warn!("failed to list Google accounts for {slug}: {e}");
            Json(serde_json::json!({ "accounts": [] })).into_response()
        }
    }
}

/// GET /api/instances/:slug/google/connect — return OAuth URL
async fn google_connect_url(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let config = state.config.read().await;
    let auth_token = &config.auth_token;

    let landing_url = &config.landing_url;
    if landing_url.is_empty() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "error": "landing_url not configured in config.toml" })),
        )
            .into_response();
    }

    // Build the redirect URL back to the client settings page
    let host = headers
        .get("host")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("localhost");
    let proto = headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("http");
    let client_redirect = format!("{proto}://{host}/{slug}/settings");

    // Build the connect URL that points to the landing OAuth flow
    let connect_url = format!(
        "{}/dashboard/connect-google?token={}&instance={}&redirect={}",
        landing_url.trim_end_matches('/'),
        auth_token,
        slug,
        client_redirect,
    );

    Json(serde_json::json!({ "url": connect_url })).into_response()
}

/// DELETE /api/instances/:slug/google/accounts/:email — disconnect a Google account
async fn disconnect_google_account(
    State(state): State<AppState>,
    Path((slug, email)): Path<(String, String)>,
) -> impl IntoResponse {
    let (auth_token, landing_url) = {
        let config = state.config.read().await;
        (config.auth_token.clone(), config.landing_url.clone())
    };

    if landing_url.is_empty() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "error": "landing_url not configured in config.toml" })),
        )
            .into_response();
    }

    // Call the landing disconnect endpoint
    let client = reqwest::Client::new();
    let res = client
        .post(format!(
            "{}/dashboard/disconnect-google",
            landing_url.trim_end_matches('/')
        ))
        .json(&serde_json::json!({
            "token": auth_token,
            "instance": slug,
            "email": email,
        }))
        .send()
        .await;

    match res {
        Ok(r) if r.status().is_success() => Json(serde_json::json!({ "ok": true })).into_response(),
        Ok(r) => {
            let status = r.status();
            let body = r.text().await.unwrap_or_default();
            log::warn!("disconnect-google failed: {status} {body}");
            (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({ "error": format!("landing returned {status}") })),
            )
                .into_response()
        }
        Err(e) => {
            log::warn!("disconnect-google request failed: {e}");
            (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({ "error": "failed to reach landing" })),
            )
                .into_response()
        }
    }
}
