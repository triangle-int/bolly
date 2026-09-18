use std::{
    collections::HashMap,
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

/// Build a public file URL, omitting `?token=` when the token is empty.
pub fn public_file_url(base: &str, instance_slug: &str, file_id: &str, token: &str) -> String {
    if token.is_empty() {
        format!("{base}/public/files/{instance_slug}/{file_id}")
    } else {
        format!("{base}/public/files/{instance_slug}/{file_id}?token={token}")
    }
}

/// Build a public memory URL, omitting `?token=` when the token is empty.
pub fn public_memory_url(base: &str, instance_slug: &str, path: &str, token: &str) -> String {
    if token.is_empty() {
        format!("{base}/public/memory/{instance_slug}/{path}")
    } else {
        format!("{base}/public/memory/{instance_slug}/{path}?token={token}")
    }
}

// Sub-modules
pub mod calendar;
pub mod communication;
pub mod companion;
pub mod drive;
pub mod files;
pub mod memory_tools;
pub mod project;
pub mod skills;
pub mod system;
pub mod image;
pub mod import_data;
pub mod media;
pub mod computer;
pub mod screen;

// Re-export public items so external code uses `tools::FooTool` paths
pub use calendar::{CreateEventTool, ListEventsTool};
pub use communication::{
    ReachOutTool, ReadEmailTool, ScheduledTask, SendEmailTool,
};
pub use companion::{
    ALLOWED_MOODS, EditSoulTool, SetVoiceTool,
    get_voice_override, load_mood_state, save_mood_state,
};
pub use drive::{ListDriveFilesTool, ReadDriveFileTool, UploadDriveFileTool};
pub use files::{EditFileTool, ListFilesTool, ReadFileTool, UploadFileTool, WriteFileTool};
pub use memory_tools::{MemoryConnectTool, MemoryForgetTool, MemoryListTool, MemoryReadTool, MemorySearchTool, MemoryWriteTool};
pub use project::{TaskItem, TaskStatus};
pub use skills::{ActivateSkillTool, ListSkillsTool, ReadSkillReferenceTool};
pub use system::{
    CallAgentTool, ClearContextTool, CreateDropTool, ExportProfileTool,
    GetSettingsTool, GetTimeTool, ImportProfileTool, InteractiveSessionTool, RequestSecretTool,
    RestartMachineTool, RunCommandTool, UpdateConfigTool,
};
pub use image::ViewImageTool;
pub use media::WatchVideoTool;
pub use computer::{ListMachinesTool, ComputerUseTool, RemoteBashTool, RemoteFilesTool};

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
            // Note: BOLLY_AUTH_TOKEN excluded — it's used in image/document URLs
            // that Anthropic needs to fetch. Redacting it breaks content blocks.
            "BOLLY_RELEASE_TOKEN",
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

    let mut result = text.to_string();

    for pat in &patterns {
        if let Ok(re) = Regex::new(pat) {
            result = re.replace_all(&result, "[REDACTED]").to_string();
        }
    }

    for secret in secret_values() {
        result = result.replace(secret.as_str(), "[REDACTED]");
    }

    result
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
        "run_command" => {
            let cmd = v["command"].as_str().unwrap_or("?");
            format!("$ {cmd}")
        }
        "edit_soul" => "rewriting soul.md".into(),
        "set_mood" => format!("mood → {}", v["mood"].as_str().unwrap_or("?")),
        "remember" => "storing a memory".into(),
        "recall" => format!("recalling '{}'", v["query"].as_str().unwrap_or("?")),
        "web_search" => format!("web search: {}", v["query"].as_str().unwrap_or("?")),
        "web_fetch" => format!("fetching {}", v["url"].as_str().unwrap_or("?")),
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
        "list_events" => {
            let days = v["days_ahead"].as_u64().unwrap_or(7);
            format!("listing calendar events ({days} days)")
        }
        "create_event" => format!("creating event: {}", v["summary"].as_str().unwrap_or("?")),
        "list_drive_files" => "listing drive files".into(),
        "read_drive_file" => format!(
            "reading drive file {}",
            v["file_id"].as_str().unwrap_or("?")
        ),
        "upload_drive_file" => format!("uploading {}", v["name"].as_str().unwrap_or("?")),
        "set_voice" => {
            let vid = v["voice_id"].as_str().unwrap_or("");
            if vid.is_empty() { "resetting voice to default".into() } else { format!("voice → {vid}") }
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
        let summary = tool_summary(&tool_name, &args);

        let start_msg = crate::domain::chat::ChatMessage {
            id: format!("tool_{}_{}", tool_call_counter(), unix_millis()),
            role: crate::domain::chat::ChatRole::Assistant,
            content: summary.clone(),
            created_at: unix_millis().to_string(),
            kind: crate::domain::chat::MessageKind::ToolCall,
            tool_name: Some(tool_name.clone()),
            mcp_app_html: None,
            mcp_app_input: None, model: None,
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
                        mcp_app_html: Some(html),
                        mcp_app_input: Some(args.clone()),
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
                Err(e) => Err(e),
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
            if tool_name == "run_command"
                || tool_name == "interactive_session"
            {
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
                        mcp_app_input: None, model: None,
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
    _plan: &str,
    google: Option<crate::services::google::GoogleClient>,
    email_accounts: Vec<crate::config::EmailConfig>,
    sent_files: SentFiles,
    mcp_snapshot: Option<crate::services::mcp::McpAppSnapshot>,
    mcp_tools: Vec<Box<dyn ToolDyn>>,
    github_token: Option<String>,
    vector_store: Arc<crate::services::vector::VectorStore>,
    google_ai_key: &str,
    machine_registry: crate::services::machine_registry::MachineRegistry,
    public_url: &str,
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
        wrap(Box::new(ReadFileTool::new(workspace_dir, instance_slug, public_url))),
        wrap(Box::new(WriteFileTool::new(workspace_dir, instance_slug))),
        wrap(Box::new(EditFileTool::new(workspace_dir, instance_slug))),
        wrap(Box::new(UploadFileTool::new(workspace_dir, instance_slug, public_url))),
        wrap(Box::new(ListFilesTool::new(workspace_dir, instance_slug))),
        wrap(Box::new(MemoryWriteTool::new(workspace_dir, instance_slug, vector_store.clone(), google_ai_key))),
        wrap(Box::new(MemoryReadTool::new(workspace_dir, instance_slug, public_url))),
        wrap(Box::new(MemoryListTool::new(workspace_dir, instance_slug))),
        wrap(Box::new(MemoryForgetTool::new(workspace_dir, instance_slug, vector_store.clone(), google_ai_key))),
        wrap(Box::new(MemorySearchTool::new(workspace_dir, instance_slug, vector_store.clone(), google_ai_key, public_url))),
        wrap(Box::new(MemoryConnectTool::new(workspace_dir, instance_slug))),
        // Mood is managed by background sentiment extraction + heartbeat, not tools.
        wrap(Box::new(EditSoulTool::new(workspace_dir, instance_slug))),
        wrap(Box::new(SetVoiceTool::new(workspace_dir, instance_slug))),
        wrap(Box::new(RunCommandTool::new(workspace_dir, instance_slug, chat_id, events.clone(), github_token))),
        wrap(Box::new(ClearContextTool::new(workspace_dir, instance_slug, chat_id, events.clone()))),
    ];

    // ── System ──
    tools.push(wrap(Box::new(InteractiveSessionTool::new(workspace_dir, instance_slug))));
    // send_file removed — images from tool results are auto-attached (see llm.rs)
    tools.push(wrap(Box::new(GetTimeTool::new(workspace_dir, instance_slug))));
    tools.push(wrap(Box::new(GetSettingsTool::new(config_path, workspace_dir, instance_slug, google.clone()))));
    tools.push(wrap(Box::new(UpdateConfigTool::new(config_path, workspace_dir, instance_slug, machine_registry.clone()))));
    if let Some(ps) = pending_secrets {
        tools.push(wrap(Box::new(RequestSecretTool::new(
            workspace_dir, instance_slug, config_path, events.clone(), ps,
        ))));
    }

    // ── Machine management ──
    tools.push(wrap(Box::new(RestartMachineTool)));

    // ── Skills ──
    tools.push(wrap(Box::new(ListSkillsTool::new(workspace_dir, &llm.api_key))));
    tools.push(wrap(Box::new(ActivateSkillTool::new(workspace_dir, &llm.api_key))));
    tools.push(wrap(Box::new(ReadSkillReferenceTool::new(workspace_dir))));

    // ── Web ──
    // web_search and web_fetch are native Anthropic server tools (added in llm.rs)
    tools.push(wrap(Box::new(ViewImageTool)));
    {
        let cfg = crate::config::load_config().ok();
        let auth_token = cfg.as_ref().map(|c| c.auth_token.as_str()).unwrap_or("");
        tools.push(wrap(Box::new(WatchVideoTool::new(
            google_ai_key, workspace_dir, instance_slug, public_url, auth_token,
        ))));
    }
    // ── Agents ──
    tools.push(wrap(Box::new(CallAgentTool::new(
        workspace_dir, instance_slug, llm.clone(), events.clone(), vector_store.clone(), google_ai_key,
    ))));

    // ── Creative ──
    tools.push(wrap(Box::new(CreateDropTool::new(workspace_dir, instance_slug, events.clone()))));
    // schedule_agent merged into call_agent (delay_seconds param)

    // ── Data ──
    tools.push(wrap(Box::new(ExportProfileTool::new(workspace_dir, instance_slug, events.clone()))));
    tools.push(wrap(Box::new(ImportProfileTool::new(workspace_dir, instance_slug))));
    {
        let api_key = llm.api_key.clone();
        tools.push(wrap(Box::new(import_data::ImportDataTool::new(
            workspace_dir, instance_slug, llm.http.clone(), &api_key,
            events.clone(), vector_store.clone(), google_ai_key,
        ))));
    }

    // ── Email (unified: Gmail + SMTP/IMAP) ──
    let has_email = google.is_some() || !email_accounts.is_empty();
    if has_email {
        tools.push(wrap(Box::new(SendEmailTool::new(google.clone(), instance_slug, email_accounts.clone()))));
        tools.push(wrap(Box::new(ReadEmailTool::new(google.clone(), instance_slug, email_accounts))));
    }

    // ── Google (calendar, drive) ──
    if let Some(g) = google {
        tools.push(wrap(Box::new(ListEventsTool::new(g.clone(), instance_slug))));
        tools.push(wrap(Box::new(CreateEventTool::new(g.clone(), instance_slug))));
        tools.push(wrap(Box::new(ListDriveFilesTool::new(g.clone(), instance_slug))));
        tools.push(wrap(Box::new(ReadDriveFileTool::new(g.clone(), instance_slug))));
        tools.push(wrap(Box::new(UploadDriveFileTool::new(g, instance_slug))));
    }


    // ── Computer use (multi-machine routing) ──
    tools.push(wrap(Box::new(ListMachinesTool::new(machine_registry.clone()))));
    {
        let cfg = crate::config::load_config().ok();
        let auth_token = cfg.as_ref().map(|c| c.auth_token.as_str()).unwrap_or("");
        tools.push(wrap(Box::new(ComputerUseTool::new(
            machine_registry.clone(), workspace_dir, instance_slug, public_url, auth_token,
        ))));
    }
    tools.push(wrap(Box::new(RemoteBashTool::new(machine_registry.clone()))));
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

/// Append a single message to a chat's messages.json with file locking.
// append_message_to_chat and update_mcp_app_result removed — rig_history.json is the single source of truth

/// Shared collector for file attachments produced by send_file during a turn.
pub type SentFiles = std::sync::Arc<std::sync::Mutex<Vec<String>>>;
