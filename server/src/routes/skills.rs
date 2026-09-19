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
    let remote = skills::fetch_registry(&config.registry_url)
        .await
        .map_err(|e| {
            log::warn!("failed to fetch skills registry: {e}");
            StatusCode::BAD_GATEWAY
        })?;
    let entries = skills::merge_registry_entries(remote);

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
    // Official bundled skills install without consulting the network.
    let remote = if req.id == "gog" {
        Vec::new()
    } else {
        let registry_url = state.config.read().await.registry_url.clone();
        skills::fetch_registry(&registry_url).await.map_err(|e| {
            log::warn!("failed to fetch registry for install: {e}");
            StatusCode::BAD_GATEWAY
        })?
    };
    let entries = skills::merge_registry_entries(remote);

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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;

    async fn offline_state(workspace: &std::path::Path) -> AppState {
        let config = crate::config::Config {
            registry_url: "http://127.0.0.1:9/registry.json".into(),
            ..Default::default()
        };
        let mut state = AppState::new(config).await;
        state.workspace_dir = workspace.to_path_buf();
        state
    }

    #[tokio::test]
    async fn registry_get_returns_bad_gateway_during_outage() {
        let workspace = tempfile::tempdir().unwrap();
        let response = router()
            .with_state(offline_state(workspace.path()).await)
            .oneshot(
                Request::builder()
                    .uri("/api/skills/registry")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    }

    #[tokio::test]
    async fn only_bundled_gog_installs_during_registry_outage() {
        let workspace = tempfile::tempdir().unwrap();
        let state = offline_state(workspace.path()).await;
        let request = |id: &str| {
            Request::builder()
                .method("POST")
                .uri("/api/skills/registry/install")
                .header("content-type", "application/json")
                .body(Body::from(format!(r#"{{"id":"{id}"}}"#)))
                .unwrap()
        };

        let non_gog = router()
            .with_state(state.clone())
            .oneshot(request("other"))
            .await
            .unwrap();
        assert_eq!(non_gog.status(), StatusCode::BAD_GATEWAY);

        let gog = router()
            .with_state(state)
            .oneshot(request("gog"))
            .await
            .unwrap();
        assert_eq!(gog.status(), StatusCode::OK);
        assert_eq!(
            std::fs::read(workspace.path().join("skills/gog/SKILL.md")).unwrap(),
            include_bytes!("../../official-skills/gog/SKILL.md")
        );
    }
}
