use crate::app::state::AppState;
use axum::{
    Json, Router,
    extract::State,
    routing::{get, post},
};
use std::sync::Mutex;

// ── GitHub API cache (avoid rate limits: 60 req/h unauthenticated) ──

struct CachedResponse {
    data: serde_json::Value,
    fetched_at: std::time::Instant,
}

static RELEASE_CACHE: Mutex<Option<(String, CachedResponse)>> = Mutex::new(None);
static CHANGELOG_CACHE: Mutex<Option<CachedResponse>> = Mutex::new(None);

const CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(300); // 5 min

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/update/check", get(check_update))
        .route("/api/update/apply", post(apply_update))
        .route("/api/update/channel", get(get_channel).put(set_channel))
        .route("/api/update/changelog", get(get_changelog))
}

#[derive(serde::Serialize)]
struct UpdateCheck {
    current: String,
    latest: String,
    update_available: bool,
    commit: String,
}

async fn check_update(State(state): State<AppState>) -> Json<UpdateCheck> {
    let current = env!("CARGO_PKG_VERSION").to_string();
    let current_tag = format!("v{current}");
    let current_commit = option_env!("GIT_HASH").unwrap_or("dev");
    let channel = get_channel_value(&state.workspace_dir);

    let display_current_fallback = if channel == "nightly" && current_commit != "dev" {
        format!("{current_tag} ({current_commit})")
    } else {
        current_tag.clone()
    };

    let release = match fetch_release_info(&channel).await {
        Some(r) => r,
        None => {
            return Json(UpdateCheck {
                current: display_current_fallback,
                latest: "unknown".to_string(),
                update_available: false,
                commit: current_commit.to_string(),
            });
        }
    };

    // Parse commit SHA from nightly release body: "Auto-built from main (abc1234)"
    let release_commit = release
        .body
        .as_deref()
        .and_then(|b| b.split('(').nth(1))
        .and_then(|s| s.strip_suffix(')'))
        .unwrap_or("");

    let update_available = if channel == "nightly" {
        !release_commit.is_empty()
            && current_commit != "dev"
            && !current_commit.starts_with(release_commit)
    } else {
        release.tag != current_tag
    };

    let display_current = if channel == "nightly" && current_commit != "dev" {
        format!("{current_tag} ({current_commit})")
    } else {
        current_tag.clone()
    };

    let display_latest = if channel == "nightly" {
        let name = release.name.as_deref().unwrap_or(&release.tag);
        if !release_commit.is_empty() {
            format!("{name} ({release_commit})")
        } else {
            name.to_string()
        }
    } else {
        release.tag
    };

    Json(UpdateCheck {
        current: display_current,
        latest: display_latest,
        update_available,
        commit: current_commit.to_string(),
    })
}

async fn apply_update(State(state): State<AppState>) -> Json<serde_json::Value> {
    // Find update script — check multiple locations
    let bolly_home = std::env::var("BOLLY_HOME")
        .unwrap_or_else(|_| state.workspace_dir.to_string_lossy().to_string());
    let candidates = [
        format!("{bolly_home}/bin/update"), // self-hosted install (new)
        "/opt/bolly/scripts/update-bolly.sh".into(), // Docker/Fly
        "/opt/bolly/bin/update".into(),     // legacy bare-metal
    ];

    let script = candidates
        .iter()
        .map(std::path::PathBuf::from)
        .find(|p| p.exists());

    let script = match script {
        Some(s) => s,
        None => {
            return Json(serde_json::json!({
                "ok": false,
                "error": format!("update script not found (checked: {})", candidates.join(", "))
            }));
        }
    };

    log::info!("[update] applying update via {}", script.display());

    let bolly_home_clone = bolly_home.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;

        // Read version before update to detect actual changes
        let version_file = std::path::Path::new(&bolly_home_clone).join("bin/.version");
        let version_before = std::fs::read_to_string(&version_file)
            .unwrap_or_default()
            .trim()
            .to_string();

        let result = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(script.to_str().unwrap_or(""))
            .output()
            .await;

        match result {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                log::info!("[update] script output: {stdout}");
                if !stderr.is_empty() {
                    log::warn!("[update] script stderr: {stderr}");
                }

                // Check if the binary actually changed (don't rely on script exit code
                // since the script in the Docker image may be outdated)
                let version_after = std::fs::read_to_string(&version_file)
                    .unwrap_or_default()
                    .trim()
                    .to_string();

                if version_after != version_before && !version_after.is_empty() {
                    log::info!(
                        "[update] binary updated: {version_before} → {version_after}, restarting..."
                    );
                    std::process::exit(0);
                } else {
                    log::info!(
                        "[update] no change after update script (before={version_before}, after={version_after})"
                    );
                }
            }
            Err(e) => {
                log::error!("[update] failed to run update script: {e}");
            }
        }
    });

    Json(serde_json::json!({ "ok": true, "message": "updating... server will restart" }))
}

fn get_channel_value(workspace_dir: &std::path::Path) -> String {
    let path = workspace_dir.join(".update-channel");
    if let Ok(ch) = std::fs::read_to_string(&path) {
        let ch = ch.trim().to_string();
        if !ch.is_empty() {
            return ch;
        }
    }
    "stable".to_string()
}

async fn get_channel(State(state): State<AppState>) -> Json<serde_json::Value> {
    let channel = get_channel_value(&state.workspace_dir);
    Json(serde_json::json!({ "channel": channel }))
}

#[derive(serde::Deserialize)]
struct SetChannelRequest {
    channel: String,
}

async fn set_channel(
    State(state): State<AppState>,
    Json(req): Json<SetChannelRequest>,
) -> Json<serde_json::Value> {
    let channel = req.channel.trim().to_lowercase();
    if channel != "stable" && channel != "nightly" {
        return Json(
            serde_json::json!({ "ok": false, "error": "channel must be 'stable' or 'nightly'" }),
        );
    }
    let path = state.workspace_dir.join(".update-channel");
    let _ = std::fs::write(&path, &channel);
    Json(serde_json::json!({ "ok": true, "channel": channel }))
}

struct ReleaseInfo {
    tag: String,
    name: Option<String>,
    body: Option<String>,
}

async fn fetch_release_info(channel: &str) -> Option<ReleaseInfo> {
    let repo = "triangle-int/bolly";

    // Check cache
    {
        let cache = RELEASE_CACHE.lock().ok()?;
        if let Some((ref cached_channel, ref entry)) = *cache {
            if cached_channel == channel && entry.fetched_at.elapsed() < CACHE_TTL {
                let data = &entry.data;
                return Some(ReleaseInfo {
                    tag: data["tag_name"].as_str()?.to_string(),
                    name: data["name"].as_str().map(|s| s.to_string()),
                    body: data["body"].as_str().map(|s| s.to_string()),
                });
            }
        }
    }

    let url = if channel == "nightly" {
        format!("https://api.github.com/repos/{repo}/releases/tags/nightly")
    } else {
        format!("https://api.github.com/repos/{repo}/releases/latest")
    };

    let client = reqwest::Client::new();
    let resp = client
        .get(&url)
        .header("User-Agent", "bolly-update")
        .send()
        .await
        .ok()?;

    if !resp.status().is_success() {
        log::warn!("[update] GitHub API returned {}", resp.status());
        return None;
    }

    let data: serde_json::Value = resp.json().await.ok()?;

    // Update cache
    if let Ok(mut cache) = RELEASE_CACHE.lock() {
        *cache = Some((
            channel.to_string(),
            CachedResponse {
                data: data.clone(),
                fetched_at: std::time::Instant::now(),
            },
        ));
    }

    Some(ReleaseInfo {
        tag: data["tag_name"].as_str()?.to_string(),
        name: data["name"].as_str().map(|s| s.to_string()),
        body: data["body"].as_str().map(|s| s.to_string()),
    })
}

#[derive(serde::Serialize)]
struct Changelog {
    version: String,
    body: String,
}

async fn get_changelog(State(state): State<AppState>) -> Json<Vec<Changelog>> {
    let channel = get_channel_value(&state.workspace_dir);
    let repo = "triangle-int/bolly";

    // Check cache
    let cached = CHANGELOG_CACHE.lock().ok().and_then(|cache| {
        cache.as_ref().and_then(|entry| {
            if entry.fetched_at.elapsed() < CACHE_TTL {
                serde_json::from_value::<Vec<serde_json::Value>>(entry.data.clone()).ok()
            } else {
                None
            }
        })
    });

    let data = if let Some(d) = cached {
        d
    } else {
        let url = format!("https://api.github.com/repos/{repo}/releases?per_page=10");
        let client = reqwest::Client::new();
        let resp = match client
            .get(&url)
            .header("User-Agent", "bolly-update")
            .send()
            .await
        {
            Ok(r) if r.status().is_success() => r,
            Ok(r) => {
                log::warn!("[changelog] GitHub API returned {}", r.status());
                return Json(vec![]);
            }
            Err(_) => return Json(vec![]),
        };

        let d: Vec<serde_json::Value> = match resp.json().await {
            Ok(d) => d,
            Err(_) => return Json(vec![]),
        };

        // Update cache
        if let Ok(mut cache) = CHANGELOG_CACHE.lock() {
            *cache = Some(CachedResponse {
                data: serde_json::to_value(&d).unwrap_or_default(),
                fetched_at: std::time::Instant::now(),
            });
        }

        d
    };

    let entries: Vec<Changelog> = data
        .iter()
        .filter(|r| {
            let tag = r["tag_name"].as_str().unwrap_or("");
            if channel == "nightly" {
                true
            } else {
                tag != "nightly"
            }
        })
        .filter_map(|r| {
            let version = r["tag_name"].as_str()?.to_string();
            let body = r["body"].as_str().unwrap_or("").to_string();
            Some(Changelog { version, body })
        })
        .collect();

    Json(entries)
}
