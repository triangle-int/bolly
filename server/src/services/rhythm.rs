//! Rhythm — the one bounded interaction-timing aggregate (#95).
//!
//! Folds each user message into fixed-size histograms and running averages so
//! the companion can time proactive behavior. Consumed only by the heartbeat
//! prompt insights; documented in docs/behavioral-metrics.md.

use std::fs;
use std::path::Path;

use chrono::{Datelike, TimeZone, Timelike, Utc};
use chrono_tz;

use crate::domain::chat::ChatRole;
use crate::domain::rhythm::InteractionRhythm;

/// Session gap threshold: 2 hours of silence = new session.
const SESSION_GAP_SECS: i64 = 2 * 3600;

/// Build a human-readable rhythm insight string for injection into prompts.
/// Compares current session behavior to historical baseline.
pub fn build_rhythm_insights(
    workspace_dir: &Path,
    slug: &str,
    rhythm: &InteractionRhythm,
) -> String {
    if rhythm.total_messages < 10 {
        return String::new(); // Not enough data yet
    }

    let mut insights = Vec::new();

    // Find peak hours
    let peak_hours = find_peak_hours(&rhythm.hourly_activity);
    if !peak_hours.is_empty() {
        let hours_str: Vec<String> = peak_hours.iter().map(|h| format!("{h}:00")).collect();
        insights.push(format!(
            "they're usually most active around {}",
            hours_str.join(", ")
        ));
    }

    // Current hour activity comparison
    let now = Utc::now();
    let current_hour = now.hour() as usize;
    let hour_avg = rhythm.hourly_activity[current_hour] as f64
        / (rhythm.total_messages as f64 / 24.0).max(1.0);
    if hour_avg < 0.3 && rhythm.hourly_activity[current_hour] > 0 {
        insights.push(format!(
            "it's unusual for them to be active at this hour ({}:00)",
            current_hour
        ));
    }

    // Average response pace
    if rhythm.avg_response_interval_secs > 0.0 {
        let pace = rhythm.avg_response_interval_secs;
        let pace_str = if pace < 30.0 {
            "very fast (under 30s)"
        } else if pace < 120.0 {
            "quick (1-2 min)"
        } else if pace < 300.0 {
            "moderate (a few minutes)"
        } else {
            "relaxed (5+ min between messages)"
        };
        insights.push(format!("their usual response pace: {pace_str}"));
    }

    // Compare current session to baseline
    let current_session = analyze_current_session(workspace_dir, slug);
    if let Some(session) = current_session {
        // Response pace comparison
        if rhythm.avg_response_interval_secs > 0.0 && session.avg_interval_secs > 0.0 {
            let ratio = session.avg_interval_secs / rhythm.avg_response_interval_secs;
            if ratio > 2.0 {
                insights.push("they're responding noticeably slower than usual right now".into());
            } else if ratio < 0.4 {
                insights.push("they're responding faster than usual — seems engaged".into());
            }
        }

        // Message length comparison
        if rhythm.avg_message_length > 0.0 && session.avg_length > 0.0 {
            let ratio = session.avg_length / rhythm.avg_message_length;
            if ratio > 2.0 {
                insights.push("their messages are longer than usual — being more detailed".into());
            } else if ratio < 0.4 && session.msg_count > 2 {
                insights.push(
                    "their messages are shorter than usual — could be distracted or terse".into(),
                );
            }
        }
    }

    if insights.is_empty() {
        return String::new();
    }

    format!(
        "## interaction rhythm\n\
         you've noticed these patterns about how they interact:\n\
         {}\n\
         use these observations naturally — don't list them. \
         only mention a pattern if it genuinely feels relevant to the moment.",
        insights
            .iter()
            .map(|i| format!("- {i}"))
            .collect::<Vec<_>>()
            .join("\n")
    )
}

/// Metrics for the current ongoing session.
struct CurrentSession {
    msg_count: usize,
    avg_length: f64,
    avg_interval_secs: f64,
}

/// Analyze the current session from the default chat.
fn analyze_current_session(workspace_dir: &Path, slug: &str) -> Option<CurrentSession> {
    let rig_path = workspace_dir
        .join("instances")
        .join(slug)
        .join("chats")
        .join("default")
        .join("rig_history.json");

    let entries = super::chat::load_rig_history(&rig_path)?;
    let messages = crate::services::llm::history_to_chat_messages(&entries);

    // Find user messages in the current session (walk backwards from end,
    // stop when gap > SESSION_GAP_SECS)
    let now_secs = Utc::now().timestamp();
    let mut session_msgs: Vec<(i64, usize)> = Vec::new();

    for msg in messages.iter().rev() {
        if let Ok(ts_millis) = msg.created_at.parse::<i64>() {
            let ts = ts_millis / 1000;
            let reference = session_msgs.last().map(|(t, _)| *t).unwrap_or(now_secs);
            if (reference - ts).abs() > SESSION_GAP_SECS {
                break;
            }
            if matches!(msg.role, ChatRole::User) {
                session_msgs.push((ts, msg.content.len()));
            }
        }
    }

    if session_msgs.len() < 2 {
        return None;
    }

    // Reverse back to chronological order
    session_msgs.reverse();

    let total_len: usize = session_msgs.iter().map(|(_, l)| l).sum();
    let avg_length = total_len as f64 / session_msgs.len() as f64;

    let mut intervals: Vec<i64> = Vec::new();
    for window in session_msgs.windows(2) {
        let gap = window[1].0 - window[0].0;
        if gap > 0 {
            intervals.push(gap);
        }
    }
    let avg_interval = if intervals.is_empty() {
        0.0
    } else {
        intervals.iter().sum::<i64>() as f64 / intervals.len() as f64
    };

    Some(CurrentSession {
        msg_count: session_msgs.len(),
        avg_length,
        avg_interval_secs: avg_interval,
    })
}

/// Find the top 2-3 peak activity hours.
fn find_peak_hours(hourly: &[u32; 24]) -> Vec<usize> {
    let max = *hourly.iter().max().unwrap_or(&0);
    if max == 0 {
        return vec![];
    }
    let threshold = (max as f64 * 0.7) as u32;
    let mut peaks: Vec<(usize, u32)> = hourly
        .iter()
        .copied()
        .enumerate()
        .filter(|(_, count)| *count >= threshold)
        .map(|(hour, count)| (hour, count))
        .collect();
    peaks.sort_by(|a, b| b.1.cmp(&a.1));
    peaks.truncate(3);
    peaks.into_iter().map(|(h, _)| h).collect()
}

// ---------------------------------------------------------------------------
// Incremental recording (#95)
//
// `rhythm.json` is the single behavioral aggregate: fixed-size histograms and
// running totals, folded in one message at a time. No per-message history is
// kept beyond `last_message_at`, and nothing rescans chat history.
// ---------------------------------------------------------------------------

const RHYTHM_FILE: &str = "rhythm.json";
const RHYTHM_VERSION: u32 = 2;

fn rhythm_path(instance_dir: &Path) -> std::path::PathBuf {
    instance_dir.join(RHYTHM_FILE)
}

/// Load the persisted rhythm aggregate, or the empty default.
pub fn load_rhythm(instance_dir: &Path) -> InteractionRhythm {
    fs::read_to_string(rhythm_path(instance_dir))
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn save_rhythm(instance_dir: &Path, rhythm: &InteractionRhythm) {
    if let Ok(json) = serde_json::to_string_pretty(rhythm) {
        let _ = fs::write(rhythm_path(instance_dir), json);
    }
}

/// Whether the companion may keep interaction-timing aggregates.
pub fn tracking_enabled(workspace_dir: &Path, slug: &str) -> bool {
    crate::config::InstanceConfig::load(workspace_dir, slug).rhythm_tracking
}

/// Fold one user message into the bounded aggregate. No-op when tracking is off.
pub fn record_user_message(workspace_dir: &Path, slug: &str, content_len: usize) {
    record_user_message_at(workspace_dir, slug, content_len, Utc::now().timestamp());
}

pub fn record_user_message_at(workspace_dir: &Path, slug: &str, content_len: usize, now: i64) {
    if !tracking_enabled(workspace_dir, slug) {
        return;
    }
    let instance_dir = workspace_dir.join("instances").join(slug);
    let tz: chrono_tz::Tz = crate::routes::instances::read_timezone(&instance_dir)
        .and_then(|value| value.parse().ok())
        .unwrap_or(chrono_tz::UTC);
    let Some(local) = Utc.timestamp_opt(now, 0).single() else {
        return;
    };
    let local = local.with_timezone(&tz);

    let mut rhythm = load_rhythm(&instance_dir);
    rhythm.hourly_activity[local.hour() as usize] += 1;
    rhythm.daily_activity[local.weekday().num_days_from_monday() as usize] += 1;
    rhythm.total_messages += 1;
    rhythm.total_chars += content_len as u64;
    rhythm.avg_message_length = rhythm.total_chars as f64 / rhythm.total_messages as f64;

    if rhythm.last_message_at > 0 {
        let gap = now - rhythm.last_message_at;
        if gap > 0 && gap < SESSION_GAP_SECS {
            rhythm.interval_count += 1;
            rhythm.interval_total_secs += gap as u64;
            rhythm.avg_response_interval_secs =
                rhythm.interval_total_secs as f64 / rhythm.interval_count as f64;
        }
    }
    rhythm.last_message_at = now;
    rhythm.updated_at = Utc::now().timestamp();
    rhythm.version = RHYTHM_VERSION;
    save_rhythm(&instance_dir, &rhythm);
}

/// Delete the aggregate. Used when the user opts out.
pub fn clear_rhythm(instance_dir: &Path) -> std::io::Result<()> {
    match fs::remove_file(rhythm_path(instance_dir)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod incremental_tests {
    use super::*;
    use crate::domain::companion::CANONICAL_SLUG;

    fn workspace() -> (tempfile::TempDir, std::path::PathBuf) {
        let ws = tempfile::tempdir().unwrap();
        let dir = ws.path().join("instances").join(CANONICAL_SLUG);
        fs::create_dir_all(&dir).unwrap();
        (ws, dir)
    }

    /// 2026-01-05 (a Monday) 09:00:00 UTC
    const MONDAY_9_UTC: i64 = 1_767_603_600;

    #[test]
    fn messages_fold_into_bounded_aggregates_without_per_message_history() {
        let (ws, dir) = workspace();
        assert!(tracking_enabled(ws.path(), CANONICAL_SLUG), "on by default");

        record_user_message_at(ws.path(), CANONICAL_SLUG, 10, MONDAY_9_UTC);
        record_user_message_at(ws.path(), CANONICAL_SLUG, 30, MONDAY_9_UTC + 60);
        // Three hours later: a new session, so no interval is counted.
        record_user_message_at(ws.path(), CANONICAL_SLUG, 20, MONDAY_9_UTC + 3 * 3600 + 120);

        let rhythm = load_rhythm(&dir);
        assert_eq!(rhythm.total_messages, 3);
        assert_eq!(rhythm.total_chars, 60);
        assert!((rhythm.avg_message_length - 20.0).abs() < f64::EPSILON);
        assert_eq!(rhythm.hourly_activity[9], 2);
        assert_eq!(rhythm.hourly_activity[12], 1);
        assert_eq!(rhythm.daily_activity[0], 3, "Monday");
        assert_eq!(rhythm.interval_count, 1);
        assert!((rhythm.avg_response_interval_secs - 60.0).abs() < f64::EPSILON);
        assert_eq!(rhythm.last_message_at, MONDAY_9_UTC + 3 * 3600 + 120);

        let raw = fs::read_to_string(dir.join("rhythm.json")).unwrap();
        let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert!(
            value.get("daily_history").is_none(),
            "per-day history is an engagement aggregate and must not persist"
        );
        assert_eq!(value["version"], 2);
    }

    #[test]
    fn hour_and_weekday_follow_the_companion_timezone() {
        let (ws, dir) = workspace();
        fs::write(
            dir.join("project_state.json"),
            r#"{"timezone":"Asia/Tokyo"}"#,
        )
        .unwrap();
        // 09:00 UTC Monday is 18:00 JST Monday.
        record_user_message_at(ws.path(), CANONICAL_SLUG, 5, MONDAY_9_UTC);
        let rhythm = load_rhythm(&dir);
        assert_eq!(rhythm.hourly_activity[18], 1);
        assert_eq!(rhythm.daily_activity[0], 1);
    }

    #[test]
    fn opting_out_stops_recording_and_clears_the_aggregate() {
        let (ws, dir) = workspace();
        record_user_message_at(ws.path(), CANONICAL_SLUG, 5, MONDAY_9_UTC);
        assert!(dir.join("rhythm.json").is_file());

        let mut config = crate::config::InstanceConfig::load(ws.path(), CANONICAL_SLUG);
        config.rhythm_tracking = false;
        config.save(ws.path(), CANONICAL_SLUG).unwrap();
        clear_rhythm(&dir).unwrap();
        assert!(!tracking_enabled(ws.path(), CANONICAL_SLUG));
        assert!(!dir.join("rhythm.json").exists());

        record_user_message_at(ws.path(), CANONICAL_SLUG, 5, MONDAY_9_UTC + 10);
        assert!(
            !dir.join("rhythm.json").exists(),
            "recording must not resume while opted out"
        );
        assert_eq!(load_rhythm(&dir), InteractionRhythm::default());
        clear_rhythm(&dir).unwrap();
    }

    #[test]
    fn insights_are_built_from_the_incremental_aggregate() {
        let (ws, dir) = workspace();
        for i in 0..12 {
            record_user_message_at(ws.path(), CANONICAL_SLUG, 40, MONDAY_9_UTC + i * 20);
        }
        let rhythm = load_rhythm(&dir);
        let insights = build_rhythm_insights(ws.path(), CANONICAL_SLUG, &rhythm);
        assert!(insights.contains("most active around 9:00"), "{insights}");
        assert!(insights.contains("very fast"), "{insights}");
    }
}
