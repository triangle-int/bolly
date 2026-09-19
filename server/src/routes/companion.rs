use axum::{Json, Router, extract::State, routing::get};

use crate::{
    app::{companion_boundary::CompanionRejection, state::AppState},
    domain::companion::CompanionContext,
    services::companion,
};

/// The one companion this server owns. Replaces the multi-instance listing.
pub fn router() -> Router<AppState> {
    Router::new().route("/api/companion", get(get_companion))
}

async fn get_companion(
    State(state): State<AppState>,
) -> Result<Json<CompanionContext>, CompanionRejection> {
    companion::context(&state.workspace_dir)
        .map(Json)
        .map_err(CompanionRejection::from)
}
