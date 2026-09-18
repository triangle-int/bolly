use base64::Engine;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::services::machine_registry::{AgentToolCall, MachineRegistry};
use crate::services::tool::{Tool, ToolDefinition};
use crate::services::tools::{ToolExecError, openai_schema};

// ═══════════════════════════════════════════════════════════════════════════
// list_machines — returns connected Tauri agents
// ═══════════════════════════════════════════════════════════════════════════

pub struct ListMachinesTool {
    registry: MachineRegistry,
}

impl ListMachinesTool {
    pub fn new(registry: MachineRegistry) -> Self {
        Self { registry }
    }
}

#[derive(Deserialize, JsonSchema)]
pub struct ListMachinesArgs {}

impl Tool for ListMachinesTool {
    const NAME: &'static str = "list_machines";
    type Error = ToolExecError;
    type Args = ListMachinesArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "list_machines".into(),
            description:
                "List all connected desktop machines that you can control via computer use. \
                Returns machine IDs, OS, hostname, and screen dimensions. \
                Use a machine_id from this list when calling computer_use."
                    .into(),
            parameters: openai_schema::<ListMachinesArgs>(),
        }
    }

    async fn call(&self, _args: Self::Args) -> Result<Self::Output, Self::Error> {
        let machines = self.registry.list().await;
        if machines.is_empty() {
            return Ok(
                "No machines connected. The user needs to open the Nolune desktop app first."
                    .into(),
            );
        }
        let info: Vec<serde_json::Value> = machines
            .iter()
            .map(|m| {
                serde_json::json!({
                    "machine_id": m.machine_id,
                    "os": m.os,
                    "hostname": m.hostname,
                    "screen": format!("{}x{}", m.screen_width, m.screen_height),
                })
            })
            .collect();
        serde_json::to_string_pretty(&info).map_err(|e| ToolExecError(e.to_string()))
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// computer_use — route action to a specific machine agent
// ═══════════════════════════════════════════════════════════════════════════

pub struct ComputerUseTool {
    registry: MachineRegistry,
    workspace_dir: std::path::PathBuf,
    instance_slug: String,
    public_url: String,
    auth_token: String,
}

impl ComputerUseTool {
    pub fn new(
        registry: MachineRegistry,
        workspace_dir: &std::path::Path,
        instance_slug: &str,
        public_url: &str,
        auth_token: &str,
    ) -> Self {
        Self {
            registry,
            workspace_dir: workspace_dir.to_path_buf(),
            instance_slug: instance_slug.to_string(),
            public_url: public_url.to_string(),
            auth_token: auth_token.to_string(),
        }
    }
}

#[derive(Deserialize, JsonSchema)]
pub struct ComputerUseArgs {
    /// ID of the machine to control (from list_machines).
    pub machine_id: String,
    /// Action to perform: "screenshot", "left_click", "right_click", "middle_click",
    /// "double_click", "mouse_move", "type", "key", "scroll".
    pub action: String,
    /// [x, y] coordinates for click/move/scroll actions (in screen pixels).
    #[serde(default)]
    pub coordinate: Option<[i32; 2]>,
    /// Text to type (for "type" action).
    #[serde(default)]
    pub text: Option<String>,
    /// Key or key combination to press (for "key" action, e.g. "ctrl+c", "Return").
    #[serde(default)]
    pub key: Option<String>,
    /// Scroll direction: "up", "down", "left", "right".
    #[serde(default)]
    pub scroll_direction: Option<String>,
    /// Number of scroll clicks (default 3).
    #[serde(default)]
    pub scroll_amount: Option<i32>,
}

impl Tool for ComputerUseTool {
    const NAME: &'static str = "computer_use";
    type Error = ToolExecError;
    type Args = ComputerUseArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "computer_use".into(),
            description: "Control a connected desktop machine — take screenshots, click, type, press keys, scroll. \
                Always take a screenshot first to see the current state. \
                Coordinates are in the screenshot's pixel space. \
                Available actions: screenshot, left_click, right_click, middle_click, double_click, \
                mouse_move, type, key, scroll, switch_desktop. \
                \n\nmacOS tips: \
                - Switch desktop/Space: use action 'switch_desktop' with scroll_direction 'left' or 'right'. \
                - Mission Control: key 'ctrl+up'. \
                - App Exposé: key 'ctrl+down'. \
                - Spotlight: key 'cmd+space'. \
                - Close window: key 'cmd+w'. \
                - Quit app: key 'cmd+q'. \
                - Switch app: key 'cmd+tab'."
                .into(),
            parameters: openai_schema::<ComputerUseArgs>(),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let request_id = uuid::Uuid::new_v4().to_string();

        let mut params = serde_json::json!({});
        if let Some(c) = &args.coordinate {
            params["coordinate"] = serde_json::json!(c);
        }
        if let Some(t) = &args.text {
            params["text"] = serde_json::json!(t);
        }
        if let Some(k) = &args.key {
            params["key"] = serde_json::json!(k);
        }
        if let Some(d) = &args.scroll_direction {
            params["scroll_direction"] = serde_json::json!(d);
        }
        if let Some(a) = &args.scroll_amount {
            params["scroll_amount"] = serde_json::json!(a);
        }

        let call = AgentToolCall {
            request_id: request_id.clone(),
            action: args.action.clone(),
            params,
        };

        log::info!(
            "[computer_use] {} on machine '{}' (req={})",
            args.action,
            args.machine_id,
            &request_id[..8]
        );

        let result = self
            .registry
            .execute(&args.machine_id, call)
            .await
            .map_err(|e| ToolExecError(e))?;

        match result.result_type.as_str() {
            "screenshot" => {
                let image_b64 = result.image.unwrap_or_default();
                let w = result.width.unwrap_or(0);
                let h = result.height.unwrap_or(0);

                // Save screenshot as upload file
                let saved = base64::engine::general_purpose::STANDARD
                    .decode(&image_b64)
                    .ok()
                    .and_then(|bytes| {
                        crate::services::uploads::save_upload(
                            &self.workspace_dir,
                            &self.instance_slug,
                            "screenshot.jpg",
                            &bytes,
                        )
                        .ok()
                    });

                if let Some(meta) = saved {
                    // URL-based image — no base64 in context, no truncation, no context bloat
                    let full_url = super::public_file_url(
                        &self.public_url,
                        &self.instance_slug,
                        &meta.id,
                        &self.auth_token,
                    );
                    let chat_url = format!(
                        "/api/instances/{}/uploads/{}/file",
                        self.instance_slug, meta.id
                    );

                    let blocks = serde_json::json!([
                        {
                            "type": "image",
                            "source": {
                                "type": "url",
                                "url": full_url,
                            }
                        },
                        {
                            "type": "text",
                            "text": format!(
                                "Screenshot captured ({}x{}). Show to user: ![screenshot]({})",
                                w, h, chat_url
                            ),
                        }
                    ]);
                    Ok(blocks.to_string())
                } else {
                    Err(ToolExecError("failed to save screenshot".into()))
                }
            }
            "action" => {
                if result.success.unwrap_or(false) {
                    Ok(format!("Action '{}' executed successfully.", args.action))
                } else {
                    let err = result.error.unwrap_or_else(|| "unknown error".to_string());
                    Err(ToolExecError(format!(
                        "Action '{}' failed: {}",
                        args.action, err
                    )))
                }
            }
            // bash/file results return output as text
            "output" => {
                let output = result.error.unwrap_or_default(); // reuse error field for output text
                Ok(output)
            }
            other => Err(ToolExecError(format!("unexpected result type: {other}"))),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// remote_bash — run a shell command on a connected machine
// ═══════════════════════════════════════════════════════════════════════════

pub struct RemoteBashTool {
    registry: MachineRegistry,
}

impl RemoteBashTool {
    pub fn new(registry: MachineRegistry) -> Self {
        Self { registry }
    }
}

#[derive(Deserialize, JsonSchema)]
pub struct RemoteBashArgs {
    /// ID of the machine (from list_machines).
    pub machine_id: String,
    /// Shell command to execute.
    pub command: String,
    /// Working directory (optional, defaults to home).
    #[serde(default)]
    pub cwd: Option<String>,
}

impl Tool for RemoteBashTool {
    const NAME: &'static str = "remote_bash";
    type Error = ToolExecError;
    type Args = RemoteBashArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "remote_bash".into(),
            description: "Execute a shell command on a connected desktop machine. \
                Returns stdout+stderr. Use for installing software, running scripts, \
                checking system state, etc. Commands run in the user's shell."
                .into(),
            parameters: openai_schema::<RemoteBashArgs>(),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let request_id = uuid::Uuid::new_v4().to_string();
        let call = AgentToolCall {
            request_id: request_id.clone(),
            action: "bash".into(),
            params: serde_json::json!({
                "command": args.command,
                "cwd": args.cwd,
            }),
        };

        log::info!("[remote_bash] '{}' on '{}'", args.command, args.machine_id);

        let result = self
            .registry
            .execute(&args.machine_id, call)
            .await
            .map_err(|e| ToolExecError(e))?;

        if result.success.unwrap_or(false) {
            Ok(result.error.unwrap_or_default()) // output in error field
        } else {
            let err = result.error.unwrap_or_else(|| "command failed".into());
            Err(ToolExecError(err))
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// remote_files — read/write/list files on a connected machine
// ═══════════════════════════════════════════════════════════════════════════

pub struct RemoteFilesTool {
    registry: MachineRegistry,
}

impl RemoteFilesTool {
    pub fn new(registry: MachineRegistry) -> Self {
        Self { registry }
    }
}

#[derive(Deserialize, JsonSchema)]
pub struct RemoteFilesArgs {
    /// ID of the machine (from list_machines).
    pub machine_id: String,
    /// Operation: "read", "write", "list".
    pub operation: String,
    /// File or directory path.
    pub path: String,
    /// Content to write (only for "write" operation).
    #[serde(default)]
    pub content: Option<String>,
}

impl Tool for RemoteFilesTool {
    const NAME: &'static str = "remote_files";
    type Error = ToolExecError;
    type Args = RemoteFilesArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "remote_files".into(),
            description: "Read, write, or list files on a connected desktop machine. \
                Operations: 'read' returns file content, 'write' creates/overwrites a file, \
                'list' returns directory listing. Paths can be absolute or ~ for home."
                .into(),
            parameters: openai_schema::<RemoteFilesArgs>(),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let request_id = uuid::Uuid::new_v4().to_string();
        let call = AgentToolCall {
            request_id: request_id.clone(),
            action: format!("file_{}", args.operation),
            params: serde_json::json!({
                "path": args.path,
                "content": args.content,
            }),
        };

        log::info!(
            "[remote_files] {} '{}' on '{}'",
            args.operation,
            args.path,
            args.machine_id
        );

        let result = self
            .registry
            .execute(&args.machine_id, call)
            .await
            .map_err(|e| ToolExecError(e))?;

        if result.success.unwrap_or(false) {
            Ok(result.error.unwrap_or_default()) // output in error field
        } else {
            let err = result
                .error
                .unwrap_or_else(|| "file operation failed".into());
            Err(ToolExecError(err))
        }
    }
}
