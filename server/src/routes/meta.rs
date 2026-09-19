use axum::{Json, Router, extract::State, routing::get};

use crate::{
    app::state::AppState,
    domain::{
        companion::CANONICAL_SLUG,
        meta::{LlmSummary, ServerMetaResponse},
    },
    services::{companion, workspace},
};

pub fn router() -> Router<AppState> {
    Router::new().route("/api/meta", get(server_meta))
}

async fn server_meta(State(state): State<AppState>) -> Json<ServerMetaResponse> {
    let skills_dir = state.workspace_dir.join("skills");
    let cfg = state.config.read().await;
    let instances_count = match companion::read_identity(&state.workspace_dir) {
        Ok(Some(_)) => 1,
        Ok(None) => 0,
        Err(error) => {
            log::warn!("companion identity is unavailable: {error}");
            0
        }
    };

    Json(ServerMetaResponse {
        app: "nolune",
        version: env!("CARGO_PKG_VERSION"),
        commit: option_env!("GIT_HASH").unwrap_or("dev"),
        port: cfg.port,
        workspace_dir: state.workspace_dir.display().to_string(),
        companion_slug: CANONICAL_SLUG,
        instances_count,
        skills_count: workspace::count_directories(&skills_dir).unwrap_or(0),
        llm: LlmSummary {
            provider: cfg.llm.provider,
            setup_required: cfg.llm.setup_required(),
            model: (cfg.llm.provider != crate::config::LlmProvider::Codex)
                .then(|| cfg.llm.model_name().to_string()),
            configured: cfg.llm.is_configured(),
        },
    })
}
