use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::{Mutex, RwLock, broadcast};
use tokio_util::sync::CancellationToken;

use crate::{
    config::{self, Config},
    domain::events::ServerEvent,
    services::browser_sessions::BrowserSessionStore,
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
    pub(crate) resources: crate::services::resource_access::ResourceAccess,
    pub workspace_dir: PathBuf,
    pub events: broadcast::Sender<ServerEvent>,
    pub llm: Arc<RwLock<Option<LlmBackend>>>,
    /// Active agent tasks per instance slug — cancellation tokens.
    pub agent_tasks: Arc<Mutex<HashMap<String, CancellationToken>>>,
    /// Pending secret requests awaiting user input.
    pub pending_secrets: Arc<Mutex<HashMap<String, PendingSecret>>>,
    /// Connected MCP servers and their tools.
    pub mcp_registry: McpRegistry,
    /// Shared HTTP client for provider and integration calls.
    pub http_client: reqwest::Client,
    /// Versioned local vector store for semantic memory search.
    pub vector_store: Arc<VectorStore>,
    /// Registry of connected Tauri agent machines (for computer use).
    pub machine_registry: MachineRegistry,
    /// Paired browsers and pending pairing codes (#112). In memory until
    /// `attach_storage` is called by the server entrypoint.
    pub browser_sessions: Arc<BrowserSessionStore>,
}

// No hardcoded MCP servers — users add them via Settings UI or config.toml.
// Suggested MCP servers are listed in the client's Extensions settings.

impl AppState {
    pub async fn new(config: Config) -> Self {
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

        // Open the local derived vector index.
        let vector_store =
            VectorStore::connect_with_config(&config::workspace_root(), &config).await;

        Self {
            resources: crate::services::resource_access::ResourceAccess::new(&config.auth_token),
            config: Arc::new(RwLock::new(config)),
            workspace_dir: config::workspace_root(),
            events,
            llm: Arc::new(RwLock::new(llm)),
            agent_tasks: Arc::new(Mutex::new(HashMap::new())),
            pending_secrets: Arc::new(Mutex::new(HashMap::new())),
            mcp_registry,
            http_client,
            vector_store: Arc::new(vector_store),
            machine_registry: MachineRegistry::new(),
            browser_sessions: Arc::new(BrowserSessionStore::new()),
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
            let mut old = self.config.write().await;
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
            if old.auth_token != new_config.auth_token {
                self.resources.replace(&new_config.auth_token);
            }
            *old = new_config.clone();
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

        if self.vector_store.embedding_needs_restart(&new_config) {
            log::info!(
                "embedding settings changed: restart required; active index remains unchanged"
            );
        }
    }
}
