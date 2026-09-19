//! One proactive execution record for everything the companion starts on its
//! own (#92): heartbeat check-ins, explicit schedules, connected-computer
//! events, and future commitment/handoff triggers. Bounded by construction:
//! no model text, no chain of thought, only trigger, reason, target,
//! approvals, and a receipt of what happened.

use serde::{Deserialize, Serialize};

pub const ACTIVITY_FORMAT_VERSION: u32 = 1;

/// Longest stated reason kept on a record.
pub const MAX_REASON_CHARS: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Trigger {
    /// Periodic companion wake-up (`agent` names the routine; "companion" by default).
    Heartbeat { agent: String },
    /// An explicit schedule created by the user or the companion.
    Schedule { task_id: String },
    /// A connected computer came online.
    MachineConnected { machine_id: String },
    /// A user pressed "run now" in a developer surface.
    Manual { agent: String },
    /// A tracked commitment came due (#85).
    Commitment { commitment_id: String },
    /// A reviewable handoff was continued on another computer (#82).
    Handoff { handoff_id: String },
}

impl Trigger {
    /// Runs sharing a key never execute concurrently and share cooldown state.
    pub fn dedupe_key(&self) -> String {
        match self {
            Self::Heartbeat { agent } => format!("heartbeat:{agent}"),
            Self::Schedule { task_id } => format!("schedule:{task_id}"),
            Self::MachineConnected { machine_id } => format!("machine:{machine_id}"),
            Self::Manual { agent } => format!("manual:{agent}"),
            Self::Commitment { commitment_id } => format!("commitment:{commitment_id}"),
            Self::Handoff { handoff_id } => format!("handoff:{handoff_id}"),
        }
    }

    /// Spontaneous triggers wait for quiet hours to end; explicit ones do not.
    pub fn respects_quiet_hours(&self) -> bool {
        matches!(self, Self::Heartbeat { .. } | Self::MachineConnected { .. })
    }

    /// Event-driven triggers can fire in bursts and share a cooldown.
    pub fn has_cooldown(&self) -> bool {
        matches!(self, Self::Heartbeat { .. } | Self::MachineConnected { .. })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Target {
    Companion,
    Chat { chat_id: String },
    Machine { machine_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SkipReason {
    QuietHours,
    Cooldown { until: i64 },
    Duplicate { of: String },
    Disabled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RunStatus {
    Running,
    Completed,
    Failed { error: String, retryable: bool },
    Cancelled,
    Skipped { reason: SkipReason },
}

impl RunStatus {
    pub fn is_finished(&self) -> bool {
        !matches!(self, Self::Running)
    }
}

/// A side effect that leaves the companion's own storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SideEffect {
    /// A spontaneous message to the user.
    ReachOut,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Approval {
    pub side_effect: SideEffect,
    pub allowed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub at: i64,
}

/// What a run did, without any model text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionReceipt {
    pub tool: String,
    pub summary: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunOutcome {
    #[serde(default)]
    pub actions: Vec<ActionReceipt>,
    #[serde(default)]
    pub messages_sent: u32,
    #[serde(default)]
    pub tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProactiveRun {
    pub version: u32,
    pub id: String,
    pub trigger: Trigger,
    pub reason: String,
    pub target: Target,
    pub dedupe_key: String,
    pub status: RunStatus,
    pub attempt: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_of: Option<String>,
    pub started_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<i64>,
    #[serde(default)]
    pub approvals: Vec<Approval>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<RunOutcome>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuietHours {
    /// Local hour (0-23) when quiet hours begin.
    pub start_hour: u8,
    /// Local hour (0-23) when they end; may be earlier than `start_hour` to wrap midnight.
    pub end_hour: u8,
}

impl QuietHours {
    pub fn contains(&self, local_hour: u32) -> bool {
        let (start, end) = (u32::from(self.start_hour), u32::from(self.end_hour));
        if start == end {
            false
        } else if start < end {
            (start..end).contains(&local_hour)
        } else {
            local_hour >= start || local_hour < end
        }
    }
}

/// User-controlled limits applied at the one proactive boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProactivePolicy {
    /// Master switch for spontaneous behavior.
    pub enabled: bool,
    pub quiet_hours: Option<QuietHours>,
    /// Minimum seconds between two runs of the same event-driven trigger.
    pub cooldown_secs: u64,
    /// Spontaneous messages allowed per rolling 24 hours.
    pub daily_reach_out_budget: u32,
    /// Finished records kept, newest first.
    pub retention_max: usize,
    /// Finished records older than this are removed.
    pub retention_days: u32,
}

impl Default for ProactivePolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            quiet_hours: None,
            cooldown_secs: 600,
            daily_reach_out_budget: 6,
            retention_max: 200,
            retention_days: 30,
        }
    }
}
