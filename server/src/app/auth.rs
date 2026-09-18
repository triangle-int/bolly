use axum::{
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::Response,
};

use super::state::AppState;

/// Decode percent-encoded cookie values (e.g. `%3D` → `=`).
fn percent_decode(input: &str) -> String {
    let mut out = Vec::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(&input[i + 1..i + 3], 16) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| input.to_string())
}

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

    // Check bolly_token cookie (for PWA standalone mode where localStorage is isolated)
    let cookie_token = request
        .headers()
        .get("cookie")
        .and_then(|v| v.to_str().ok())
        .and_then(|cookies| {
            cookies.split(';').find_map(|c| {
                let c = c.trim();
                c.strip_prefix("bolly_token=").map(percent_decode)
            })
        });

    let provided = bearer.or(query_token).or(cookie_token);

    if provided.as_deref() == Some(&expected) {
        Ok(next.run(request).await)
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}
