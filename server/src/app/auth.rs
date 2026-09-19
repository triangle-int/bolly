use axum::{
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::Response,
};

use super::state::AppState;

/// Auth middleware: if `auth_token` is configured, require Bearer token on all /api/* requests.
/// WebSocket connections can pass the token as `?token=<value>` query parameter.
pub async fn auth_middleware(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let expected = {
        let config = state.config.read().await;
        config.auth_token.clone()
    };

    // No auth configured — pass through
    if expected.is_empty() {
        return Ok(next.run(request).await);
    }

    // Check Authorization: Bearer <token>
    let bearer = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|s| s.to_string());

    // Check ?token=<value> query param (for WebSocket connections)
    let query_token = request.uri().query().and_then(|q| {
        q.split('&')
            .find_map(|p| p.strip_prefix("token="))
            .map(|s| s.to_string())
    });

    let provided = bearer.or(query_token);

    if provided.as_deref() == Some(&expected) {
        Ok(next.run(request).await)
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}
