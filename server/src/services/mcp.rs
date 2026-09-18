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

    /// List connected server names.
    pub async fn server_names(&self) -> Vec<String> {
        self.connections
            .read()
            .await
            .iter()
            .map(|c| c.name.clone())
            .collect()
    }

    /// Get all MCP tools as boxed ToolDyn.
    pub async fn tools_as_dyn(&self) -> Vec<Box<dyn ToolDyn>> {
        self.connections
            .read()
            .await
            .iter()
            .flat_map(|conn| {
                conn.tools.iter().map(|t| {
                    let boxed: Box<dyn ToolDyn> = Box::new(t.clone());
                    boxed
                })
            })
            .collect()
    }

    /// Snapshot app tool info for sync access.
    pub async fn snapshot_app_tools(&self) -> McpAppSnapshot {
        let guard = self.connections.read().await;
        let mut tool_html = HashMap::new();
        for conn in guard.iter() {
            for (tool_name, uri) in &conn.ui_tools {
                if let Some(html) = conn.resources.get(uri) {
                    tool_html.insert(tool_name.clone(), html.clone());
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
