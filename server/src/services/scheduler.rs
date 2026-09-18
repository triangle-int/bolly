use std::fs;
use std::time::Duration;

use chrono::Utc;
use tokio_util::sync::CancellationToken;

use crate::app::state::AppState;
use crate::services::chat;
use crate::services::tools::ScheduledTask;

/// Spawn a background task that checks for scheduled tasks every 30 seconds.
pub fn start(state: AppState) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(30));
        loop {
            interval.tick().await;
            check_and_trigger(&state).await;
        }
    });
    log::info!("scheduler started — checking for scheduled tasks every 30s");
}

async fn check_and_trigger(state: &AppState) {
    let instances_dir = state.workspace_dir.join("instances");
    let entries = match fs::read_dir(&instances_dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    let now = Utc::now().timestamp();

    for entry in entries.filter_map(Result::ok) {
        let instance_dir = entry.path();
        if !instance_dir.is_dir() {
            continue;
        }
        let instance_slug = entry.file_name().to_string_lossy().to_string();
        let schedule_dir = instance_dir.join("scheduled");
        if !schedule_dir.is_dir() {
            continue;
        }

        let files = match fs::read_dir(&schedule_dir) {
            Ok(f) => f,
            Err(_) => continue,
        };

        for file in files.filter_map(Result::ok) {
            let path = file.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }

            let raw = match fs::read_to_string(&path) {
                Ok(r) => r,
                Err(_) => continue,
            };

            let scheduled: ScheduledTask = match serde_json::from_str(&raw) {
                Ok(s) => s,
                Err(_) => {
                    // Corrupt file, remove it
                    let _ = fs::remove_file(&path);
                    continue;
                }
            };

            if scheduled.deliver_at > now {
                continue; // Not yet due
            }

            // Remove the scheduled file first (prevent re-trigger on next tick)
            let _ = fs::remove_file(&path);

            // Inject the task as a user message so the agent sees it
            let label = format!("[scheduled task] {}", scheduled.task);
            let user_msg =
                chat::save_user_message(&state.workspace_dir, &instance_slug, "default", &label);

            match user_msg {
                Ok(msg) => {
                    // Broadcast so the client sees it
                    let _ =
                        state
                            .events
                            .send(crate::domain::events::ServerEvent::ChatMessageCreated {
                                instance_slug: instance_slug.clone(),
                                chat_id: "default".to_string(),
                                message: msg,
                            });
                }
                Err(e) => {
                    log::warn!("[scheduler] failed to save task message for {instance_slug}: {e}");
                    continue;
                }
            }

            // Trigger the agent loop (same mechanism as POST /api/chat)
            let key = format!("{instance_slug}/default");
            let already_running = {
                let tasks = state.agent_tasks.lock().await;
                tasks.contains_key(&key)
            };

            if !already_running {
                let cancel = CancellationToken::new();
                {
                    let mut tasks = state.agent_tasks.lock().await;
                    if !tasks.contains_key(&key) {
                        tasks.insert(key.clone(), cancel.clone());
                    }
                }

                let bg_state = state.clone();
                let bg_slug = instance_slug.clone();
                tokio::spawn(async move {
                    crate::routes::chat::run_agent_loop(
                        bg_state,
                        bg_slug,
                        "default".to_string(),
                        cancel,
                        false,
                    )
                    .await;
                });

                log::info!(
                    "[scheduler] triggered agent for {instance_slug}: {}",
                    scheduled.task
                );
            } else {
                log::info!(
                    "[scheduler] agent already running for {instance_slug}, task injected as message"
                );
            }
        }
    }
}
