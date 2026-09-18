use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::{Mutex, RwLock, broadcast};
use tokio_util::sync::CancellationToken;

use crate::{
    config::{self, Config},
    domain::events::ServerEvent,
    services::keyword_search::KeywordStore,
    services::llm::LlmBackend,
    services::machine_registry::MachineRegistry,
    services::mcp::McpRegistry,
    services::vector::VectorStore,
};

/// A pending secret request waiting for user input.
pub struct PendingSecret {
    #[allow(dead_code)]
    pub target: String,
    pub responder: tokio::sync::oneshot::Sender<String>,
}

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<RwLock<Config>>,
    pub workspace_dir: PathBuf,
    pub events: broadcast::Sender<ServerEvent>,
    pub llm: Arc<RwLock<Option<LlmBackend>>>,
    /// Active agent tasks per instance slug — cancellation tokens.
    pub agent_tasks: Arc<Mutex<HashMap<String, CancellationToken>>>,
    /// Pending secret requests awaiting user input.
    pub pending_secrets: Arc<Mutex<HashMap<String, PendingSecret>>>,
    /// Connected MCP servers and their tools.
    pub mcp_registry: McpRegistry,
    /// Shared HTTP client for landing API calls.
    pub http_client: reqwest::Client,
    /// Landing server URL (empty for self-hosted).
    pub landing_url: String,
    /// Auth token for landing API calls.
    pub _landing_auth_token: String,
    /// Versioned local vector store for semantic memory search.
    pub vector_store: Arc<VectorStore>,
    /// BM25 keyword search over memory files.
    pub keyword_store: Arc<KeywordStore>,
    /// Registry of connected Tauri agent machines (for computer use).
    pub machine_registry: MachineRegistry,
}

// No hardcoded MCP servers — users add them via Settings UI or config.toml.
// Suggested MCP servers are listed in the client's Extensions settings.

impl AppState {
    pub async fn new(mut config: Config) -> Self {
        let (events, _) = broadcast::channel(4096);
        let llm = LlmBackend::from_config(&config);

        // Connect to configured MCP servers
        let mcp_connections = crate::services::mcp::connect_all(&config.mcp_servers).await;
        let mcp_registry = McpRegistry::new(mcp_connections, config.mcp_servers.clone());
        let mcp_tool_count = mcp_registry.tool_count().await;
        if mcp_tool_count > 0 {
            log::info!("MCP: {} tools from external servers", mcp_tool_count);
        }

        let http_client = reqwest::Client::new();
        let landing_url = config.landing_url.clone();
        let landing_auth_token = config.auth_token.clone();

        // Open the local derived vector index.
        let vector_store = VectorStore::connect(&config::workspace_root()).await;

        // Fetch plan from landing API if configured
        if !landing_url.is_empty() && !landing_auth_token.is_empty() {
            match http_client
                .get(format!("{landing_url}/api/internal/plan"))
                .bearer_auth(&landing_auth_token)
                .send()
                .await
            {
                Ok(res) => {
                    if let Ok(body) = res.json::<serde_json::Value>().await {
                        if let Some(plan) = body["plan"].as_str() {
                            log::info!("fetched plan from landing API: {plan}");
                            config.plan = plan.to_string();
                        }
                    }
                }
                Err(e) => log::warn!("failed to fetch plan from landing API: {e}"),
            }
        }

        Self {
            config: Arc::new(RwLock::new(config)),
            workspace_dir: config::workspace_root(),
            events,
            llm: Arc::new(RwLock::new(llm)),
            agent_tasks: Arc::new(Mutex::new(HashMap::new())),
            pending_secrets: Arc::new(Mutex::new(HashMap::new())),
            mcp_registry,
            http_client,
            landing_url,
            _landing_auth_token: landing_auth_token,
            vector_store: Arc::new(vector_store),
            keyword_store: Arc::new(KeywordStore::new()),
            machine_registry: MachineRegistry::new(),
        }
    }

    /// Reload config from disk and rebuild LLM if credentials or model selection changed.
    pub async fn reload_config(&self) {
        let new_config = match config::load_config() {
            Ok(c) => c,
            Err(e) => {
                log::warn!("failed to reload config: {e}");
                return;
            }
        };

        let (llm_changed, mcp_changed) = {
            let old = self.config.read().await;
            let tokens = old.llm.tokens != new_config.llm.tokens;
            let provider = old.llm.provider != new_config.llm.provider;
            let models = old.llm.profiles != new_config.llm.profiles;
            let llm = tokens || provider || models;
            let mcp = old.mcp_servers.len() != new_config.mcp_servers.len()
                || old
                    .mcp_servers
                    .iter()
                    .zip(new_config.mcp_servers.iter())
                    .any(|(a, b)| a.name != b.name || a.url != b.url);
            (llm, mcp)
        };

        if llm_changed {
            let new_llm = LlmBackend::from_config(&new_config);
            *self.llm.write().await = new_llm;
            log::info!(
                "config reloaded: LLM rebuilt (provider={:?})",
                new_config.llm.provider
            );
        }

        if mcp_changed {
            self.mcp_registry.reconnect(&new_config.mcp_servers).await;
            log::info!(
                "config reloaded: MCP servers reconnected ({} tools)",
                self.mcp_registry.tool_count().await
            );
        }

        // Preserve plan from API — it's not in config.toml
        let mut cfg = self.config.write().await;
        let plan = cfg.plan.clone();
        *cfg = new_config;
        if cfg.plan.is_empty() {
            cfg.plan = plan;
        }
    }
}
