use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use std::sync::Mutex;
use tauri::Emitter;
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};

use crate::computer_use;
use crate::overlay;

const MAX_QUEUED_REQUEST_BYTES: usize = 1024 * 1024;
const MAX_QUEUED_REQUESTS: usize = 128;
const MAX_COMMAND_OUTPUT_BYTES: usize = 8 * 1024 * 1024;

fn request_len(frame: &Message) -> Option<usize> {
    match frame {
        Message::Text(text) => Some(text.len()),
        Message::Binary(bytes) => Some(bytes.len()),
        _ => None,
    }
}

fn enqueue_request(
    queue: &mut std::collections::VecDeque<Message>,
    queued_bytes: &mut usize,
    frame: Message,
) -> Result<(), ()> {
    let size = request_len(&frame).ok_or(())?;
    if queue.len() >= MAX_QUEUED_REQUESTS
        || size > MAX_QUEUED_REQUEST_BYTES.saturating_sub(*queued_bytes)
    {
        return Err(());
    }
    *queued_bytes += size;
    queue.push_back(frame);
    Ok(())
}

fn request_text(frame: Message) -> Result<String, ()> {
    match frame {
        Message::Text(text) => Ok(text.to_string()),
        Message::Binary(bytes) => String::from_utf8(bytes.to_vec()).map_err(|_| ()),
        _ => Err(()),
    }
}

fn stop_recording_after_connection_loss() {
    let _ = crate::screen_recorder::stop();
    *RECORDING_ACTIVE.lock().unwrap_or_else(|e| e.into_inner()) = false;
}

// Each connection owns a distinct cancellation generation. Work holds a permit
// until its actual main-thread/blocking callback exits, not just its async waiter.
#[derive(Clone)]
struct Session {
    stop: tokio::sync::watch::Sender<bool>,
    work: std::sync::Arc<tokio::sync::Semaphore>,
    connection_stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
}
impl Session {
    fn new() -> Self {
        Self {
            stop: tokio::sync::watch::channel(false).0,
            work: std::sync::Arc::new(tokio::sync::Semaphore::new(1)),
            connection_stop: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }
    fn cancelled(&self) -> bool {
        *self.stop.borrow()
            || self
                .connection_stop
                .load(std::sync::atomic::Ordering::Acquire)
    }
    fn cancel(&self) {
        self.stop.send_replace(true);
        self.cancel_connection();
    }
    fn begin_connection(&self) {
        self.connection_stop
            .store(false, std::sync::atomic::Ordering::Release);
    }
    fn cancel_connection(&self) {
        self.connection_stop
            .store(true, std::sync::atomic::Ordering::Release);
    }
    async fn stopped(&self) {
        let mut rx = self.stop.subscribe();
        if *rx.borrow_and_update() {
            return;
        }
        let _ = rx.changed().await;
    }
    async fn until_stopped<T>(&self, future: impl std::future::Future<Output = T>) -> Option<T> {
        tokio::select! {
            biased;
            _ = self.stopped() => None,
            result = future => Some(result),
        }
    }
    async fn drain(&self) {
        let _ = self.work.acquire().await;
    }
}
struct Work {
    _permit: tokio::sync::OwnedSemaphorePermit,
    session: Session,
}
impl Work {
    fn run<T>(self, action: impl FnOnce(&Session) -> Result<T, String>) -> Result<T, String> {
        if self.session.cancelled() {
            return Err("Session stopped".into());
        }
        action(&self.session)
    }
}
struct Bridge {
    session: Session,
    task: tokio::task::JoinHandle<()>,
}
static BRIDGE_TASK: Mutex<Option<Bridge>> = Mutex::new(None);

/// Whether this machine allows screen recording for observation (off by default).
static SCREEN_RECORDING_ALLOWED: Mutex<bool> = Mutex::new(false);

/// Whether screen recording is currently active (for overlay indicator).
static RECORDING_ACTIVE: Mutex<bool> = Mutex::new(false);

/// Instance slug this machine is bound to (set from URL or explicitly).
static INSTANCE_SLUG: Mutex<Option<String>> = Mutex::new(None);

/// Server URL for overlay (set on connect).
static SERVER_URL: Mutex<Option<String>> = Mutex::new(None);

/// Start the machine agent — connects to the server's machine WebSocket,
/// registers this machine, then listens for toolcalls and executes them.
pub async fn connect_computer_use(
    app: tauri::AppHandle,
    instance_url: String,
    auth_token: String,
) -> Result<(), String> {
    // Cancel the old socket and retry loop before starting another connection.
    disconnect_computer_use(app.clone()).await?;
    let mut task = BRIDGE_TASK.lock().map_err(|e| e.to_string())?;
    {
        let mut url = SERVER_URL.lock().map_err(|e| e.to_string())?;
        *url = Some(instance_url.clone());
    }

    let session = Session::new();
    let running = session.clone();
    let handle = tokio::spawn(async move {
        loop {
            if running
                .until_stopped(run_agent(&app, &instance_url, &auth_token, &running))
                .await
                .is_none()
            {
                // `until_stopped` drops `run_agent`, so its normal connection-loss
                // cleanup does not run. Stop capture before waiting for any
                // non-cooperative in-flight action to drain.
                stop_recording_after_connection_loss();
                break;
            }
            stop_recording_after_connection_loss();
            // A dropped network future may have dispatched a native callback.
            // Drain it before another socket/session can receive work.
            running.drain().await;
            tokio::select! {
                biased;
                _ = running.stopped() => break,
                _ = tokio::time::sleep(std::time::Duration::from_secs(3)) => {},
            }
        }
        running.drain().await;
    });
    *task = Some(Bridge {
        session,
        task: handle,
    });

    Ok(())
}

pub async fn disconnect_computer_use(app: tauri::AppHandle) -> Result<(), String> {
    let task = BRIDGE_TASK.lock().map_err(|e| e.to_string())?.take();
    if let Some(task) = task {
        task.session.cancel();
        // Privacy teardown must not wait behind an in-flight native action.
        stop_recording_after_connection_loss();
        app.emit("screen-recording-state", false).ok();
        overlay::hide(&app);
        let _ = task.task.await;
        task.session.drain().await;
    }
    *SERVER_URL.lock().map_err(|e| e.to_string())? = None;
    *INSTANCE_SLUG.lock().map_err(|e| e.to_string())? = None;
    *RECORDING_ACTIVE.lock().map_err(|e| e.to_string())? = false;
    let recording = crate::screen_recorder::stop();
    app.emit("screen-recording-state", false).ok();
    overlay::hide(&app);
    recording
}

#[tauri::command]
pub fn set_screen_recording_allowed(app: tauri::AppHandle, allowed: bool) -> Result<(), String> {
    use tauri_plugin_store::StoreExt;
    *SCREEN_RECORDING_ALLOWED.lock().map_err(|e| e.to_string())? = allowed;
    let store = app.store("settings.json").map_err(|e| e.to_string())?;
    store.set("screen_recording_allowed", serde_json::Value::Bool(allowed));
    store.save().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_screen_recording_allowed(app: tauri::AppHandle) -> Result<bool, String> {
    use tauri_plugin_store::StoreExt;
    let allowed = app
        .store("settings.json")
        .map_err(|e| e.to_string())?
        .get("screen_recording_allowed")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    *SCREEN_RECORDING_ALLOWED.lock().map_err(|e| e.to_string())? = allowed;
    Ok(allowed)
}

#[tauri::command]
pub fn get_server_url() -> Result<String, String> {
    SERVER_URL
        .lock()
        .map_err(|e| e.to_string())?
        .clone()
        .ok_or_else(|| "not connected".into())
}

/// Set the instance slug this machine is bound to.
#[tauri::command]
pub fn set_instance_slug(slug: String) -> Result<(), String> {
    let mut val = INSTANCE_SLUG.lock().map_err(|e| e.to_string())?;
    *val = if slug.is_empty() { None } else { Some(slug) };
    Ok(())
}

/// Stop and join active native screen capture.
#[tauri::command]
pub fn stop_screen_recording(app: tauri::AppHandle) -> Result<(), String> {
    let mut rec = RECORDING_ACTIVE.lock().map_err(|e| e.to_string())?;
    *rec = false;
    crate::screen_recorder::stop()?;
    app.emit("screen-recording-state", false).ok();
    overlay::hide(&app);
    Ok(())
}

async fn run_agent(
    app: &tauri::AppHandle,
    instance_url: &str,
    auth_token: &str,
    session: &Session,
) -> Result<(), String> {
    let result = run_agent_connection(app, instance_url, auth_token, session).await;
    session.cancel_connection();
    stop_recording_after_connection_loss();
    app.emit("screen-recording-state", false).ok();
    overlay::hide(app);
    result
}

async fn run_agent_connection(
    app: &tauri::AppHandle,
    instance_url: &str,
    auth_token: &str,
    session: &Session,
) -> Result<(), String> {
    session.begin_connection();
    let request = machine_request(instance_url, auth_token)?;
    let (ws, _) = tokio_tungstenite::connect_async(request)
        .await
        .map_err(|_| "Could not connect to machine WebSocket".to_string())?;

    let (mut write, mut read) = ws.split();

    // Register this machine
    let machine_id = hostname();
    let os = std::env::consts::OS.to_string();

    // Get screen dimensions
    let screen = screenshots::Screen::all()
        .ok()
        .and_then(|s| s.into_iter().next());
    let (sw, sh) = screen
        .map(|s| {
            let info = s.display_info;
            (info.width, info.height)
        })
        .unwrap_or((1920, 1080));

    let screen_recording_allowed = SCREEN_RECORDING_ALLOWED.lock().map(|v| *v).unwrap_or(false);

    // Instance slug: only set explicitly via set_instance_slug.
    let instance_slug = INSTANCE_SLUG.lock().ok().and_then(|v| v.clone());

    let register = serde_json::json!({
        "type": "register",
        "machine_id": machine_id,
        "os": os,
        "hostname": machine_id,
        "screen_width": sw,
        "screen_height": sh,
        "screen_recording_allowed": screen_recording_allowed,
        "instance_slug": instance_slug,
    });
    write
        .send(Message::Text(register.to_string().into()))
        .await
        .map_err(|e| format!("send register: {e}"))?;

    eprintln!("[agent] registered as '{machine_id}' ({os}, {sw}x{sh})");

    // Emit server URL so overlay can build iframe src
    app.emit("server-url", instance_url.to_string()).ok();

    // Scale cache from last screenshot (shared with spawn_blocking tasks)
    let cached_scale = std::sync::Arc::new(std::sync::Mutex::new(1.0f64));

    let mut ping_interval = tokio::time::interval(std::time::Duration::from_secs(20));
    ping_interval.tick().await;
    let mut last_pong = std::time::Instant::now();

    // Frame streaming interval (1 fps when recording)
    let mut frame_interval = tokio::time::interval(std::time::Duration::from_secs(1));
    frame_interval.tick().await;
    let mut queued_requests = std::collections::VecDeque::new();
    let mut queued_request_bytes: usize = 0;

    // Main loop: receive toolcalls, execute, send results
    loop {
        if session.cancelled() {
            break;
        }

        let (frame, was_queued) = if let Some(frame) = queued_requests.pop_front() {
            (frame, true)
        } else {
            let frame = tokio::select! {
                msg = read.next() => {
                    match msg {
                        Some(Ok(frame @ (Message::Text(_) | Message::Binary(_)))) => frame,
                        Some(Ok(Message::Ping(data))) => {
                            if write.send(Message::Pong(data)).await.is_err() {
                                break;
                            }
                            continue;
                        }
                        Some(Ok(Message::Pong(_))) => {
                            last_pong = std::time::Instant::now();
                            continue;
                        }
                        Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                        Some(Ok(_)) => continue,
                    }
                }
                _ = ping_interval.tick() => {
                    if write.send(Message::Ping(vec![].into())).await.is_err()
                        || last_pong.elapsed() > std::time::Duration::from_secs(45)
                    {
                        break;
                    }
                    continue;
                }
                _ = frame_interval.tick() => {
                    if crate::screen_recorder::is_recording() {
                        if let Some(jpeg) = crate::screen_recorder::get_last_frame() {
                            let b64 = base64::engine::general_purpose::STANDARD.encode(&jpeg);
                            let message = serde_json::json!({
                                "type": "screen_frame",
                                "machine_id": machine_id,
                                "image": b64,
                                "width": 0,
                                "height": 0,
                            });
                            if write.send(Message::Text(message.to_string().into())).await.is_err() {
                                break;
                            }
                        }
                    }
                    continue;
                }
            };
            (frame, false)
        };
        if was_queued {
            queued_request_bytes =
                queued_request_bytes.saturating_sub(request_len(&frame).unwrap_or_default());
        }
        let Ok(text) = request_text(frame) else {
            continue;
        };

        let call: serde_json::Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(_) => continue,
        };

        // Skip non-toolcall messages (e.g. "registered" ack)
        let request_id = match call.get("request_id").and_then(|v| v.as_str()) {
            Some(id) => id.to_string(),
            None => continue,
        };
        let action = call
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        // Temporarily hide overlay before screenshot so it doesn't appear in the capture
        // But never hide when recording — the REC indicator must stay visible
        let hide_for_screenshot =
            action == "screenshot" && !RECORDING_ACTIVE.lock().map(|v| *v).unwrap_or(false);
        if hide_for_screenshot {
            overlay::set_visible(app, false);
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }

        // Input actions (keyboard, mouse) must run on main thread on macOS
        // because enigo calls HIToolbox APIs that assert main queue.
        // Other actions (screenshot, bash, file I/O) use spawn_blocking.
        let is_input_action = matches!(
            action.as_str(),
            "key"
                | "type"
                | "left_click"
                | "right_click"
                | "middle_click"
                | "double_click"
                | "mouse_move"
                | "scroll"
                | "switch_desktop"
        );

        let permit = session
            .work
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| "Session stopped")?;
        if session.cancelled() {
            break;
        }
        let work = Work {
            _permit: permit,
            session: session.clone(),
        };
        let action_call = call.clone();
        let action_name = action.clone();
        let action_scale = cached_scale.clone();
        let action_app = app.clone();
        let action_future = async move {
            if is_input_action {
                let (tx, rx) = tokio::sync::oneshot::channel();
                let _ = action_app.run_on_main_thread(move || {
                    let result = work.run(|session| {
                        let mut scale = action_scale.lock().unwrap();
                        execute_action(&action_call, &action_name, &mut scale, session)
                    });
                    let _ = tx.send(result);
                });
                rx.await
                    .unwrap_or_else(|error| Err(format!("main thread recv: {error}")))
            } else {
                tokio::task::spawn_blocking(move || {
                    work.run(|session| {
                        let mut scale = action_scale.lock().unwrap();
                        execute_action(&action_call, &action_name, &mut scale, session)
                    })
                })
                .await
                .unwrap_or_else(|error| Err(format!("task panic: {error}")))
            }
        };
        tokio::pin!(action_future);
        let result = loop {
            tokio::select! {
                result = &mut action_future => break Some(result),
                frame = read.next() => match frame {
                    Some(Ok(Message::Ping(data))) => {
                        if write.send(Message::Pong(data)).await.is_err() {
                            session.cancel_connection();
                            stop_recording_after_connection_loss();
                            app.emit("screen-recording-state", false).ok();
                            overlay::hide(app);
                            let _ = action_future.await;
                            break None;
                        }
                    }
                    Some(Ok(Message::Pong(_))) => last_pong = std::time::Instant::now(),
                    Some(Ok(frame @ (Message::Text(_) | Message::Binary(_)))) => {
                        if enqueue_request(
                            &mut queued_requests,
                            &mut queued_request_bytes,
                            frame,
                        )
                        .is_err()
                        {
                            session.cancel_connection();
                            stop_recording_after_connection_loss();
                            app.emit("screen-recording-state", false).ok();
                            overlay::hide(app);
                            let _ = action_future.await;
                            break None;
                        }
                    }
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => {
                        session.cancel_connection();
                        stop_recording_after_connection_loss();
                        app.emit("screen-recording-state", false).ok();
                        overlay::hide(app);
                        let _ = action_future.await;
                        break None;
                    }
                    Some(Ok(_)) => {}
                }
            }
        };
        let Some(result) = result else {
            break;
        };

        if session.cancelled() {
            break;
        }

        // Detect screen recording start/stop
        if action == "start_recording" && result.is_ok() {
            let mut rec = RECORDING_ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
            *rec = true;
            overlay::show(app);
            app.emit("screen-recording-state", true).ok();
        } else if action == "stop_recording" {
            let mut rec = RECORDING_ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
            *rec = false;
            app.emit("screen-recording-state", false).ok();
        }

        // Show overlay after any action (restore if hidden for screenshot)
        overlay::show(app);
        if hide_for_screenshot {
            overlay::set_visible(app, true);
        }

        // Build human-readable detail for the overlay
        let detail = match action.as_str() {
            "key" => call
                .get("key")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            "type" => {
                let t = call.get("text").and_then(|v| v.as_str()).unwrap_or("");
                let preview: String = t.chars().take(30).collect();
                if t.chars().count() > 30 {
                    format!("{preview}...")
                } else {
                    preview
                }
            }
            "left_click" | "right_click" | "double_click" | "middle_click" => {
                let (x, y) = parse_coordinate(&call);
                format!("{x}, {y}")
            }
            "scroll" => call
                .get("scroll_direction")
                .and_then(|v| v.as_str())
                .unwrap_or("down")
                .to_string(),
            "bash" => {
                let c = call.get("command").and_then(|v| v.as_str()).unwrap_or("");
                let preview: String = c.chars().take(40).collect();
                if c.chars().count() > 40 {
                    format!("{preview}...")
                } else {
                    preview
                }
            }
            _ => String::new(),
        };
        overlay::emit_action_detail(
            app,
            &crate::companion_relay::redact_secret(&action, auth_token),
            &crate::companion_relay::redact_secret(&detail, auth_token),
        );

        let response = match &result {
            Ok(AgentResult::Screenshot {
                image,
                width,
                height,
                scale,
            }) => serde_json::json!({
                "type": "action_result",
                "request_id": request_id,
                "result_type": "screenshot",
                "image": image,
                "width": width,
                "height": height,
                "scale": scale,
                "success": true,
            }),
            Ok(AgentResult::Action) => serde_json::json!({
                "type": "action_result",
                "request_id": request_id,
                "result_type": "action",
                "success": true,
            }),
            Ok(AgentResult::Output(text)) => serde_json::json!({
                "type": "action_result",
                "request_id": request_id,
                "result_type": "output",
                "success": true,
                "error": text, // reuse error field for output text
            }),
            Err(e) => serde_json::json!({
                "type": "action_result",
                "request_id": request_id,
                "result_type": "action",
                "success": false,
                "error": e,
            }),
        };

        if write
            .send(Message::Text(response.to_string().into()))
            .await
            .is_err()
        {
            break;
        }
    }

    // Connection lost — reset everything and hide overlay
    {
        let mut rec = RECORDING_ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
        *rec = false;
    }
    overlay::emit_idle(app);
    app.emit("screen-recording-state", false).ok();
    overlay::hide(app);

    Ok(())
}

enum AgentResult {
    Screenshot {
        image: String,
        width: u32,
        height: u32,
        scale: f64,
    },
    Action,
    /// Text output (bash stdout, file content, directory listing).
    Output(String),
}

fn execute_action(
    call: &serde_json::Value,
    action: &str,
    cached_scale: &mut f64,
    session: &Session,
) -> Result<AgentResult, String> {
    match action {
        "screenshot" => {
            let result = computer_use::computer_screenshot()?;
            *cached_scale = result.scale;
            Ok(AgentResult::Screenshot {
                image: result.image,
                width: result.width,
                height: result.height,
                scale: result.scale,
            })
        }
        "left_click" | "right_click" | "middle_click" => {
            let (x, y) = parse_coordinate(call);
            let button = action.trim_end_matches("_click").to_string();
            computer_use::computer_click(x, y, *cached_scale, button)?;
            Ok(AgentResult::Action)
        }
        "double_click" => {
            let (x, y) = parse_coordinate(call);
            computer_use::computer_double_click(x, y, *cached_scale)?;
            Ok(AgentResult::Action)
        }
        "mouse_move" => {
            let (x, y) = parse_coordinate(call);
            computer_use::computer_mouse_move(x, y, *cached_scale)?;
            Ok(AgentResult::Action)
        }
        "type" => {
            let text = call["text"].as_str().unwrap_or("").to_string();
            computer_use::computer_type(text)?;
            Ok(AgentResult::Action)
        }
        "key" => {
            let key = call["key"].as_str().unwrap_or("").to_string();
            computer_use::computer_key(key)?;
            Ok(AgentResult::Action)
        }
        "scroll" => {
            let (x, y) = parse_coordinate(call);
            let direction = call["scroll_direction"].as_str().unwrap_or("down");
            let amount = call["scroll_amount"].as_i64().unwrap_or(3) as i32;
            let (dx, dy) = match direction {
                "up" => (0, amount),
                "down" => (0, -amount),
                "left" => (-amount, 0),
                "right" => (amount, 0),
                _ => (0, -amount),
            };
            computer_use::computer_scroll(x, y, *cached_scale, dx, dy)?;
            Ok(AgentResult::Action)
        }
        // ── Switch desktop (macOS Spaces) ──
        "switch_desktop" => {
            let direction = call["scroll_direction"].as_str().unwrap_or("right");
            let key = match direction {
                "left" => "ctrl+left",
                "right" => "ctrl+right",
                _ => "ctrl+right",
            };
            computer_use::computer_key(key.to_string())?;
            // Wait for animation to complete
            std::thread::sleep(std::time::Duration::from_millis(700));
            Ok(AgentResult::Action)
        }
        // ── Native screen capture ──
        "start_recording" => crate::screen_recorder::start()
            .map(|_| AgentResult::Output("streaming started".into()))
            .map_err(|e| format!("start_recording: {e}")),
        "stop_recording" => crate::screen_recorder::stop()
            .map(|_| AgentResult::Output("streaming stopped".into()))
            .map_err(|e| format!("stop_recording: {e}")),
        "get_frame" => {
            // Get the latest captured frame as base64 JPEG
            match crate::screen_recorder::get_last_frame() {
                Some(jpeg) => {
                    let b64 = base64::engine::general_purpose::STANDARD.encode(&jpeg);
                    Ok(AgentResult::Screenshot {
                        image: b64,
                        width: 0,
                        height: 0,
                        scale: 1.0,
                    })
                }
                None => Err("no frame captured yet".into()),
            }
        }
        // ── Bash ──
        "bash" => {
            let command = call["command"].as_str().unwrap_or("").to_string();
            let cwd = call["cwd"].as_str().map(|s| s.to_string());
            execute_bash(&command, cwd.as_deref(), session)
        }
        // ── File operations ──
        "file_read" => {
            let path = expand_path(call["path"].as_str().unwrap_or(""));
            match std::fs::read_to_string(&path) {
                Ok(content) => Ok(AgentResult::Output(content)),
                Err(e) => Err(format!("read {path}: {e}")),
            }
        }
        "upload_file" => {
            // Upload a local file to the server via HTTP POST multipart
            let path = expand_path(call["path"].as_str().unwrap_or(""));
            let upload_url = call["upload_url"].as_str().unwrap_or("").to_string();
            let auth_token = call["auth_token"].as_str().unwrap_or("").to_string();
            upload_file_to_server(&path, &upload_url, &auth_token, session)
        }
        "file_write" => {
            let path = expand_path(call["path"].as_str().unwrap_or(""));
            let content = call["content"].as_str().unwrap_or("");
            if let Some(parent) = std::path::Path::new(&path).parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            match std::fs::write(&path, content) {
                Ok(_) => Ok(AgentResult::Output(format!(
                    "written {} bytes to {path}",
                    content.len()
                ))),
                Err(e) => Err(format!("write {path}: {e}")),
            }
        }
        "file_list" => {
            let path = expand_path(call["path"].as_str().unwrap_or("."));
            match std::fs::read_dir(&path) {
                Ok(entries) => {
                    let mut lines = Vec::new();
                    for entry in entries.flatten() {
                        let name = entry.file_name().to_string_lossy().to_string();
                        let meta = entry.metadata().ok();
                        let is_dir = meta.as_ref().map(|m| m.is_dir()).unwrap_or(false);
                        let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
                        if is_dir {
                            lines.push(format!("{name}/"));
                        } else {
                            lines.push(format!("{name}  ({size} bytes)"));
                        }
                    }
                    lines.sort();
                    Ok(AgentResult::Output(lines.join("\n")))
                }
                Err(e) => Err(format!("list {path}: {e}")),
            }
        }
        _ => Err(format!("unknown action: {action}")),
    }
}

// Poll the child while independently draining nonblocking pipes. On Unix we
// never wait for EOF: a detached descendant may inherit a pipe after the shell
// exits, and joining an EOF reader would deadlock disconnect/reconnect.
#[cfg(unix)]
fn cancellable_output(
    cmd: &mut std::process::Command,
    session: &Session,
) -> std::io::Result<std::process::Output> {
    use std::io::Read;
    use std::os::fd::AsRawFd;
    use std::os::unix::process::CommandExt;
    use std::process::Stdio;

    if session.cancelled() {
        return Err(std::io::ErrorKind::Interrupted.into());
    }
    cmd.process_group(0);
    let mut child = cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
    let mut stdout_pipe = child.stdout.take().unwrap();
    let mut stderr_pipe = child.stderr.take().unwrap();
    for fd in [stdout_pipe.as_raw_fd(), stderr_pipe.as_raw_fd()] {
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags >= 0 {
            unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) };
        }
    }
    fn drain<R: Read>(
        reader: &mut R,
        output: &mut Vec<u8>,
        session: &Session,
    ) -> std::io::Result<()> {
        let mut buffer = [0_u8; 8192];
        // Bound each pass so a continuously writable pipe cannot starve the
        // cancellation/process checks in the outer loop.
        for _ in 0..8 {
            if session.cancelled() {
                return Err(std::io::ErrorKind::Interrupted.into());
            }
            let remaining = MAX_COMMAND_OUTPUT_BYTES.saturating_sub(output.len());
            if remaining == 0 {
                return Err(std::io::Error::other("command output limit exceeded"));
            }
            let read_len = remaining.min(buffer.len());
            match reader.read(&mut buffer[..read_len]) {
                Ok(0) => return Ok(()),
                Ok(count) => output.extend_from_slice(&buffer[..count]),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(()),
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let kill = |child: &mut std::process::Child| {
        unsafe { libc::kill(-(child.id() as i32), libc::SIGKILL) };
        let _ = child.kill();
    };
    let status = loop {
        if let Err(error) = drain(&mut stdout_pipe, &mut stdout, session)
            .and_then(|_| drain(&mut stderr_pipe, &mut stderr, session))
        {
            kill(&mut child);
            let _ = child.wait();
            return Err(error);
        }
        if session.cancelled() {
            kill(&mut child);
            break child.wait()?;
        }
        if let Some(status) = child.try_wait()? {
            // Drain only a bounded amount already in the kernel, then close our
            // read ends even if an escaped descendant still owns a write end.
            let _ = drain(&mut stdout_pipe, &mut stdout, session);
            let _ = drain(&mut stderr_pipe, &mut stderr, session);
            kill(&mut child);
            break status;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    drop(stdout_pipe);
    drop(stderr_pipe);
    if session.cancelled() {
        return Err(std::io::ErrorKind::Interrupted.into());
    }
    Ok(std::process::Output {
        status,
        stdout,
        stderr,
    })
}

#[cfg(not(unix))]
fn cancellable_output(
    cmd: &mut std::process::Command,
    session: &Session,
) -> std::io::Result<std::process::Output> {
    use std::io::Read;
    use std::process::Stdio;
    if session.cancelled() {
        return Err(std::io::ErrorKind::Interrupted.into());
    }
    let mut child = cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    fn reader<R: Read + Send + 'static>(
        mut pipe: R,
    ) -> std::sync::mpsc::Receiver<std::io::Result<Vec<u8>>> {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut value = Vec::new();
            let mut buffer = [0_u8; 8192];
            let result = loop {
                if value.len() == MAX_COMMAND_OUTPUT_BYTES {
                    break Err(std::io::Error::other("command output limit exceeded"));
                }
                let remaining = MAX_COMMAND_OUTPUT_BYTES - value.len();
                let read_len = remaining.min(buffer.len());
                match pipe.read(&mut buffer[..read_len]) {
                    Ok(0) => break Ok(value),
                    Ok(count) => value.extend_from_slice(&buffer[..count]),
                    Err(error) => break Err(error),
                }
            };
            let _ = tx.send(result);
        });
        rx
    }
    let out = reader(stdout);
    let err = reader(stderr);
    let status = loop {
        if session.cancelled() {
            let _ = std::process::Command::new("taskkill")
                .args(["/F", "/T", "/PID", &child.id().to_string()])
                .status();
            let _ = child.kill();
            break child.wait()?;
        }
        if let Some(status) = child.try_wait()? {
            // Descendants may inherit the pipes. Best-effort tree termination and
            // bounded receives ensure reconnect never waits forever for EOF.
            let _ = std::process::Command::new("taskkill")
                .args(["/F", "/T", "/PID", &child.id().to_string()])
                .status();
            break status;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    let wait = std::time::Duration::from_secs(1);
    let stdout = out.recv_timeout(wait).map_err(|_| {
        std::io::Error::new(std::io::ErrorKind::TimedOut, "stdout drain timed out")
    })??;
    let stderr = err.recv_timeout(wait).map_err(|_| {
        std::io::Error::new(std::io::ErrorKind::TimedOut, "stderr drain timed out")
    })??;
    if session.cancelled() {
        return Err(std::io::ErrorKind::Interrupted.into());
    }
    Ok(std::process::Output {
        status,
        stdout,
        stderr,
    })
}

fn execute_bash(
    command: &str,
    cwd: Option<&str>,
    session: &Session,
) -> Result<AgentResult, String> {
    use std::process::Command;

    let mut cmd = if cfg!(target_os = "windows") {
        let mut c = Command::new("cmd");
        c.arg("/C").arg(command);
        c
    } else {
        let mut c = Command::new("sh");
        c.arg("-c").arg(command);
        c
    };

    if let Some(dir) = cwd {
        cmd.current_dir(expand_path(dir));
    }

    match cancellable_output(&mut cmd, session) {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let mut result = String::new();
            if !stdout.is_empty() {
                result.push_str(&stdout);
            }
            if !stderr.is_empty() {
                if !result.is_empty() {
                    result.push('\n');
                }
                result.push_str("[stderr] ");
                result.push_str(&stderr);
            }
            if result.is_empty() {
                result = format!("(exit code: {})", output.status.code().unwrap_or(-1));
            }
            if output.status.success() {
                Ok(AgentResult::Output(result))
            } else {
                Err(result)
            }
        }
        Err(e) => Err(format!("failed to run command: {e}")),
    }
}

fn expand_path(path: &str) -> String {
    if path.starts_with('~') {
        if let Some(home) = dirs::home_dir() {
            return path.replacen('~', &home.to_string_lossy(), 1);
        }
    }
    path.to_string()
}

fn parse_coordinate(call: &serde_json::Value) -> (i32, i32) {
    let coord = &call["coordinate"];
    let x = coord.get(0).and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    let y = coord.get(1).and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    (x, y)
}

fn hostname() -> String {
    gethostname::gethostname().to_string_lossy().to_string()
}

/// Upload a local file to the server via curl.
/// Returns the upload_id on success.
fn upload_file_to_server(
    path: &str,
    upload_url: &str,
    auth_token: &str,
    session: &Session,
) -> Result<AgentResult, String> {
    // Check file exists and has content
    let size = match std::fs::metadata(path) {
        Ok(m) => m.len(),
        Err(e) => return Err(format!("file not found: {path} ({e})")),
    };
    if size < 1000 {
        return Err(format!(
            "file too small ({size} bytes), recording may not have started: {path}"
        ));
    }

    let output = cancellable_output(
        std::process::Command::new("curl").args([
            "-s",
            "-w",
            "\n%{http_code}",
            "-X",
            "POST",
            "-H",
            &format!("Authorization: Bearer {auth_token}"),
            "-F",
            &format!("file=@{path}"),
            upload_url,
        ]),
        session,
    )
    .map_err(|_| "Upload failed")?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    if !output.status.success() {
        return Err(format!(
            "curl exit {}: {stderr}",
            output.status.code().unwrap_or(-1)
        ));
    }

    // stdout ends with \nHTTP_CODE, split it
    let (body, _status) = stdout.rsplit_once('\n').unwrap_or((&stdout, ""));

    if let Ok(json) = serde_json::from_str::<serde_json::Value>(body) {
        let upload_id = json["id"].as_str().unwrap_or("").to_string();
        if upload_id.is_empty() {
            return Err(format!("server returned no id: {body}"));
        }

        Ok(AgentResult::Output(upload_id))
    } else {
        Err(format!("unexpected response: {body}"))
    }
}

fn machine_request(
    instance_url: &str,
    auth_token: &str,
) -> Result<tokio_tungstenite::tungstenite::http::Request<()>, String> {
    let mut url = crate::connection_url(instance_url)?;
    url.set_path(&format!(
        "{}/api/agents/ws/machine",
        url.path().trim_end_matches('/')
    ));
    let scheme = if url.scheme() == "https" { "wss" } else { "ws" };
    url.set_scheme(scheme)
        .map_err(|_| "Invalid WebSocket URL")?;
    let mut request = url
        .as_str()
        .into_client_request()
        .map_err(|_| "Invalid WebSocket request")?;
    request.headers_mut().insert(
        "Authorization",
        format!("Bearer {auth_token}")
            .parse()
            .map_err(|_| "Invalid auth token")?,
    );
    Ok(request)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concurrent_request_queue_preserves_text_binary_order_and_bounds() {
        let mut queue = std::collections::VecDeque::new();
        let mut bytes = 0;
        enqueue_request(
            &mut queue,
            &mut bytes,
            Message::Text(r#"{"request_id":"one"}"#.into()),
        )
        .unwrap();
        enqueue_request(
            &mut queue,
            &mut bytes,
            Message::Binary(br#"{"request_id":"two"}"#.to_vec().into()),
        )
        .unwrap();
        let ids: Vec<_> = queue
            .drain(..)
            .map(|frame| {
                serde_json::from_str::<serde_json::Value>(&request_text(frame).unwrap()).unwrap()
                    ["request_id"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            })
            .collect();
        assert_eq!(ids, ["one", "two"]);
        let oversized = Message::Binary(vec![0; MAX_QUEUED_REQUEST_BYTES + 1].into());
        assert!(enqueue_request(&mut queue, &mut bytes, oversized).is_err());
    }

    #[test]
    fn connection_loss_stops_active_screen_capture_state() {
        crate::screen_recorder::mark_recording_for_test();
        *RECORDING_ACTIVE.lock().unwrap() = true;
        stop_recording_after_connection_loss();
        assert!(!crate::screen_recorder::is_recording());
        assert!(!*RECORDING_ACTIVE.lock().unwrap());
    }

    async fn work(session: &Session) -> Work {
        Work {
            _permit: session.work.clone().acquire_owned().await.unwrap(),
            session: session.clone(),
        }
    }

    #[tokio::test]
    async fn queued_callback_is_cancelled_and_drain_waits_for_callback_exit() {
        let session = Session::new();
        let queued = work(&session).await;
        session.cancel();
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), session.drain())
                .await
                .is_err()
        );
        assert!(queued
            .run::<()>(|_| panic!("cancelled queued action executed"))
            .is_err());
        tokio::time::timeout(std::time::Duration::from_secs(1), session.drain())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn in_flight_blocking_action_must_finish_before_disconnect_and_new_generation() {
        let session = Session::new();
        let active = work(&session).await;
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let task = tokio::task::spawn_blocking(move || {
            active.run(|_| {
                started_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                Ok("old result")
            })
        });
        started_rx.await.unwrap();
        session.cancel();
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), session.drain())
                .await
                .is_err()
        );
        release_tx.send(()).unwrap();
        let result = task.await.unwrap().unwrap();
        session.drain().await;
        let next = Session::new();
        assert!(session.cancelled(), "old result {result} must be discarded");
        assert!(!next.cancelled());
        assert_eq!(
            work(&next).await.run(|_| Ok("new result")).unwrap(),
            "new result"
        );
        session.cancel();
        assert!(
            !next.cancelled(),
            "old cancellation must not affect reconnect"
        );
    }

    #[tokio::test]
    async fn stopped_generation_cannot_publish_an_in_flight_result_to_reconnect() {
        let old = Session::new();
        let active = work(&old).await;
        let running = old.clone();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let task = tokio::spawn(async move {
            let result = running
                .until_stopped(async move {
                    tokio::task::spawn_blocking(move || {
                        active.run(|_| {
                            started_tx.send(()).unwrap();
                            release_rx.recv().unwrap();
                            Ok("stale")
                        })
                    })
                    .await
                    .unwrap()
                })
                .await;
            running.drain().await;
            result
        });
        started_rx.await.unwrap();
        old.cancel();
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), old.drain())
                .await
                .is_err()
        );
        release_tx.send(()).unwrap();
        assert!(task.await.unwrap().is_none());
        let new = Session::new();
        assert_eq!(new.until_stopped(async { "fresh" }).await, Some("fresh"));
        assert!(old
            .until_stopped(async { panic!("stale socket polled") })
            .await
            .is_none());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cancellation_kills_subprocess_and_descendants_and_drains_work() {
        let session = Session::new();
        let active = work(&session).await;
        let marker = std::env::temp_dir().join(format!("nolune-cancel-{}", uuid::Uuid::new_v4()));
        let command = format!("touch '{}'; sleep 30 & wait", marker.display());
        let task = tokio::task::spawn_blocking(move || {
            active.run(|session| execute_bash(&command, None, session))
        });
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            while !marker.exists() {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        session.cancel();
        tokio::time::timeout(std::time::Duration::from_secs(3), session.drain())
            .await
            .unwrap();
        assert!(task.await.unwrap().is_err());
        std::fs::remove_file(marker).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn network_generation_cancellation_stops_subprocess_and_allows_reconnect() {
        let session = Session::new();
        session.begin_connection();
        let active = work(&session).await;
        let task = tokio::task::spawn_blocking(move || {
            active.run(|session| execute_bash("sleep 30", None, session))
        });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        session.cancel_connection();
        tokio::time::timeout(std::time::Duration::from_secs(2), session.drain())
            .await
            .expect("WebSocket EOF must cancel and drain the active subprocess");
        assert!(task.await.unwrap().is_err());
        assert!(session.cancelled());
        session.begin_connection();
        assert!(
            !session.cancelled(),
            "network EOF must not cancel the reconnect loop"
        );
        assert_eq!(
            work(&session).await.run(|_| Ok("reconnected")).unwrap(),
            "reconnected"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn detached_pipe_holder_cannot_block_command_completion_or_drain() {
        let session = Session::new();
        let active = work(&session).await;
        let task = tokio::task::spawn_blocking(move || {
            active.run(|session| {
                execute_bash(
                    "python3 -c 'import os,time; os.setsid(); time.sleep(2)' &",
                    None,
                    session,
                )
            })
        });
        tokio::time::timeout(std::time::Duration::from_millis(750), task)
            .await
            .expect("detached descendants holding pipes must not block completion")
            .unwrap()
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_millis(100), session.drain())
            .await
            .unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn subprocess_output_is_bounded() {
        let session = Session::new();
        let result = execute_bash(
            "python3 -c 'import sys; sys.stdout.write(\"x\" * 9000000)'",
            None,
            &session,
        );
        let error = match result {
            Err(error) => error,
            Ok(_) => panic!("unbounded command output unexpectedly succeeded"),
        };
        assert!(error.contains("output limit"));
    }

    #[test]
    fn machine_auth_uses_header_and_rejects_base_paths() {
        assert!(machine_request("https://example.org:8443/nolune/", "secret").is_err());
        let request = machine_request("https://example.org:8443/", "secret").unwrap();
        assert_eq!(
            request.uri().to_string(),
            "wss://example.org:8443/api/agents/ws/machine"
        );
        assert_eq!(request.headers()["Authorization"], "Bearer secret");
        assert!(request.uri().query().is_none());
        assert_eq!(
            machine_request("http://localhost:3000", "secret")
                .unwrap()
                .uri()
                .scheme_str(),
            Some("ws")
        );
    }

    #[test]
    fn machine_auth_rejects_header_injection() {
        assert!(machine_request("http://localhost:3000", "secret\r\nInjected: yes").is_err());
    }
}
