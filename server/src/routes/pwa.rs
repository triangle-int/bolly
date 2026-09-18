use axum::{Json, Router, extract::State, http::header, response::IntoResponse, routing::get};
use serde_json::json;

use crate::app::state::AppState;

/// Serve manifest.webmanifest with `start_url` set to `/auth?token=<token>`
/// so that standalone PWAs (which have isolated localStorage) can authenticate
/// on launch.
async fn manifest(State(state): State<AppState>) -> impl IntoResponse {
    let config = state.config.read().await;
    let start_url = if config.auth_token.is_empty() {
        "/".to_string()
    } else {
        format!("/auth?token={}", urlencoding_encode(&config.auth_token))
    };

    let manifest = json!({
        "name": "Nolune",
        "short_name": "Nolune",
        "description": "Your AI companion",
        "start_url": start_url,
        "display": "standalone",
        "background_color": "#0d0b09",
        "theme_color": "#0d0b09",
        "lang": "en",
        "scope": "/",
        "icons": [
            { "src": "pwa-192x192.png", "sizes": "192x192", "type": "image/png" },
            { "src": "pwa-512x512.png", "sizes": "512x512", "type": "image/png" },
            { "src": "pwa-512x512.png", "sizes": "512x512", "type": "image/png", "purpose": "maskable" }
        ]
    });

    (
        [(header::CONTENT_TYPE, "application/manifest+json")],
        Json(manifest),
    )
}

/// Percent-encode a string for use in URLs.
fn urlencoding_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len() * 3);
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => {
                out.push('%');
                out.push_str(&format!("{byte:02X}"));
            }
        }
    }
    out
}

pub fn router() -> Router<AppState> {
    Router::new().route("/manifest.webmanifest", get(manifest))
}
