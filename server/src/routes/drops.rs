use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};

use crate::{app::state::AppState, domain::drop::Drop, services::drops};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/instances/{instance_slug}/drops", get(list_drops))
        .route(
            "/api/instances/{instance_slug}/drops/{drop_id}",
            get(get_drop).delete(delete_drop),
        )
}

async fn list_drops(
    State(state): State<AppState>,
    Path(instance_slug): Path<String>,
) -> Result<Json<Vec<Drop>>, (StatusCode, String)> {
    let items = drops::list_drops(&state.workspace_dir, &instance_slug)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(items))
}

async fn get_drop(
    State(state): State<AppState>,
    Path((instance_slug, drop_id)): Path<(String, String)>,
) -> Result<Json<Drop>, StatusCode> {
    match drops::get_drop(&state.workspace_dir, &instance_slug, &drop_id) {
        Ok(Some(drop)) => Ok(Json(drop)),
        Ok(None) => Err(StatusCode::NOT_FOUND),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn delete_drop(
    State(state): State<AppState>,
    Path((instance_slug, drop_id)): Path<(String, String)>,
) -> StatusCode {
    match drops::delete_drop(&state.workspace_dir, &instance_slug, &drop_id) {
        Ok(true) => StatusCode::OK,
        Ok(false) => StatusCode::NOT_FOUND,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}
