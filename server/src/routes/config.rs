use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    routing::{delete, get, post, put},
};
// Note: `put` still used by update_model_mode
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    app::state::AppState,
    config::{self, McpServerConfig},
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/config/model-mode", put(update_model_mode))
        .route("/api/config/status", get(get_status))
        .route(
            "/api/config/embedding",
            get(get_embedding).put(update_embedding),
        )
        .route("/api/config/llm", put(update_llm_key))
        .route("/api/config/provider", put(update_provider))
        .route("/api/config/mcp", get(list_mcp_servers))
        .route("/api/config/mcp", post(add_mcp_server))
        .route("/api/config/mcp/suggested", get(suggested_mcp_servers))
        .route("/api/config/mcp/{name}", delete(remove_mcp_server))
        .route("/api/config/github", get(get_github))
        .route("/api/config/github", put(update_github))
        .route("/api/config/server", get(get_server))
        .route("/api/config/server", put(update_server))
}

async fn get_status(State(state): State<AppState>) -> Json<serde_json::Value> {
    let config = state.config.read().await;
    let mode = match config.llm.model_mode {
        config::ModelMode::Auto => "auto",
        config::ModelMode::Fast => "fast",
        config::ModelMode::Heavy => "heavy",
    };
    // Which optional keys are configured
    let t = &config.llm.tokens;
    let keys: Vec<&str> = [
        ("anthropic", !t.anthropic.is_empty()),
        ("openai", !t.open_ai.is_empty()),
        ("google_ai", !t.google_ai.is_empty()),
        ("elevenlabs", !t.elevenlabs.is_empty()),
        ("openrouter", !t.open_router.is_empty()),
    ]
    .iter()
    .filter(|(_, configured)| *configured)
    .map(|(name, _)| *name)
    .collect();

    let provider = match config.llm.provider {
        config::LlmProvider::Anthropic => "anthropic",
        config::LlmProvider::Openai => "openai",
        config::LlmProvider::Codex => "codex",
    };

    Json(json!({
        "embedding": embedding_status(&state, &config),
        "llm_configured": config.llm.is_configured(),
        "provider": provider,
        "setup_required": config.llm.setup_required(),
        "capabilities": crate::services::llm::provider_capabilities(config.llm.provider),
        "model": (config.llm.provider != config::LlmProvider::Codex).then(|| config.llm.model_name()),
        "fast_model": (config.llm.provider != config::LlmProvider::Codex).then(|| config.llm.fast_model_name()),
        "model_mode": mode,
        "configured_keys": keys,
        "host": config.host,
        "port": config.port,
        "auth_token_set": !config.auth_token.is_empty(),
        "is_managed": !config.landing_url.is_empty() || std::env::var("FLY_APP_NAME").is_ok(),
    }))
}

/// Report the active vector space separately from pending persisted settings.
fn embedding_status(state: &AppState, config: &config::Config) -> serde_json::Value {
    let mut status = state.vector_store.embedding_status();
    let needs_restart = state.vector_store.embedding_needs_restart(config);
    status["needs_restart"] = json!(needs_restart);
    status["pending"] = if needs_restart {
        config.embedding_status()
    } else {
        serde_json::Value::Null
    };
    status
}

async fn get_embedding(State(state): State<AppState>) -> Json<serde_json::Value> {
    let config = state.config.read().await;
    Json(embedding_status(&state, &config))
}

async fn update_embedding(
    State(state): State<AppState>,
    Json(settings): Json<config::EmbeddingConfig>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    settings
        .validate()
        .map_err(|error| (StatusCode::BAD_REQUEST, error.to_owned()))?;
    let mut cfg = state.config.write().await;
    let mut next = cfg.clone();
    next.embedding = settings;
    save_config_at(&next, &state.workspace_dir.join("config.toml"))?;
    *cfg = next;
    Ok(Json(embedding_status(&state, &cfg)))
}

// ---------------------------------------------------------------------------
// Model mode
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct UpdateModelModeRequest {
    mode: String,
}

async fn update_model_mode(
    State(state): State<AppState>,
    Json(request): Json<UpdateModelModeRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let mode = match request.mode.to_lowercase().as_str() {
        "auto" => config::ModelMode::Auto,
        "fast" => config::ModelMode::Fast,
        "heavy" => config::ModelMode::Heavy,
        other => return Err((StatusCode::BAD_REQUEST, format!("unknown mode: {other}"))),
    };

    {
        let mut cfg = state.config.write().await;
        cfg.llm.model_mode = mode;
        save_config(&cfg)?;
    }

    Ok(Json(
        json!({ "status": "ok", "model_mode": request.mode.to_lowercase() }),
    ))
}

// ---------------------------------------------------------------------------
// LLM API keys
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct UpdateLlmKeyRequest {
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    openai: Option<String>,
    #[serde(default)]
    google_ai: Option<String>,
    #[serde(default)]
    elevenlabs: Option<String>,
    #[serde(default)]
    openrouter: Option<String>,
}

async fn update_llm_key(
    State(state): State<AppState>,
    Json(req): Json<UpdateLlmKeyRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    // Validate Anthropic key before saving
    if let Some(key) = &req.api_key {
        let key = key.trim();
        if !key.is_empty() {
            let http = reqwest::Client::new();
            let res = http
                .post("https://api.anthropic.com/v1/messages")
                .header("x-api-key", key)
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .body(r#"{"model":"claude-haiku-4-5-20241022","max_tokens":1,"messages":[{"role":"user","content":"hi"}]}"#)
                .send()
                .await
                .map_err(|e| (StatusCode::BAD_GATEWAY, format!("failed to reach Anthropic: {e}")))?;

            if res.status() == reqwest::StatusCode::UNAUTHORIZED {
                return Err((StatusCode::UNAUTHORIZED, "invalid API key".into()));
            }
            // 400 (bad request) is fine — means key is valid but request was malformed (shouldn't happen)
            // 429 (rate limited) is fine — means key is valid
            // 200 is fine — means key works
            if res.status().is_server_error() {
                return Err((
                    StatusCode::BAD_GATEWAY,
                    "Anthropic API error — try again".into(),
                ));
            }
        }
    }

    let mut changes = Vec::new();

    {
        let mut cfg = state.config.write().await;

        if let Some(key) = &req.api_key {
            cfg.llm.tokens.anthropic = key.trim().to_string();
            changes.push("anthropic");
        }
        if let Some(key) = &req.openai {
            cfg.llm.tokens.open_ai = key.trim().to_string();
            changes.push("openai");
        }
        if let Some(key) = &req.google_ai {
            cfg.llm.tokens.google_ai = key.trim().to_string();
            changes.push("google_ai");
        }
        if let Some(key) = &req.elevenlabs {
            cfg.llm.tokens.elevenlabs = key.trim().to_string();
            changes.push("elevenlabs");
        }
        if let Some(key) = &req.openrouter {
            cfg.llm.tokens.open_router = key.trim().to_string();
            changes.push("openrouter");
        }

        save_config(&cfg)?;
    }

    // Rebuild LLM backend after any key change.
    // Can't use reload_config() — it compares disk vs in-memory, but we
    // already updated in-memory above, so it sees no diff.
    if !changes.is_empty() {
        let cfg = state.config.read().await;
        let new_llm = crate::services::llm::LlmBackend::from_config(&cfg);
        drop(cfg);
        *state.llm.write().await = new_llm;
        log::info!("LLM backend rebuilt after API key change");
    }

    let cfg = state.config.read().await;
    Ok(Json(json!({ "status": "ok", "updated": changes,
        "embedding": embedding_status(&state, &cfg),
        "needs_restart": state.vector_store.embedding_needs_restart(&cfg),
    })))
}

// ---------------------------------------------------------------------------
// MCP server management
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct McpServerInfo {
    name: String,
    url: Option<String>,
    connected: bool,
}

async fn list_mcp_servers(State(state): State<AppState>) -> Json<Vec<McpServerInfo>> {
    let config = state.config.read().await;
    let connected_names = state.mcp_registry.server_names().await;
    let servers: Vec<McpServerInfo> = config
        .mcp_servers
        .iter()
        .map(|s| McpServerInfo {
            name: s.name.clone(),
            url: s.url.clone(),
            connected: connected_names.contains(&s.name),
        })
        .collect();
    Json(servers)
}

/// Curated list of popular MCP servers users can add with one click.
async fn suggested_mcp_servers(State(state): State<AppState>) -> Json<serde_json::Value> {
    let config = state.config.read().await;
    let installed: Vec<String> = config.mcp_servers.iter().map(|s| s.name.clone()).collect();

    let suggested = serde_json::json!([
        {
            "name": "fal-ai",
            "description": "AI image & video generation (Flux, SDXL, etc.)",
            "url": "https://mcp.fal.ai/mcp",
            "requires_key": true,
            "key_env": "FAL_KEY",
            "key_url": "https://fal.ai/dashboard/keys",
            "installed": installed.contains(&"fal-ai".to_string()),
        },
        {
            "name": "brave-search",
            "description": "Web search via Brave Search API",
            "url": "https://mcp.bravesearch.com/sse",
            "requires_key": true,
            "key_env": "BRAVE_API_KEY",
            "key_url": "https://brave.com/search/api/",
            "installed": installed.contains(&"brave-search".to_string()),
        },
        {
            "name": "github",
            "description": "GitHub repos, issues, PRs, code search",
            "url": "https://api.githubcopilot.com/mcp/",
            "requires_key": true,
            "key_env": "GITHUB_TOKEN",
            "key_url": "https://github.com/settings/tokens",
            "installed": installed.contains(&"github".to_string()),
        },
        {
            "name": "firecrawl",
            "description": "Web scraping and crawling",
            "url": "https://mcp.firecrawl.dev/sse",
            "requires_key": true,
            "key_env": "FIRECRAWL_API_KEY",
            "key_url": "https://firecrawl.dev",
            "installed": installed.contains(&"firecrawl".to_string()),
        },
    ]);

    Json(suggested)
}

#[derive(Deserialize)]
struct AddMcpServerRequest {
    name: String,
    url: String,
}

async fn add_mcp_server(
    State(state): State<AppState>,
    Json(request): Json<AddMcpServerRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let name = request.name.trim().to_string();
    let url = request.url.trim().to_string();
    if name.is_empty() || url.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "name and url are required".into()));
    }

    // Update config
    {
        let mut config = state.config.write().await;
        if config.mcp_servers.iter().any(|s| s.name == name) {
            return Err((
                StatusCode::CONFLICT,
                format!("MCP server '{name}' already exists"),
            ));
        }
        config.mcp_servers.push(McpServerConfig {
            name: name.clone(),
            url: Some(url.clone()),
            command: None,
            args: Default::default(),
            headers: Default::default(),
        });
        save_config(&config)?;
    }

    // Reconnect all MCP servers
    let configs = state.config.read().await.mcp_servers.clone();
    state.mcp_registry.reconnect(&configs).await;
    let tool_count = state.mcp_registry.tool_count().await;

    Ok(Json(json!({
        "status": "ok",
        "name": name,
        "tool_count": tool_count,
    })))
}

async fn remove_mcp_server(
    State(state): State<AppState>,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    {
        let mut config = state.config.write().await;
        let before = config.mcp_servers.len();
        config.mcp_servers.retain(|s| s.name != name);
        if config.mcp_servers.len() == before {
            return Err((
                StatusCode::NOT_FOUND,
                format!("MCP server '{name}' not found"),
            ));
        }
        save_config(&config)?;
    }

    // Reconnect
    let configs = state.config.read().await.mcp_servers.clone();
    state.mcp_registry.reconnect(&configs).await;

    Ok(Json(json!({ "status": "ok" })))
}

// ---------------------------------------------------------------------------
// GitHub integration
// ---------------------------------------------------------------------------

async fn get_github(State(state): State<AppState>) -> Json<serde_json::Value> {
    let config = state.config.read().await;
    let has_token = !config.github.token.is_empty();
    Json(json!({
        "configured": has_token,
    }))
}

#[derive(Deserialize)]
struct UpdateGithubRequest {
    token: String,
}

async fn update_github(
    State(state): State<AppState>,
    Json(request): Json<UpdateGithubRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let token = request.token.trim().to_string();

    {
        let mut config = state.config.write().await;
        config.github.token = token.clone();
        save_config(&config)?;
    }

    let configured = !token.is_empty();
    Ok(Json(json!({
        "status": "ok",
        "configured": configured,
    })))
}

// ---------------------------------------------------------------------------
// Server settings (host, port, auth_token)
// ---------------------------------------------------------------------------

async fn get_server(State(state): State<AppState>) -> Json<serde_json::Value> {
    let config = state.config.read().await;
    Json(json!({
        "host": config.host,
        "port": config.port,
        "auth_token_set": !config.auth_token.is_empty(),
    }))
}

#[derive(Deserialize)]
struct UpdateServerRequest {
    host: Option<String>,
    port: Option<u16>,
    auth_token: Option<String>,
}

async fn update_server(
    State(state): State<AppState>,
    Json(request): Json<UpdateServerRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let mut needs_restart = false;

    {
        let mut config = state.config.write().await;
        if let Some(host) = &request.host {
            if config.host != *host {
                config.host = host.trim().to_string();
                needs_restart = true;
            }
        }
        if let Some(port) = request.port {
            if port > 0 && config.port != port {
                config.port = port;
                needs_restart = true;
            }
        }
        if let Some(token) = &request.auth_token {
            config.auth_token = token.trim().to_string();
        }
        save_config(&config)?;
    }

    Ok(Json(json!({
        "status": "ok",
        "needs_restart": needs_restart,
    })))
}

/// Write the current config back to disk.
fn save_config(config: &config::Config) -> Result<(), (StatusCode, String)> {
    save_config_at(config, &config::config_path())
}

fn save_config_at(
    config: &config::Config,
    config_path: &std::path::Path,
) -> Result<(), (StatusCode, String)> {
    let original = match std::fs::read_to_string(&config_path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to read config: {e}"),
            ));
        }
    };
    let raw = config::serialize_config_preserving_keys(config, &original).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to serialize config: {e}"),
        )
    })?;
    std::fs::write(&config_path, &raw).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to write config: {e}"),
        )
    })?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Provider switching
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct UpdateProviderRequest {
    provider: String,
}

async fn update_provider(
    State(state): State<AppState>,
    Json(req): Json<UpdateProviderRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let provider = match req.provider.as_str() {
        "api" | "anthropic" => config::LlmProvider::Anthropic,
        "openai" => config::LlmProvider::Openai,
        other => {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("unknown provider: {other}"),
            ));
        }
    };

    {
        let mut cfg = state.config.write().await;
        cfg.llm.provider = provider;
        save_config(&cfg)?;
    }
    // Force LLM rebuild (reload_config diff would be empty since we changed in-memory first)
    {
        let cfg = state.config.read().await;
        let new_llm = crate::services::llm::LlmBackend::from_config(&cfg);
        *state.llm.write().await = new_llm;
        log::info!("LLM rebuilt (provider={:?})", provider);
    }

    Ok(Json(json!({ "status": "ok", "provider": provider })))
}

#[cfg(test)]
mod embedding_status_tests {
    use super::*;
    use tower::ServiceExt;

    #[tokio::test]
    async fn embedding_update_validates_persists_and_requires_restart() {
        let workspace = tempfile::tempdir().unwrap();
        let mut state = AppState::new(config::Config::default()).await;
        state.workspace_dir = workspace.path().to_owned();
        let mut settings = config::EmbeddingConfig::default();
        settings.model = "updated-model".into();
        let Json(saved) = update_embedding(State(state.clone()), Json(settings.clone()))
            .await
            .unwrap();
        assert_eq!(saved["needs_restart"], true);
        let persisted: config::Config =
            toml::from_str(&std::fs::read_to_string(workspace.path().join("config.toml")).unwrap())
                .unwrap();
        assert_eq!(persisted.embedding, settings);
        settings.base_url = "https://user:secret@invalid.test/v1".into();
        let (code, error) = update_embedding(State(state.clone()), Json(settings))
            .await
            .unwrap_err();
        assert_eq!(code, StatusCode::BAD_REQUEST);
        assert!(!error.contains("secret"));
        assert_eq!(state.config.read().await.embedding.model, "updated-model");
    }

    #[tokio::test]
    async fn embedding_status_distinguishes_active_and_pending_config() {
        let state = AppState::new(config::Config::default()).await;
        {
            let mut cfg = state.config.write().await;
            cfg.embedding.model = "pending-model".into();
            cfg.llm.tokens.open_ai = "new-secret".into();
        }
        let Json(status) = get_status(State(state)).await;
        assert_eq!(status["embedding"]["model"], "text-embedding-3-small");
        assert_eq!(status["embedding"]["needs_restart"], true);
        assert_eq!(status["embedding"]["pending"]["model"], "pending-model");
        assert_eq!(status["embedding"]["status"], "unavailable");
        assert!(!status.to_string().contains("new-secret"));
    }

    #[tokio::test]
    async fn status_api_exposes_embedding_settings_without_tokens() {
        let mut cfg = config::Config::default();
        cfg.llm.tokens.open_ai = "secret-openai-key".into();
        cfg.llm.tokens.google_ai = "secret-google-key".into();
        let state = AppState::new(cfg).await;
        let Json(status) = get_status(State(state)).await;
        assert_eq!(status["embedding"]["provider"], "openai");
        assert_eq!(status["embedding"]["dimensions"], 768);
        assert_eq!(status["embedding"]["fallback"], "bm25");
        assert_eq!(status["embedding"]["update_semantics"], "full_replacement");
        assert!(!status.to_string().contains("secret-"));
    }

    #[tokio::test]
    async fn embedding_api_rejects_unknown_fields() {
        let state = AppState::new(config::Config::default()).await;
        let response = router()
            .with_state(state)
            .oneshot(
                axum::http::Request::builder()
                    .method("PUT")
                    .uri("/api/config/embedding")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(
                        serde_json::json!({
                            "version": 1,
                            "enabled": true,
                            "provider": "openai",
                            "model": "text-embedding-3-small",
                            "dimensions": 768,
                            "base_url": "https://api.openai.com/v1",
                            "unexpected": true
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }
}
