//! Thought persistence — stores heartbeat inner monologue to disk.

use std::fs;
use std::path::Path;

use crate::domain::thought::Thought;

/// Save a thought to `instances/{slug}/thoughts/{id}.json`.
pub fn save_thought(workspace_dir: &Path, slug: &str, thought: &Thought) -> anyhow::Result<()> {
    let dir = workspace_dir.join("instances").join(slug).join("thoughts");
    fs::create_dir_all(&dir)?;

    let path = dir.join(format!("{}.json", thought.id));
    let json = serde_json::to_string_pretty(thought)?;
    fs::write(path, json)?;
    Ok(())
}

/// List all thoughts for an instance, newest first.
pub fn list_thoughts(workspace_dir: &Path, slug: &str) -> anyhow::Result<Vec<Thought>> {
    let dir = workspace_dir.join("instances").join(slug).join("thoughts");

    if !dir.is_dir() {
        return Ok(vec![]);
    }

    let mut thoughts: Vec<Thought> = fs::read_dir(&dir)?
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("json"))
        .filter_map(|e| {
            let raw = fs::read_to_string(e.path()).ok()?;
            serde_json::from_str(&raw).ok()
        })
        .collect();

    // Newest first (created_at is millis timestamp string)
    thoughts.sort_by(|a, b| b.created_at.cmp(&a.created_at));

    Ok(thoughts)
}
