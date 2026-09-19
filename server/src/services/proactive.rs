//! The one proactive companion loop (#92).
//!
//! Every piece of work the companion starts on its own passes through
//! [`ProactiveLoop::begin`], which admits or skips it under one policy
//! (enabled, quiet hours, cooldown, duplicate suppression), persists one
//! bounded [`ProactiveRun`] record, and hands back a [`RunHandle`] that the
//! caller completes, fails, or cancels. Side effects that leave companion
//! storage ask [`ProactiveLoop::approve_side_effect`], which enforces quiet
//! hours and the daily attention budget and records the decision on the run.

use std::{
    collections::HashMap,
    fs, io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use chrono::{TimeZone, Timelike, Utc};
use tokio_util::sync::CancellationToken;

use crate::domain::proactive::{
    ACTIVITY_FORMAT_VERSION, ActionReceipt, Approval, MAX_REASON_CHARS, ProactivePolicy,
    ProactiveRun, RunOutcome, RunStatus, SideEffect, SkipReason, Target, Trigger,
};

const ACTIVITY_DIR: &str = "activity";
const POLICY_FILE: &str = "proactive_policy.json";
const LEDGER_FILE: &str = ".ledger.json";
const RESTART_ERROR: &str = "interrupted by server restart";

/// Result of asking the loop to start work.
pub enum Admission {
    Admitted(RunHandle),
    Skipped(ProactiveRun),
}

/// Why a side effect was not allowed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Denied {
    QuietHours,
    AttentionBudget,
    UnknownRun,
    Io(String),
}

impl std::fmt::Display for Denied {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::QuietHours => f.write_str("quiet hours are active"),
            Self::AttentionBudget => f.write_str("daily attention budget is used up"),
            Self::UnknownRun => f.write_str("unknown proactive run"),
            Self::Io(message) => f.write_str(message),
        }
    }
}

struct Active {
    id: String,
    token: CancellationToken,
}

struct Retry {
    attempt: u32,
    of: String,
}

/// Rolling ledger of recent side effects for the attention budget.
#[derive(Default, serde::Serialize, serde::Deserialize)]
struct Ledger {
    #[serde(default)]
    reach_outs: Vec<i64>,
}

#[derive(Clone)]
pub struct ProactiveLoop {
    workspace_dir: PathBuf,
    slug: String,
    active: Arc<Mutex<HashMap<String, Active>>>,
    /// Receipt updates for connected clients (#94). None in tests that do not care.
    events: Option<tokio::sync::broadcast::Sender<crate::domain::events::ServerEvent>>,
}

impl ProactiveLoop {
    pub fn new(workspace_dir: &Path, slug: &str) -> Self {
        Self {
            workspace_dir: workspace_dir.to_path_buf(),
            slug: slug.to_owned(),
            active: Arc::new(Mutex::new(HashMap::new())),
            events: None,
        }
    }

    /// Broadcast every record change as `activity_updated`.
    pub fn with_events(
        mut self,
        events: tokio::sync::broadcast::Sender<crate::domain::events::ServerEvent>,
    ) -> Self {
        self.events = Some(events);
        self
    }

    fn instance_dir(&self) -> PathBuf {
        self.workspace_dir.join("instances").join(&self.slug)
    }

    fn activity_dir(&self) -> PathBuf {
        self.instance_dir().join(ACTIVITY_DIR)
    }

    fn run_path(&self, id: &str) -> PathBuf {
        self.activity_dir().join(format!("{id}.json"))
    }

    // ── policy ─────────────────────────────────────────────────────────────

    pub fn policy(&self) -> ProactivePolicy {
        fs::read_to_string(self.instance_dir().join(POLICY_FILE))
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }

    pub fn set_policy(&self, policy: &ProactivePolicy) -> io::Result<()> {
        fs::create_dir_all(self.instance_dir())?;
        write_atomic(
            &self.instance_dir().join(POLICY_FILE),
            &serde_json::to_string_pretty(policy).map_err(io::Error::other)?,
        )
    }

    fn local_hour(&self, now: i64) -> Option<u32> {
        let tz: chrono_tz::Tz = crate::routes::instances::read_timezone(&self.instance_dir())
            .and_then(|value| value.parse().ok())
            .unwrap_or(chrono_tz::UTC);
        Utc.timestamp_opt(now, 0)
            .single()
            .map(|utc| utc.with_timezone(&tz).hour())
    }

    fn in_quiet_hours(&self, policy: &ProactivePolicy, now: i64) -> bool {
        match (policy.quiet_hours, self.local_hour(now)) {
            (Some(quiet), Some(hour)) => quiet.contains(hour),
            _ => false,
        }
    }

    // ── lifecycle ──────────────────────────────────────────────────────────

    pub fn begin(&self, trigger: Trigger, reason: &str, target: Target) -> Admission {
        self.begin_at(trigger, reason, target, Utc::now().timestamp())
    }

    pub fn begin_at(&self, trigger: Trigger, reason: &str, target: Target, now: i64) -> Admission {
        self.admit(trigger, reason, target, now, None)
    }

    /// `retry` is present for explicit re-runs, which bypass cooldown.
    fn admit(
        &self,
        trigger: Trigger,
        reason: &str,
        target: Target,
        now: i64,
        retry: Option<Retry>,
    ) -> Admission {
        let policy = self.policy();
        let dedupe_key = trigger.dedupe_key();
        let reason: String = reason.chars().take(MAX_REASON_CHARS).collect();
        let id = new_run_id(now);
        let explicit = retry.is_some();
        let (attempt, retry_of) = match retry {
            Some(retry) => (retry.attempt, Some(retry.of)),
            None => (1, None),
        };
        let mut run = ProactiveRun {
            version: ACTIVITY_FORMAT_VERSION,
            id: id.clone(),
            trigger: trigger.clone(),
            reason,
            target,
            dedupe_key: dedupe_key.clone(),
            status: RunStatus::Running,
            attempt,
            retry_of,
            started_at: now,
            finished_at: None,
            approvals: Vec::new(),
            outcome: None,
        };

        let skip = if !policy.enabled {
            Some(SkipReason::Disabled)
        } else if let Some(active) = self.active_id(&dedupe_key) {
            Some(SkipReason::Duplicate { of: active })
        } else if trigger.respects_quiet_hours() && self.in_quiet_hours(&policy, now) {
            Some(SkipReason::QuietHours)
        } else if trigger.has_cooldown() && !explicit {
            self.last_finished(&dedupe_key).and_then(|finished| {
                let until = finished + policy.cooldown_secs as i64;
                (now < until).then_some(SkipReason::Cooldown { until })
            })
        } else {
            None
        };

        if let Some(reason) = skip {
            run.status = RunStatus::Skipped { reason };
            run.finished_at = Some(now);
            let _ = self.save(&run);
            return Admission::Skipped(run);
        }

        let token = CancellationToken::new();
        {
            let mut active = self.active.lock().unwrap_or_else(|e| e.into_inner());
            active.insert(
                dedupe_key,
                Active {
                    id: id.clone(),
                    token: token.clone(),
                },
            );
        }
        let _ = self.save(&run);
        Admission::Admitted(RunHandle {
            r#loop: self.clone(),
            run,
            token,
        })
    }

    fn active_id(&self, dedupe_key: &str) -> Option<String> {
        self.active
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(dedupe_key)
            .map(|active| active.id.clone())
    }

    /// When a trigger last finished (completed, failed, or cancelled), if ever.
    pub fn last_finished_for(&self, trigger: &Trigger) -> Option<i64> {
        self.last_finished(&trigger.dedupe_key())
    }

    fn last_finished(&self, dedupe_key: &str) -> Option<i64> {
        self.list(usize::MAX)
            .into_iter()
            .filter(|run| run.dedupe_key == dedupe_key)
            .filter(|run| {
                matches!(
                    run.status,
                    RunStatus::Completed | RunStatus::Failed { .. } | RunStatus::Cancelled
                )
            })
            .filter_map(|run| run.finished_at)
            .max()
    }

    fn finish(&self, mut run: ProactiveRun, status: RunStatus, now: i64) -> ProactiveRun {
        run.status = status;
        run.finished_at = Some(now);
        {
            let mut active = self.active.lock().unwrap_or_else(|e| e.into_inner());
            if active.get(&run.dedupe_key).is_some_and(|a| a.id == run.id) {
                active.remove(&run.dedupe_key);
            }
        }
        let _ = self.save(&run);
        run
    }

    /// Signal cancellation to a running run. Returns false when nothing is running under that id.
    pub fn cancel(&self, id: &str) -> bool {
        let active = self.active.lock().unwrap_or_else(|e| e.into_inner());
        match active.values().find(|active| active.id == id) {
            Some(active) => {
                active.token.cancel();
                true
            }
            None => false,
        }
    }

    /// Start a new attempt of a failed-retryable or cancelled run.
    pub fn retry(&self, id: &str, now: i64) -> Result<Admission, String> {
        let previous = self.get(id).ok_or_else(|| format!("unknown run {id}"))?;
        let retryable = matches!(
            previous.status,
            RunStatus::Failed {
                retryable: true,
                ..
            } | RunStatus::Cancelled
        );
        if !retryable {
            return Err(format!("run {id} is not retryable"));
        }
        Ok(self.admit(
            previous.trigger,
            &previous.reason,
            previous.target,
            now,
            Some(Retry {
                attempt: previous.attempt + 1,
                of: id.to_owned(),
            }),
        ))
    }

    /// Runs left `Running` by a previous process can never finish; mark them
    /// failed and retryable so the user can see and re-run them.
    pub fn recover_on_restart(&self, now: i64) -> usize {
        let mut recovered = 0;
        for mut run in self.list(usize::MAX) {
            if run.status == RunStatus::Running {
                run.status = RunStatus::Failed {
                    error: RESTART_ERROR.to_owned(),
                    retryable: true,
                };
                run.finished_at = Some(now);
                if self.save(&run).is_ok() {
                    recovered += 1;
                }
            }
        }
        recovered
    }

    /// Drop finished records beyond `retention_max` or older than `retention_days`.
    pub fn enforce_retention(&self, now: i64) -> usize {
        let policy = self.policy();
        let oldest_allowed = now - i64::from(policy.retention_days) * 86_400;
        let mut removed = 0;
        for (index, run) in self.list(usize::MAX).into_iter().enumerate() {
            if !run.status.is_finished() {
                continue;
            }
            let too_old = run.finished_at.unwrap_or(run.started_at) < oldest_allowed;
            if (index >= policy.retention_max || too_old)
                && fs::remove_file(self.run_path(&run.id)).is_ok()
            {
                removed += 1;
            }
        }
        removed
    }

    // ── side effects ───────────────────────────────────────────────────────

    /// Decide whether a side effect may happen now and record the decision on
    /// the run (when one is given). Denials never leave companion storage.
    pub fn approve_side_effect(
        &self,
        run_id: Option<&str>,
        side_effect: SideEffect,
        now: i64,
    ) -> Result<(), Denied> {
        let policy = self.policy();
        let decision = if self.in_quiet_hours(&policy, now) {
            Err(Denied::QuietHours)
        } else {
            match side_effect {
                SideEffect::ReachOut => {
                    let mut ledger = self.ledger();
                    ledger.reach_outs.retain(|at| now - at < 86_400);
                    if ledger.reach_outs.len() >= policy.daily_reach_out_budget as usize {
                        Err(Denied::AttentionBudget)
                    } else {
                        ledger.reach_outs.push(now);
                        self.save_ledger(&ledger)
                            .map_err(|error| Denied::Io(error.to_string()))
                    }
                }
            }
        };
        if let Some(id) = run_id {
            let mut run = self.get(id).ok_or(Denied::UnknownRun)?;
            run.approvals.push(Approval {
                side_effect,
                allowed: decision.is_ok(),
                reason: decision.as_ref().err().map(ToString::to_string),
                at: now,
            });
            self.save(&run)
                .map_err(|error| Denied::Io(error.to_string()))?;
        }
        decision
    }

    fn ledger(&self) -> Ledger {
        fs::read_to_string(self.activity_dir().join(LEDGER_FILE))
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }

    fn save_ledger(&self, ledger: &Ledger) -> io::Result<()> {
        fs::create_dir_all(self.activity_dir())?;
        write_atomic(
            &self.activity_dir().join(LEDGER_FILE),
            &serde_json::to_string(ledger).map_err(io::Error::other)?,
        )
    }

    // ── storage ────────────────────────────────────────────────────────────

    fn save(&self, run: &ProactiveRun) -> io::Result<()> {
        fs::create_dir_all(self.activity_dir())?;
        write_atomic(
            &self.run_path(&run.id),
            &serde_json::to_string_pretty(run).map_err(io::Error::other)?,
        )?;
        if let Some(events) = &self.events {
            let _ = events.send(crate::domain::events::ServerEvent::ActivityUpdated {
                instance_slug: self.slug.clone(),
                run: run.clone(),
            });
        }
        Ok(())
    }

    pub fn get(&self, id: &str) -> Option<ProactiveRun> {
        if id.contains('/') || id.contains('\\') || id.starts_with('.') {
            return None;
        }
        let raw = fs::read_to_string(self.run_path(id)).ok()?;
        serde_json::from_str(&raw).ok()
    }

    /// Newest first.
    pub fn list(&self, limit: usize) -> Vec<ProactiveRun> {
        let Ok(entries) = fs::read_dir(self.activity_dir()) else {
            return Vec::new();
        };
        let mut runs: Vec<ProactiveRun> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension().and_then(|ext| ext.to_str()) == Some("json")
                    && !path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.starts_with('.'))
            })
            .filter_map(|path| fs::read_to_string(path).ok())
            .filter_map(|raw| serde_json::from_str(&raw).ok())
            .collect();
        runs.sort_by(|a, b| {
            b.started_at
                .cmp(&a.started_at)
                .then_with(|| b.id.cmp(&a.id))
        });
        runs.truncate(limit);
        runs
    }
}

/// A running admission. Exactly one of `complete`, `fail`, or `cancel` ends it.
pub struct RunHandle {
    r#loop: ProactiveLoop,
    run: ProactiveRun,
    token: CancellationToken,
}

impl RunHandle {
    pub fn id(&self) -> &str {
        &self.run.id
    }

    pub fn token(&self) -> CancellationToken {
        self.token.clone()
    }

    #[cfg(test)]
    pub fn is_cancelled(&self) -> bool {
        self.token.is_cancelled()
    }

    pub fn complete(self, outcome: RunOutcome) -> ProactiveRun {
        self.complete_at(outcome, Utc::now().timestamp())
    }

    pub fn complete_at(self, outcome: RunOutcome, now: i64) -> ProactiveRun {
        let mut run = self.reload();
        run.outcome = Some(outcome);
        self.r#loop.finish(run, RunStatus::Completed, now)
    }

    pub fn fail(self, error: &str, retryable: bool) -> ProactiveRun {
        self.fail_at(error, retryable, Utc::now().timestamp())
    }

    pub fn fail_at(self, error: &str, retryable: bool, now: i64) -> ProactiveRun {
        let run = self.reload();
        let error: String = error.chars().take(500).collect();
        self.r#loop
            .finish(run, RunStatus::Failed { error, retryable }, now)
    }

    pub fn cancel(self) -> ProactiveRun {
        self.cancel_at(Utc::now().timestamp())
    }

    pub fn cancel_at(self, now: i64) -> ProactiveRun {
        let run = self.reload();
        self.r#loop.finish(run, RunStatus::Cancelled, now)
    }

    /// Approvals may have been appended by tools while the run executed.
    fn reload(&self) -> ProactiveRun {
        self.r#loop
            .get(&self.run.id)
            .unwrap_or_else(|| self.run.clone())
    }
}

/// Turn a tool trace into receipts. Model text is deliberately dropped.
pub fn outcome_from_trace(trace: &[crate::services::llm::Message], tokens: u64) -> RunOutcome {
    use crate::services::llm::{ContentBlock, Message};
    let mut outcome = RunOutcome {
        tokens,
        ..Default::default()
    };
    for message in trace {
        let Message::Assistant { content } = message else {
            continue;
        };
        for block in content {
            if let ContentBlock::ToolCall {
                name, arguments, ..
            } = block
            {
                if name == "reach_out" {
                    outcome.messages_sent += 1;
                }
                outcome.actions.push(ActionReceipt {
                    tool: name.clone(),
                    summary: crate::services::tools::tool_summary(name, &arguments.to_string()),
                });
            }
        }
    }
    outcome
}

fn new_run_id(now: i64) -> String {
    let suffix: String = uuid::Uuid::new_v4().simple().to_string()[..8].to_owned();
    format!("run_{now}_{suffix}")
}

fn write_atomic(path: &Path, content: &str) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, content)?;
    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::companion::CANONICAL_SLUG;
    use crate::domain::proactive::QuietHours;

    /// 2026-01-05 (Monday) 09:00:00 UTC
    const T0: i64 = 1_767_603_600;

    fn harness() -> (tempfile::TempDir, ProactiveLoop) {
        let ws = tempfile::tempdir().unwrap();
        fs::create_dir_all(ws.path().join("instances").join(CANONICAL_SLUG)).unwrap();
        let r#loop = ProactiveLoop::new(ws.path(), CANONICAL_SLUG);
        (ws, r#loop)
    }

    fn heartbeat() -> Trigger {
        Trigger::Heartbeat {
            agent: "companion".into(),
        }
    }

    fn admitted(admission: Admission) -> RunHandle {
        match admission {
            Admission::Admitted(handle) => handle,
            Admission::Skipped(run) => panic!("unexpectedly skipped: {:?}", run.status),
        }
    }

    fn skipped(admission: Admission) -> ProactiveRun {
        match admission {
            Admission::Skipped(run) => run,
            Admission::Admitted(_) => panic!("unexpectedly admitted"),
        }
    }

    #[test]
    fn every_run_has_one_canonical_bounded_record() {
        let (ws, r#loop) = harness();
        let long_reason = "x".repeat(MAX_REASON_CHARS + 50);
        let handle = admitted(r#loop.begin_at(heartbeat(), &long_reason, Target::Companion, T0));
        let id = handle.id().to_owned();
        assert!(id.starts_with("run_"));

        let stored = r#loop.get(&id).unwrap();
        assert_eq!(stored.version, ACTIVITY_FORMAT_VERSION);
        assert_eq!(stored.status, RunStatus::Running);
        assert_eq!(stored.trigger, heartbeat());
        assert_eq!(stored.target, Target::Companion);
        assert_eq!(stored.reason.chars().count(), MAX_REASON_CHARS);
        assert_eq!(stored.attempt, 1);

        let outcome = RunOutcome {
            actions: vec![ActionReceipt {
                tool: "memory_write".into(),
                summary: "writing notes/tea.md".into(),
            }],
            messages_sent: 1,
            tokens: 321,
        };
        let finished = handle.complete_at(outcome.clone(), T0 + 5);
        assert_eq!(finished.status, RunStatus::Completed);
        assert_eq!(finished.finished_at, Some(T0 + 5));
        assert_eq!(finished.outcome, Some(outcome));

        let raw = fs::read_to_string(
            ws.path()
                .join("instances")
                .join(CANONICAL_SLUG)
                .join("activity")
                .join(format!("{id}.json")),
        )
        .unwrap();
        for forbidden in ["raw", "thought", "monologue"] {
            assert!(
                !raw.contains(forbidden),
                "record leaks model text: {forbidden}"
            );
        }
        assert_eq!(r#loop.list(10).len(), 1);
    }

    #[test]
    fn repeated_triggers_cannot_start_duplicate_work() {
        let (_ws, r#loop) = harness();
        let first = admitted(r#loop.begin_at(heartbeat(), "check-in", Target::Companion, T0));
        let dup = skipped(r#loop.begin_at(heartbeat(), "check-in", Target::Companion, T0 + 1));
        assert_eq!(
            dup.status,
            RunStatus::Skipped {
                reason: SkipReason::Duplicate {
                    of: first.id().to_owned()
                }
            }
        );
        // A different trigger is independent work.
        let machine = Trigger::MachineConnected {
            machine_id: "mac".into(),
        };
        admitted(r#loop.begin_at(
            machine,
            "desktop connected",
            Target::Machine {
                machine_id: "mac".into(),
            },
            T0 + 1,
        ))
        .complete_at(RunOutcome::default(), T0 + 2);
        first.complete_at(RunOutcome::default(), T0 + 3);
        assert_eq!(r#loop.list(10).len(), 3, "the skip is recorded, not hidden");
    }

    #[test]
    fn reconnect_bursts_share_a_cooldown_but_explicit_schedules_do_not() {
        let (_ws, r#loop) = harness();
        let machine = || Trigger::MachineConnected {
            machine_id: "mac".into(),
        };
        let target = || Target::Machine {
            machine_id: "mac".into(),
        };
        admitted(r#loop.begin_at(machine(), "connected", target(), T0))
            .complete_at(RunOutcome::default(), T0 + 1);
        let again = skipped(r#loop.begin_at(machine(), "connected", target(), T0 + 30));
        assert_eq!(
            again.status,
            RunStatus::Skipped {
                reason: SkipReason::Cooldown {
                    until: T0 + 1 + 600
                }
            }
        );
        admitted(r#loop.begin_at(machine(), "connected", target(), T0 + 700))
            .complete_at(RunOutcome::default(), T0 + 701);

        let schedule = |id: &str| Trigger::Schedule { task_id: id.into() };
        let chat = || Target::Chat {
            chat_id: "default".into(),
        };
        admitted(r#loop.begin_at(schedule("a"), "water plants", chat(), T0))
            .complete_at(RunOutcome::default(), T0 + 1);
        admitted(r#loop.begin_at(schedule("a"), "water plants", chat(), T0 + 2))
            .complete_at(RunOutcome::default(), T0 + 3);
    }

    #[test]
    fn quiet_hours_hold_spontaneous_runs_and_deny_reach_out_in_companion_time() {
        let (ws, r#loop) = harness();
        fs::write(
            ws.path()
                .join("instances")
                .join(CANONICAL_SLUG)
                .join("project_state.json"),
            r#"{"timezone":"Asia/Tokyo"}"#,
        )
        .unwrap();
        r#loop
            .set_policy(&ProactivePolicy {
                quiet_hours: Some(QuietHours {
                    start_hour: 22,
                    end_hour: 7,
                }),
                ..ProactivePolicy::default()
            })
            .unwrap();
        // 09:00 UTC is 18:00 in Tokyo: not quiet.
        admitted(r#loop.begin_at(heartbeat(), "check-in", Target::Companion, T0))
            .complete_at(RunOutcome::default(), T0 + 1);
        // 14:00 UTC is 23:00 in Tokyo: quiet.
        let night = T0 + 5 * 3600;
        let held = skipped(r#loop.begin_at(heartbeat(), "check-in", Target::Companion, night));
        assert_eq!(
            held.status,
            RunStatus::Skipped {
                reason: SkipReason::QuietHours
            }
        );

        // An explicit schedule still runs, but may not message the user.
        let scheduled = admitted(r#loop.begin_at(
            Trigger::Schedule {
                task_id: "t".into(),
            },
            "reminder",
            Target::Chat {
                chat_id: "default".into(),
            },
            night,
        ));
        assert_eq!(
            r#loop.approve_side_effect(Some(scheduled.id()), SideEffect::ReachOut, night),
            Err(Denied::QuietHours)
        );
        let run = scheduled.complete_at(RunOutcome::default(), night + 1);
        assert_eq!(run.approvals.len(), 1);
        assert!(!run.approvals[0].allowed);
        assert_eq!(
            run.approvals[0].reason.as_deref(),
            Some("quiet hours are active")
        );
    }

    #[test]
    fn attention_budget_caps_spontaneous_messages_per_day_and_records_each_decision() {
        let (_ws, r#loop) = harness();
        r#loop
            .set_policy(&ProactivePolicy {
                daily_reach_out_budget: 2,
                ..ProactivePolicy::default()
            })
            .unwrap();
        let handle = admitted(r#loop.begin_at(heartbeat(), "check-in", Target::Companion, T0));
        let id = handle.id().to_owned();
        assert_eq!(
            r#loop.approve_side_effect(Some(&id), SideEffect::ReachOut, T0),
            Ok(())
        );
        assert_eq!(
            r#loop.approve_side_effect(Some(&id), SideEffect::ReachOut, T0 + 60),
            Ok(())
        );
        assert_eq!(
            r#loop.approve_side_effect(Some(&id), SideEffect::ReachOut, T0 + 120),
            Err(Denied::AttentionBudget)
        );
        // The budget rolls over after 24 hours.
        assert_eq!(
            r#loop.approve_side_effect(None, SideEffect::ReachOut, T0 + 86_400 + 1),
            Ok(())
        );
        let run = handle.complete_at(RunOutcome::default(), T0 + 200);
        let allowed: Vec<bool> = run.approvals.iter().map(|a| a.allowed).collect();
        assert_eq!(allowed, vec![true, true, false]);
        assert_eq!(
            r#loop.approve_side_effect(Some("run_missing"), SideEffect::ReachOut, T0),
            Err(Denied::UnknownRun)
        );
    }

    #[test]
    fn cancellation_is_visible_to_the_worker_and_recorded_once() {
        let (_ws, r#loop) = harness();
        let handle = admitted(r#loop.begin_at(heartbeat(), "check-in", Target::Companion, T0));
        let token = handle.token();
        assert!(!token.is_cancelled());
        assert!(r#loop.cancel(handle.id()));
        assert!(token.is_cancelled());
        assert!(handle.is_cancelled());
        let run = handle.cancel_at(T0 + 3);
        assert_eq!(run.status, RunStatus::Cancelled);
        assert!(
            !r#loop.cancel(&run.id),
            "finished runs cannot be cancelled again"
        );
        // The key is free again.
        admitted(r#loop.begin_at(heartbeat(), "check-in", Target::Companion, T0 + 4000));
    }

    #[test]
    fn failures_stay_visible_and_retry_links_attempts_without_duplicating_history() {
        let (_ws, r#loop) = harness();
        let handle = admitted(r#loop.begin_at(heartbeat(), "check-in", Target::Companion, T0));
        let failed = handle.fail_at("provider offline", true, T0 + 1);
        assert_eq!(
            failed.status,
            RunStatus::Failed {
                error: "provider offline".into(),
                retryable: true
            }
        );
        let retry = admitted(r#loop.retry(&failed.id, T0 + 5000).unwrap());
        let record = r#loop.get(retry.id()).unwrap();
        assert_eq!(record.attempt, 2);
        assert_eq!(record.retry_of.as_deref(), Some(failed.id.as_str()));
        assert_eq!(record.trigger, failed.trigger);
        retry.complete_at(RunOutcome::default(), T0 + 5001);
        assert_eq!(r#loop.list(10).len(), 2, "one record per attempt");

        let done = admitted(r#loop.begin_at(
            Trigger::Manual {
                agent: "companion".into(),
            },
            "run now",
            Target::Companion,
            T0 + 6000,
        ))
        .complete_at(RunOutcome::default(), T0 + 6001);
        assert!(
            r#loop.retry(&done.id, T0 + 7000).is_err(),
            "completed runs are not retried"
        );
        let fatal = admitted(r#loop.begin_at(
            Trigger::Manual {
                agent: "companion".into(),
            },
            "run now",
            Target::Companion,
            T0 + 8000,
        ))
        .fail_at("bad config", false, T0 + 8001);
        assert!(
            r#loop.retry(&fatal.id, T0 + 9000).is_err(),
            "non-retryable failures stay failed"
        );
    }

    #[test]
    fn restart_marks_interrupted_runs_failed_and_retryable() {
        let (ws, r#loop) = harness();
        let handle = admitted(r#loop.begin_at(heartbeat(), "check-in", Target::Companion, T0));
        let id = handle.id().to_owned();
        std::mem::forget(handle); // process died mid-run

        let fresh = ProactiveLoop::new(ws.path(), CANONICAL_SLUG);
        assert_eq!(fresh.recover_on_restart(T0 + 60), 1);
        let run = fresh.get(&id).unwrap();
        assert_eq!(
            run.status,
            RunStatus::Failed {
                error: RESTART_ERROR.into(),
                retryable: true
            }
        );
        assert_eq!(fresh.recover_on_restart(T0 + 61), 0, "idempotent");
        admitted(fresh.retry(&id, T0 + 100).unwrap());
    }

    #[test]
    fn retention_drops_old_or_excess_finished_runs_but_never_running_ones() {
        let (_ws, r#loop) = harness();
        r#loop
            .set_policy(&ProactivePolicy {
                retention_max: 3,
                retention_days: 1,
                ..ProactivePolicy::default()
            })
            .unwrap();
        for i in 0..5 {
            let t = T0 + i * 10;
            admitted(r#loop.begin_at(
                Trigger::Schedule {
                    task_id: format!("t{i}"),
                },
                "task",
                Target::Chat {
                    chat_id: "default".into(),
                },
                t,
            ))
            .complete_at(RunOutcome::default(), t + 1);
        }
        let running =
            admitted(r#loop.begin_at(heartbeat(), "check-in", Target::Companion, T0 - 10 * 86_400));

        let removed = r#loop.enforce_retention(T0 + 100);
        assert_eq!(removed, 2, "excess finished runs beyond retention_max");
        let remaining = r#loop.list(10);
        assert_eq!(remaining.len(), 4);
        assert!(
            remaining.iter().any(|run| run.id == running.id()),
            "running runs survive"
        );

        let removed = r#loop.enforce_retention(T0 + 3 * 86_400);
        assert_eq!(removed, 3, "finished runs older than retention_days");
        assert_eq!(r#loop.list(10).len(), 1);
        running.complete_at(RunOutcome::default(), T0 + 3 * 86_400);
    }

    #[test]
    fn every_record_change_is_broadcast_as_an_activity_receipt() {
        let (ws, _) = harness();
        let (events, mut rx) = tokio::sync::broadcast::channel(16);
        let r#loop = ProactiveLoop::new(ws.path(), CANONICAL_SLUG).with_events(events);
        let handle = admitted(r#loop.begin_at(heartbeat(), "check-in", Target::Companion, T0));
        r#loop
            .approve_side_effect(Some(handle.id()), SideEffect::ReachOut, T0 + 1)
            .unwrap();
        handle.complete_at(RunOutcome::default(), T0 + 2);
        skipped(r#loop.begin_at(heartbeat(), "check-in", Target::Companion, T0 + 3));

        let mut statuses = Vec::new();
        while let Ok(event) = rx.try_recv() {
            let crate::domain::events::ServerEvent::ActivityUpdated { instance_slug, run } = event
            else {
                panic!("unexpected event");
            };
            assert_eq!(instance_slug, CANONICAL_SLUG);
            statuses.push(run.status);
        }
        assert!(
            matches!(
                statuses.as_slice(),
                [
                    RunStatus::Running,
                    RunStatus::Running,
                    RunStatus::Completed,
                    RunStatus::Skipped { .. }
                ]
            ),
            "{statuses:?}"
        );
        let json = serde_json::to_string(&crate::domain::events::ServerEvent::ActivityUpdated {
            instance_slug: CANONICAL_SLUG.into(),
            run: r#loop.list(1).remove(0),
        })
        .unwrap();
        assert!(json.contains("\"type\":\"activity_updated\""));
        assert!(!json.contains("trace"));
    }

    #[test]
    fn disabled_policy_skips_everything_and_receipts_carry_no_model_text() {
        let (_ws, r#loop) = harness();
        r#loop
            .set_policy(&ProactivePolicy {
                enabled: false,
                ..ProactivePolicy::default()
            })
            .unwrap();
        let run = skipped(r#loop.begin_at(heartbeat(), "check-in", Target::Companion, T0));
        assert_eq!(
            run.status,
            RunStatus::Skipped {
                reason: SkipReason::Disabled
            }
        );

        use crate::services::llm::{ContentBlock, Message};
        let trace = vec![
            Message::assistant("secret inner monologue"),
            Message::Assistant {
                content: vec![
                    ContentBlock::ToolCall {
                        id: "1".into(),
                        name: "reach_out".into(),
                        arguments: serde_json::json!({"message": "hi there"}),
                    },
                    ContentBlock::ToolCall {
                        id: "2".into(),
                        name: "memory_write".into(),
                        arguments: serde_json::json!({"path": "notes/tea.md", "content": "..."}),
                    },
                ],
            },
        ];
        let outcome = outcome_from_trace(&trace, 42);
        assert_eq!(outcome.messages_sent, 1);
        assert_eq!(outcome.tokens, 42);
        assert_eq!(outcome.actions.len(), 2);
        assert_eq!(outcome.actions[1].tool, "memory_write");
        let json = serde_json::to_string(&outcome).unwrap();
        assert!(!json.contains("secret inner monologue"));
    }
}
