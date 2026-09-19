use serde::{Deserialize, Serialize};

/// Aggregated interaction rhythm patterns for a user.
/// Computed from message history and persisted to rhythm.json.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct InteractionRhythm {
    /// Message count per hour of day (0-23), user messages only.
    #[serde(default)]
    pub hourly_activity: [u32; 24],

    /// Message count per day of week (0=Mon, 6=Sun), user messages only.
    #[serde(default)]
    pub daily_activity: [u32; 7],

    /// Average user message length in characters.
    #[serde(default)]
    pub avg_message_length: f64,

    /// Average seconds between consecutive user messages within a session.
    /// A session gap is defined as >2 hours of silence.
    #[serde(default)]
    pub avg_response_interval_secs: f64,

    /// Total user messages analyzed.
    #[serde(default)]
    pub total_messages: u32,

    /// Unix timestamp of last rhythm update.
    #[serde(default)]
    pub updated_at: i64,

    /// Accumulated total chars for avg_message_length.
    #[serde(default)]
    pub total_chars: u64,

    /// Unix timestamp of the most recent user message; the only per-event
    /// datum kept, needed to measure the next within-session interval.
    #[serde(default)]
    pub last_message_at: i64,

    /// Number of within-session intervals folded into the average.
    #[serde(default)]
    pub interval_count: u32,

    /// Sum of within-session intervals in seconds.
    #[serde(default)]
    pub interval_total_secs: u64,

    /// Aggregate format version. 2 = incremental, no per-day history.
    #[serde(default)]
    pub version: u32,
}
