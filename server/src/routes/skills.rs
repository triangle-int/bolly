use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get, post},
};
use serde::{Deserialize, Serialize};

use crate::{
    app::state::AppState,
    domain::skill::{RegistryEntry, Skill},
    services::skills,
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/skills", get(list_skills))
        .route("/api/skills", post(create_skill))
        .route("/api/skills/registry", get(list_registry))
        .route("/api/skills/registry/install", post(install_registry_skill))
        .route("/api/skills/{skill_id}", get(get_skill))
        .route("/api/skills/{skill_id}", delete(delete_skill))
}

async fn list_skills(State(state): State<AppState>) -> Json<Vec<Skill>> {
    let all = skills::list_skills(&state.workspace_dir);
    Json(all)
}

async fn get_skill(
    State(state): State<AppState>,
    Path(skill_id): Path<String>,
) -> Result<Json<Skill>, StatusCode> {
    skills::get_skill(&state.workspace_dir, &skill_id)
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn create_skill(
    State(state): State<AppState>,
    Json(skill): Json<Skill>,
) -> Result<Json<Skill>, StatusCode> {
    skills::create_skill(&state.workspace_dir, &skill)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(skill))
}

async fn delete_skill(State(state): State<AppState>, Path(skill_id): Path<String>) -> StatusCode {
    match skills::delete_skill(&state.workspace_dir, &skill_id) {
        Ok(true) => StatusCode::NO_CONTENT,
        Ok(false) => StatusCode::NOT_FOUND,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct RegistryEntryWithStatus {
    #[serde(flatten)]
    entry: RegistryEntry,
    installed: bool,
}

async fn list_registry(
    State(state): State<AppState>,
) -> Result<Json<Vec<RegistryEntryWithStatus>>, StatusCode> {
    let config = state.config.read().await;
    let entries = skills::fetch_registry(&config.registry_url)
        .await
        .map_err(|e| {
            log::warn!("failed to fetch skills registry: {e}");
            StatusCode::BAD_GATEWAY
        })?;

    let annotated = entries
        .into_iter()
        .map(|e| {
            let installed = skills::is_installed(&state.workspace_dir, &e.id);
            RegistryEntryWithStatus {
                entry: e,
                installed,
            }
        })
        .collect();

    Ok(Json(annotated))
}

#[derive(Deserialize)]
struct InstallRequest {
    id: String,
}

async fn install_registry_skill(
    State(state): State<AppState>,
    Json(req): Json<InstallRequest>,
) -> Result<Json<Skill>, StatusCode> {
    let config = state.config.read().await;
    let entries = skills::fetch_registry(&config.registry_url)
        .await
        .map_err(|e| {
            log::warn!("failed to fetch registry for install: {e}");
            StatusCode::BAD_GATEWAY
        })?;

    let entry = entries
        .iter()
        .find(|e| e.id == req.id)
        .ok_or(StatusCode::NOT_FOUND)?;

    let skill = skills::install_from_registry(&state.workspace_dir, entry)
        .await
        .map_err(|e| {
            log::error!("failed to install skill '{}': {e}", req.id);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(skill))
}
