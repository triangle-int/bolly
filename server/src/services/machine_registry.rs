use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, oneshot};

/// Info about a connected Tauri agent machine.
#[derive(Clone, Debug, Serialize)]
pub struct MachineInfo {
    pub machine_id: String,
    pub os: String,
    pub hostname: String,
    pub screen_width: u32,
    pub screen_height: u32,
    pub last_seen: i64,
    /// Whether this machine allows screen recording for observation.
    pub screen_recording_allowed: bool,
    /// Instance this machine is bound to (if any).
    pub instance_slug: Option<String>,
}

/// A pending computer use request waiting for an agent to respond.
pub struct PendingAction {
    pub responder: oneshot::Sender<ActionResult>,
}

/// Result from a Tauri agent executing a computer use action.
#[derive(Debug, Clone, Deserialize)]
pub struct ActionResult {
    /// "screenshot" or "action"
    pub result_type: String,
    /// Base64 PNG (only for screenshots)
    pub image: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    #[allow(dead_code)]
    pub scale: Option<f64>,
    /// For action results
    pub success: Option<bool>,
    pub error: Option<String>,
}

/// Message sent to a connected agent over WebSocket.
/// Generic toolcall message — action + arbitrary params.
#[derive(Debug, Clone, Serialize)]
pub struct AgentToolCall {
    pub request_id: String,
    pub action: String,
    /// Action-specific parameters (flattened into the JSON).
    #[serde(flatten)]
    pub params: serde_json::Value,
}

/// Channel to send toolcalls to a connected agent.
type AgentSender = tokio::sync::mpsc::UnboundedSender<String>;

/// A captured screen frame.
#[derive(Clone)]
pub struct ScreenFrame {
    pub jpeg: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub timestamp: i64,
}

/// Ring buffer of recent screen frames per machine.
const MAX_FRAMES: usize = 900; // 15 min at 1fps

/// Registry of connected Tauri agent machines.
#[derive(Clone)]
pub struct MachineRegistry {
    /// Connected agents: machine_id → (info, sender)
    agents: Arc<Mutex<HashMap<String, (MachineInfo, AgentSender)>>>,
    /// Pending action requests: request_id → oneshot sender
    pending: Arc<Mutex<HashMap<String, PendingAction>>>,
    /// Screen frame buffers: machine_id → ring buffer of frames
    frames: Arc<Mutex<HashMap<String, Vec<ScreenFrame>>>>,
    /// Latest frame per machine (for live preview)
    latest_frame: Arc<Mutex<HashMap<String, ScreenFrame>>>,
}

impl MachineRegistry {
    pub fn new() -> Self {
        Self {
            agents: Arc::new(Mutex::new(HashMap::new())),
            pending: Arc::new(Mutex::new(HashMap::new())),
            frames: Arc::new(Mutex::new(HashMap::new())),
            latest_frame: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Register a new agent connection.
    pub async fn register(&self, info: MachineInfo, sender: AgentSender) {
        let id = info.machine_id.clone();
        log::info!("[machines] registered: {} ({})", id, info.os);
        self.agents.lock().await.insert(id, (info, sender));
    }

    /// Remove a disconnected agent.
    pub async fn unregister(&self, machine_id: &str) {
        log::info!("[machines] unregistered: {machine_id}");
        self.agents.lock().await.remove(machine_id);
    }

    /// Update last_seen timestamp.
    pub async fn heartbeat(&self, machine_id: &str) {
        if let Some((info, _)) = self.agents.lock().await.get_mut(machine_id) {
            info.last_seen = chrono::Utc::now().timestamp();
        }
    }

    /// Push a screen frame into the ring buffer.
    pub async fn push_frame(&self, machine_id: &str, frame: ScreenFrame) {
        // Update latest
        self.latest_frame
            .lock()
            .await
            .insert(machine_id.to_string(), frame.clone());

        // Push to ring buffer
        let mut buffers = self.frames.lock().await;
        let buf = buffers
            .entry(machine_id.to_string())
            .or_insert_with(Vec::new);
        buf.push(frame);
        // Trim to max size
        if buf.len() > MAX_FRAMES {
            let excess = buf.len() - MAX_FRAMES;
            buf.drain(..excess);
        }
    }

    /// Get the latest frame for a machine.
    pub async fn get_latest_frame(&self, machine_id: &str) -> Option<ScreenFrame> {
        self.latest_frame.lock().await.get(machine_id).cloned()
    }

    /// Take all buffered frames for a machine (clears the buffer).
    pub async fn take_frames(&self, machine_id: &str) -> Vec<ScreenFrame> {
        let mut buffers = self.frames.lock().await;
        buffers.remove(machine_id).unwrap_or_default()
    }

    /// Get the count of buffered frames.
    pub async fn frame_count(&self, machine_id: &str) -> usize {
        self.frames
            .lock()
            .await
            .get(machine_id)
            .map(|v| v.len())
            .unwrap_or(0)
    }

    /// List all connected machines.
    pub async fn list(&self) -> Vec<MachineInfo> {
        self.agents
            .lock()
            .await
            .values()
            .map(|(info, _)| info.clone())
            .collect()
    }

    /// Send a toolcall to a specific machine and wait for the result.
    pub async fn execute(
        &self,
        machine_id: &str,
        call: AgentToolCall,
    ) -> Result<ActionResult, String> {
        let request_id = call.request_id.clone();

        // Find the agent's sender
        let sender = {
            let agents = self.agents.lock().await;
            agents
                .get(machine_id)
                .map(|(_, s)| s.clone())
                .ok_or_else(|| format!("machine '{machine_id}' not connected"))?
        };

        // Create oneshot for the response
        let (tx, rx) = oneshot::channel();
        self.pending
            .lock()
            .await
            .insert(request_id.clone(), PendingAction { responder: tx });

        // Send the toolcall to the agent
        let msg = serde_json::to_string(&call).map_err(|e| e.to_string())?;
        if sender.send(msg).is_err() {
            self.pending.lock().await.remove(&request_id);
            self.agents.lock().await.remove(machine_id);
            return Err(format!("machine '{machine_id}' disconnected (send failed)"));
        }

        log::info!(
            "[machines] sent toolcall {} to '{machine_id}', waiting...",
            &request_id[..8]
        );

        // Wait for response with timeout
        let result = tokio::time::timeout(std::time::Duration::from_secs(120), rx)
            .await
            .map_err(|_| {
                // Clean up pending on timeout
                let pending = self.pending.clone();
                let rid = request_id.clone();
                tokio::spawn(async move {
                    pending.lock().await.remove(&rid);
                });
                format!("computer use action timed out (120s) on '{machine_id}'")
            })?
            .map_err(|_| "agent disconnected before responding".to_string())?;

        log::info!("[machines] result received for {}", &request_id[..8]);

        Ok(result)
    }

    /// Complete a pending action request (called when agent sends result back).
    pub async fn complete(&self, request_id: &str, result: ActionResult) -> bool {
        if let Some(pending) = self.pending.lock().await.remove(request_id) {
            let _ = pending.responder.send(result);
            true
        } else {
            log::warn!("[machines] no pending request for {request_id}");
            false
        }
    }
}
