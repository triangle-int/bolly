//! Browser pairing and session management (#112).
//!
//! * `POST /api/session/pair` (public) redeems a pairing code for a cookie.
//! * `POST /api/session/pairing` (authenticated) mints a pairing code; the
//!   CLI and desktop app call it with the API token, a paired browser calls
//!   it from Settings.
//! * `GET /api/session`, `POST /api/session/logout` and the
//!   `/api/session/devices` routes let owners see and revoke paired browsers.

use axum::{
    Extension, Json, Router,
    extract::{Path, Request, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
};
use serde::Deserialize;
use serde_json::json;

use crate::{
    app::{
        auth::{self, AuthContext},
        state::AppState,
    },
    services::browser_sessions::{self, PairingError},
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/session", get(current_session))
        .route("/api/session/logout", post(logout))
        .route("/api/session/pairing", post(create_pairing_code))
        .route(
            "/api/session/devices",
            get(list_devices).delete(revoke_all_devices),
        )
        .route("/api/session/devices/{id}", delete(revoke_device))
}

/// Mounted outside the auth middleware: a browser has no credential yet.
pub fn public_router() -> Router<AppState> {
    Router::new().route("/api/session/pair", post(pair))
}

fn auth_kind(context: Option<&AuthContext>) -> &'static str {
    match context {
        None | Some(AuthContext::Disabled) => "disabled",
        Some(AuthContext::ApiToken) => "token",
        Some(AuthContext::BrowserSession { .. }) => "session",
    }
}

fn current_session_id(context: Option<&AuthContext>) -> Option<&str> {
    match context {
        Some(AuthContext::BrowserSession { id }) => Some(id),
        _ => None,
    }
}

async fn current_session(
    State(state): State<AppState>,
    context: Option<Extension<AuthContext>>,
) -> Json<serde_json::Value> {
    let context = context.map(|Extension(c)| c);
    let session = current_session_id(context.as_ref()).and_then(|id| {
        state
            .browser_sessions
            .list()
            .into_iter()
            .find(|s| s.id == id)
    });
    Json(json!({
        "auth": auth_kind(context.as_ref()),
        "session": session,
    }))
}

async fn logout(
    State(state): State<AppState>,
    headers: HeaderMap,
    context: Option<Extension<AuthContext>>,
) -> Response {
    let context = context.map(|Extension(c)| c);
    if let Some(id) = current_session_id(context.as_ref()) {
        state.browser_sessions.revoke(id);
    }
    let secure = auth::secure_cookie(&headers, &*state.config.read().await);
    (
        StatusCode::OK,
        [(header::SET_COOKIE, auth::clear_session_cookie(secure))],
        Json(json!({ "status": "ok" })),
    )
        .into_response()
}

#[derive(Deserialize, Default)]
struct CreatePairingRequest {
    /// Free-form origin of the request when made with the API token, for the
    /// device list ("cli", "desktop"). Ignored for browser sessions.
    #[serde(default)]
    source: Option<String>,
}

fn sanitize_source(source: Option<String>) -> String {
    source
        .map(|s| {
            s.chars()
                .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
                .take(32)
                .collect::<String>()
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "api-token".to_string())
}

async fn create_pairing_code(
    State(state): State<AppState>,
    context: Option<Extension<AuthContext>>,
    request: Request,
) -> Result<Json<browser_sessions::PairingChallenge>, (StatusCode, Json<serde_json::Value>)> {
    let context = context.map(|Extension(c)| c);
    let (bound_host, created_by) = match context {
        Some(AuthContext::BrowserSession { id }) => {
            let host = auth::request_host(request.headers(), &request).ok_or_else(|| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(json!({ "error": "missing_host" })),
                )
            })?;
            (Some(host), format!("browser:{id}"))
        }
        Some(AuthContext::ApiToken) => {
            let body: CreatePairingRequest =
                match axum::body::to_bytes(request.into_body(), 4096).await {
                    Ok(bytes) if bytes.is_empty() => CreatePairingRequest::default(),
                    Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
                    Err(_) => CreatePairingRequest::default(),
                };
            (None, sanitize_source(body.source))
        }
        None | Some(AuthContext::Disabled) => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": "auth_disabled",
                    "message": "authentication is disabled; browsers do not need pairing",
                })),
            ));
        }
    };
    Ok(Json(
        state
            .browser_sessions
            .create_challenge(bound_host, &created_by),
    ))
}

#[derive(Deserialize)]
struct PairRequest {
    code: String,
}

async fn pair(State(state): State<AppState>, request: Request) -> Response {
    let (secure, enabled) = {
        let config = state.config.read().await;
        (
            auth::secure_cookie(request.headers(), &config),
            !config.auth_token.is_empty(),
        )
    };
    if !enabled {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "auth_disabled" })),
        )
            .into_response();
    }
    let Some(host) = auth::request_host(request.headers(), &request) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "missing_host" })),
        )
            .into_response();
    };
    if !auth::same_origin(request.headers(), &host) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({ "error": "cross_origin" })),
        )
            .into_response();
    }
    let label = request
        .headers()
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(browser_sessions::describe_user_agent)
        .unwrap_or_else(|| "Browser".to_string());
    let body: PairRequest = match axum::body::to_bytes(request.into_body(), 4096).await {
        Ok(bytes) => match serde_json::from_slice(&bytes) {
            Ok(body) => body,
            Err(_) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({ "error": "invalid_body" })),
                )
                    .into_response();
            }
        },
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "invalid_body" })),
            )
                .into_response();
        }
    };

    match state
        .browser_sessions
        .confirm_challenge(&body.code, &host, &label)
    {
        Ok(issued) => (
            StatusCode::OK,
            [(
                header::SET_COOKIE,
                auth::session_cookie(&issued.cookie_value, issued.max_age_secs, secure),
            )],
            Json(json!({ "session": issued.summary })),
        )
            .into_response(),
        Err(PairingError::Invalid) => (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "invalid_code" })),
        )
            .into_response(),
        Err(PairingError::RateLimited) => (
            StatusCode::TOO_MANY_REQUESTS,
            [(
                header::RETRY_AFTER,
                browser_sessions::FAILURE_WINDOW_SECS.to_string(),
            )],
            Json(json!({ "error": "rate_limited" })),
        )
            .into_response(),
    }
}

async fn list_devices(
    State(state): State<AppState>,
    context: Option<Extension<AuthContext>>,
) -> Json<serde_json::Value> {
    let context = context.map(|Extension(c)| c);
    let current = current_session_id(context.as_ref()).map(str::to_string);
    let devices: Vec<serde_json::Value> = state
        .browser_sessions
        .list()
        .into_iter()
        .map(|summary| {
            let current = current.as_deref() == Some(summary.id.as_str());
            let mut value = serde_json::to_value(summary).unwrap_or_default();
            value["current"] = json!(current);
            value
        })
        .collect();
    Json(json!({
        "auth": auth_kind(context.as_ref()),
        "devices": devices,
    }))
}

async fn revoke_device(
    State(state): State<AppState>,
    headers: HeaderMap,
    context: Option<Extension<AuthContext>>,
    Path(id): Path<String>,
) -> Response {
    if !state.browser_sessions.revoke(&id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let context = context.map(|Extension(c)| c);
    if current_session_id(context.as_ref()) == Some(id.as_str()) {
        let secure = auth::secure_cookie(&headers, &*state.config.read().await);
        return (
            StatusCode::NO_CONTENT,
            [(header::SET_COOKIE, auth::clear_session_cookie(secure))],
        )
            .into_response();
    }
    StatusCode::NO_CONTENT.into_response()
}

async fn revoke_all_devices(
    State(state): State<AppState>,
    headers: HeaderMap,
    context: Option<Extension<AuthContext>>,
) -> Response {
    let revoked = state.browser_sessions.revoke_all();
    let context = context.map(|Extension(c)| c);
    let body = Json(json!({ "revoked": revoked }));
    if current_session_id(context.as_ref()).is_some() {
        let secure = auth::secure_cookie(&headers, &*state.config.read().await);
        return (
            StatusCode::OK,
            [(header::SET_COOKIE, auth::clear_session_cookie(secure))],
            body,
        )
            .into_response();
    }
    (StatusCode::OK, body).into_response()
}
