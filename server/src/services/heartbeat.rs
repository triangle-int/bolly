//! Heartbeat — runs the companion's routines on their intervals through the
//! one proactive loop (#92, #93). There is no configurable agent set: the
//! check-in always runs and reflection runs when the user opts in.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::Utc;
use tokio::sync::{RwLock, broadcast};

use crate::domain::events::ServerEvent;
use crate::domain::proactive::{ProactivePolicy, Target, Trigger};
use crate::domain::thought::Thought;
use crate::services::companion_routine::{self, Routine};
use crate::services::proactive::{Admission, ProactiveLoop, outcome_from_trace};
use crate::services::tools::load_mood_state;
use crate::services::{chat, companion, llm::LlmBackend, rhythm, thoughts};

/// Routines the heartbeat keeps alive. Reflection stays in the list so an
/// opt-in later takes effect without a restart; its loop idles while disabled.
pub fn routines() -> [Routine; 2] {
    [Routine::CheckIn, Routine::Reflection]
}

/// Seconds to wait before a routine is due again.
pub fn wait_secs(last_finished: Option<i64>, interval_hours: f64, now: i64) -> u64 {
    let interval = interval_secs(interval_hours);
    match last_finished {
        Some(finished) => {
            let elapsed = (now - finished).max(0) as u64;
            interval.saturating_sub(elapsed)
        }
        None => 0,
    }
}

fn interval_secs(interval_hours: f64) -> u64 {
    (interval_hours.max(0.25) * 3600.0) as u64
}

pub fn start(
    workspace_dir: &Path,
    llm: Arc<RwLock<Option<LlmBackend>>>,
    events: broadcast::Sender<ServerEvent>,
    vector_store: Arc<crate::services::vector::VectorStore>,
    resources: crate::services::resource_access::ResourceAccess,
    proactive: ProactiveLoop,
) {
    // One companion per server: only the canonical identity has an inner life.
    if !companion::companion_dir(workspace_dir)
        .join("soul.md")
        .exists()
    {
        log::info!("heartbeat: companion not onboarded yet; no loops spawned");
        return;
    }

    for routine in routines() {
        let ws = workspace_dir.to_path_buf();
        let llm = llm.clone();
        let events = events.clone();
        let vector_store = vector_store.clone();
        let resources = resources.clone();
        let proactive = proactive.clone();
        tokio::spawn(async move {
            routine_loop(
                &ws,
                routine,
                llm,
                events,
                vector_store,
                resources,
                proactive,
            )
            .await;
        });
        log::info!("heartbeat: spawned '{}' routine", routine.name());
    }
}

async fn routine_loop(
    workspace_dir: &Path,
    routine: Routine,
    llm: Arc<RwLock<Option<LlmBackend>>>,
    events: broadcast::Sender<ServerEvent>,
    vector_store: Arc<crate::services::vector::VectorStore>,
    resources: crate::services::resource_access::ResourceAccess,
    proactive: ProactiveLoop,
) {
    let slug = crate::domain::companion::CANONICAL_SLUG;
    let instance_dir = companion::companion_dir(workspace_dir);
    let trigger = Trigger::Heartbeat {
        agent: routine.name().to_owned(),
    };

    loop {
        if !instance_dir.join("soul.md").exists() {
            log::info!("[heartbeat] {}: soul.md gone, stopping", routine.name());
            break;
        }
        let policy: ProactivePolicy = proactive.policy();
        if !routine.enabled(&policy) {
            tokio::time::sleep(Duration::from_secs(300)).await;
            continue;
        }
        let wait = wait_secs(
            proactive.last_finished_for(&trigger),
            routine.interval_hours(&policy),
            Utc::now().timestamp(),
        );
        if wait > 0 {
            log::info!("[heartbeat] {}: next run in {}m", routine.name(), wait / 60);
            tokio::time::sleep(Duration::from_secs(wait)).await;
            continue;
        }

        let llm_guard = llm.read().await;
        if let Some(backend) = llm_guard.as_ref() {
            run_tick(
                workspace_dir,
                slug,
                &instance_dir,
                backend,
                &events,
                &vector_store,
                routine,
                &resources,
                &proactive,
            )
            .await;
        }
        drop(llm_guard);
        // Whatever happened, do not spin: wait a full interval unless a skip
        // said otherwise. Skips do not count as finished runs.
        tokio::time::sleep(Duration::from_secs(interval_secs(
            routine.interval_hours(&proactive.policy()),
        )))
        .await;
    }
}

/// One admitted wake-up of a routine.
#[allow(clippy::too_many_arguments)]
async fn run_tick(
    workspace_dir: &Path,
    slug: &str,
    instance_dir: &Path,
    llm: &LlmBackend,
    events: &broadcast::Sender<ServerEvent>,
    vector_store: &Arc<crate::services::vector::VectorStore>,
    routine: Routine,
    resources: &crate::services::resource_access::ResourceAccess,
    proactive: &ProactiveLoop,
) {
    let handle = match proactive.begin(
        Trigger::Heartbeat {
            agent: routine.name().to_owned(),
        },
        routine.description(),
        Target::Companion,
    ) {
        Admission::Admitted(handle) => handle,
        Admission::Skipped(run) => {
            log::info!("[heartbeat] {}: skipped ({:?})", routine.name(), run.status);
            return;
        }
    };
    let run_id = handle.id().to_owned();
    let cancelled = handle.token();
    log::info!("[heartbeat] running '{}' ({run_id})", routine.name());

    // Rhythm hints (check-in only): the incremental aggregate, never a rescan (#95).
    if routine == Routine::CheckIn && rhythm::tracking_enabled(workspace_dir, slug) {
        let rhythm_data = rhythm::load_rhythm(instance_dir);
        let insights = rhythm::build_rhythm_insights(workspace_dir, slug, &rhythm_data);
        if !insights.trim().is_empty() {
            let label = format!("[system] rhythm update\n{insights}");
            let _ = chat::save_system_message(workspace_dir, slug, "default", &label);
        }
    }

    let work = companion_routine::run(
        workspace_dir,
        slug,
        instance_dir,
        llm,
        events,
        vector_store,
        resources,
        routine,
        None,
        "heartbeat",
        (proactive, run_id.as_str()),
    );
    let result = tokio::select! {
        result = work => result,
        _ = cancelled.cancelled() => {
            handle.cancel();
            log::info!("[heartbeat] {}: cancelled", routine.name());
            return;
        }
    };
    match result {
        Ok(r) => {
            let _ = chat::save_system_message(
                workspace_dir,
                slug,
                "default",
                &format!(
                    "[system] routine '{}' ran ({} tokens)",
                    routine.name(),
                    r.tokens
                ),
            );
            log::info!("[heartbeat] {}: done ({} tokens)", routine.name(), r.tokens);

            // Thought capture stays until #94 replaces it with activity receipts.
            let final_mood = load_mood_state(instance_dir).companion_mood;
            let thought = Thought {
                id: format!("thought_{}", unix_millis()),
                raw: r.response,
                actions: vec![format!("wake:{}", routine.name())],
                mood: final_mood,
                created_at: unix_millis().to_string(),
            };
            let _ = thoughts::save_thought(workspace_dir, slug, &thought);
            let _ = events.send(ServerEvent::HeartbeatThought {
                instance_slug: slug.to_string(),
                thought,
            });
            handle.complete(outcome_from_trace(&r.trace, r.tokens));
        }
        Err(e) => {
            log::warn!("[heartbeat] {}: failed: {e}", routine.name());
            handle.fail(&e.to_string(), true);
        }
    }
}

fn unix_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after unix epoch")
        .as_millis()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routine_schedule_derives_from_the_last_finished_run() {
        assert_eq!(wait_secs(None, 1.0, 1_000), 0, "never run: due now");
        assert_eq!(wait_secs(Some(1_000), 1.0, 1_000 + 600), 3_000);
        assert_eq!(wait_secs(Some(1_000), 1.0, 1_000 + 3_600), 0);
        assert_eq!(
            wait_secs(Some(5_000), 1.0, 1_000),
            3_600,
            "clock skew never underflows"
        );
        assert_eq!(wait_secs(None, 0.01, 0), 0);
        assert_eq!(interval_secs(0.01), 900, "intervals floor at 15 minutes");
    }

    #[test]
    fn only_the_check_in_and_reflection_routines_exist() {
        assert_eq!(routines(), [Routine::CheckIn, Routine::Reflection]);
    }
}
