use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, fs, path::Path as StdPath};

use crate::app::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/instances/{instance_slug}/heartbeat/updates",
            get(get_pending_updates),
        )
        .route(
            "/api/instances/{instance_slug}/heartbeat/updates/{update_id}/apply",
            post(apply_update),
        )
        .route(
            "/api/instances/{instance_slug}/heartbeat/updates/{update_id}/dismiss",
            post(dismiss_update),
        )
}

// ---------------------------------------------------------------------------
// Update registry — add new entries here when the heartbeat prompt changes
// ---------------------------------------------------------------------------

struct HeartbeatUpdate {
    id: &'static str,
    description: &'static str,
    /// What will be added/changed (shown as preview in UI).
    preview: &'static str,
    /// Pairs of (find, replace). Applied in order. If find is empty, it's an append.
    patches: &'static [Patch],
}

struct Patch {
    find: &'static str,
    replace: &'static str,
}

const UPDATES: &[HeartbeatUpdate] = &[
    HeartbeatUpdate {
        id: "memory-library-v1",
        description: "Update memory tools: new file-based memory library replaces remember/recall",
        preview: "\
Tools updated: memory_write, memory_read, memory_list, memory_forget\n\
New section: memory maintenance — automatic cleanup and reorganization during heartbeat",
        patches: &[
            // Replace old memory tool line with new ones
            Patch {
                find: "- recall / remember — search or save memories about the user",
                replace: "- memory_write / memory_read / memory_list / memory_forget — manage your memory library",
            },
            // Append memory maintenance section before CRITICAL line
            Patch {
                find: "CRITICAL: if you want the user to see a message",
                replace: "\
## memory maintenance\n\
your memory library is your long-term knowledge base. during heartbeat, take a moment \
to review and maintain it:\n\
- READ files with memory_read to check if content is still accurate and relevant\n\
- MERGE related files — if two files cover the same topic, combine them into one\n\
- SPLIT files that grew too large or cover unrelated topics\n\
- DELETE outdated info with memory_forget (old projects, changed preferences, stale facts)\n\
- REORGANIZE — move files to better folders if the structure has grown messy\n\
- UPDATE facts that have changed since they were written\n\
- keep files concise — a few lines each. trim fluff, keep substance.\n\
\n\
don't overdo it — 1-3 maintenance ops per heartbeat is plenty. \
focus on what looks wrong or messy in the catalog.\n\
\n\
CRITICAL: if you want the user to see a message",
            },
        ],
    },
    HeartbeatUpdate {
        id: "image-generation-v1",
        description: "New: image generation — your companion can now create and send images",
        preview: "\
New section: image generation — create visual greetings, illustrate drops, \
generate selfies and artwork using fal.ai tools",
        patches: &[
            // Insert image generation section before memory maintenance
            Patch {
                find: "## memory maintenance",
                replace: "\
## image generation\n\
you can generate images using fal.ai tools when they're available. use them to:\n\
- create a visual greeting or postcard for the user (reach_out with the image URL)\n\
- illustrate a drop with a generated image\n\
- create a visual surprise — a selfie, a scene, an artwork that fits the mood\n\
- respond to user requests for images during conversation\n\
\n\
when generating images, be creative with prompts — describe the scene, style, lighting, \
mood in detail. save memorable generated images to memory (moments/ folder).\n\
\n\
## memory maintenance",
            },
        ],
    },
];

// ---------------------------------------------------------------------------
// State tracking
// ---------------------------------------------------------------------------

#[derive(Default, Serialize, Deserialize)]
struct UpdateState {
    #[serde(default)]
    applied: HashSet<String>,
    #[serde(default)]
    dismissed: HashSet<String>,
}

fn state_path(workspace_dir: &StdPath, slug: &str) -> std::path::PathBuf {
    workspace_dir
        .join("instances")
        .join(slug)
        .join("heartbeat_updates.json")
}

fn load_state(workspace_dir: &StdPath, slug: &str) -> UpdateState {
    let path = state_path(workspace_dir, slug);
    fs::read_to_string(&path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn save_state(workspace_dir: &StdPath, slug: &str, state: &UpdateState) {
    let path = state_path(workspace_dir, slug);
    if let Ok(json) = serde_json::to_string_pretty(state) {
        let _ = fs::write(&path, json);
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct PendingUpdate {
    id: String,
    description: String,
    preview: String,
}

async fn get_pending_updates(
    State(app): State<AppState>,
    Path(slug): Path<String>,
) -> Json<Vec<PendingUpdate>> {
    let instance_dir = app.workspace_dir.join("instances").join(&slug);
    if !instance_dir.join("heartbeat.md").exists() {
        return Json(vec![]);
    }

    let state = load_state(&app.workspace_dir, &slug);
    let heartbeat = fs::read_to_string(instance_dir.join("heartbeat.md")).unwrap_or_default();

    let pending: Vec<PendingUpdate> = UPDATES
        .iter()
        .filter(|u| {
            !state.applied.contains(u.id)
                && !state.dismissed.contains(u.id)
                && is_update_applicable(u, &heartbeat)
        })
        .map(|u| PendingUpdate {
            id: u.id.to_string(),
            description: u.description.to_string(),
            preview: u.preview.to_string(),
        })
        .collect();

    Json(pending)
}

/// Check if at least one patch in the update would change the file.
fn is_update_applicable(update: &HeartbeatUpdate, content: &str) -> bool {
    for patch in update.patches {
        if patch.find.is_empty() {
            // Append — always applicable if the content isn't already there
            if !content.contains(patch.replace) {
                return true;
            }
        } else if content.contains(patch.find) && !content.contains(patch.replace) {
            return true;
        }
    }
    false
}

async fn apply_update(
    State(app): State<AppState>,
    Path((slug, update_id)): Path<(String, String)>,
) -> StatusCode {
    let update = match UPDATES.iter().find(|u| u.id == update_id) {
        Some(u) => u,
        None => return StatusCode::NOT_FOUND,
    };

    let heartbeat_path = app
        .workspace_dir
        .join("instances")
        .join(&slug)
        .join("heartbeat.md");
    let mut content = match fs::read_to_string(&heartbeat_path) {
        Ok(c) => c,
        Err(_) => return StatusCode::NOT_FOUND,
    };

    // Apply patches
    for patch in update.patches {
        if patch.find.is_empty() {
            // Append
            if !content.contains(patch.replace) {
                if !content.ends_with('\n') {
                    content.push('\n');
                }
                content.push_str(patch.replace);
            }
        } else if content.contains(patch.find) {
            content = content.replace(patch.find, patch.replace);
        }
    }

    if fs::write(&heartbeat_path, &content).is_err() {
        return StatusCode::INTERNAL_SERVER_ERROR;
    }

    let mut state = load_state(&app.workspace_dir, &slug);
    state.applied.insert(update_id);
    save_state(&app.workspace_dir, &slug, &state);

    StatusCode::OK
}

async fn dismiss_update(
    State(app): State<AppState>,
    Path((slug, update_id)): Path<(String, String)>,
) -> StatusCode {
    let mut state = load_state(&app.workspace_dir, &slug);
    state.dismissed.insert(update_id);
    save_state(&app.workspace_dir, &slug, &state);

    StatusCode::OK
}
