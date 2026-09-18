//! Rhythm — tracks user interaction patterns over time.
//!
//! Analyzes message timestamps to detect when the user is typically active,
//! how fast they respond, and how their current session compares to baseline.

use std::fs;
use std::path::Path;

use chrono::{DateTime, Datelike, TimeZone, Timelike, Utc};
use chrono_tz;

use crate::domain::chat::ChatRole;
use crate::domain::rhythm::InteractionRhythm;

/// Session gap threshold: 2 hours of silence = new session.
const SESSION_GAP_SECS: i64 = 2 * 3600;

/// Save rhythm state to disk.
pub fn save_rhythm(instance_dir: &Path, rhythm: &InteractionRhythm) {
    let path = instance_dir.join("rhythm.json");
    if let Ok(json) = serde_json::to_string_pretty(rhythm) {
        let _ = fs::write(path, json);
    }
}

/// Recompute rhythm from all message history across all chats.
pub fn recompute_rhythm(workspace_dir: &Path, slug: &str) -> InteractionRhythm {
    let chats_dir = workspace_dir.join("instances").join(slug).join("chats");

    let mut all_user_msgs: Vec<(i64, usize)> = Vec::new(); // (unix_secs, content_len)

    if let Ok(entries) = fs::read_dir(&chats_dir) {
        for entry in entries.filter_map(Result::ok) {
            if !entry.path().is_dir() {
                continue;
            }
            let rig_path = entry.path().join("rig_history.json");
            let history_entries = super::chat::load_rig_history(&rig_path).unwrap_or_default();
            for he in &history_entries {
                if let crate::services::llm::Message::User { content } = &he.message {
                    // Use entry timestamp if available
                    if let Some(ref ts_str) = he.ts {
                        if let Ok(ts_millis) = ts_str.parse::<i64>() {
                            let content_len: usize = content
                                .iter()
                                .map(|b| {
                                    if let crate::services::llm::ContentBlock::Text { text } = b {
                                        text.len()
                                    } else {
                                        0
                                    }
                                })
                                .sum();
                            all_user_msgs.push((ts_millis / 1000, content_len));
                        }
                    }
                }
            }
        }
    }

    if all_user_msgs.is_empty() {
        return InteractionRhythm::default();
    }

    // Sort by timestamp
    all_user_msgs.sort_by_key(|(ts, _)| *ts);

    let mut rhythm = InteractionRhythm {
        total_messages: all_user_msgs.len() as u32,
        updated_at: Utc::now().timestamp(),
        ..Default::default()
    };

    // Compute hourly/daily histograms and average message length
    let mut total_len: u64 = 0;
    for &(ts, len) in &all_user_msgs {
        total_len += len as u64;
        if let Some(dt) = timestamp_to_utc(ts) {
            let hour = dt.hour() as usize;
            rhythm.hourly_activity[hour] += 1;
            // Chrono: Mon=0..Sun=6
            let weekday = dt.weekday().num_days_from_monday() as usize;
            rhythm.daily_activity[weekday] += 1;
        }
    }
    rhythm.avg_message_length = total_len as f64 / all_user_msgs.len() as f64;

    // Compute average response interval (within sessions)
    let mut intervals: Vec<i64> = Vec::new();
    for window in all_user_msgs.windows(2) {
        let gap = window[1].0 - window[0].0;
        if gap > 0 && gap < SESSION_GAP_SECS {
            intervals.push(gap);
        }
    }
    if !intervals.is_empty() {
        rhythm.avg_response_interval_secs =
            intervals.iter().sum::<i64>() as f64 / intervals.len() as f64;
    }

    rhythm
}

/// Snapshot message data into rhythm.json before clearing messages.
/// Merges current messages into the accumulated stats so nothing is lost.
#[allow(dead_code)]
pub fn snapshot_before_clear(workspace_dir: &Path, slug: &str) {
    let instance_dir = workspace_dir.join("instances").join(slug);
    let tz: chrono_tz::Tz = crate::routes::instances::read_timezone(&instance_dir)
        .and_then(|s| s.parse().ok())
        .unwrap_or(chrono_tz::UTC);

    // Load existing rhythm (accumulated data)
    let mut rhythm: InteractionRhythm = fs::read_to_string(instance_dir.join("rhythm.json"))
        .ok()
        .and_then(|r| serde_json::from_str(&r).ok())
        .unwrap_or_default();

    // Scan current messages
    let chats_dir = workspace_dir.join("instances").join(slug).join("chats");
    let mut msg_count = 0u32;
    let mut total_chars = 0u64;
    let mut timestamps: Vec<i64> = Vec::new();

    if let Ok(entries) = fs::read_dir(&chats_dir) {
        for entry in entries.filter_map(Result::ok) {
            let rig_path = entry.path().join("rig_history.json");
            let history_entries = super::chat::load_rig_history(&rig_path).unwrap_or_default();
            for he in &history_entries {
                if let crate::services::llm::Message::User { content } = &he.message {
                    if let Some(ref ts_str) = he.ts {
                        if let Ok(ts_ms) = ts_str.parse::<i64>() {
                            let ts = ts_ms / 1000;
                            let content_len: usize = content
                                .iter()
                                .map(|b| {
                                    if let crate::services::llm::ContentBlock::Text { text } = b {
                                        text.len()
                                    } else {
                                        0
                                    }
                                })
                                .sum();
                            timestamps.push(ts);
                            msg_count += 1;
                            total_chars += content_len as u64;

                            if let Some(dt) = Utc.timestamp_opt(ts, 0).single() {
                                let local = dt.with_timezone(&tz);
                                rhythm.hourly_activity[local.hour() as usize] += 1;
                                rhythm.daily_activity
                                    [local.weekday().num_days_from_monday() as usize] += 1;
                                let date = local.format("%Y-%m-%d").to_string();
                                *rhythm.daily_history.entry(date).or_insert(0) += 1;
                            }
                        }
                    }
                }
            }
        }
    }

    if msg_count == 0 {
        return;
    }

    rhythm.total_messages += msg_count;
    rhythm.total_chars += total_chars;
    rhythm.avg_message_length = rhythm.total_chars as f64 / rhythm.total_messages as f64;

    // Update avg response interval
    timestamps.sort();
    let mut intervals: Vec<i64> = Vec::new();
    for w in timestamps.windows(2) {
        let gap = w[1] - w[0];
        if gap > 0 && gap < SESSION_GAP_SECS {
            intervals.push(gap);
        }
    }
    if !intervals.is_empty() {
        rhythm.avg_response_interval_secs =
            intervals.iter().sum::<i64>() as f64 / intervals.len() as f64;
    }

    rhythm.updated_at = Utc::now().timestamp();
    save_rhythm(&instance_dir, &rhythm);
    log::info!(
        "[rhythm] snapshot before clear: {} messages accumulated for {slug}",
        rhythm.total_messages
    );
}

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

fn timestamp_to_utc(secs: i64) -> Option<DateTime<Utc>> {
    Utc.timestamp_opt(secs, 0).single()
}
