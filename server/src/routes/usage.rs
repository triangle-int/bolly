use crate::app::state::AppState;
use axum::{Json, Router, routing::get};

pub fn router() -> Router<AppState> {
    Router::new().route("/api/usage", get(get_usage))
}

/// Rate limits removed — BYOK has no limits. Returns zeros for backwards compat.
async fn get_usage() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "tokens_last_4h": 0,
        "tokens_4h_limit": 0,
        "tokens_this_week": 0,
        "tokens_week_limit": 0,
        "tokens_this_month": 0,
        "tokens_month_limit": 0,
        "resets_at": null,
    }))
}
