//! Drops — autonomous creative artifacts the companion generates.
//!
//! Each drop is a JSON file stored in `instances/{slug}/drops/`.
//! Drops are created during heartbeats or via the `create_drop` tool in chat.

use std::{
    fs,
    io::{self, ErrorKind},
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::domain::drop::{Drop, DropKind};

/// Max drops per instance per 24h window.
const MAX_DROPS_PER_DAY: usize = 3;

/// Check how many drops were created in the last 24 hours and whether a
/// similar title already exists among recent drops.
fn check_drop_limits(drops_dir: &Path, title: &str) -> io::Result<()> {
    if !drops_dir.is_dir() {
        return Ok(());
    }

    let now = unix_millis();
    let day_ms: u128 = 24 * 60 * 60 * 1000;
    let cutoff = now.saturating_sub(day_ms);

    let title_lower = title.to_lowercase();
    // Extract significant words (>3 chars) for fuzzy matching
    let title_words: Vec<&str> = title_lower
        .split_whitespace()
        .filter(|w| w.len() > 3)
        .collect();

    let mut recent_count = 0usize;

    for entry in fs::read_dir(drops_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let raw = match fs::read_to_string(&path) {
            Ok(r) => r,
            Err(_) => continue,
        };
        let drop: Drop = match serde_json::from_str(&raw) {
            Ok(d) => d,
            Err(_) => continue,
        };

        let created: u128 = drop.created_at.parse().unwrap_or(0);
        if created < cutoff {
            continue;
        }

        recent_count += 1;

        // Check title similarity — if most significant words overlap, it's a duplicate
        if !title_words.is_empty() {
            let existing_lower = drop.title.to_lowercase();
            let matching = title_words
                .iter()
                .filter(|w| existing_lower.contains(**w))
                .count();
            if matching >= title_words.len().max(1) / 2 + 1 {
                return Err(io::Error::new(
                    ErrorKind::Other,
                    format!("a similar drop already exists: \"{}\"", drop.title),
                ));
            }
        }
    }

    if recent_count >= MAX_DROPS_PER_DAY {
        return Err(io::Error::new(
            ErrorKind::Other,
            format!("daily drop limit reached ({MAX_DROPS_PER_DAY} per day)"),
        ));
    }

    Ok(())
}

/// Create a new drop and save it to disk.
pub fn create_drop_with_image(
    workspace_dir: &Path,
    instance_slug: &str,
    kind: &str,
    title: &str,
    content: &str,
    mood: &str,
    image_url: Option<&str>,
) -> io::Result<Drop> {
    let drops_dir = workspace_dir
        .join("instances")
        .join(instance_slug)
        .join("drops");
    fs::create_dir_all(&drops_dir)?;

    check_drop_limits(&drops_dir, title)?;

    let ts = unix_millis();
    let id = format!("drop_{ts}");

    let drop = Drop {
        id: id.clone(),
        kind: DropKind::from_str(kind),
        title: title.to_string(),
        content: content.to_string(),
        mood: mood.to_string(),
        created_at: ts.to_string(),
        image_url: image_url.map(|s| s.to_string()),
    };

    let path = drops_dir.join(format!("{id}.json"));
    let body = serde_json::to_string_pretty(&drop)
        .map_err(|e| io::Error::new(ErrorKind::InvalidData, e))?;
    fs::write(&path, body)?;

    log::info!("[drops] created {id} ({kind}) for {instance_slug}: {title}");
    Ok(drop)
}

/// List all drops for an instance, newest first.
pub fn list_drops(workspace_dir: &Path, instance_slug: &str) -> io::Result<Vec<Drop>> {
    let drops_dir = workspace_dir
        .join("instances")
        .join(instance_slug)
        .join("drops");

    if !drops_dir.is_dir() {
        return Ok(vec![]);
    }

    let mut drops: Vec<Drop> = Vec::new();

    for entry in fs::read_dir(&drops_dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }

        match fs::read_to_string(&path) {
            Ok(raw) => match serde_json::from_str::<Drop>(&raw) {
                Ok(drop) => drops.push(drop),
                Err(e) => log::warn!("skipping malformed drop {}: {e}", path.display()),
            },
            Err(e) => log::warn!("failed to read drop {}: {e}", path.display()),
        }
    }

    // Newest first
    drops.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(drops)
}

/// Get a single drop by ID.
pub fn get_drop(
    workspace_dir: &Path,
    instance_slug: &str,
    drop_id: &str,
) -> io::Result<Option<Drop>> {
    let path = workspace_dir
        .join("instances")
        .join(instance_slug)
        .join("drops")
        .join(format!("{drop_id}.json"));

    if !path.exists() {
        return Ok(None);
    }

    let raw = fs::read_to_string(&path)?;
    let drop: Drop =
        serde_json::from_str(&raw).map_err(|e| io::Error::new(ErrorKind::InvalidData, e))?;
    Ok(Some(drop))
}

/// Delete a drop by ID.
pub fn delete_drop(workspace_dir: &Path, instance_slug: &str, drop_id: &str) -> io::Result<bool> {
    let path = workspace_dir
        .join("instances")
        .join(instance_slug)
        .join("drops")
        .join(format!("{drop_id}.json"));

    if path.exists() {
        fs::remove_file(&path)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

fn unix_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after unix epoch")
        .as_millis()
}
