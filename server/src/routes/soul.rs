use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post, put},
};

use crate::{
    app::state::AppState,
    domain::soul::{ApplyTemplateRequest, Soul, SoulTemplate, UpdateSoulRequest},
    services::soul,
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/instances/{instance_slug}/soul", get(get_soul))
        .route("/api/instances/{instance_slug}/soul", put(put_soul))
        .route(
            "/api/instances/{instance_slug}/soul/apply-template",
            post(apply_template),
        )
        .route("/api/soul/templates", get(get_templates))
}

async fn get_soul(State(state): State<AppState>, Path(instance_slug): Path<String>) -> Json<Soul> {
    Json(soul::read_soul(&state.workspace_dir, &instance_slug))
}

async fn put_soul(
    State(state): State<AppState>,
    Path(instance_slug): Path<String>,
    Json(request): Json<UpdateSoulRequest>,
) -> Result<Json<Soul>, (StatusCode, String)> {
    soul::write_soul(&state.workspace_dir, &instance_slug, &request.content)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(Soul {
        content: request.content,
        exists: true,
    }))
}

async fn apply_template(
    State(state): State<AppState>,
    Path(instance_slug): Path<String>,
    Json(request): Json<ApplyTemplateRequest>,
) -> Result<Json<Soul>, (StatusCode, String)> {
    let template = soul::find_template(&request.template_id).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            format!("template '{}' not found", request.template_id),
        )
    })?;

    soul::write_soul(&state.workspace_dir, &instance_slug, &template.content)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(Soul {
        content: template.content,
        exists: true,
    }))
}

async fn get_templates() -> Json<Vec<SoulTemplate>> {
    Json(soul::templates())
}
