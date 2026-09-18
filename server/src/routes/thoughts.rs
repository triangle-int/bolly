use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};

use crate::{app::state::AppState, domain::thought::Thought, services::thoughts};

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/instances/{instance_slug}/thoughts",
            get(list_thoughts),
        )
        .route(
            "/api/instances/{instance_slug}/observations",
            get(list_observations),
        )
}

async fn list_thoughts(
    State(state): State<AppState>,
    Path(instance_slug): Path<String>,
) -> Result<Json<Vec<Thought>>, (StatusCode, String)> {
    let items = thoughts::list_thoughts(&state.workspace_dir, &instance_slug)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(items))
}

async fn list_observations(
    State(state): State<AppState>,
    Path(instance_slug): Path<String>,
) -> Json<Vec<crate::services::tools::screen::ScreenObservation>> {
    Json(crate::services::tools::screen::list_observations(
        &state.workspace_dir,
        &instance_slug,
    ))
}
