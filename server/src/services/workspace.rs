use std::{
    fs, io,
    path::{Path, PathBuf},
};

use crate::domain::instance::InstanceSummary;

pub fn count_directories(path: &Path) -> io::Result<usize> {
    Ok(fs::read_dir(path)?
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .count())
}

/// Summary of the one canonical companion, if its directory exists.
pub fn companion_summary(workspace_dir: &Path) -> Option<InstanceSummary> {
    let dir = super::companion::companion_dir(workspace_dir);
    dir.is_dir().then(|| summarize_instance(&dir)).flatten()
}

pub fn summarize_instance(path: &Path) -> Option<InstanceSummary> {
    let slug = path.file_name()?.to_string_lossy().to_string();
    let drops_dir = path.join("drops");
    let memory_dir = path.join("memory");

    let companion_name = read_companion_name(path).unwrap_or_default();

    Some(InstanceSummary {
        slug,
        companion_name,
        soul_exists: path.join("soul.md").exists(),
        drops_count: count_markdown_files(&drops_dir).unwrap_or(0),
        has_memory: memory_dir.join("facts.md").exists(),
        has_skin: has_skin_file(path),
    })
}

fn read_companion_name(path: &Path) -> Option<String> {
    let raw = fs::read_to_string(path.join("project_state.json")).ok()?;
    let state: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let name = state.get("identity")?.get("name")?.as_str()?;
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

fn count_markdown_files(path: &Path) -> io::Result<usize> {
    if !path.exists() {
        return Ok(0);
    }

    Ok(fs::read_dir(path)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "md"))
        .count())
}

fn has_skin_file(path: &Path) -> bool {
    path.join("skin.glb").exists() || contains_glb_file(path)
}

fn contains_glb_file(path: &Path) -> bool {
    fs::read_dir(path)
        .ok()
        .into_iter()
        .flat_map(|entries| entries.filter_map(Result::ok))
        .map(|entry| entry.path())
        .any(|path: PathBuf| path.extension().is_some_and(|ext| ext == "glb"))
}
