use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post, put},
};
use serde::{Deserialize, Serialize};
use std::fs;

use crate::app::state::AppState;
use crate::domain::child_agent::ChildAgentConfig;
use crate::services::child_agents;

fn reject_reserved_agent(name: &str) -> Result<(), (StatusCode, String)> {
    if child_agents::is_reserved_agent_name(name) {
        Err((StatusCode::NOT_FOUND, format!("agent '{name}' not found")))
    } else {
        Ok(())
    }
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/instances/{instance_slug}/agents", get(list_agents))
        .route(
            "/api/instances/{instance_slug}/agents/{agent_name}",
            put(update_agent),
        )
        .route(
            "/api/instances/{instance_slug}/agents/{agent_name}/run",
            post(trigger_agent),
        )
        .route(
            "/api/instances/{instance_slug}/agents/{agent_name}/reset",
            post(reset_agent),
        )
        .route(
            "/api/instances/{instance_slug}/agents/{agent_name}/history",
            get(agent_history),
        )
        .route("/api/instances/{instance_slug}/agent-runs", get(list_runs))
        .route(
            "/api/instances/{instance_slug}/agent-runs/{run_id}",
            get(get_run),
        )
}

#[derive(Serialize)]
struct AgentInfo {
    #[serde(flatten)]
    config: ChildAgentConfig,
    /// Unix timestamp of last run (0 if never).
    last_run: i64,
    /// Whether the agent is currently due to run.
    is_due: bool,
    /// Whether this is a built-in agent.
    is_builtin: bool,
    /// Fields that differ from the built-in default (empty if not built-in or identical).
    modified_fields: Vec<String>,
}

async fn list_agents(
    State(state): State<AppState>,
    Path(instance_slug): Path<String>,
) -> Result<Json<Vec<AgentInfo>>, (StatusCode, String)> {
    let agents_dir = state
        .workspace_dir
        .join("instances")
        .join(&instance_slug)
        .join("agents");

    child_agents::ensure_builtins(&state.workspace_dir, &instance_slug);

    let mut result = Vec::new();
    for config in child_agents::load_agents(&state.workspace_dir, &instance_slug) {
        let marker_path = agents_dir.join(format!(".last_run_{}", config.name));
        let last_run: i64 = fs::read_to_string(&marker_path)
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0);

        let now = chrono::Utc::now().timestamp();
        let interval_secs = (config.interval_hours * 3600.0) as i64;
        let is_due = now - last_run >= interval_secs;

        let builtin = child_agents::get_builtin_default(&config.name);
        let is_builtin = builtin.is_some();
        let modified_fields = if let Some(ref def) = builtin {
            let mut mods = Vec::new();
            if config.description != def.description {
                mods.push("description".into());
            }
            if config.prompt != def.prompt {
                mods.push("prompt".into());
            }
            if (config.interval_hours - def.interval_hours).abs() > 0.001 {
                mods.push("interval_hours".into());
            }
            if config.model != def.model {
                mods.push("model".into());
            }
            if config.enabled != def.enabled {
                mods.push("enabled".into());
            }
            if config.tool_groups != def.tool_groups {
                mods.push("tool_groups".into());
            }
            mods
        } else {
            vec![]
        };

        result.push(AgentInfo {
            config,
            last_run,
            is_due,
            is_builtin,
            modified_fields,
        });
    }

    result.sort_by(|a, b| a.config.name.cmp(&b.config.name));
    Ok(Json(result))
}

async fn trigger_agent(
    State(state): State<AppState>,
    Path((instance_slug, agent_name)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, super::ProviderRequestError> {
    reject_reserved_agent(&agent_name)?;
    super::require_provider(&state).await?;
    let agents_dir = state
        .workspace_dir
        .join("instances")
        .join(&instance_slug)
        .join("agents");

    let config_path = agents_dir.join(format!("{agent_name}.toml"));
    let content = fs::read_to_string(&config_path).map_err(|_| {
        (
            StatusCode::NOT_FOUND,
            format!("agent '{agent_name}' not found"),
        )
    })?;
    let agent: ChildAgentConfig =
        toml::from_str(&content).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    let instance_dir = state.workspace_dir.join("instances").join(&instance_slug);

    let llm_guard = state.llm.read().await;
    let llm = llm_guard.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "LLM not configured".to_string(),
        )
    })?;

    let google_ai_key = {
        let cfg = state.config.read().await;
        cfg.llm.tokens.google_ai.clone()
    };

    // Run the agent in background
    let ws = state.workspace_dir.clone();
    let slug = instance_slug.clone();
    let events = state.events.clone();
    let vs = state.vector_store.clone();
    let llm_clone = llm.clone();

    tokio::spawn(async move {
        match child_agents::run_single_agent(
            &ws,
            &slug,
            &instance_dir,
            &llm_clone,
            &events,
            &vs,
            &google_ai_key,
            &agent,
            None,
            "manual",
            None,
        )
        .await
        {
            Ok(r) => {
                log::info!(
                    "[agents-api] {slug}: manually triggered '{}' ({} tokens, {})",
                    agent.name,
                    r.tokens,
                    r.run_id
                );
            }
            Err(e) => {
                log::warn!(
                    "[agents-api] {slug}: manual trigger '{}' failed: {e}",
                    agent.name
                );
            }
        }
    });

    Ok(Json(
        serde_json::json!({"status": "triggered", "agent": agent_name}),
    ))
}

#[derive(Serialize)]
struct HistoryEntry {
    content: String,
    timestamp: String,
    id: String,
}

async fn agent_history(
    State(state): State<AppState>,
    Path((instance_slug, agent_name)): Path<(String, String)>,
) -> Result<Json<Vec<HistoryEntry>>, (StatusCode, String)> {
    reject_reserved_agent(&agent_name)?;
    let history_path = state
        .workspace_dir
        .join("instances")
        .join(&instance_slug)
        .join("agents")
        .join(format!("{agent_name}_history.json"));

    let entries = crate::services::chat::load_rig_history(&history_path).unwrap_or_default();

    let result: Vec<HistoryEntry> = entries
        .iter()
        .rev()
        .take(20)
        .map(|e| {
            let content = match &e.message {
                crate::services::llm::Message::Assistant { content, .. } => content
                    .iter()
                    .filter_map(|b| {
                        if let crate::services::llm::ContentBlock::Text { text } = b {
                            Some(text.as_str())
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" "),
                crate::services::llm::Message::User { content } => content
                    .iter()
                    .filter_map(|b| {
                        if let crate::services::llm::ContentBlock::Text { text } = b {
                            Some(text.as_str())
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" "),
            };
            HistoryEntry {
                content,
                timestamp: e.ts.clone().unwrap_or_default(),
                id: e.id.clone().unwrap_or_default(),
            }
        })
        .collect();

    Ok(Json(result))
}

// ── Agent Runs ──────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct ListRunsParams {
    #[serde(default = "default_limit")]
    limit: usize,
    agent_name: Option<String>,
}

fn default_limit() -> usize {
    50
}

async fn list_runs(
    State(state): State<AppState>,
    Path(instance_slug): Path<String>,
    Query(params): Query<ListRunsParams>,
) -> Result<Json<Vec<crate::domain::agent_run::AgentRunSummary>>, (StatusCode, String)> {
    let mut runs = crate::services::agent_runs::list_runs(
        &state.workspace_dir,
        &instance_slug,
        params.limit,
        params.agent_name.as_deref(),
    )
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    runs.retain(|run| !child_agents::is_reserved_agent_name(&run.agent_name));
    Ok(Json(runs))
}

async fn get_run(
    State(state): State<AppState>,
    Path((instance_slug, run_id)): Path<(String, String)>,
) -> Result<Json<crate::domain::agent_run::AgentRun>, (StatusCode, String)> {
    let run = crate::services::agent_runs::load_run(&state.workspace_dir, &instance_slug, &run_id)
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    reject_reserved_agent(&run.agent_name)?;
    Ok(Json(run))
}

/// Update an agent's config (partial — only provided fields change).
#[derive(Deserialize)]
struct UpdateAgentRequest {
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    interval_hours: Option<f64>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    tool_groups: Option<Vec<String>>,
}

async fn update_agent(
    State(state): State<AppState>,
    Path((instance_slug, agent_name)): Path<(String, String)>,
    Json(req): Json<UpdateAgentRequest>,
) -> Result<Json<ChildAgentConfig>, (StatusCode, String)> {
    reject_reserved_agent(&agent_name)?;
    let agent_path = state
        .workspace_dir
        .join("instances")
        .join(&instance_slug)
        .join("agents")
        .join(format!("{agent_name}.toml"));

    let raw = fs::read_to_string(&agent_path).map_err(|_| {
        (
            StatusCode::NOT_FOUND,
            format!("agent '{agent_name}' not found"),
        )
    })?;
    let mut agent: ChildAgentConfig =
        toml::from_str(&raw).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    if let Some(v) = req.description {
        agent.description = v;
    }
    if let Some(v) = req.prompt {
        agent.prompt = v;
    }
    if let Some(v) = req.interval_hours {
        agent.interval_hours = v;
    }
    if let Some(v) = req.model {
        agent.model = v;
    }
    if let Some(v) = req.enabled {
        agent.enabled = v;
    }
    if let Some(v) = req.tool_groups {
        agent.tool_groups = v;
    }

    let toml_str = toml::to_string_pretty(&agent)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    fs::write(&agent_path, toml_str)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    log::info!("[agents-api] updated agent '{agent_name}' for {instance_slug}");
    Ok(Json(agent))
}

async fn reset_agent(
    State(state): State<AppState>,
    Path((instance_slug, agent_name)): Path<(String, String)>,
) -> Result<Json<ChildAgentConfig>, (StatusCode, String)> {
    reject_reserved_agent(&agent_name)?;
    let default = child_agents::get_builtin_default(&agent_name).ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            format!("'{agent_name}' is not a built-in agent"),
        )
    })?;

    let agent_path = state
        .workspace_dir
        .join("instances")
        .join(&instance_slug)
        .join("agents")
        .join(format!("{agent_name}.toml"));

    let toml_str = toml::to_string_pretty(&default)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    fs::write(&agent_path, toml_str)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    log::info!("[agents-api] reset agent '{agent_name}' to defaults for {instance_slug}");
    Ok(Json(default))
}
