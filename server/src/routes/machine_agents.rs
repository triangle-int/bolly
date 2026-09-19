use axum::{
    Router,
    extract::{
        Path, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::StatusCode,
    response::Response,
    routing::{get, post},
};
use serde::Deserialize;

use crate::app::state::AppState;
use crate::services::machine_registry::{ActionResult, MachineInfo};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/agents/ws/machine", get(upgrade))
        .route(
            "/api/instances/{instance_slug}/machine-hello",
            post(machine_hello),
        )
        .route(
            "/api/instances/{instance_slug}/machine-bye",
            post(machine_bye),
        )
}

/// the companion that a desktop is connected (if any machines are online).
async fn machine_hello(
    State(state): State<AppState>,
    Path(instance_slug): Path<String>,
) -> StatusCode {
    let machines = state.machine_registry.list().await;
    if machines.is_empty() {
        return StatusCode::OK;
    }

    let machine = &machines[0];
    let mid = machine.machine_id.clone();

    tokio::spawn({
        let bg_state = state.clone();
        let slug = instance_slug.clone();
        async move {
            on_machine_connected(&bg_state, &mid, Some(&slug)).await;
        }
    });

    StatusCode::OK
}

/// Called when the user leaves an instance — logs disconnection.
async fn machine_bye(
    State(state): State<AppState>,
    Path(instance_slug): Path<String>,
) -> StatusCode {
    let machines = state.machine_registry.list().await;
    if machines.is_empty() {
        return StatusCode::OK;
    }

    let machine_id = &machines[0].machine_id;
    let _ = crate::services::chat::save_system_message(
        &state.workspace_dir,
        &instance_slug,
        "default",
        &format!(
            "[system] user left this instance. desktop '{}' still connected to server.",
            machine_id
        ),
    );

    StatusCode::OK
}

async fn upgrade(ws: WebSocketUpgrade, State(state): State<AppState>) -> Response {
    ws.on_upgrade(move |socket| handle_agent(socket, state))
}

/// Message types from the agent.
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AgentMessage {
    /// Agent registers itself on connect.
    Register {
        machine_id: String,
        os: String,
        hostname: String,
        screen_width: u32,
        screen_height: u32,
        #[serde(default)]
        instance_slug: Option<String>,
    },
    /// Agent sends back the result of a toolcall.
    ActionResult {
        request_id: String,
        #[serde(flatten)]
        result: ActionResult,
    },
    /// Heartbeat/ping from agent.
    Heartbeat { machine_id: String },
}

async fn handle_agent(mut socket: WebSocket, state: AppState) {
    // The agent must send a Register message first.
    let (machine_id, mut agent_rx) = match wait_for_registration(&mut socket, &state).await {
        Some(v) => v,
        None => return,
    };

    log::info!("[machine-ws] agent '{machine_id}' connected");

    let mut ping_interval = tokio::time::interval(std::time::Duration::from_secs(15));
    ping_interval.tick().await; // skip first immediate tick

    loop {
        tokio::select! {
            // Forward toolcalls to the agent
            toolcall_msg = agent_rx.recv() => {
                match toolcall_msg {
                    Some(msg) => {
                        log::info!("[machine-ws] sending toolcall to '{machine_id}'");
                        if socket.send(Message::Text(msg.into())).await.is_err() {
                            log::warn!("[machine-ws] failed to send to '{machine_id}', disconnecting");
                            break;
                        }
                    }
                    None => break, // channel closed
                }
            }
            // Receive messages from the agent
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        if let Ok(msg) = serde_json::from_str::<AgentMessage>(&text) {
                            match msg {
                                AgentMessage::ActionResult { request_id, result } => {
                                    log::info!("[machine-ws] result from '{machine_id}' for {}", &request_id[..8.min(request_id.len())]);
                                    state.machine_registry.complete(&request_id, result).await;
                                }
                                AgentMessage::Heartbeat { machine_id: mid } => {
                                    state.machine_registry.heartbeat(&mid).await;
                                }
                                AgentMessage::Register { .. } => {
                                    // Already registered, ignore duplicate
                                }
                            }
                        } else {
                            log::warn!("[machine-ws] unparseable message from '{machine_id}': {}", &text[..100.min(text.len())]);
                        }
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        if socket.send(Message::Pong(payload)).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Pong(_))) => {
                        // Pong received — connection alive
                        state.machine_registry.heartbeat(&machine_id).await;
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {}
                    Some(Err(e)) => {
                        log::warn!("[machine-ws] error from '{machine_id}': {e}");
                        break;
                    }
                }
            }
            // Periodic ping to detect dead connections
            _ = ping_interval.tick() => {
                if socket.send(Message::Ping(vec![].into())).await.is_err() {
                    log::warn!("[machine-ws] ping failed for '{machine_id}', disconnecting");
                    break;
                }
            }
        }
    }

    log::info!("[machine-ws] agent '{machine_id}' disconnected");
    state.machine_registry.unregister(&machine_id).await;
}

/// Wait for the agent to send a Register message.
/// Returns the machine ID and receiver used to forward tool calls.
async fn wait_for_registration(
    socket: &mut WebSocket,
    state: &AppState,
) -> Option<(String, tokio::sync::mpsc::UnboundedReceiver<String>)> {
    // Give agent 10s to register
    let deadline = tokio::time::sleep(std::time::Duration::from_secs(10));
    tokio::pin!(deadline);

    loop {
        tokio::select! {
            _ = &mut deadline => {
                log::warn!("[machine-ws] agent timed out waiting for registration");
                return None;
            }
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        if let Ok(AgentMessage::Register { machine_id, os, hostname, screen_width, screen_height, instance_slug }) =
                            serde_json::from_str::<AgentMessage>(&text)
                        {
                            let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
                            let info = MachineInfo {
                                machine_id: machine_id.clone(),
                                os: os.clone(),
                                hostname,
                                screen_width,
                                screen_height,
                                last_seen: chrono::Utc::now().timestamp(),
                                instance_slug: instance_slug.clone(),
                            };
                            state.machine_registry.register(info, tx).await;

                            // Send ack
                            let ack = serde_json::json!({"type": "registered", "machine_id": machine_id});
                            let _ = socket.send(Message::Text(serde_json::to_string(&ack).unwrap().into())).await;

                            return Some((machine_id, rx));
                        }
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        let _ = socket.send(Message::Pong(payload)).await;
                    }
                    Some(Ok(Message::Close(_))) | None => return None,
                    _ => {}
                }
            }
        }
    }
}

/// When any desktop machine connects, notify the companion agent.
/// If `bound_slug` is set, only notify that instance.
async fn on_machine_connected(state: &AppState, machine_id: &str, bound_slug: Option<&str>) {
    use crate::domain::companion::{CANONICAL_SLUG, is_canonical};

    if let Some(slug) = bound_slug
        && !is_canonical(slug)
    {
        log::warn!(
            "[machine-connect] '{machine_id}' asked for foreign companion {slug:?}; this server owns only {CANONICAL_SLUG}"
        );
        return;
    }
    let companion_dir = crate::services::companion::companion_dir(&state.workspace_dir);
    if !companion_dir.join("soul.md").exists() {
        log::warn!(
            "[machine-connect] '{machine_id}' connected before the companion was onboarded; nothing to notify"
        );
        return;
    }
    let all_slugs = vec![CANONICAL_SLUG.to_owned()];

    log::info!(
        "[machine-connect] '{}' connected, notifying {} instance(s): {:?}",
        machine_id,
        all_slugs.len(),
        all_slugs.iter().map(String::as_str).collect::<Vec<_>>()
    );

    // Notify ALL matching instances about the connection
    for slug in &all_slugs {
        let msg = format!("[system] desktop '{}' connected.", machine_id);
        if let Err(e) =
            crate::services::chat::save_system_message(&state.workspace_dir, slug, "default", &msg)
        {
            log::error!("[machine-connect] failed to save system message for {slug}: {e}");
        }

        // Trigger the companion agent
        let llm_guard = state.llm.read().await;
        if let Some(llm) = llm_guard.as_ref() {
            let instance_dir = state.workspace_dir.join("instances").join(slug);

            let agents = crate::services::child_agents::load_agents(&state.workspace_dir, slug);
            if let Some(companion) = agents.iter().find(|a| a.name == "companion") {
                let task = format!(
                    "the user's desktop computer '{}' just connected.\n\
                     USE reach_out NOW to let the user know their computer is connected. \
                     keep it brief and friendly.",
                    machine_id
                );
                log::info!(
                    "[machine-connect] triggering companion for {slug} (task: machine_connected)"
                );
                let ws = state.workspace_dir.clone();
                let s = slug.clone();
                let events = state.events.clone();
                let vs = state.vector_store.clone();
                let llm_c = llm.clone();
                let agent = companion.clone();
                let ws2 = ws.clone();
                let s2 = s.clone();
                let events2 = events.clone();
                let resources = state.resources.clone();
                tokio::spawn(async move {
                    match crate::services::child_agents::run_single_agent(
                        &ws,
                        &s,
                        &instance_dir,
                        &llm_c,
                        &events,
                        &vs,
                        &agent,
                        Some(&task),
                        "machine_connected",
                        None,
                        &resources,
                    )
                    .await
                    {
                        Ok(r) => log::info!(
                            "[machine-connect] {s}: companion reach_out done ({} tokens)",
                            r.tokens
                        ),
                        Err(e) => {
                            log::error!("[machine-connect] {s}: companion failed: {e}");
                            let err_msg = format!(
                                "[system] failed to notify companion about desktop connection: {e}"
                            );
                            let _ = crate::services::chat::save_system_message(
                                &ws2, &s2, "default", &err_msg,
                            );
                            let _ = events2.send(
                                crate::domain::events::ServerEvent::ChatMessageCreated {
                                    instance_slug: s2.clone(),
                                    chat_id: "default".to_string(),
                                    message: crate::domain::chat::ChatMessage {
                                        id: format!(
                                            "err_{}",
                                            std::time::SystemTime::now()
                                                .duration_since(std::time::UNIX_EPOCH)
                                                .unwrap()
                                                .as_millis()
                                        ),
                                        role: crate::domain::chat::ChatRole::Assistant,
                                        content: format!(
                                            "could not process desktop connection notification: {e}"
                                        ),
                                        created_at: std::time::SystemTime::now()
                                            .duration_since(std::time::UNIX_EPOCH)
                                            .unwrap()
                                            .as_millis()
                                            .to_string(),
                                        kind: Default::default(),
                                        tool_name: None,
                                        mcp_app_html: None,
                                        mcp_app_input: None,
                                        model: None,
                                    },
                                },
                            );
                        }
                    }
                });
            } else {
                log::warn!("[machine-connect] {slug}: no companion agent found");
                let _ = crate::services::chat::save_system_message(
                    &state.workspace_dir,
                    slug,
                    "default",
                    &format!(
                        "[system] desktop '{}' connected, but companion agent not found — cannot send notification.",
                        machine_id
                    ),
                );
            }
        } else {
            log::error!("[machine-connect] {slug}: LLM not configured, cannot trigger companion");
            let _ = crate::services::chat::save_system_message(
                &state.workspace_dir,
                slug,
                "default",
                &format!(
                    "[system] desktop '{}' connected, but LLM is not configured — cannot notify companion.",
                    machine_id
                ),
            );
        }
    }
}
