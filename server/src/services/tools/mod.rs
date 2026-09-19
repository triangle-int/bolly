use std::{
    collections::{HashMap, VecDeque},
    fmt,
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{Arc, Mutex, OnceLock},
};

use crate::services::tool::{ToolDefinition, ToolDyn, ToolError};
use schemars::JsonSchema;
use tokio::sync::broadcast;

use regex::Regex;

use crate::domain::events::ServerEvent;

/// Mint a bounded, retryable model-provider resource URL.
pub fn public_file_url(
    base: &str,
    instance_slug: &str,
    file_id: &str,
    resources: &crate::services::resource_access::ResourceAccess,
) -> String {
    use crate::services::resource_capability::{CapabilityAudience, CapabilityResource};
    CapabilityResource::uploaded_file(file_id)
        .and_then(|resource| {
            resources.url(
                base,
                instance_slug,
                resource,
                CapabilityAudience::ModelProvider,
            )
        })
        .unwrap_or_default()
}

pub fn public_memory_url(
    base: &str,
    instance_slug: &str,
    path: &str,
    resources: &crate::services::resource_access::ResourceAccess,
) -> String {
    use crate::services::resource_capability::{CapabilityAudience, CapabilityResource};
    CapabilityResource::memory(path)
        .and_then(|resource| {
            resources.url(
                base,
                instance_slug,
                resource,
                CapabilityAudience::ModelProvider,
            )
        })
        .unwrap_or_default()
}

pub fn media_result_url(
    base: &str,
    slug: &str,
    result: &crate::services::vector::VectorSearchResult,
    resources: &crate::services::resource_access::ResourceAccess,
) -> Option<String> {
    if base.is_empty() || !result.source_type.starts_with("media_") {
        return None;
    }
    let id = result.upload_id.as_deref()?;
    Some(if id == result.path || id.contains('/') {
        public_memory_url(base, slug, id, resources)
    } else {
        public_file_url(base, slug, id, resources)
    })
}

// Sub-modules
pub mod communication;
pub mod companion;
pub mod computer;
pub mod files;
pub mod image;
pub mod media;
pub mod memory_tools;
pub mod project;
pub mod skills;
pub mod system;

// Re-export public items so external code uses `tools::FooTool` paths
pub use communication::{ReachOutTool, ReadEmailTool, ScheduledTask, SendEmailTool};
pub use companion::{
    ALLOWED_MOODS, EditSoulTool, SetVoiceTool, get_voice_override, load_mood_state, save_mood_state,
};
pub use computer::{ComputerUseTool, ListMachinesTool, RemoteBashTool, RemoteFilesTool};

pub use files::{EditFileTool, ListFilesTool, ReadFileTool, UploadFileTool, WriteFileTool};
pub use image::ViewImageTool;
pub use media::WatchVideoTool;
pub use memory_tools::{
    MemoryConnectTool, MemoryForgetTool, MemoryListTool, MemoryReadTool, MemorySearchTool,
    MemoryWriteTool,
};
pub use project::{TaskItem, TaskStatus};
pub use skills::{ActivateSkillTool, ListSkillsTool, ReadSkillReferenceTool};
pub use system::{
    CallAgentTool, ClearContextTool, CreateDropTool, ExportProfileTool, GetSettingsTool,
    GetTimeTool, ImportProfileTool, InteractiveSessionTool, RequestSecretTool, RunCommandTool,
    UpdateConfigTool,
};
// ---------------------------------------------------------------------------
// Cached tool definitions snapshot (populated by build_tools, read by stats)
// ---------------------------------------------------------------------------

/// Snapshot of tool definition info, updated every time build_tools runs.
#[derive(Clone, Default)]
pub struct ToolDefsSnapshot {
    pub names: Vec<String>,
    pub total_json_chars: usize,
    /// Full tool definition JSON values for use in count_tokens API.
    pub defs_json: Vec<serde_json::Value>,
}

static TOOL_DEFS_CACHE: OnceLock<Mutex<ToolDefsSnapshot>> = OnceLock::new();

fn tool_defs_cache() -> &'static Mutex<ToolDefsSnapshot> {
    TOOL_DEFS_CACHE.get_or_init(|| Mutex::new(ToolDefsSnapshot::default()))
}

/// Read the latest cached tool definitions snapshot.
pub fn cached_tool_defs() -> ToolDefsSnapshot {
    tool_defs_cache()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

/// Compute and cache tool definition info from actual tool instances.
/// Call this after `build_tools` to keep the cache up to date.
pub async fn cache_tool_defs(tools: &[Box<dyn ToolDyn>]) {
    let mut names = Vec::with_capacity(tools.len());
    let mut total_json_chars = 0usize;
    let mut defs_json = Vec::with_capacity(tools.len());
    for tool in tools {
        let def = tool.definition(String::new()).await;
        names.push(def.name.clone());
        if let Ok(json_str) = serde_json::to_string(&def) {
            total_json_chars += json_str.len();
        }
        if let Ok(val) = serde_json::to_value(&def) {
            defs_json.push(val);
        }
    }
    let mut cache = tool_defs_cache().lock().unwrap_or_else(|e| e.into_inner());
    *cache = ToolDefsSnapshot {
        names,
        total_json_chars,
        defs_json,
    };
}

// ---------------------------------------------------------------------------
// Per-chat file lock
// ---------------------------------------------------------------------------

type ChatLocks = Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>;

fn chat_locks() -> &'static ChatLocks {
    static LOCKS: OnceLock<ChatLocks> = OnceLock::new();
    LOCKS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Get a mutex for a specific messages.json path. All writers must use this.
pub fn chat_file_lock(path: &Path) -> Arc<Mutex<()>> {
    let mut map = chat_locks().lock().unwrap_or_else(|e| e.into_inner());
    map.entry(path.to_path_buf())
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

// ---------------------------------------------------------------------------
// Secret redaction
// ---------------------------------------------------------------------------

const MAX_CONTROL_SECRETS: usize = 64;
const MAX_CAPABILITY_TOKENS: usize = 4096;

#[derive(Default)]
struct ControlSecretRegistry {
    secrets: VecDeque<String>,
}

impl ControlSecretRegistry {
    fn register(&mut self, secret: &str) {
        if secret.is_empty() {
            return;
        }
        if let Some(index) = self.secrets.iter().position(|value| value == secret) {
            self.secrets.remove(index);
        }
        self.secrets.push_back(secret.into());
        while self.secrets.len() > MAX_CONTROL_SECRETS {
            self.secrets.pop_front();
        }
    }
}

static CONTROL_SECRETS: OnceLock<Mutex<ControlSecretRegistry>> = OnceLock::new();
pub(crate) fn register_control_secret(secret: &str) {
    CONTROL_SECRETS
        .get_or_init(Default::default)
        .lock()
        .expect("secrets lock")
        .register(secret);
}

#[derive(Default)]
struct CapabilityTokenRegistry {
    tokens: VecDeque<String>,
}

impl CapabilityTokenRegistry {
    fn register(&mut self, token: &str) {
        if token.is_empty() {
            return;
        }
        if let Some(index) = self.tokens.iter().position(|value| value == token) {
            self.tokens.remove(index);
        }
        self.tokens.push_back(token.into());
        while self.tokens.len() > MAX_CAPABILITY_TOKENS {
            self.tokens.pop_front();
        }
    }
}

static CAPABILITY_TOKENS: OnceLock<Mutex<CapabilityTokenRegistry>> = OnceLock::new();

pub(crate) fn register_capability_token(token: &str) {
    CAPABILITY_TOKENS
        .get_or_init(Default::default)
        .lock()
        .expect("capability token lock")
        .register(token);
}

fn secret_values() -> &'static Vec<String> {
    use std::sync::OnceLock;
    static SECRETS: OnceLock<Vec<String>> = OnceLock::new();
    SECRETS.get_or_init(|| {
        let env_keys = [
            "ANTHROPIC_API_KEY",
            "OPENAI_API_KEY",
            "OPENROUTER_API_KEY",
            "BRAVE_SEARCH_API_KEY",
            "DATABASE_URL",
            "NOLUNE_AUTH_TOKEN",
            "NOLUNE_RELEASE_TOKEN",
            "STRIPE_SECRET_KEY",
            "GITHUB_TOKEN",
            "GOOGLE_CLIENT_SECRET",
            "ELEVENLABS_API_KEY",
            "GOOGLE_AI_API_KEY",
        ];
        env_keys
            .iter()
            .filter_map(|k| std::env::var(k).ok())
            .filter(|v| v.len() >= 8)
            .collect()
    })
}

pub(crate) fn redact_value(mut value: serde_json::Value) -> serde_json::Value {
    match &mut value {
        serde_json::Value::String(s) => *s = redact_secrets(s),
        serde_json::Value::Array(items) => {
            for item in items {
                *item = redact_value(std::mem::take(item));
            }
        }
        serde_json::Value::Object(map) => {
            *map = std::mem::take(map)
                .into_iter()
                .map(|(key, value)| {
                    let sensitive = matches!(
                        key.to_ascii_lowercase().as_str(),
                        "authorization" | "token" | "auth_token" | "control_token"
                    );
                    let value = if sensitive && value.is_string() {
                        serde_json::Value::String("[REDACTED]".into())
                    } else {
                        redact_value(value)
                    };
                    (redact_secrets(&key), value)
                })
                .collect();
        }
        _ => {}
    }
    value
}

/// Redact known secret patterns and exact env var values from text.
pub fn redact_secrets(text: &str) -> String {
    let patterns = [
        r#"sk-ant-api03-[A-Za-z0-9_\-]{80,}"#,
        r#"sk-ant-[A-Za-z0-9_\-]{20,}"#,
        r#"sk-proj-[A-Za-z0-9_\-]{20,}"#,
        r"sk-[A-Za-z0-9]{20,}",
        r"ghp_[A-Za-z0-9]{36,}",
        r"github_pat_[A-Za-z0-9_]{80,}",
        r"gho_[A-Za-z0-9]{36,}",
        r#"postgresql://[^\s"']+[^\s"'.]"#,
        r#"postgres://[^\s"']+[^\s"'.]"#,
        r"AIza[A-Za-z0-9_\-]{30,}",
    ];

    let capability_tokens = CAPABILITY_TOKENS
        .get_or_init(Default::default)
        .lock()
        .expect("capability token lock")
        .tokens
        .iter()
        .cloned()
        .collect::<Vec<_>>();
    let mut result = text.to_string();
    for secret in CONTROL_SECRETS
        .get_or_init(Default::default)
        .lock()
        .expect("secrets lock")
        .secrets
        .iter()
    {
        result = replace_exact_secret(&result, secret, &capability_tokens);
    }

    for pat in &patterns {
        if let Ok(re) = Regex::new(pat) {
            result = replace_regex_preserving_capabilities(&result, &re, &capability_tokens);
        }
    }

    for secret in secret_values() {
        result = replace_exact_secret(&result, secret, &capability_tokens);
    }

    result
}

fn replace_exact_secret(text: &str, secret: &str, capability_tokens: &[String]) -> String {
    if secret.is_empty() {
        return text.to_owned();
    }

    let exempt = capability_ranges(text, capability_tokens);
    let mut output = String::with_capacity(text.len());
    let mut cursor = 0;
    while let Some(relative) = text[cursor..].find(secret) {
        let start = cursor + relative;
        let end = start + secret.len();
        output.push_str(&text[cursor..start]);
        if exempt
            .iter()
            .any(|range| start >= range.start && end <= range.end)
        {
            output.push_str(secret);
        } else {
            output.push_str("[REDACTED]");
        }
        cursor = end;
    }
    output.push_str(&text[cursor..]);
    output
}

fn capability_ranges(text: &str, capability_tokens: &[String]) -> Vec<std::ops::Range<usize>> {
    fn capability_byte(byte: u8) -> bool {
        byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'~' | b'-')
    }

    let mut ranges = Vec::new();
    for token in capability_tokens {
        let needle = format!("cap={token}");
        let mut cursor = 0;
        while let Some(relative) = text[cursor..].find(&needle) {
            let cap_start = cursor + relative;
            let value_start = cap_start + 4;
            let value_end = value_start + token.len();
            let left_is_name = cap_start > 0
                && (text.as_bytes()[cap_start - 1].is_ascii_alphanumeric()
                    || text.as_bytes()[cap_start - 1] == b'_');
            let right_continues_value =
                value_end < text.len() && capability_byte(text.as_bytes()[value_end]);
            if !left_is_name && !right_continues_value {
                ranges.push(value_start..value_end);
            }
            cursor = value_end;
        }
    }
    ranges.sort_by_key(|range| range.start);
    ranges.dedup();
    ranges
}

fn replace_regex_preserving_capabilities(
    text: &str,
    regex: &Regex,
    capability_tokens: &[String],
) -> String {
    let ranges = capability_ranges(text, capability_tokens);
    let mut output = String::with_capacity(text.len());
    let mut cursor = 0;
    for range in ranges {
        output.push_str(&regex.replace_all(&text[cursor..range.start], "[REDACTED]"));
        output.push_str(&text[range.clone()]);
        cursor = range.end;
    }
    output.push_str(&regex.replace_all(&text[cursor..], "[REDACTED]"));
    output
}

// ---------------------------------------------------------------------------
// OpenAI-compatible schema helper
// ---------------------------------------------------------------------------

pub(crate) fn openai_schema<T: JsonSchema>() -> serde_json::Value {
    let mut val = serde_json::to_value(schemars::schema_for!(T)).unwrap();
    if let Some(obj) = val.as_object_mut() {
        obj.remove("$schema");
        obj.remove("$id");
        obj.remove("title");
        if !obj.contains_key("properties") {
            obj.insert("properties".into(), serde_json::json!({}));
        }
    }
    val
}

// ---------------------------------------------------------------------------
// Tool activity summary helper
// ---------------------------------------------------------------------------

pub fn tool_summary(name: &str, args: &str) -> String {
    let v: serde_json::Value = serde_json::from_str(args).unwrap_or_default();
    match name {
        "read_file" => format!("reading {}", v["path"].as_str().unwrap_or("?")),
        "write_file" => format!("writing {}", v["path"].as_str().unwrap_or("?")),
        "edit_file" => format!("editing {}", v["path"].as_str().unwrap_or("?")),
        "list_files" => format!("listing {}", v["path"].as_str().unwrap_or(".")),
        "run_command" => "running command".into(),
        "edit_soul" => "rewriting soul.md".into(),
        "set_mood" => format!("mood → {}", v["mood"].as_str().unwrap_or("?")),
        "remember" => "storing a memory".into(),
        "recall" => format!("recalling '{}'", v["query"].as_str().unwrap_or("?")),
        "web_search" => format!("web search: {}", v["query"].as_str().unwrap_or("?")),
        "web_fetch" => "fetching URL".into(),
        "update_config" => "updating config".into(),
        "create_drop" => format!("creating drop: {}", v["title"].as_str().unwrap_or("?")),
        "get_settings" => "reading current settings".into(),
        "send_email" => {
            let to = v["to"].as_str().unwrap_or("?");
            format!("sending email to {to}")
        }
        "read_email" => {
            let count = v["count"].as_u64().unwrap_or(5);
            format!("reading {count} emails")
        }

        "set_voice" => {
            let vid = v["voice_id"].as_str().unwrap_or("");
            if vid.is_empty() {
                "resetting voice to default".into()
            } else {
                format!("voice → {vid}")
            }
        }
        "request_secret" => format!("requesting secret: {}", v["prompt"].as_str().unwrap_or("?")),
        "read_skill_reference" => format!(
            "reading skill ref {}/{}",
            v["skill_id"].as_str().unwrap_or("?"),
            v["filename"].as_str().unwrap_or("?")
        ),
        // send_file removed
        _ => format!("calling {name}"),
    }
}

// ---------------------------------------------------------------------------
// ObservableTool
// ---------------------------------------------------------------------------

pub struct ObservableTool {
    inner: Box<dyn ToolDyn>,
    events: broadcast::Sender<ServerEvent>,
    workspace_dir: PathBuf,
    instance_slug: String,
    chat_id: String,
    mcp_snapshot: Option<crate::services::mcp::McpAppSnapshot>,
}

impl ObservableTool {
    pub fn new(
        inner: Box<dyn ToolDyn>,
        events: broadcast::Sender<ServerEvent>,
        workspace_dir: &Path,
        instance_slug: String,
        chat_id: String,
        mcp_snapshot: Option<crate::services::mcp::McpAppSnapshot>,
    ) -> Self {
        Self {
            inner,
            events,
            workspace_dir: workspace_dir.to_path_buf(),
            instance_slug,
            chat_id,
            mcp_snapshot,
        }
    }
}

impl ToolDyn for ObservableTool {
    fn name(&self) -> String {
        self.inner.name()
    }

    fn trusts_resource_provenance(&self) -> bool {
        self.inner.trusts_resource_provenance()
    }

    fn definition(
        &self,
        prompt: String,
    ) -> Pin<Box<dyn Future<Output = ToolDefinition> + Send + '_>> {
        self.inner.definition(prompt)
    }

    fn call(
        &self,
        args: String,
    ) -> Pin<Box<dyn Future<Output = Result<String, ToolError>> + Send + '_>> {
        let tool_name = self.inner.name();
        let summary = redact_secrets(&tool_summary(&tool_name, &args));

        let start_msg = crate::domain::chat::ChatMessage {
            id: format!("tool_{}_{}", tool_call_counter(), unix_millis()),
            role: crate::domain::chat::ChatRole::Assistant,
            content: summary.clone(),
            created_at: unix_millis().to_string(),
            kind: crate::domain::chat::MessageKind::ToolCall,
            tool_name: Some(tool_name.clone()),
            mcp_app_html: None,
            mcp_app_input: None,
            model: None,
        };
        // Tool activity is already captured in rig_history via ToolUse/ToolResult blocks.
        // Only broadcast via WebSocket for real-time UI updates.
        let _ = self.events.send(ServerEvent::ChatMessageCreated {
            instance_slug: self.instance_slug.clone(),
            chat_id: self.chat_id.clone(),
            message: start_msg,
        });

        // Emit MCP App BEFORE the tool call so the viewer appears immediately
        let mut mcp_app_msg_id: Option<String> = None;
        if let Some(ref snapshot) = self.mcp_snapshot {
            if snapshot.is_app_tool(&tool_name) {
                if let Some(html) = snapshot.get_html(&tool_name).cloned() {
                    let msg_id = format!("mcp_app_{}_{}", tool_call_counter(), unix_millis());
                    mcp_app_msg_id = Some(msg_id.clone());
                    let app_msg = crate::domain::chat::ChatMessage {
                        id: msg_id,
                        role: crate::domain::chat::ChatRole::Assistant,
                        content: String::new(), // result not yet available
                        created_at: unix_millis().to_string(),
                        kind: crate::domain::chat::MessageKind::McpApp,
                        tool_name: Some(tool_name.clone()),
                        mcp_app_html: Some(redact_secrets(&html)),
                        mcp_app_input: Some(redact_secrets(&args)),
                        model: None,
                    };
                    let _ = self.events.send(ServerEvent::ChatMessageCreated {
                        instance_slug: self.instance_slug.clone(),
                        chat_id: self.chat_id.clone(),
                        message: app_msg,
                    });
                }
            }
        }

        let events = self.events.clone();
        let _workspace_dir = self.workspace_dir.clone();
        let instance_slug = self.instance_slug.clone();
        let chat_id = self.chat_id.clone();
        let fut = self.inner.call(args);
        Box::pin(async move {
            const MAX_TOOL_RESULT: usize = 12_000;
            let result = match fut.await {
                Ok(s) => {
                    let redacted = redact_secrets(&s);
                    if redacted.len() > MAX_TOOL_RESULT {
                        let truncated: String = redacted.chars().take(MAX_TOOL_RESULT).collect();
                        Ok(format!(
                            "{truncated}\n\n...(tool output truncated at {MAX_TOOL_RESULT} chars, total: {})",
                            redacted.len()
                        ))
                    } else {
                        Ok(redacted)
                    }
                }
                Err(e) => Err(ToolError::ToolCallError(Box::new(ToolExecError(
                    redact_secrets(&e.to_string()),
                )))),
            };
            // Send tool result to the MCP App viewer
            if let Some(msg_id) = mcp_app_msg_id {
                let tool_output = match &result {
                    Ok(s) => s.clone(),
                    Err(e) => format!("error: {e}"),
                };
                // MCP app results are delivered via WebSocket only (rig_history has the tool result)
                let _ = events.send(ServerEvent::McpAppResult {
                    instance_slug: instance_slug.clone(),
                    chat_id: chat_id.clone(),
                    message_id: msg_id,
                    tool_output,
                });
            }
            if tool_name == "run_command" || tool_name == "interactive_session" {
                let output = match &result {
                    Ok(s) => s.clone(),
                    Err(e) => format!("error: {e}"),
                };
                if !output.is_empty() {
                    let output_msg = crate::domain::chat::ChatMessage {
                        id: format!("tool_{}_{}", tool_call_counter(), unix_millis()),
                        role: crate::domain::chat::ChatRole::Assistant,
                        content: output,
                        created_at: unix_millis().to_string(),
                        kind: crate::domain::chat::MessageKind::ToolOutput,
                        tool_name: Some(tool_name.clone()),
                        mcp_app_html: None,
                        mcp_app_input: None,
                        model: None,
                    };
                    let _ = events.send(ServerEvent::ChatMessageCreated {
                        instance_slug,
                        chat_id,
                        message: output_msg,
                    });
                }
            }
            result
        })
    }
}

fn configured_email_tools(
    email_accounts: Vec<crate::config::EmailConfig>,
) -> Vec<Box<dyn ToolDyn>> {
    if email_accounts.is_empty() {
        return Vec::new();
    }
    vec![
        Box::new(SendEmailTool::new(email_accounts.clone())),
        Box::new(ReadEmailTool::new(email_accounts)),
    ]
}

/// Build tools gated by category. Core is always loaded; others depend on triage.
pub fn build_tools(
    workspace_dir: &Path,
    instance_slug: &str,
    chat_id: &str,
    config_path: &Path,
    events: broadcast::Sender<ServerEvent>,
    llm: &crate::services::llm::LlmBackend,
    pending_secrets: Option<
        Arc<
            tokio::sync::Mutex<std::collections::HashMap<String, crate::app::state::PendingSecret>>,
        >,
    >,
    email_accounts: Vec<crate::config::EmailConfig>,
    sent_files: SentFiles,
    mcp_snapshot: Option<crate::services::mcp::McpAppSnapshot>,
    mcp_tools: Vec<Box<dyn ToolDyn>>,
    github_token: Option<String>,
    vector_store: Arc<crate::services::vector::VectorStore>,
    google_ai_key: &str,
    machine_registry: crate::services::machine_registry::MachineRegistry,
    public_url: &str,
    resources: &crate::services::resource_access::ResourceAccess,
) -> (Vec<Box<dyn ToolDyn>>, SentFiles) {
    let snap = mcp_snapshot;
    let wrap = |tool: Box<dyn ToolDyn>| -> Box<dyn ToolDyn> {
        Box::new(ObservableTool::new(
            tool,
            events.clone(),
            workspace_dir,
            instance_slug.to_string(),
            chat_id.to_string(),
            snap.clone(),
        ))
    };

    // ── Core ──
    let mut tools: Vec<Box<dyn ToolDyn>> = vec![
        wrap(Box::new(ReadFileTool::new(
            workspace_dir,
            instance_slug,
            public_url,
            resources,
        ))),
        wrap(Box::new(WriteFileTool::new(workspace_dir, instance_slug))),
        wrap(Box::new(EditFileTool::new(workspace_dir, instance_slug))),
        wrap(Box::new(UploadFileTool::new(
            workspace_dir,
            instance_slug,
            public_url,
            resources,
        ))),
        wrap(Box::new(ListFilesTool::new(workspace_dir, instance_slug))),
        wrap(Box::new(MemoryWriteTool::new(
            workspace_dir,
            instance_slug,
            vector_store.clone(),
        ))),
        wrap(Box::new(MemoryReadTool::new(
            workspace_dir,
            instance_slug,
            public_url,
            vector_store.clone(),
            resources,
        ))),
        wrap(Box::new(MemoryListTool::new(
            workspace_dir,
            instance_slug,
            vector_store.clone(),
        ))),
        wrap(Box::new(MemoryForgetTool::new(
            workspace_dir,
            instance_slug,
            vector_store.clone(),
        ))),
        wrap(Box::new(MemorySearchTool::new(
            workspace_dir,
            instance_slug,
            vector_store.clone(),
            public_url,
            resources,
        ))),
        wrap(Box::new(MemoryConnectTool::new(
            instance_slug,
            vector_store.clone(),
        ))),
        // Mood is managed by background sentiment extraction + heartbeat, not tools.
        wrap(Box::new(EditSoulTool::new(workspace_dir, instance_slug))),
        wrap(Box::new(SetVoiceTool::new(workspace_dir, instance_slug))),
        wrap(Box::new(RunCommandTool::new(
            workspace_dir,
            instance_slug,
            chat_id,
            events.clone(),
            github_token,
        ))),
        wrap(Box::new(ClearContextTool::new(
            workspace_dir,
            instance_slug,
            chat_id,
            events.clone(),
        ))),
    ];

    // ── System ──
    tools.push(wrap(Box::new(InteractiveSessionTool::new(
        workspace_dir,
        instance_slug,
    ))));
    // send_file removed — images from tool results are auto-attached (see llm.rs)
    tools.push(wrap(Box::new(GetTimeTool::new(
        workspace_dir,
        instance_slug,
    ))));
    tools.push(wrap(Box::new(GetSettingsTool::new(
        config_path,
        workspace_dir,
        instance_slug,
    ))));
    tools.push(wrap(Box::new(UpdateConfigTool::new(
        config_path,
        workspace_dir,
        instance_slug,
    ))));
    if let Some(ps) = pending_secrets {
        tools.push(wrap(Box::new(RequestSecretTool::new(
            workspace_dir,
            instance_slug,
            config_path,
            events.clone(),
            ps,
        ))));
    }

    // ── Skills ──
    tools.push(wrap(Box::new(ListSkillsTool::new(
        workspace_dir,
        &llm.api_key,
    ))));
    tools.push(wrap(Box::new(ActivateSkillTool::new(
        workspace_dir,
        &llm.api_key,
    ))));
    tools.push(wrap(Box::new(ReadSkillReferenceTool::new(workspace_dir))));

    // ── Web ──
    // web_search and web_fetch are native Anthropic server tools (added in llm.rs)
    tools.push(wrap(Box::new(ViewImageTool)));
    {
        tools.push(wrap(Box::new(WatchVideoTool::new(
            google_ai_key,
            workspace_dir,
            instance_slug,
            public_url,
            resources,
        ))));
    }
    // ── Agents ──
    tools.push(wrap(Box::new(CallAgentTool::new(
        workspace_dir,
        instance_slug,
        llm.clone(),
        events.clone(),
        vector_store.clone(),
        google_ai_key,
        resources,
    ))));

    // ── Creative ──
    tools.push(wrap(Box::new(CreateDropTool::new(
        workspace_dir,
        instance_slug,
        events.clone(),
    ))));
    // schedule_agent merged into call_agent (delay_seconds param)

    // ── Data ──
    tools.push(wrap(Box::new(ExportProfileTool::new(
        workspace_dir,
        instance_slug,
        events.clone(),
    ))));
    tools.push(wrap(Box::new(ImportProfileTool::new(
        workspace_dir,
        instance_slug,
        vector_store.clone(),
    ))));
    // ── Email (SMTP/IMAP) ──
    for email_tool in configured_email_tools(email_accounts) {
        tools.push(wrap(email_tool));
    }

    // ── Computer use (multi-machine routing) ──
    tools.push(wrap(Box::new(ListMachinesTool::new(
        machine_registry.clone(),
    ))));
    {
        tools.push(wrap(Box::new(ComputerUseTool::new(
            machine_registry.clone(),
            workspace_dir,
            instance_slug,
            public_url,
            resources,
        ))));
    }
    tools.push(wrap(Box::new(RemoteBashTool::new(
        machine_registry.clone(),
    ))));
    tools.push(wrap(Box::new(RemoteFilesTool::new(machine_registry))));

    // MCP tools
    for mcp_tool in mcp_tools {
        tools.push(wrap(mcp_tool));
    }

    log::info!("built {} tools", tools.len());
    (tools, sent_files)
}

// ---------------------------------------------------------------------------
// Shared error
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct ToolExecError(pub(crate) String);

impl fmt::Display for ToolExecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for ToolExecError {}

// ---------------------------------------------------------------------------
// Helpers for tool activity persistence
// ---------------------------------------------------------------------------

use std::sync::atomic::{AtomicU64, Ordering};

static TOOL_CALL_COUNTER: AtomicU64 = AtomicU64::new(0);

fn tool_call_counter() -> u64 {
    TOOL_CALL_COUNTER.fetch_add(1, Ordering::Relaxed)
}

pub fn unix_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time after epoch")
        .as_millis()
}

/// Shared collector for file attachments produced by send_file during a turn.
pub type SentFiles = std::sync::Arc<std::sync::Mutex<Vec<String>>>;

#[cfg(test)]
mod email_tool_tests {
    use super::*;

    #[test]
    fn configured_smtp_imap_builds_send_and_read_email_tools() {
        let account = crate::config::EmailConfig {
            smtp_host: "smtp.example.com".into(),
            smtp_user: "user@example.com".into(),
            smtp_from: "user@example.com".into(),
            imap_host: "imap.example.com".into(),
            imap_user: "user@example.com".into(),
            ..Default::default()
        };
        let names: Vec<_> = configured_email_tools(vec![account])
            .into_iter()
            .map(|tool| tool.name())
            .collect();
        assert_eq!(names, ["send_email", "read_email"]);
    }

    #[test]
    fn unconfigured_email_builds_no_email_tools() {
        assert!(configured_email_tools(Vec::new()).is_empty());
    }

    #[test]
    fn short_control_token_redacts_text_but_not_scoped_capability_values() {
        let value = replace_exact_secret(
            "prompt v1 /resources/browser/files/moon/id?cap=abc.v1.xyz&note=v1 token=v1 Bearer v1",
            "v1",
            &["abc.v1.xyz".into()],
        );
        assert_eq!(value.matches("v1").count(), 1);
        assert!(value.contains("cap=abc.v1.xyz"));
        assert!(!value.contains("note=v1"));
        assert!(!value.contains("token=v1"));
        assert!(!value.contains("Bearer v1"));
    }

    #[test]
    fn forged_scoped_cap_equal_to_control_token_is_redacted() {
        let secret = "issue116-forged-cap-secret";
        let value = replace_exact_secret(
            &format!("/resources/model-provider/files/moon/id?cap={secret}"),
            secret,
            &[],
        );
        assert_eq!(
            value,
            "/resources/model-provider/files/moon/id?cap=[REDACTED]"
        );
    }

    #[test]
    fn forged_secret_variants_are_fully_redacted() {
        let secret = "control-secret";
        let text = format!(
            "/resources/browser/files/moon/id?cap={secret} cap=x{secret}x Bearer {secret}X arbitrary={secret}"
        );
        let value = replace_exact_secret(&text, secret, &[]);
        assert!(!value.contains(secret));
        assert_eq!(value.matches("[REDACTED]").count(), 4);
    }

    #[test]
    fn only_exact_registered_cap_values_are_exempt() {
        let registered = "abc.v1.xyz".to_string();
        let text = "cap=abc.v1.xyz cap=xabc.v1.xyz cap=abc.v1.xyzX Bearer abc.v1.xyz";
        let value = replace_exact_secret(text, "v1", &[registered]);
        assert_eq!(
            value,
            "cap=abc.v1.xyz cap=xabc.[REDACTED].xyz cap=abc.[REDACTED].xyzX Bearer abc.[REDACTED].xyz"
        );
    }

    #[test]
    fn registered_capability_is_byte_exact_even_when_it_contains_a_secret_pattern() {
        let registered = format!("abc.sk-{}.xyz", "A".repeat(24));
        let text = format!("cap={registered} forged={registered}");
        let regex = Regex::new(r"sk-[A-Za-z0-9]{20,}").unwrap();
        let value = replace_regex_preserving_capabilities(&text, &regex, &[registered.clone()]);
        assert!(value.starts_with(&format!("cap={registered}")));
        assert_eq!(value.matches(&registered).count(), 1);
        assert!(value.ends_with("forged=abc.[REDACTED].xyz"));
    }

    #[test]
    fn capability_registry_is_bounded_deduplicated_fifo() {
        let mut registry = CapabilityTokenRegistry::default();
        for index in 0..=MAX_CAPABILITY_TOKENS {
            registry.register(&format!("cap-{index}"));
        }
        assert_eq!(registry.tokens.len(), MAX_CAPABILITY_TOKENS);
        assert!(!registry.tokens.iter().any(|token| token == "cap-0"));
        registry.register("cap-1");
        registry.register("cap-new");
        assert_eq!(registry.tokens.len(), MAX_CAPABILITY_TOKENS);
        assert_eq!(registry.tokens.back().map(String::as_str), Some("cap-new"));
        assert!(registry.tokens.iter().any(|token| token == "cap-1"));
        assert!(!registry.tokens.iter().any(|token| token == "cap-2"));
    }

    #[test]
    fn capability_registry_is_parallel_safe() {
        let registry = Arc::new(Mutex::new(CapabilityTokenRegistry::default()));
        std::thread::scope(|scope| {
            for worker in 0..8 {
                let registry = registry.clone();
                scope.spawn(move || {
                    for index in 0..1024 {
                        registry
                            .lock()
                            .unwrap()
                            .register(&format!("worker-{worker}-cap-{index}"));
                    }
                });
            }
        });
        let registry = registry.lock().unwrap();
        assert_eq!(registry.tokens.len(), MAX_CAPABILITY_TOKENS);
        let unique = registry
            .tokens
            .iter()
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(unique.len(), registry.tokens.len());
    }

    #[test]
    fn control_secret_rotations_are_bounded() {
        let mut registry = ControlSecretRegistry::default();
        for index in 0..=MAX_CONTROL_SECRETS {
            registry.register(&format!("rotation-secret-{index}"));
        }
        assert_eq!(registry.secrets.len(), MAX_CONTROL_SECRETS);
        assert!(
            !registry
                .secrets
                .iter()
                .any(|value| value == "rotation-secret-0")
        );
        assert!(
            registry
                .secrets
                .iter()
                .any(|value| value == "rotation-secret-1")
        );
        assert!(
            registry
                .secrets
                .iter()
                .any(|value| value == "rotation-secret-64")
        );

        registry.register("rotation-secret-1");
        registry.register("rotation-secret-65");
        assert_eq!(registry.secrets.len(), MAX_CONTROL_SECRETS);
        assert_eq!(registry.secrets.back().unwrap(), "rotation-secret-65");
        assert!(
            registry
                .secrets
                .iter()
                .any(|value| value == "rotation-secret-1")
        );
        assert!(
            !registry
                .secrets
                .iter()
                .any(|value| value == "rotation-secret-2")
        );
    }
}
