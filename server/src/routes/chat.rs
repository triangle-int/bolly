use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get, post},
};
use std::io::ErrorKind;
use tokio_util::sync::CancellationToken;

use crate::{
    app::state::AppState,
    config,
    domain::{
        chat::{ChatMessage, ChatRequest, ChatResponse, ChatRole, ChatSummary},
        events::ServerEvent,
    },
    services::chat,
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/chat", post(post_chat))
        .route("/api/chat/{instance_slug}/chats", get(list_chats))
        .route(
            "/api/chat/{instance_slug}/messages",
            get(get_messages_default),
        )
        .route(
            "/api/chat/{instance_slug}/{chat_id}/messages",
            get(get_messages),
        )
        .route("/api/chat/{instance_slug}/{chat_id}/stop", post(stop_agent))
        .route(
            "/api/chat/{instance_slug}/{chat_id}/context",
            delete(clear_context),
        )
        // Legacy routes (use default chat_id)
        .route("/api/chat/{instance_slug}/stop", post(stop_agent_default))
        .route(
            "/api/chat/{instance_slug}/context",
            delete(clear_context_default),
        )
}

fn task_key(slug: &str, chat_id: &str) -> String {
    format!("{slug}/{chat_id}")
}

async fn post_chat(
    State(state): State<AppState>,
    Json(request): Json<ChatRequest>,
) -> Result<Json<ChatResponse>, super::ProviderRequestError> {
    super::require_provider(&state).await?;
    let instance_slug = request.instance_slug.clone();
    let chat_id = request.chat_id.clone();
    let content = request.content.trim().to_string();
    let voice_mode = request.voice_mode;

    if instance_slug.is_empty() || content.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "slug and content required".into()).into());
    }

    // Save user message immediately
    let user_message =
        chat::save_user_message(&state.workspace_dir, &instance_slug, &chat_id, &content)
            .map_err(map_chat_error)?;

    // Broadcast user message
    let _ = state.events.send(ServerEvent::ChatMessageCreated {
        instance_slug: instance_slug.clone(),
        chat_id: chat_id.clone(),
        message: user_message.clone(),
    });

    // Discover instance
    if let Ok(Some(instance)) = chat::discover_instance(&state.workspace_dir, &instance_slug) {
        let _ = state
            .events
            .send(ServerEvent::InstanceDiscovered { instance });
    }

    let key = task_key(&instance_slug, &chat_id);

    // If an agent is already running for this chat, don't start another one.
    // The running agent will pick up the new message on its next turn
    // (it re-reads messages from disk each iteration).
    let already_running = {
        let tasks = state.agent_tasks.lock().await;
        tasks.contains_key(&key)
    };

    if !already_running {
        let cancel = CancellationToken::new();
        {
            let mut tasks = state.agent_tasks.lock().await;
            // Double-check after re-acquiring lock
            if !tasks.contains_key(&key) {
                tasks.insert(key.clone(), cancel.clone());
            }
        }

        let bg_state = state.clone();
        let bg_chat_id = chat_id.clone();
        tokio::spawn(async move {
            run_agent_loop(bg_state, instance_slug, bg_chat_id, cancel, voice_mode).await;
        });
    }

    // Return immediately with the user message
    Ok(Json(ChatResponse {
        instance_slug: request.instance_slug,
        chat_id: request.chat_id,
        messages: vec![user_message],
        agent_running: true,
    }))
}

/// Agent loop: keeps calling the LLM until it responds without tool use or is cancelled.
/// New user messages are automatically picked up because each turn re-reads from disk.
pub async fn run_agent_loop(
    state: AppState,
    instance_slug: String,
    chat_id: String,
    cancel: CancellationToken,
    voice_mode: bool,
) {
    let _ = state.events.send(ServerEvent::AgentRunning {
        instance_slug: instance_slug.clone(),
        chat_id: chat_id.clone(),
    });

    // Persist marker so we can detect interrupted agents across restarts
    chat::set_agent_running(&state.workspace_dir, &instance_slug, &chat_id);

    // Background TTS: subscribe to events and voice ALL assistant messages
    // TTS pipeline: forwarder task → mpsc channel → synthesizer task.
    // Forwarder stops when cancel fires. Synthesizer drains remaining
    // messages in the channel before exiting (no abort, no message loss).
    let tts_cancel = cancel.clone();
    let (tts_fwd_handle, tts_synth_handle) = if voice_mode {
        let (tts_tx, mut tts_rx) = tokio::sync::mpsc::unbounded_channel::<ChatMessage>();
        let mut rx = state.events.subscribe();
        let fwd_slug = instance_slug.clone();
        let fwd_chat = chat_id.clone();
        let fwd_cancel = tts_cancel.clone();

        // Forwarder: drains broadcast into mpsc. Stops when cancel fires,
        // dropping tts_tx so the synthesizer's recv() returns None.
        let fwd = tokio::spawn(async move {
            while !fwd_cancel.is_cancelled() {
                let event = match rx.recv().await {
                    Ok(e) => e,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        log::warn!("[tts] broadcast lagged by {n} events");
                        continue;
                    }
                    Err(_) => break,
                };
                if let ServerEvent::ChatMessageCreated {
                    instance_slug: ref slug,
                    chat_id: ref cid,
                    ref message,
                } = event
                {
                    if slug != &fwd_slug || cid != &fwd_chat {
                        continue;
                    }
                    if message.role != ChatRole::Assistant {
                        continue;
                    }
                    if message.content.trim().is_empty() {
                        continue;
                    }
                    if message.kind == crate::domain::chat::MessageKind::ToolCall
                        || message.kind == crate::domain::chat::MessageKind::ToolOutput
                    {
                        continue;
                    }
                    let _ = tts_tx.send(message.clone());
                }
            }
            // tts_tx is dropped here → synthesizer's recv() will return None
            // after processing remaining queued messages
        });

        let tts_state = state.clone();
        let tts_slug = instance_slug.clone();
        let tts_chat = chat_id.clone();

        // Synthesizer: processes all messages until channel is drained
        let synth = tokio::spawn(async move {
            let api_key = {
                let cfg = tts_state.config.read().await;
                cfg.llm.tokens.elevenlabs.clone()
            };
            if api_key.is_empty() {
                return;
            }

            while let Some(message) = tts_rx.recv().await {
                let voice_id =
                    crate::routes::tts::resolve_voice_id(&tts_state.workspace_dir, &tts_slug);
                let idir = tts_state.workspace_dir.join("instances").join(&tts_slug);
                let mood = crate::services::tools::companion::load_mood_state(&idir).companion_mood;
                match crate::routes::tts::synthesize_bytes(
                    &tts_state.http_client,
                    &api_key,
                    &voice_id,
                    &message.content,
                    &mood,
                )
                .await
                {
                    Ok(audio_bytes) => {
                        use base64::Engine;
                        let audio_base64 =
                            base64::engine::general_purpose::STANDARD.encode(&audio_bytes);
                        let _ = tts_state.events.send(ServerEvent::ChatAudioReady {
                            instance_slug: tts_slug.clone(),
                            chat_id: tts_chat.clone(),
                            audio_base64,
                            message_ids: vec![message.id.clone()],
                        });
                    }
                    Err(e) => log::warn!("[tts] failed for {}: {e}", message.id),
                }
            }
        });

        (Some(fwd), Some(synth))
    } else {
        (None, None)
    };

    const MAX_ITERATIONS: usize = 5;
    let mut iteration = 0;

    // Classify model ONCE before the loop — the same model is used for all
    // iterations of this request. Re-classifying mid-task would downgrade
    // from heavy to fast in the middle of complex tool-use chains.
    let classified_heavy: Option<bool> = {
        let cfg = state.config.read().await;
        let model_mode = cfg.llm.model_mode;
        drop(cfg);
        match model_mode {
            config::ModelMode::Heavy => Some(true),
            config::ModelMode::Fast => Some(false),
            config::ModelMode::Auto => {
                let llm_guard = state.llm.read().await;
                if let Some(llm_ref) = llm_guard.as_ref() {
                    let llm = llm_ref.clone();
                    drop(llm_guard);
                    let last_msg =
                        chat::last_user_content(&state.workspace_dir, &instance_slug, &chat_id);
                    if let Some(msg) = last_msg {
                        Some(llm.classify_needs_heavy(&msg).await)
                    } else {
                        Some(true) // default to heavy if no message
                    }
                } else {
                    None // no LLM configured
                }
            }
        }
    };

    loop {
        if cancel.is_cancelled() {
            log::info!("[agent] {instance_slug}/{chat_id} — cancelled by user");
            break;
        }

        if iteration >= MAX_ITERATIONS {
            log::info!("[agent] {instance_slug}/{chat_id} — reached max iterations");
            break;
        }

        iteration += 1;

        let config_path = config::config_path();
        let (fast_model_name, google_ai_key, public_url) = {
            let cfg = state.config.read().await;
            (
                cfg.llm.fast_model_name().to_string(),
                cfg.llm.tokens.google_ai.clone(),
                cfg.public_url.clone(),
            )
        };

        let llm_guard = state.llm.read().await;
        let llm_ref = match llm_guard.as_ref() {
            Some(l) => l.clone(),
            None => {
                log::warn!("[agent] {instance_slug}/{chat_id} — no LLM configured");
                break;
            }
        };
        drop(llm_guard);

        // Timeout must exceed stream item timeout (480s) to allow long tools to complete.
        const TURN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);

        // Use the model decided before the loop — consistent across all iterations.
        let used_heavy = classified_heavy.unwrap_or(true);
        log::info!(
            "[agent] {instance_slug}/{chat_id} — used_heavy={used_heavy}, base_model={}",
            llm_ref.model
        );
        let effective_llm = if used_heavy {
            llm_ref.clone()
        } else {
            llm_ref.fast_variant_with(Some(&fast_model_name))
        };

        let turn_fut = chat::run_single_turn(
            &state.workspace_dir,
            &config_path,
            &instance_slug,
            &chat_id,
            &effective_llm,
            state.events.clone(),
            state.pending_secrets.clone(),
            &state.mcp_registry,
            voice_mode,
            state.vector_store.clone(),
            &google_ai_key,
            state.machine_registry.clone(),
            &public_url,
        );

        let result = tokio::select! {
            r = tokio::time::timeout(TURN_TIMEOUT, turn_fut) => {
                match r {
                    Ok(inner) => inner,
                    Err(_) => {
                        log::warn!("[agent] {instance_slug}/{chat_id} — turn timed out after {}s", TURN_TIMEOUT.as_secs());
                        Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "turn timed out — the model or a tool took too long"))
                    }
                }
            }
            _ = cancel.cancelled() => {
                log::info!("[agent] {instance_slug}/{chat_id} — cancelled during turn");
                break;
            }
        };

        // Reload config in case LLM changed it
        state.reload_config().await;

        match result {
            Ok(turn) => {
                for msg in &turn.messages {
                    let _ = state.events.send(ServerEvent::ChatMessageCreated {
                        instance_slug: instance_slug.clone(),
                        chat_id: chat_id.clone(),
                        message: msg.clone(),
                    });
                    // TTS handled by background subscriber task
                }

                // Check if a new user message arrived while agent was processing
                if has_pending_user_message(&state, &instance_slug, &chat_id).await {
                    log::info!(
                        "[agent] {instance_slug}/{chat_id} — new user message arrived, continuing"
                    );
                    continue;
                }
                break;
            }
            Err(e) => {
                let msg = e.to_string();
                log::warn!("[agent] {instance_slug}/{chat_id} — error: {msg}");

                let error_label = if matches!(
                    e.get_ref()
                        .and_then(|e| e.downcast_ref::<crate::services::llm::contract::LlmError>()),
                    Some(crate::services::llm::contract::LlmError::SetupRequired(_))
                ) {
                    msg.as_str()
                } else if msg.contains("rate limit") || msg.contains("429") {
                    "rate limited — try again in a moment"
                } else if msg.contains("timed out") {
                    "request timed out"
                } else if msg.contains("token_not_found")
                    || msg.contains("authentication")
                    || msg.contains("401")
                {
                    "not authenticated — reconnect your account in Settings → Provider"
                } else if msg.contains("no LLM") || msg.contains("not configured") {
                    "no API key configured — add one in Settings"
                } else {
                    "something went wrong"
                };
                let error_msg = chat::save_system_message(
                    &state.workspace_dir,
                    &instance_slug,
                    &chat_id,
                    &format!("[system] {error_label}"),
                );
                if let Ok(m) = error_msg {
                    let _ = state.events.send(ServerEvent::ChatMessageCreated {
                        instance_slug: instance_slug.clone(),
                        chat_id: chat_id.clone(),
                        message: m,
                    });
                }
                break;
            }
        }
    }

    // Auto-generate title if missing
    if let Ok(response) = chat::load_messages(&state.workspace_dir, &instance_slug, &chat_id) {
        let needs_title = chat::get_chat_title(&state.workspace_dir, &instance_slug, &chat_id)
            .map(|t| t.is_empty())
            .unwrap_or(true);

        if needs_title && !response.messages.is_empty() {
            let llm_guard = state.llm.read().await;
            if let Some(llm) = llm_guard.as_ref() {
                let snippet: String = response
                    .messages
                    .iter()
                    .take(6)
                    .map(|m| {
                        format!(
                            "{}: {}",
                            if m.role == ChatRole::User {
                                "user"
                            } else {
                                "assistant"
                            },
                            m.content.chars().take(200).collect::<String>()
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");

                let prompt = format!(
                    "Generate a very short title (3-6 words, no quotes) for this conversation:\n\n{snippet}"
                );

                if let Ok((title, _)) = llm.chat("You generate short chat titles. Respond with only the title, nothing else.", &prompt, vec![]).await {
                    let title = title.trim().trim_matches('"').to_string();
                    let _ = chat::update_chat_title(&state.workspace_dir, &instance_slug, &chat_id, &title);
                }
            }
        }
    }

    // Clean up
    chat::clear_agent_running(&state.workspace_dir, &instance_slug, &chat_id);

    let key = task_key(&instance_slug, &chat_id);
    {
        let mut tasks = state.agent_tasks.lock().await;
        tasks.remove(&key);
    }

    // Final snapshot — client gets complete state before agent_stopped
    send_snapshot(&state, &instance_slug, &chat_id, false);

    let _ = state.events.send(ServerEvent::AgentStopped {
        instance_slug: instance_slug.clone(),
        chat_id: chat_id.clone(),
    });

    // Stop the TTS forwarder (drops sender → synthesizer drains remaining queue)
    if let Some(fwd) = tts_fwd_handle {
        fwd.abort(); // forwarder can be killed immediately
    }
    // Wait for synthesizer to finish processing queued messages (up to 30s)
    if let Some(synth) = tts_synth_handle {
        let _ = tokio::time::timeout(std::time::Duration::from_secs(30), synth).await;
    }
}

/// Send a full chat state snapshot so all clients converge to the same state.
fn send_snapshot(state: &AppState, instance_slug: &str, chat_id: &str, agent_running: bool) {
    match chat::load_messages(&state.workspace_dir, instance_slug, chat_id) {
        Ok(resp) => {
            let _ = state.events.send(ServerEvent::ChatSnapshot {
                instance_slug: instance_slug.to_string(),
                chat_id: chat_id.to_string(),
                messages: resp.messages,
                agent_running,
            });
        }
        Err(e) => log::warn!("[snapshot] failed to load messages: {e}"),
    }
}

/// Check if the last message in the chat is from the user (meaning they sent something
/// while the agent was processing and we should do another turn).
async fn has_pending_user_message(state: &AppState, instance_slug: &str, chat_id: &str) -> bool {
    match chat::load_messages(&state.workspace_dir, instance_slug, chat_id) {
        Ok(response) => response
            .messages
            .last()
            .is_some_and(|m| m.role == ChatRole::User),
        Err(_) => false,
    }
}

async fn list_chats(
    State(state): State<AppState>,
    Path(instance_slug): Path<String>,
) -> Result<Json<Vec<ChatSummary>>, (StatusCode, String)> {
    let chats = chat::list_chats(&state.workspace_dir, &instance_slug).map_err(map_chat_error)?;
    Ok(Json(chats))
}

async fn get_messages_default(
    State(state): State<AppState>,
    Path(instance_slug): Path<String>,
) -> Result<Json<ChatResponse>, super::ProviderRequestError> {
    super::require_provider(&state).await?;
    let mut response = chat::load_messages(&state.workspace_dir, &instance_slug, "default")
        .map_err(map_chat_error)?;
    response.agent_running = is_agent_running(&state, &instance_slug, "default").await;
    Ok(Json(response))
}

async fn get_messages(
    State(state): State<AppState>,
    Path((instance_slug, chat_id)): Path<(String, String)>,
) -> Result<Json<ChatResponse>, super::ProviderRequestError> {
    super::require_provider(&state).await?;
    let mut response = chat::load_messages(&state.workspace_dir, &instance_slug, &chat_id)
        .map_err(map_chat_error)?;
    response.agent_running = is_agent_running(&state, &instance_slug, &chat_id).await;
    Ok(Json(response))
}

async fn is_agent_running(state: &AppState, instance_slug: &str, chat_id: &str) -> bool {
    let key = task_key(instance_slug, chat_id);
    let tasks = state.agent_tasks.lock().await;
    tasks.contains_key(&key)
}

async fn stop_agent(
    State(state): State<AppState>,
    Path((instance_slug, chat_id)): Path<(String, String)>,
) -> StatusCode {
    let key = task_key(&instance_slug, &chat_id);
    let mut tasks = state.agent_tasks.lock().await;
    if let Some(token) = tasks.remove(&key) {
        token.cancel();
        StatusCode::OK
    } else {
        StatusCode::NOT_FOUND
    }
}

async fn stop_agent_default(
    State(state): State<AppState>,
    Path(instance_slug): Path<String>,
) -> StatusCode {
    let key = task_key(&instance_slug, "default");
    let mut tasks = state.agent_tasks.lock().await;
    if let Some(token) = tasks.remove(&key) {
        token.cancel();
        StatusCode::OK
    } else {
        StatusCode::NOT_FOUND
    }
}

async fn clear_context(
    State(state): State<AppState>,
    Path((instance_slug, chat_id)): Path<(String, String)>,
) -> StatusCode {
    chat::clear_context(&state.workspace_dir, &instance_slug, &chat_id);
    StatusCode::OK
}

async fn clear_context_default(
    State(state): State<AppState>,
    Path(instance_slug): Path<String>,
) -> StatusCode {
    chat::clear_context(&state.workspace_dir, &instance_slug, "default");
    StatusCode::OK
}

fn map_chat_error(error: std::io::Error) -> (StatusCode, String) {
    let status = match error.kind() {
        ErrorKind::InvalidInput => StatusCode::BAD_REQUEST,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };

    (status, error.to_string())
}
