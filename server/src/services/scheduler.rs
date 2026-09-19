use std::fs;
use std::time::Duration;

use chrono::Utc;
use tokio_util::sync::CancellationToken;

use crate::app::state::AppState;
use crate::domain::companion::CANONICAL_SLUG;
use crate::services::tools::ScheduledTask;
use crate::services::{chat, companion};

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
    let now = Utc::now().timestamp();
    let instance_slug = CANONICAL_SLUG.to_owned();

    for (path, scheduled) in due_scheduled_tasks(&state.workspace_dir, now) {
        // Remove the scheduled file first (prevent re-trigger on next tick)
        let _ = fs::remove_file(&path);

        // Inject the task as a user message so the agent sees it
        let label = format!("[scheduled task] {}", scheduled.task);
        let user_msg =
            chat::save_user_message(&state.workspace_dir, &instance_slug, "default", &label);

        match user_msg {
            Ok(msg) => {
                // Broadcast so the client sees it
                let _ = state
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

/// Scheduled tasks under the canonical companion that are due at `now`.
///
/// Only `instances/companion/scheduled/*.json` is scanned. Corrupt files
/// there are removed; obsolete sibling directories are never read.
pub fn due_scheduled_tasks(
    workspace_dir: &std::path::Path,
    now: i64,
) -> Vec<(std::path::PathBuf, ScheduledTask)> {
    let schedule_dir = companion::companion_dir(workspace_dir).join("scheduled");
    let Ok(files) = fs::read_dir(&schedule_dir) else {
        return Vec::new();
    };

    let mut due = Vec::new();
    for file in files.filter_map(Result::ok) {
        let path = file.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(raw) = fs::read_to_string(&path) else {
            continue;
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
        due.push((path, scheduled));
    }
    due.sort_by(|a, b| {
        a.1.deliver_at
            .cmp(&b.1.deliver_at)
            .then_with(|| a.0.cmp(&b.0))
    });
    due
}

#[cfg(test)]
mod companion_boundary_tests {
    use super::*;
    use crate::domain::companion::CANONICAL_SLUG;
    use std::path::Path;

    fn write_task(dir: &Path, id: &str, deliver_at: i64) {
        fs::create_dir_all(dir).unwrap();
        let task = ScheduledTask {
            id: id.into(),
            task: format!("task {id}"),
            deliver_at,
            created_at: 0,
        };
        fs::write(
            dir.join(format!("{id}.json")),
            serde_json::to_string(&task).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn only_canonical_due_tasks_are_collected_and_obsolete_dirs_stay_untouched() {
        let workspace = tempfile::tempdir().unwrap();
        let instances = workspace.path().join("instances");
        let canonical = instances.join(CANONICAL_SLUG).join("scheduled");
        let obsolete = instances.join("alice").join("scheduled");

        write_task(&canonical, "due", 10);
        write_task(&canonical, "future", 1_000);
        fs::write(canonical.join("corrupt.json"), "{").unwrap();
        write_task(&obsolete, "alice-due", 10);
        fs::write(obsolete.join("corrupt.json"), "{").unwrap();

        let due = due_scheduled_tasks(workspace.path(), 100);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].1.id, "due");
        assert_eq!(due[0].0, canonical.join("due.json"));

        assert!(canonical.join("future.json").is_file());
        assert!(
            !canonical.join("corrupt.json").exists(),
            "corrupt canonical entries are removed"
        );
        assert!(obsolete.join("alice-due.json").is_file());
        assert!(
            obsolete.join("corrupt.json").is_file(),
            "obsolete directories are never modified"
        );
    }

    #[test]
    fn missing_companion_yields_no_tasks() {
        let workspace = tempfile::tempdir().unwrap();
        assert!(due_scheduled_tasks(workspace.path(), i64::MAX).is_empty());
        fs::create_dir_all(workspace.path().join("instances/alice/scheduled")).unwrap();
        assert!(due_scheduled_tasks(workspace.path(), i64::MAX).is_empty());
    }
}
