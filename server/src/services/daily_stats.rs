use std::{fs, path::Path};

use chrono::{Datelike, Timelike, Utc};

use crate::domain::daily_stats::DailyStats;

/// Directory for daily stats files.
fn stats_dir(workspace_dir: &Path, instance_slug: &str) -> std::path::PathBuf {
    workspace_dir
        .join("instances")
        .join(instance_slug)
        .join("stats")
}

/// Record a user message: increment today's stats file.
pub fn record_message(workspace_dir: &Path, instance_slug: &str, content_len: usize) {
    let instance_dir = workspace_dir.join("instances").join(instance_slug);
    let tz: chrono_tz::Tz = crate::routes::instances::read_timezone(&instance_dir)
        .and_then(|s| s.parse().ok())
        .unwrap_or(chrono_tz::UTC);

    let local = Utc::now().with_timezone(&tz);
    let date = local.format("%Y-%m-%d").to_string();
    let hour = local.hour() as usize;
    let weekday = local.weekday().num_days_from_monday() as u8;

    let dir = stats_dir(workspace_dir, instance_slug);
    let _ = fs::create_dir_all(&dir);
    let path = dir.join(format!("{date}.json"));

    let mut day = load_day(&path).unwrap_or_else(|| DailyStats {
        date: date.clone(),
        weekday,
        ..Default::default()
    });

    day.messages += 1;
    day.chars += content_len as u64;
    day.hours[hour] += 1;

    if let Ok(json) = serde_json::to_string(&day) {
        let _ = fs::write(&path, json);
    }
}

/// Load all daily stats files for an instance.
pub fn load_all(workspace_dir: &Path, instance_slug: &str) -> Vec<DailyStats> {
    let dir = stats_dir(workspace_dir, instance_slug);
    let mut days: Vec<DailyStats> = Vec::new();

    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            if let Ok(raw) = fs::read_to_string(&path) {
                if let Ok(day) = serde_json::from_str::<DailyStats>(&raw) {
                    days.push(day);
                }
            }
        }
    }

    days.sort_by(|a, b| a.date.cmp(&b.date));
    days
}

fn load_day(path: &Path) -> Option<DailyStats> {
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}
