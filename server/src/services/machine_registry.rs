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

/// Registry of connected Tauri agent machines.
#[derive(Clone)]
pub struct MachineRegistry {
    /// Connected agents: machine_id → (info, sender)
    agents: Arc<Mutex<HashMap<String, (MachineInfo, AgentSender)>>>,
    /// Pending action requests: request_id → oneshot sender
    pending: Arc<Mutex<HashMap<String, PendingAction>>>,
}

impl MachineRegistry {
    pub fn new() -> Self {
        Self {
            agents: Arc::new(Mutex::new(HashMap::new())),
            pending: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Register a new agent connection.
    ///
    /// Machines are contexts of the one companion this server owns; a
    /// registration naming any other slug is bound to the canonical one.
    pub async fn register(&self, mut info: MachineInfo, sender: AgentSender) {
        use crate::domain::companion::{CANONICAL_SLUG, is_canonical};
        if let Some(requested) = info.instance_slug.as_deref()
            && !is_canonical(requested)
        {
            log::warn!(
                "[machines] {} requested foreign companion {requested:?}; binding to {CANONICAL_SLUG}",
                info.machine_id
            );
        }
        info.instance_slug = Some(CANONICAL_SLUG.to_owned());
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

#[cfg(test)]
mod companion_boundary_tests {
    use super::*;
    use crate::domain::companion::CANONICAL_SLUG;

    fn info(instance_slug: Option<&str>) -> MachineInfo {
        MachineInfo {
            machine_id: "machine-1".into(),
            os: "macos".into(),
            hostname: "studio".into(),
            screen_width: 1440,
            screen_height: 900,
            last_seen: 0,
            instance_slug: instance_slug.map(str::to_owned),
        }
    }

    #[tokio::test]
    async fn registered_machines_are_always_bound_to_the_canonical_companion() {
        for provided in [None, Some("alice"), Some("Companion"), Some(CANONICAL_SLUG)] {
            let registry = MachineRegistry::new();
            let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
            registry.register(info(provided), tx).await;
            let listed = registry.list().await;
            assert_eq!(listed.len(), 1, "{provided:?}");
            assert_eq!(
                listed[0].instance_slug.as_deref(),
                Some(CANONICAL_SLUG),
                "{provided:?} must bind to the canonical companion"
            );
        }
    }
}
