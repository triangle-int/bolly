use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use std::sync::Mutex;
use tauri::Emitter;
use tokio_tungstenite::tungstenite::Message;

use crate::computer_use;
use crate::overlay;

/// Whether the bridge is active.
static BRIDGE_ACTIVE: Mutex<bool> = Mutex::new(false);

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
#[tauri::command]
pub async fn connect_computer_use(
    app: tauri::AppHandle,
    instance_url: String,
    auth_token: String,
) -> Result<(), String> {
    // Stop any existing connection first
    let was_active = {
        let mut active = BRIDGE_ACTIVE.lock().map_err(|e| e.to_string())?;
        let prev = *active;
        *active = false;
        prev
    };
    if was_active {
        // Give the old loop time to see the flag and exit
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    {
        let mut active = BRIDGE_ACTIVE.lock().map_err(|e| e.to_string())?;
        *active = true;
    }
    {
        let mut url = SERVER_URL.lock().map_err(|e| e.to_string())?;
        *url = Some(instance_url.clone());
    }

    tokio::spawn(async move {
        loop {
            // Check if still active
            {
                let active = BRIDGE_ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
                if !*active {
                    break;
                }
            }

            match run_agent(&app, &instance_url, &auth_token).await {
                Ok(_) => {
                    eprintln!("[agent] connection closed, reconnecting in 3s...");
                }
                Err(e) => {
                    eprintln!("[agent] error: {e}, reconnecting in 3s...");
                }
            }

            // Check if still active before reconnecting
            {
                let active = BRIDGE_ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
                if !*active {
                    break;
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        }
        eprintln!("[agent] bridge stopped");
    });

    Ok(())
}

#[tauri::command]
pub fn disconnect_computer_use(app: tauri::AppHandle) -> Result<(), String> {
    let mut active = BRIDGE_ACTIVE.lock().map_err(|e| e.to_string())?;
    *active = false;
    drop(active);
    // Hide overlay immediately
    overlay::hide(&app);
    Ok(())
}

#[tauri::command]
pub fn set_screen_recording_allowed(allowed: bool) -> Result<(), String> {
    let mut val = SCREEN_RECORDING_ALLOWED.lock().map_err(|e| e.to_string())?;
    *val = allowed;
    Ok(())
}

#[tauri::command]
pub fn get_server_url() -> Result<String, String> {
    SERVER_URL
        .lock()
        .map_err(|e| e.to_string())?
        .clone()
        .ok_or_else(|| "not connected".into())
}

#[tauri::command]
pub fn get_screen_recording_allowed() -> Result<bool, String> {
    let val = SCREEN_RECORDING_ALLOWED.lock().map_err(|e| e.to_string())?;
    Ok(*val)
}

/// Set the instance slug this machine is bound to.
#[tauri::command]
pub fn set_instance_slug(slug: String) -> Result<(), String> {
    let mut val = INSTANCE_SLUG.lock().map_err(|e| e.to_string())?;
    *val = if slug.is_empty() { None } else { Some(slug) };
    Ok(())
}

/// Stop any active screen recording (kill ffmpeg).
#[tauri::command]
pub fn stop_screen_recording(app: tauri::AppHandle) -> Result<(), String> {
    let mut rec = RECORDING_ACTIVE.lock().map_err(|e| e.to_string())?;
    *rec = false;
    std::process::Command::new("sh")
        .args(["-c", "pkill -f 'ffmpeg.*bolly_screen' 2>/dev/null"])
        .spawn()
        .ok();
    app.emit("screen-recording-state", false).ok();
    overlay::hide(&app);
    Ok(())
}

async fn run_agent(
    app: &tauri::AppHandle,
    instance_url: &str,
    auth_token: &str,
) -> Result<(), String> {
    let ws_proto = if instance_url.starts_with("https") {
        "wss"
    } else {
        "ws"
    };
    let host = instance_url
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    let ws_url = format!(
        "{ws_proto}://{host}/api/agents/ws/machine?token={}",
        urlencoding::encode(auth_token)
    );

    eprintln!("[agent] connecting to {ws_url}");

    let (ws, _) = tokio_tungstenite::connect_async(&ws_url)
        .await
        .map_err(|e| format!("ws connect: {e}"))?;

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
    // URL parsing removed — tenant slug != instance directory name.
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

    // Main loop: receive toolcalls, execute, send results
    loop {
        // Check if bridge is still active
        {
            let active = BRIDGE_ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
            if !*active {
                break;
            }
        }

        let text;

        tokio::select! {
            msg = read.next() => {
                match msg {
                    Some(Ok(Message::Text(t))) => {
                        text = t.to_string();
                    }
                    Some(Ok(Message::Ping(d))) => {
                        let _ = write.send(Message::Pong(d)).await;
                        continue;
                    }
                    Some(Ok(Message::Pong(_))) => {
                        last_pong = std::time::Instant::now();
                        continue;
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => continue,
                    Some(Err(e)) => {
                        eprintln!("[agent] ws error: {e}");
                        break;
                    }
                }
            }
            _ = ping_interval.tick() => {
                if write.send(Message::Ping(vec![].into())).await.is_err() {
                    eprintln!("[agent] ping send failed, reconnecting...");
                    break;
                }
                if last_pong.elapsed() > std::time::Duration::from_secs(45) {
                    eprintln!("[agent] no pong in 45s, reconnecting...");
                    break;
                }
                continue;
            }
            _ = frame_interval.tick() => {
                // Stream latest frame to server if recording
                if crate::screen_recorder::is_recording() {
                    if let Some(jpeg) = crate::screen_recorder::get_last_frame() {
                        let b64 = base64::engine::general_purpose::STANDARD.encode(&jpeg);
                        let msg = serde_json::json!({
                            "type": "screen_frame",
                            "machine_id": machine_id,
                            "image": b64,
                            "width": 0,
                            "height": 0,
                        });
                        if write.send(Message::Text(msg.to_string().into())).await.is_err() {
                            break;
                        }
                    }
                }
                continue;
            }
        }

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

        eprintln!("[agent] toolcall: {action} (req={request_id})");

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

        let result = if is_input_action {
            let call_clone = call.clone();
            let action_clone = action.clone();
            let scale = cached_scale.clone();
            let app_handle = app.clone();
            let (tx, rx) = tokio::sync::oneshot::channel();
            let _ = app_handle.run_on_main_thread(move || {
                let mut s = scale.lock().unwrap();
                let r = execute_action(&call_clone, &action_clone, &mut *s);
                let _ = tx.send(r);
            });
            rx.await
                .unwrap_or_else(|e| Err(format!("main thread recv: {e}")))
        } else {
            let call_clone = call.clone();
            let action_clone = action.clone();
            let scale = cached_scale.clone();
            tokio::task::spawn_blocking(move || {
                let mut s = scale.lock().unwrap();
                execute_action(&call_clone, &action_clone, &mut *s)
            })
            .await
            .unwrap_or_else(|e| Err(format!("task panic: {e}")))
        };

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
        overlay::emit_action_detail(app, &action, &detail);

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
            execute_bash(&command, cwd.as_deref())
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
            upload_file_to_server(&path, &upload_url, &auth_token)
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

fn execute_bash(command: &str, cwd: Option<&str>) -> Result<AgentResult, String> {
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

    match cmd.output() {
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

    eprintln!(
        "[upload] uploading {path} ({:.1} MB) to {upload_url}",
        size as f64 / 1024.0 / 1024.0
    );

    let output = std::process::Command::new("curl")
        .args([
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
        ])
        .output()
        .map_err(|e| format!("curl failed: {e}"))?;

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
        eprintln!("[upload] success: {upload_id}");
        Ok(AgentResult::Output(upload_id))
    } else {
        Err(format!("unexpected response: {body}"))
    }
}
