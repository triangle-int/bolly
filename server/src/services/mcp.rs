use std::{collections::HashMap, fmt, future::Future, pin::Pin, sync::Arc};

use rmcp::{
    ServiceExt,
    model::{ClientInfo, Implementation, ReadResourceRequestParams},
    service::ServerSink,
};

use crate::config::McpServerConfig;
use crate::services::tool::{ToolDefinition, ToolDyn, ToolError};

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct McpToolError(String);

impl fmt::Display for McpToolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "MCP tool error: {}", self.0)
    }
}

impl std::error::Error for McpToolError {}

// ---------------------------------------------------------------------------
// Transport helpers
// ---------------------------------------------------------------------------

fn client_info() -> ClientInfo {
    let mut info = ClientInfo::default();
    info.client_info = Implementation::new("nolune", env!("CARGO_PKG_VERSION"));
    info
}

/// Connect via streamable HTTP transport.
async fn connect_http(
    config: &McpServerConfig,
) -> anyhow::Result<(ServerSink, tokio::task::JoinHandle<()>)> {
    let url = config
        .url
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("no url"))?;
    let mut transport_config =
        rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig::with_uri(url);
    for (key, value) in &config.headers {
        if let (Ok(name), Ok(val)) = (
            key.parse::<reqwest::header::HeaderName>(),
            value.parse::<reqwest::header::HeaderValue>(),
        ) {
            transport_config.custom_headers.insert(name, val);
        }
    }
    let transport = rmcp::transport::StreamableHttpClientTransport::from_config(transport_config);
    let running = client_info().serve(transport).await?;
    let sink = running.peer().clone();
    let handle = tokio::spawn(async move {
        let _ = running.waiting().await;
    });
    Ok((sink, handle))
}

/// Connect via stdio (child process) transport.
async fn connect_stdio(
    config: &McpServerConfig,
) -> anyhow::Result<(ServerSink, tokio::task::JoinHandle<()>)> {
    let cmd = config
        .command
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("no command"))?;

    let mut command = tokio::process::Command::new(cmd);
    command.args(&config.args);
    // Pass headers as env vars for stdio servers
    for (key, value) in &config.headers {
        command.env(key, value);
    }

    let transport = rmcp::transport::TokioChildProcess::new(command)?;
    let running = client_info().serve(transport).await?;
    let sink = running.peer().clone();
    let handle = tokio::spawn(async move {
        let _ = running.waiting().await;
    });
    Ok((sink, handle))
}

/// Connect using the appropriate transport based on config.
async fn connect(
    config: &McpServerConfig,
) -> anyhow::Result<(ServerSink, tokio::task::JoinHandle<()>)> {
    if config.command.is_some() {
        connect_stdio(config).await
    } else {
        connect_http(config).await
    }
}

// ---------------------------------------------------------------------------
// McpTool
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct McpTool {
    definition: rmcp::model::Tool,
    config: McpServerConfig,
    /// Shared connection to the MCP server (kept alive for the session).
    sink: Arc<ServerSink>,
}

fn format_result(result: rmcp::model::CallToolResult) -> Result<String, ToolError> {
    if let Some(true) = result.is_error {
        let error_msg: String = result
            .content
            .iter()
            .filter_map(|c| c.raw.as_text().map(|t| t.text.as_str()))
            .collect::<Vec<_>>()
            .join("\n");
        let msg = if error_msg.is_empty() {
            "No message returned".to_string()
        } else {
            error_msg
        };
        return Err(ToolError::ToolCallError(Box::new(McpToolError(msg))));
    }

    let has_images = result
        .content
        .iter()
        .any(|c| matches!(&c.raw, rmcp::model::RawContent::Image(_)));

    if has_images {
        // Return JSON array of content blocks — images as base64 blocks for Claude,
        // llm.rs will convert to URL-based blocks after saving as uploads.
        let blocks: Vec<serde_json::Value> = result
            .content
            .into_iter()
            .filter_map(|c| match c.raw {
                rmcp::model::RawContent::Text(raw) => {
                    Some(serde_json::json!({"type": "text", "text": raw.text}))
                }
                rmcp::model::RawContent::Image(raw) => Some(serde_json::json!({
                    "type": "image",
                    "source": {
                        "type": "base64",
                        "media_type": raw.mime_type,
                        "data": raw.data,
                    }
                })),
                _ => None,
            })
            .collect();
        Ok(serde_json::to_string(&blocks).unwrap_or_default())
    } else {
        // Text-only: return plain string
        Ok(result
            .content
            .into_iter()
            .map(|c| match c.raw {
                rmcp::model::RawContent::Text(raw) => raw.text,
                rmcp::model::RawContent::Resource(raw) => match raw.resource {
                    rmcp::model::ResourceContents::TextResourceContents { text, .. } => text,
                    rmcp::model::ResourceContents::BlobResourceContents {
                        uri,
                        mime_type,
                        blob,
                        ..
                    } => format!(
                        "{mime_type}{uri}:{blob}",
                        mime_type = mime_type.map(|m| format!("data:{m};")).unwrap_or_default(),
                    ),
                },
                other => format!("{other:?}"),
            })
            .collect::<String>())
    }
}

/// Sanitize a string for use in tool names: replace non-alphanumeric chars with underscore.
fn sanitize_tool_name_part(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

impl ToolDyn for McpTool {
    fn name(&self) -> String {
        format!(
            "mcp_{}_{}",
            sanitize_tool_name_part(&self.config.name),
            sanitize_tool_name_part(&self.definition.name),
        )
    }

    fn definition<'a>(
        &'a self,
        _prompt: String,
    ) -> Pin<Box<dyn Future<Output = ToolDefinition> + Send + 'a>> {
        Box::pin(async move {
            ToolDefinition {
                name: format!(
                    "mcp_{}_{}",
                    sanitize_tool_name_part(&self.config.name),
                    sanitize_tool_name_part(&self.definition.name),
                ),
                description: format!(
                    "[{}] {}",
                    self.config.name,
                    self.definition.description.as_deref().unwrap_or(""),
                ),
                parameters: serde_json::to_value(self.definition.input_schema.as_ref())
                    .unwrap_or_default(),
            }
        })
    }

    fn call<'a>(
        &'a self,
        args: String,
    ) -> Pin<Box<dyn Future<Output = Result<String, ToolError>> + Send + 'a>> {
        let name = self.definition.name.clone();
        let sink = self.sink.clone();
        let arguments: Option<serde_json::Map<String, serde_json::Value>> =
            serde_json::from_str(&args).unwrap_or_default();

        Box::pin(async move {
            let mut params = rmcp::model::CallToolRequestParams::new(name);
            params.arguments = arguments;

            let result = sink
                .call_tool(params)
                .await
                .map_err(|e| ToolError::ToolCallError(Box::new(McpToolError(format!("{e}")))))?;
            format_result(result)
        })
    }
}

// ---------------------------------------------------------------------------
// McpConnection — stores tool definitions and UI resources
// ---------------------------------------------------------------------------

pub struct McpConnection {
    pub name: String,
    pub tools: Vec<McpTool>,
    /// Tool name → resource URI for tools with MCP Apps UI.
    pub ui_tools: HashMap<String, String>,
    /// Resource URI → cached HTML content.
    pub resources: HashMap<String, String>,
    /// Keep-alive handle — holds the connection (and child process for stdio).
    _keep_alive: tokio::task::JoinHandle<()>,
}

/// Extract `_meta.ui.resourceUri` from a tool's metadata.
fn extract_ui_resource_uri(tool: &rmcp::model::Tool) -> Option<String> {
    let meta = tool.meta.as_ref()?;
    let ui = meta.0.get("ui")?.as_object()?;
    let uri = ui.get("resourceUri")?.as_str()?;
    Some(uri.to_string())
}

/// Connect to an MCP server, discover tools, cache UI resources.
/// For persistent servers, the connection stays alive; for stateless, it's dropped.
async fn connect_one(config: &McpServerConfig) -> anyhow::Result<McpConnection> {
    let (sink, handle) = connect(config).await?;

    let raw_tools = sink.list_all_tools().await?;

    // Detect tools with MCP Apps UI
    let mut ui_tools: HashMap<String, String> = HashMap::new();
    for t in &raw_tools {
        if let Some(uri) = extract_ui_resource_uri(t) {
            log::info!(
                "MCP '{}': tool '{}' has UI resource: {}",
                config.name,
                t.name,
                uri
            );
            let prefixed = format!(
                "mcp_{}_{}",
                sanitize_tool_name_part(&config.name),
                sanitize_tool_name_part(&t.name),
            );
            ui_tools.insert(prefixed, uri);
        }
    }

    // Fetch HTML resources for UI tools
    let mut resources: HashMap<String, String> = HashMap::new();
    let unique_uris: Vec<String> = ui_tools
        .values()
        .cloned()
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    for uri in unique_uris {
        match sink
            .read_resource(ReadResourceRequestParams::new(uri.clone()))
            .await
        {
            Ok(result) => {
                for content in &result.contents {
                    if let rmcp::model::ResourceContents::TextResourceContents { text, .. } =
                        &content
                    {
                        log::info!(
                            "MCP '{}': cached resource '{}' ({} bytes)",
                            config.name,
                            uri,
                            text.len()
                        );
                        resources.insert(uri.clone(), text.clone());
                        break;
                    }
                }
            }
            Err(e) => {
                log::warn!(
                    "MCP '{}': failed to fetch resource '{}': {e}",
                    config.name,
                    uri
                );
            }
        }
    }

    // MCP standard: keep connection alive for the session lifetime.
    let persistent_sink = Arc::new(sink);

    let tools: Vec<McpTool> = raw_tools
        .into_iter()
        .map(|t| McpTool {
            definition: t,
            config: config.clone(),
            sink: persistent_sink.clone(),
        })
        .collect();

    Ok(McpConnection {
        name: config.name.clone(),
        tools,
        ui_tools,
        resources,
        _keep_alive: handle,
    })
}

/// Connect to all configured MCP servers, discover tools, cache resources.
pub async fn connect_all(configs: &[McpServerConfig]) -> Vec<McpConnection> {
    let mut connections = Vec::new();

    for config in configs {
        match connect_one(config).await {
            Ok(conn) => {
                log::info!(
                    "MCP '{}': {} tools ({} with UI)",
                    conn.name,
                    conn.tools.len(),
                    conn.ui_tools.len(),
                );
                connections.push(conn);
            }
            Err(e) => {
                log::error!("MCP '{}': failed to connect: {e}", config.name);
            }
        }
    }

    connections
}

// ---------------------------------------------------------------------------
// Curated catalog and grants (#97)
// ---------------------------------------------------------------------------

/// A reviewed server users can add with one click.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CuratedServer {
    pub name: &'static str,
    pub description: &'static str,
    pub url: &'static str,
    pub requires_key: bool,
    pub key_env: &'static str,
    pub key_url: &'static str,
}

pub fn curated_servers() -> &'static [CuratedServer] {
    &[
        CuratedServer {
            name: "fal-ai",
            description: "AI image & video generation (Flux, SDXL, etc.)",
            url: "https://mcp.fal.ai/mcp",
            requires_key: true,
            key_env: "FAL_KEY",
            key_url: "https://fal.ai/dashboard/keys",
        },
        CuratedServer {
            name: "brave-search",
            description: "Web search via Brave Search API",
            url: "https://mcp.bravesearch.com/sse",
            requires_key: true,
            key_env: "BRAVE_API_KEY",
            key_url: "https://brave.com/search/api/",
        },
        CuratedServer {
            name: "github",
            description: "GitHub repos, issues, PRs, code search",
            url: "https://api.githubcopilot.com/mcp/",
            requires_key: true,
            key_env: "GITHUB_TOKEN",
            key_url: "https://github.com/settings/tokens",
        },
        CuratedServer {
            name: "firecrawl",
            description: "Web scraping and crawling",
            url: "https://mcp.firecrawl.dev/sse",
            requires_key: true,
            key_env: "FIRECRAWL_API_KEY",
            key_url: "https://firecrawl.dev",
        },
    ]
}

/// Exact name and URL match against the catalog.
pub fn is_curated(name: &str, url: &str) -> bool {
    curated_servers()
        .iter()
        .any(|entry| entry.name == name && entry.url == url)
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ToolGrant {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub enabled: bool,
}

/// What the config API reports per server: never headers.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ServerGrants {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub trust: crate::config::McpTrust,
    pub connected: bool,
    pub tools: Vec<ToolGrant>,
}

impl ServerGrants {
    /// `discovered` is `(raw name, description)` per tool when connected.
    pub fn from_config(
        config: &McpServerConfig,
        discovered: Option<&[(String, Option<String>)]>,
    ) -> Self {
        let tools = match discovered {
            Some(discovered) => discovered
                .iter()
                .map(|(name, description)| ToolGrant {
                    name: name.clone(),
                    description: description.clone(),
                    enabled: config.allows_tool(name),
                })
                .collect(),
            None => config
                .enabled_tools
                .iter()
                .map(|name| ToolGrant {
                    name: name.clone(),
                    description: None,
                    enabled: true,
                })
                .collect(),
        };
        Self {
            name: config.name.clone(),
            url: config.url.clone(),
            trust: config.trust,
            connected: discovered.is_some(),
            tools,
        }
    }
}

/// Raw names a chat may use: the grant intersected with what the server offers.
#[cfg(test)]
pub fn allowed_tool_names(config: &McpServerConfig, discovered: &[String]) -> Vec<String> {
    discovered
        .iter()
        .filter(|name| config.allows_tool(name))
        .cloned()
        .collect()
}

impl McpTool {
    pub fn raw_name(&self) -> &str {
        &self.definition.name
    }

    pub fn raw_description(&self) -> Option<String> {
        self.definition.description.as_deref().map(str::to_owned)
    }
}

/// A shared handle holding all MCP tool registrations.
/// Tools are discovered once at startup and reconnect per call (or use persistent sink).
#[derive(Clone, Default)]
pub struct McpRegistry {
    connections: Arc<tokio::sync::RwLock<Vec<McpConnection>>>,
    configs: Arc<tokio::sync::RwLock<Vec<McpServerConfig>>>,
}

impl McpRegistry {
    pub fn new(connections: Vec<McpConnection>, configs: Vec<McpServerConfig>) -> Self {
        Self {
            connections: Arc::new(tokio::sync::RwLock::new(connections)),
            configs: Arc::new(tokio::sync::RwLock::new(configs)),
        }
    }

    /// Replace all connections with newly discovered ones.
    pub async fn reconnect(&self, configs: &[McpServerConfig]) {
        let new_connections = connect_all(configs).await;
        *self.configs.write().await = configs.to_vec();
        *self.connections.write().await = new_connections;
    }

    /// Tools ordinary chats may use: only the explicit grant of each
    /// connected server (#97). Nothing is exposed just because it connected.
    pub async fn active_tools_as_dyn(&self) -> Vec<Box<dyn ToolDyn>> {
        let configs = self.configs.read().await;
        self.connections
            .read()
            .await
            .iter()
            .flat_map(|conn| {
                let config = configs.iter().find(|c| c.name == conn.name);
                conn.tools
                    .iter()
                    .filter(move |t| config.is_some_and(|c| c.allows_tool(t.raw_name())))
                    .map(|t| {
                        let boxed: Box<dyn ToolDyn> = Box::new(t.clone());
                        boxed
                    })
            })
            .collect()
    }

    /// Discovered raw tool names for a connected server, if connected.
    pub async fn discovered_tools(&self, server: &str) -> Option<Vec<(String, Option<String>)>> {
        self.connections
            .read()
            .await
            .iter()
            .find(|conn| conn.name == server)
            .map(|conn| {
                conn.tools
                    .iter()
                    .map(|t| (t.raw_name().to_owned(), t.raw_description()))
                    .collect()
            })
    }

    /// Per-server trust and grants for the config API. Never includes headers.
    pub async fn grants(&self, configs: &[McpServerConfig]) -> Vec<ServerGrants> {
        let connections = self.connections.read().await;
        configs
            .iter()
            .map(|config| {
                let discovered: Option<Vec<(String, Option<String>)>> = connections
                    .iter()
                    .find(|conn| conn.name == config.name)
                    .map(|conn| {
                        conn.tools
                            .iter()
                            .map(|t| (t.raw_name().to_owned(), t.raw_description()))
                            .collect()
                    });
                ServerGrants::from_config(config, discovered.as_deref())
            })
            .collect()
    }

    /// Snapshot app tool HTML for sync access. Only curated servers' granted
    /// tools may render an app frame (#97).
    pub async fn snapshot_app_tools(&self) -> McpAppSnapshot {
        let configs = self.configs.read().await;
        let guard = self.connections.read().await;
        let mut tool_html = HashMap::new();
        for conn in guard.iter() {
            let Some(config) = configs.iter().find(|c| c.name == conn.name) else {
                continue;
            };
            if config.trust != crate::config::McpTrust::Curated {
                continue;
            }
            for tool in &conn.tools {
                if !config.allows_tool(tool.raw_name()) {
                    continue;
                }
                let Some(uri) = conn.ui_tools.get(&tool.name()) else {
                    continue;
                };
                if let Some(html) = conn.resources.get(uri) {
                    tool_html.insert(tool.name(), html.clone());
                }
            }
        }
        McpAppSnapshot { tool_html }
    }

    pub async fn tool_count(&self) -> usize {
        self.connections
            .read()
            .await
            .iter()
            .map(|c| c.tools.len())
            .sum()
    }
}

// ---------------------------------------------------------------------------
// McpAppSnapshot
// ---------------------------------------------------------------------------

/// Snapshot of MCP App tool HTML (safe for sync access without holding locks).
#[derive(Clone, Default)]
pub struct McpAppSnapshot {
    pub tool_html: HashMap<String, String>,
}

impl McpAppSnapshot {
    pub fn is_app_tool(&self, tool_name: &str) -> bool {
        self.tool_html.contains_key(tool_name)
    }

    pub fn get_html(&self, tool_name: &str) -> Option<&String> {
        self.tool_html.get(tool_name)
    }
}

#[cfg(test)]
mod grant_tests {
    use super::*;
    use crate::config::McpTrust;

    fn config(trust: McpTrust, enabled: &[&str]) -> McpServerConfig {
        McpServerConfig {
            name: "srv".into(),
            url: Some("https://srv.example/mcp".into()),
            command: None,
            args: Vec::new(),
            headers: [("Authorization".to_owned(), "Bearer secret".to_owned())]
                .into_iter()
                .collect(),
            trust,
            enabled_tools: enabled.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn custom_servers_grant_nothing_until_enabled_and_grants_are_exact() {
        let discovered = vec!["search".to_owned(), "fetch".to_owned(), "run".to_owned()];
        assert!(allowed_tool_names(&config(McpTrust::Custom, &[]), &discovered).is_empty());
        assert_eq!(
            allowed_tool_names(
                &config(McpTrust::Custom, &["fetch", "missing"]),
                &discovered
            ),
            vec!["fetch"]
        );
        assert_eq!(
            allowed_tool_names(
                &config(McpTrust::Curated, &["search", "fetch", "run"]),
                &discovered
            ),
            discovered
        );
    }

    #[test]
    fn catalog_matches_require_exact_name_and_url() {
        assert!(is_curated(
            "brave-search",
            "https://mcp.bravesearch.com/sse"
        ));
        assert!(!is_curated("brave-search", "https://evil.example/sse"));
        assert!(!is_curated("mine", "https://mcp.bravesearch.com/sse"));
        assert!(
            curated_servers()
                .iter()
                .all(|entry| entry.url.starts_with("https://"))
        );
    }

    #[test]
    fn grants_report_discovery_and_never_headers() {
        let cfg = config(McpTrust::Custom, &["fetch"]);
        let discovered = vec![
            ("search".to_owned(), Some("web search".to_owned())),
            ("fetch".to_owned(), None),
        ];
        let grants = ServerGrants::from_config(&cfg, Some(&discovered));
        assert!(grants.connected);
        assert_eq!(
            grants
                .tools
                .iter()
                .map(|t| (t.name.as_str(), t.enabled))
                .collect::<Vec<_>>(),
            vec![("search", false), ("fetch", true)]
        );
        let offline = ServerGrants::from_config(&cfg, None);
        assert!(!offline.connected);
        assert_eq!(offline.tools.len(), 1);
        for value in [&grants, &offline] {
            let json = serde_json::to_string(value).unwrap();
            assert!(!json.contains("secret") && !json.contains("headers"));
        }
    }
}
