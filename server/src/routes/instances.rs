use axum::response::IntoResponse;
use axum::{
    Json, Router,
    body::Body,
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get, post, put},
};
use serde::{Deserialize, Serialize};
use std::fs;

use crate::{
    app::state::AppState,
    domain::instance::InstanceSummary,
    domain::memory::MemoryEntry,
    services::{chat, companion, memory, tools, workspace},
};

/// Retired control-token resource namespace; always denies access.
pub fn public_memory_router() -> Router<AppState> {
    Router::new().route(
        "/public/memory/{instance_slug}/{*path}",
        get(serve_memory_file_public),
    )
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/instances", get(list_instances))
        .route("/api/instances/{instance_slug}", delete(delete_instance))
        .route("/api/instances/{instance_slug}/mood", get(get_mood))
        .route(
            "/api/instances/{instance_slug}/companion-name",
            get(get_companion_name),
        )
        .route(
            "/api/instances/{instance_slug}/companion-name",
            put(set_companion_name),
        )
        .route("/api/instances/{instance_slug}/timezone", get(get_timezone))
        .route("/api/instances/{instance_slug}/timezone", put(set_timezone))
        .route("/api/instances/{instance_slug}/secret", post(submit_secret))
        .route(
            "/api/instances/{instance_slug}/secret/{secret_id}",
            delete(cancel_secret),
        )
        .route(
            "/api/instances/{instance_slug}/context-stats",
            get(get_context_stats),
        )
        .route(
            "/api/instances/{instance_slug}/{chat_id}/context-stats",
            get(get_context_stats_chat),
        )
        .route("/api/instances/{instance_slug}/stats", get(get_stats))
        .route("/api/instances/{instance_slug}/memory", get(list_memory))
        .route(
            "/api/instances/{instance_slug}/memory/search",
            get(search_memory),
        )
        .route(
            "/api/instances/{instance_slug}/memory/reindex",
            post(reindex_memory),
        )
        .route(
            "/api/instances/{instance_slug}/memory/vectors",
            get(list_vectors),
        )
        .route(
            "/api/instances/{instance_slug}/memory/graph",
            get(get_memory_graph),
        )
        .route(
            "/api/instances/{instance_slug}/memory/{*path}",
            get(read_memory_file).delete(delete_memory_file),
        )
        .route(
            "/api/instances/{instance_slug}/email",
            get(get_email_config),
        )
        .route(
            "/api/instances/{instance_slug}/email",
            put(set_email_config),
        )
        .route(
            "/api/instances/{instance_slug}/email",
            delete(delete_email_config),
        )
        .route("/api/instances/{instance_slug}/voice", get(get_voice_id))
        .route("/api/instances/{instance_slug}/voice", put(set_voice_id))
        .route(
            "/api/instances/{instance_slug}/voice-mode",
            get(get_voice_enabled),
        )
        .route(
            "/api/instances/{instance_slug}/voice-mode",
            put(set_voice_enabled),
        )
        .route("/api/instances/{instance_slug}/skin", get(get_skin))
        .route("/api/instances/{instance_slug}/skin", put(set_skin))
        .route(
            "/api/instances/{instance_slug}/scheduled",
            get(list_scheduled),
        )
        .route(
            "/api/instances/{instance_slug}/scheduled/{message_id}",
            delete(cancel_scheduled),
        )
        .route(
            "/api/instances/{instance_slug}/export",
            get(export_instance),
        )
        .route(
            "/api/instances/{instance_slug}/import",
            post(import_instance),
        )
}

/// The one canonical companion, or an empty list until it has been created.
/// An unsupported identity marker fails closed instead of listing anything.
async fn list_instances(
    State(state): State<AppState>,
) -> Result<Json<Vec<InstanceSummary>>, axum::response::Response> {
    match companion::read_identity(&state.workspace_dir) {
        Ok(Some(_)) => Ok(Json(
            workspace::companion_summary(&state.workspace_dir)
                .into_iter()
                .collect(),
        )),
        Ok(None) => Ok(Json(Vec::new())),
        Err(error) => {
            Err(crate::app::companion_boundary::CompanionRejection::from(error).into_response())
        }
    }
}

async fn delete_instance(
    State(state): State<AppState>,
    Path(instance_slug): Path<String>,
) -> StatusCode {
    // Validate slug to prevent path traversal
    if instance_slug.contains('/')
        || instance_slug.contains('\\')
        || instance_slug == ".."
        || instance_slug == "."
    {
        return StatusCode::BAD_REQUEST;
    }

    let instance_dir = state.workspace_dir.join("instances").join(&instance_slug);
    if !instance_dir.exists() {
        return StatusCode::NOT_FOUND;
    }

    match fs::remove_dir_all(&instance_dir) {
        Ok(_) => StatusCode::NO_CONTENT,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

#[derive(Serialize)]
struct MoodResponse {
    mood: String,
}

async fn get_mood(
    State(state): State<AppState>,
    Path(instance_slug): Path<String>,
) -> Json<MoodResponse> {
    let instance_dir = state.workspace_dir.join("instances").join(&instance_slug);
    let mood_state = tools::load_mood_state(&instance_dir);
    Json(MoodResponse {
        mood: mood_state.companion_mood,
    })
}

#[derive(Serialize)]
struct CompanionNameResponse {
    name: String,
}

#[derive(Deserialize)]
struct SetCompanionNameRequest {
    name: String,
}

async fn get_companion_name(
    State(state): State<AppState>,
    Path(instance_slug): Path<String>,
) -> Json<CompanionNameResponse> {
    let instance_dir = state.workspace_dir.join("instances").join(&instance_slug);
    let name = read_identity_name(&instance_dir).unwrap_or_default();
    Json(CompanionNameResponse { name })
}

async fn set_companion_name(
    State(state): State<AppState>,
    Path(instance_slug): Path<String>,
    Json(req): Json<SetCompanionNameRequest>,
) -> StatusCode {
    let instance_dir = state.workspace_dir.join("instances").join(&instance_slug);
    let state_path = instance_dir.join("project_state.json");

    let mut project_state: serde_json::Value = fs::read_to_string(&state_path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_else(|| serde_json::json!({}));

    if project_state.get("identity").is_none() {
        project_state["identity"] = serde_json::json!({});
    }
    project_state["identity"]["name"] = serde_json::Value::String(req.name);

    fs::create_dir_all(&instance_dir).ok();
    match serde_json::to_string_pretty(&project_state) {
        Ok(body) => {
            if fs::write(&state_path, body).is_ok() {
                StatusCode::OK
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        }
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

fn read_identity_name(instance_dir: &std::path::Path) -> Option<String> {
    let raw = fs::read_to_string(instance_dir.join("project_state.json")).ok()?;
    let state: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let name = state.get("identity")?.get("name")?.as_str()?;
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

// ---------------------------------------------------------------------------
// Timezone
// ---------------------------------------------------------------------------

async fn get_timezone(
    State(state): State<AppState>,
    Path(instance_slug): Path<String>,
) -> Json<serde_json::Value> {
    let instance_dir = state.workspace_dir.join("instances").join(&instance_slug);
    let tz = read_timezone(&instance_dir).unwrap_or_default();
    Json(serde_json::json!({ "timezone": tz }))
}

#[derive(Deserialize)]
struct SetTimezoneRequest {
    timezone: String,
}

async fn set_timezone(
    State(state): State<AppState>,
    Path(instance_slug): Path<String>,
    Json(req): Json<SetTimezoneRequest>,
) -> StatusCode {
    // Validate timezone string
    if !req.timezone.is_empty() {
        if req.timezone.parse::<chrono_tz::Tz>().is_err() {
            return StatusCode::BAD_REQUEST;
        }
    }

    let instance_dir = state.workspace_dir.join("instances").join(&instance_slug);
    let state_path = instance_dir.join("project_state.json");

    let mut project_state: serde_json::Value = fs::read_to_string(&state_path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_else(|| serde_json::json!({}));

    project_state["timezone"] = serde_json::Value::String(req.timezone);

    fs::create_dir_all(&instance_dir).ok();
    match serde_json::to_string_pretty(&project_state) {
        Ok(body) => {
            if fs::write(&state_path, body).is_ok() {
                StatusCode::OK
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        }
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

/// Format current time in the instance's configured timezone (or UTC).
pub fn format_instance_now(instance_dir: &std::path::Path) -> String {
    let now = chrono::Utc::now();
    if let Some(tz_str) = read_timezone(instance_dir) {
        if let Ok(tz) = tz_str.parse::<chrono_tz::Tz>() {
            return now
                .with_timezone(&tz)
                .format("%A, %B %-d, %Y %H:%M %Z")
                .to_string();
        }
    }
    now.format("%A, %B %-d, %Y %H:%M UTC").to_string()
}

pub fn read_timezone(instance_dir: &std::path::Path) -> Option<String> {
    let raw = fs::read_to_string(instance_dir.join("project_state.json")).ok()?;
    let state: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let tz = state.get("timezone")?.as_str()?;
    if tz.is_empty() {
        None
    } else {
        Some(tz.to_string())
    }
}

// ---------------------------------------------------------------------------
// Voice ID (ElevenLabs)
// ---------------------------------------------------------------------------

async fn get_voice_id(
    State(state): State<AppState>,
    Path(instance_slug): Path<String>,
) -> Json<serde_json::Value> {
    let inst = crate::config::InstanceConfig::load(&state.workspace_dir, &instance_slug);
    Json(serde_json::json!({ "voice_id": inst.elevenlabs_voice_id }))
}

#[derive(Deserialize)]
struct SetVoiceIdRequest {
    voice_id: String,
}

async fn set_voice_id(
    State(state): State<AppState>,
    Path(instance_slug): Path<String>,
    Json(req): Json<SetVoiceIdRequest>,
) -> StatusCode {
    let mut inst = crate::config::InstanceConfig::load(&state.workspace_dir, &instance_slug);
    inst.elevenlabs_voice_id = req.voice_id;
    match inst.save(&state.workspace_dir, &instance_slug) {
        Ok(_) => StatusCode::OK,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

// ---------------------------------------------------------------------------
// Voice enabled
// ---------------------------------------------------------------------------

async fn get_voice_enabled(
    State(state): State<AppState>,
    Path(instance_slug): Path<String>,
) -> Json<serde_json::Value> {
    let inst = crate::config::InstanceConfig::load(&state.workspace_dir, &instance_slug);
    Json(serde_json::json!({ "voice_enabled": inst.voice_enabled }))
}

#[derive(Deserialize)]
struct SetVoiceEnabledRequest {
    voice_enabled: bool,
}

async fn set_voice_enabled(
    State(state): State<AppState>,
    Path(instance_slug): Path<String>,
    Json(req): Json<SetVoiceEnabledRequest>,
) -> StatusCode {
    let mut inst = crate::config::InstanceConfig::load(&state.workspace_dir, &instance_slug);
    inst.voice_enabled = req.voice_enabled;
    match inst.save(&state.workspace_dir, &instance_slug) {
        Ok(_) => StatusCode::OK,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

// ---------------------------------------------------------------------------
// Skin
// ---------------------------------------------------------------------------

async fn get_skin(
    State(state): State<AppState>,
    Path(instance_slug): Path<String>,
) -> Json<serde_json::Value> {
    let inst = crate::config::InstanceConfig::load(&state.workspace_dir, &instance_slug);
    Json(serde_json::json!({ "skin": inst.skin }))
}

#[derive(Deserialize)]
struct SetSkinRequest {
    skin: String,
}

async fn set_skin(
    State(state): State<AppState>,
    Path(instance_slug): Path<String>,
    Json(req): Json<SetSkinRequest>,
) -> StatusCode {
    let mut inst = crate::config::InstanceConfig::load(&state.workspace_dir, &instance_slug);
    inst.skin = req.skin;
    match inst.save(&state.workspace_dir, &instance_slug) {
        Ok(_) => StatusCode::OK,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

// ---------------------------------------------------------------------------
// Secret submission endpoint
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct SubmitSecretRequest {
    id: String,
    value: String,
}

async fn submit_secret(
    State(state): State<AppState>,
    Path(_instance_slug): Path<String>,
    Json(req): Json<SubmitSecretRequest>,
) -> StatusCode {
    let mut secrets = state.pending_secrets.lock().await;
    match secrets.remove(&req.id) {
        Some(pending) => {
            let _ = pending.responder.send(req.value);
            StatusCode::OK
        }
        None => StatusCode::NOT_FOUND,
    }
}

async fn cancel_secret(
    State(state): State<AppState>,
    Path((_instance_slug, secret_id)): Path<(String, String)>,
) -> StatusCode {
    let mut secrets = state.pending_secrets.lock().await;
    match secrets.remove(&secret_id) {
        Some(_pending) => {
            // Dropping the PendingSecret drops the oneshot Sender,
            // which causes the tool's rx.await to return Err → "cancelled"
            StatusCode::OK
        }
        None => StatusCode::NOT_FOUND,
    }
}

// ---------------------------------------------------------------------------
// Context stats endpoint
// ---------------------------------------------------------------------------

async fn get_context_stats(
    State(state): State<AppState>,
    Path(instance_slug): Path<String>,
) -> Json<chat::ContextStats> {
    let wd = state.workspace_dir.clone();
    let slug = instance_slug.clone();
    let public_url = state.config.read().await.public_url.clone();
    let resources = state.resources.clone();
    let http_client = state.http_client.clone();
    let stats = tokio::spawn(async move {
        chat::compute_context_stats_async(
            wd,
            slug,
            "default".to_string(),
            public_url,
            resources,
            http_client,
        )
        .await
    })
    .await
    .unwrap_or_else(|_| {
        chat::compute_context_stats(&state.workspace_dir, &instance_slug, "default")
    });
    Json(stats)
}

async fn get_context_stats_chat(
    State(state): State<AppState>,
    Path((instance_slug, chat_id)): Path<(String, String)>,
) -> Json<chat::ContextStats> {
    let wd = state.workspace_dir.clone();
    let slug = instance_slug.clone();
    let cid = chat_id.clone();
    let public_url = state.config.read().await.public_url.clone();
    let resources = state.resources.clone();
    let http_client = state.http_client.clone();
    let stats = tokio::spawn(async move {
        chat::compute_context_stats_async(wd, slug, cid, public_url, resources, http_client).await
    })
    .await
    .unwrap_or_else(|_| {
        chat::compute_context_stats(&state.workspace_dir, &instance_slug, &chat_id)
    });
    Json(stats)
}

// ---------------------------------------------------------------------------
// Stats / Analytics
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct StatsResponse {
    /// Messages per hour of day (0-23)
    hourly_activity: [u32; 24],
    /// Messages per day of week (0=Mon, 6=Sun)
    daily_activity: [u32; 7],
    /// Total user messages
    total_messages: u32,
    /// Average message length (chars)
    avg_message_length: f64,
    /// Average seconds between messages in a session
    avg_response_interval_secs: f64,
    /// Daily message counts: [(date_str, count)]
    daily_history: Vec<(String, u32)>,
    /// Mood distribution: {mood: count}
    mood_counts: std::collections::HashMap<String, u32>,
    /// Current streak (consecutive days with messages)
    streak_days: u32,
    /// First message timestamp (millis)
    first_message_at: Option<String>,
}

async fn get_stats(
    State(state): State<AppState>,
    Path(instance_slug): Path<String>,
) -> Json<StatsResponse> {
    let instance_dir = state.workspace_dir.join("instances").join(&instance_slug);

    // Load all daily stats files — one per day, no double-counting
    let days = crate::services::daily_stats::load_all(&state.workspace_dir, &instance_slug);

    let mut hourly_activity = [0u32; 24];
    let mut daily_activity = [0u32; 7];
    let mut total_messages: u32 = 0;
    let mut total_chars: u64 = 0;

    for day in &days {
        total_messages += day.messages;
        total_chars += day.chars;
        for (h, count) in day.hours.iter().enumerate() {
            hourly_activity[h] += count;
        }
        daily_activity[day.weekday as usize % 7] += day.messages;
    }

    let daily_history: Vec<(String, u32)> =
        days.iter().map(|d| (d.date.clone(), d.messages)).collect();

    let avg_message_length = if total_messages > 0 {
        total_chars as f64 / total_messages as f64
    } else {
        0.0
    };

    // Load avg_response_interval from rhythm.json (still computed by heartbeat)
    let rhythm: crate::domain::rhythm::InteractionRhythm =
        fs::read_to_string(instance_dir.join("rhythm.json"))
            .ok()
            .and_then(|r| serde_json::from_str(&r).ok())
            .unwrap_or_default();
    let avg_response_interval_secs = rhythm.avg_response_interval_secs;

    // Scan thoughts for mood distribution
    let mut mood_counts: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
    let thoughts_dir = instance_dir.join("thoughts");
    if let Ok(entries) = fs::read_dir(&thoughts_dir) {
        for entry in entries.filter_map(Result::ok) {
            if entry.path().extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            if let Ok(raw) = fs::read_to_string(entry.path()) {
                if let Ok(thought) = serde_json::from_str::<serde_json::Value>(&raw) {
                    if let Some(m) = thought["mood"].as_str() {
                        if !m.is_empty() {
                            *mood_counts.entry(m.to_string()).or_insert(0) += 1;
                        }
                    }
                }
            }
        }
    }

    // Streak: consecutive days ending today or yesterday (local time)
    let tz: chrono_tz::Tz = read_timezone(&instance_dir)
        .and_then(|s| s.parse().ok())
        .unwrap_or(chrono_tz::UTC);
    let local_now = chrono::Utc::now().with_timezone(&tz);
    let today = local_now.format("%Y-%m-%d").to_string();
    let yesterday = (local_now - chrono::Duration::days(1))
        .format("%Y-%m-%d")
        .to_string();
    let dates: std::collections::HashSet<String> =
        daily_history.iter().map(|(d, _)| d.clone()).collect();
    let mut streak_days = 0u32;
    let mut check_date = if dates.contains(&today) {
        local_now.date_naive()
    } else if dates.contains(&yesterday) {
        (local_now - chrono::Duration::days(1)).date_naive()
    } else {
        local_now.date_naive()
    };
    loop {
        if dates.contains(&check_date.format("%Y-%m-%d").to_string()) {
            streak_days += 1;
            check_date -= chrono::Duration::days(1);
        } else {
            break;
        }
    }

    // First message: earliest date from daily stats
    let first_message_at = days.first().and_then(|d| {
        chrono::NaiveDate::parse_from_str(&d.date, "%Y-%m-%d")
            .ok()
            .map(|nd| {
                let ts = nd.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp();
                (ts * 1000).to_string()
            })
    });

    Json(StatsResponse {
        hourly_activity,
        daily_activity,
        total_messages,
        avg_message_length,
        avg_response_interval_secs,
        daily_history,
        mood_counts,
        streak_days,
        first_message_at,
    })
}

// ---------------------------------------------------------------------------
// Memory library
// ---------------------------------------------------------------------------

async fn list_memory(
    State(state): State<AppState>,
    Path(instance_slug): Path<String>,
) -> Json<Vec<MemoryEntry>> {
    Json(memory::scan_library(
        &state.vector_store.media_store(),
        &instance_slug,
    ))
}

#[derive(Deserialize)]
struct SearchQuery {
    q: String,
    #[serde(default = "default_search_limit")]
    limit: usize,
}

fn default_search_limit() -> usize {
    10
}

async fn search_memory(
    State(state): State<AppState>,
    Path(instance_slug): Path<String>,
    axum::extract::Query(params): axum::extract::Query<SearchQuery>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let results = state
        .vector_store
        .search_text(&instance_slug, &params.q, params.limit)
        .await;

    let media = state.vector_store.media_store();

    let json: Vec<serde_json::Value> = results
        .into_iter()
        .map(|r| {
            let is_media = r.source_type.starts_with("media_");

            let text = if r.content_preview.is_empty() && !is_media {
                media
                    .read_memory_text(&instance_slug, &r.path)
                    .unwrap_or_default()
            } else {
                r.content_preview
            };

            let mut obj = serde_json::json!({
                "path": r.path,
                "text": text,
                "score": r.score,
                "source_type": r.source_type,
            });

            // For media results, include a URL to the file
            if is_media {
                if let Some(upload_id) = &r.upload_id {
                    use crate::services::resource_capability::{
                        CapabilityAudience, CapabilityResource,
                    };
                    let resource = if upload_id == &r.path || upload_id.contains('/') {
                        CapabilityResource::memory(upload_id)
                    } else {
                        CapabilityResource::uploaded_file(upload_id)
                    };
                    let url = resource
                        .and_then(|resource| {
                            state.resources.url(
                                "",
                                &instance_slug,
                                resource,
                                CapabilityAudience::Browser,
                            )
                        })
                        .unwrap_or_default();
                    obj["media_url"] = serde_json::Value::String(url);
                }
            }

            obj
        })
        .collect();

    Ok(Json(serde_json::Value::Array(json)))
}

async fn list_vectors(
    State(state): State<AppState>,
    Path(instance_slug): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let results = state
        .vector_store
        .list_all(&instance_slug, 500)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let json: Vec<serde_json::Value> = results
        .into_iter()
        .map(|r| {
            serde_json::json!({
                "path": r.path,
                "source_type": r.source_type,
                "content_preview": r.content_preview,
                "upload_id": r.upload_id,
            })
        })
        .collect();

    Ok(Json(serde_json::Value::Array(json)))
}

async fn get_memory_graph(
    State(state): State<AppState>,
    Path(instance_slug): Path<String>,
) -> Json<crate::domain::memory::MemoryGraph> {
    Json(memory::load_graph(
        &state.vector_store.media_store(),
        &instance_slug,
    ))
}

async fn reindex_memory(
    State(state): State<AppState>,
    Path(instance_slug): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    // Backfill builds and atomically commits a candidate; keep the last good
    // index searchable if any embedding request fails.
    // Backfill in background
    let vs = state.vector_store.clone();
    let ws = state.workspace_dir.clone();
    let slug = instance_slug.clone();
    tokio::spawn(async move {
        match vs.backfill_text_memories(&ws, &slug).await {
            Ok(count) => log::info!("[reindex] {slug}: indexed {count} chunks"),
            Err(e) => log::warn!("[reindex] {slug}: failed: {e}"),
        }
    });

    Ok(Json(serde_json::json!({ "status": "reindexing" })))
}

async fn serve_memory_file_public() -> StatusCode {
    StatusCode::UNAUTHORIZED
}

async fn read_memory_file(
    State(state): State<AppState>,
    Path((instance_slug, file_path)): Path<(String, String)>,
) -> Result<axum::response::Response<axum::body::Body>, StatusCode> {
    serve_memory_file_inner(&state, &instance_slug, &file_path).await
}

pub(super) async fn serve_memory_file_inner(
    state: &AppState,
    instance_slug: &str,
    file_path: &str,
) -> Result<axum::response::Response<axum::body::Body>, StatusCode> {
    const MAX_MEMORY_FILE_BYTES: usize = 64 * 1024 * 1024;
    let media = state.vector_store.media_store();
    let slug = instance_slug.to_owned();
    let path = file_path.to_owned();
    let bytes = tokio::task::spawn_blocking(move || {
        media.read_memory_file(&slug, &path, MAX_MEMORY_FILE_BYTES)
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    .map_err(|_| StatusCode::NOT_FOUND)?;

    let extension = std::path::Path::new(file_path)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase);
    let content_type = match extension.as_deref() {
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("png") => "image/png",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        Some("mp4") => "video/mp4",
        Some("webm") => "video/webm",
        Some("mov") => "video/quicktime",
        Some("ogg") => "audio/ogg",
        Some("m4a") => "audio/mp4",
        Some("flac") => "audio/flac",
        Some("aac") => "audio/aac",
        Some("avif") => "image/avif",
        Some("bmp") => "image/bmp",
        Some("ico") => "image/x-icon",
        Some("mp3") => "audio/mpeg",
        Some("wav") => "audio/wav",
        Some("pdf") => "application/pdf",
        Some("md" | "txt") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    };

    let is_media = content_type.starts_with("image/")
        || content_type.starts_with("video/")
        || content_type.starts_with("audio/");
    let filename = std::path::Path::new(file_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file");
    let disposition = if is_media { "inline" } else { "attachment" };

    axum::response::Response::builder()
        .header(axum::http::header::CONTENT_TYPE, content_type)
        .header(
            axum::http::header::CONTENT_DISPOSITION,
            format!("{disposition}; filename=\"{filename}\""),
        )
        .header(axum::http::header::CACHE_CONTROL, "private, no-store")
        .body(axum::body::Body::from(bytes))
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

async fn delete_memory_file(
    State(state): State<AppState>,
    Path((instance_slug, file_path)): Path<(String, String)>,
) -> StatusCode {
    if let Err(error) = state
        .vector_store
        .delete_memory(&instance_slug, &file_path)
        .await
    {
        log::warn!("[delete_memory] cleanup failed for {file_path}: {error}");
        return StatusCode::INTERNAL_SERVER_ERROR;
    }
    StatusCode::OK
}

// ---------------------------------------------------------------------------
// Email config (per-instance SMTP/IMAP)
// ---------------------------------------------------------------------------

async fn get_email_config(
    State(state): State<AppState>,
    Path(instance_slug): Path<String>,
) -> Json<serde_json::Value> {
    let accounts = crate::config::EmailAccounts::load(&state.workspace_dir, &instance_slug);
    let items: Vec<serde_json::Value> = accounts
        .iter()
        .map(|cfg| {
            serde_json::json!({
                "smtp_host": cfg.smtp_host,
                "smtp_port": cfg.smtp_port,
                "smtp_user": cfg.smtp_user,
                "smtp_from": cfg.smtp_from,
                "imap_host": cfg.imap_host,
                "imap_port": cfg.imap_port,
                "imap_user": cfg.imap_user,
                // Never expose passwords
            })
        })
        .collect();
    Json(serde_json::json!({ "accounts": items }))
}

async fn set_email_config(
    State(state): State<AppState>,
    Path(instance_slug): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> StatusCode {
    // Accept either { accounts: [...] } or a single account object (legacy)
    let accounts: Vec<crate::config::EmailConfig> =
        if let Some(arr) = body.get("accounts").and_then(|v| v.as_array()) {
            match serde_json::from_value::<Vec<crate::config::EmailConfig>>(
                serde_json::Value::Array(arr.clone()),
            ) {
                Ok(a) => a,
                Err(_) => return StatusCode::BAD_REQUEST,
            }
        } else {
            match serde_json::from_value::<crate::config::EmailConfig>(body) {
                Ok(single) => vec![single],
                Err(_) => return StatusCode::BAD_REQUEST,
            }
        };

    // Reject accounts with empty passwords — they won't work and are likely
    // Google OAuth accounts mistakenly added as SMTP/IMAP
    for acct in &accounts {
        if acct.smtp_password.is_empty() || acct.imap_password.is_empty() {
            return StatusCode::BAD_REQUEST;
        }
    }

    match crate::config::EmailAccounts::save(&accounts, &state.workspace_dir, &instance_slug) {
        Ok(_) => StatusCode::OK,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

async fn delete_email_config(
    State(state): State<AppState>,
    Path(instance_slug): Path<String>,
) -> StatusCode {
    let path = state
        .workspace_dir
        .join("instances")
        .join(&instance_slug)
        .join("email.toml");
    if path.exists() {
        match fs::remove_file(&path) {
            Ok(_) => StatusCode::NO_CONTENT,
            Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    } else {
        StatusCode::NO_CONTENT
    }
}

// ---------------------------------------------------------------------------
// Scheduled messages
// ---------------------------------------------------------------------------

async fn list_scheduled(
    State(state): State<AppState>,
    Path(instance_slug): Path<String>,
) -> Json<Vec<serde_json::Value>> {
    let dir = state
        .workspace_dir
        .join("instances")
        .join(&instance_slug)
        .join("scheduled");
    let mut items = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            if let Ok(raw) = std::fs::read_to_string(&path) {
                if let Ok(task) = serde_json::from_str::<tools::ScheduledTask>(&raw) {
                    items.push(serde_json::json!({
                        "id": task.id,
                        "task": task.task,
                        "deliver_at": task.deliver_at,
                        "created_at": task.created_at,
                    }));
                }
            }
        }
    }
    items.sort_by_key(|v| v["deliver_at"].as_i64().unwrap_or(0));
    Json(items)
}

async fn cancel_scheduled(
    State(state): State<AppState>,
    Path((instance_slug, message_id)): Path<(String, String)>,
) -> StatusCode {
    let file = state
        .workspace_dir
        .join("instances")
        .join(&instance_slug)
        .join("scheduled")
        .join(format!("{message_id}.json"));
    if file.exists() {
        match std::fs::remove_file(&file) {
            Ok(_) => StatusCode::OK,
            Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    } else {
        StatusCode::NOT_FOUND
    }
}

// ---------------------------------------------------------------------------
// Export / Import
// ---------------------------------------------------------------------------

/// GET /api/instances/{slug}/export → tar.gz download of the entire instance directory.
/// Streams the tar output directly so the client receives data immediately.
async fn export_instance(
    Path(instance_slug): Path<String>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let instance_dir = state.workspace_dir.join("instances").join(&instance_slug);
    if !instance_dir.is_dir() {
        return (StatusCode::NOT_FOUND, "instance not found").into_response();
    }

    // Spawn tar and stream stdout directly to the response.
    let child = tokio::process::Command::new("tar")
        .arg("czf")
        .arg("-") // stdout
        .arg("--exclude=node_modules")
        .arg("--exclude=.git")
        .arg("--exclude=target")
        .arg("--exclude=.venv")
        .arg("--exclude=__pycache__")
        .arg("--exclude=.next")
        .arg("--exclude=dist")
        .arg("--exclude=build")
        .arg("-C")
        .arg(state.workspace_dir.join("instances"))
        .arg(&instance_slug)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn();

    match child {
        Ok(mut child) => {
            let stdout = child.stdout.take().unwrap();
            let stream = tokio_util::io::ReaderStream::new(stdout);
            let body = Body::from_stream(stream);

            // Reap the child process in the background to avoid zombies.
            tokio::spawn(async move {
                let _ = child.wait().await;
            });

            let headers = [
                (axum::http::header::CONTENT_TYPE, "application/gzip"),
                (
                    axum::http::header::CONTENT_DISPOSITION,
                    &format!("attachment; filename=\"{instance_slug}.tar.gz\""),
                ),
            ];
            (headers, body).into_response()
        }
        Err(e) => {
            log::error!("[export] failed to spawn tar: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "export failed").into_response()
        }
    }
}

/// POST /api/instances/{slug}/import is disabled until a capability-safe
/// importer exists for the stabilized storage format.
async fn import_instance(Path(_instance_slug): Path<String>) -> impl IntoResponse {
    (
        StatusCode::NOT_IMPLEMENTED,
        "profile import is temporarily unavailable during storage format stabilization",
    )
        .into_response()
}

#[cfg(test)]
mod media_tests {
    use super::*;
    use crate::services::embedding::tests::{MockServer, response};

    #[tokio::test]
    async fn import_endpoint_is_disabled_without_filesystem_or_index_mutation() {
        use axum::{
            body::{Body, to_bytes},
            http::Request,
        };
        use tower::ServiceExt;

        let workspace = tempfile::tempdir().unwrap();
        let store = std::sync::Arc::new(
            crate::services::vector::VectorStore::connect(workspace.path()).await,
        );
        let mut vector = vec![0.; 768];
        vector[0] = 1.;
        store
            .upsert_text_memory("existing", "note.md", vec![("sentinel".into(), vector)])
            .await
            .unwrap();
        let mut state = AppState::new(crate::config::Config::default()).await;
        state.workspace_dir = workspace.path().to_owned();
        state.vector_store = store.clone();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/instances/new/import")
                    .body(Body::from(b"not an archive".as_slice()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_IMPLEMENTED);
        let body = to_bytes(response.into_body(), 4096).await.unwrap();
        assert!(
            String::from_utf8_lossy(&body).contains("storage format stabilization"),
            "{}",
            String::from_utf8_lossy(&body)
        );
        assert!(!workspace.path().join("instances/new").exists());
        assert_eq!(store.list_all("existing", 10).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn media_reindex_endpoint_preserves_committed_index_during_provider_failure() {
        let mock = MockServer::new(vec![
            (200, response(vec![1., 0., 0.])),
            (503, serde_json::json!({"error":"offline"})),
        ])
        .await;
        let ws = tempfile::tempdir().unwrap();
        let dir = ws.path().join("instances/one/memory");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("photo.png"), [0xff]).unwrap();
        crate::services::media_text::write(&dir, "photo.png", "sky").unwrap();
        let mut state = AppState::new(crate::config::Config::default()).await;
        state.workspace_dir = ws.path().to_owned();
        state.vector_store = std::sync::Arc::new(
            crate::services::vector::VectorStore::connect_with_config(ws.path(), &mock.config)
                .await,
        );
        state
            .vector_store
            .backfill_text_memories(ws.path(), "one")
            .await
            .unwrap();
        let _ = reindex_memory(State(state.clone()), Path("one".into()))
            .await
            .unwrap();
        assert_eq!(
            state.vector_store.list_all("one", 10).await.unwrap().len(),
            1
        );
    }
    #[tokio::test]
    async fn delete_endpoint_reconciles_missing_source_and_stale_derived_state() {
        let ws = tempfile::tempdir().unwrap();
        let store =
            std::sync::Arc::new(crate::services::vector::VectorStore::connect(ws.path()).await);
        let mut vector = vec![0.; 768];
        vector[0] = 1.;
        store
            .upsert_text_memory("one", "photo.png", vec![("stale".into(), vector)])
            .await
            .unwrap();
        let mut state = AppState::new(crate::config::Config::default()).await;
        state.workspace_dir = ws.path().to_owned();
        state.vector_store = store.clone();

        assert_eq!(
            delete_memory_file(State(state), Path(("one".into(), "photo.png".into())),).await,
            StatusCode::OK
        );
        assert!(store.list_all("one", 10).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn serving_uses_case_insensitive_media_mime_detection() {
        let ws = tempfile::tempdir().unwrap();
        let memory = ws.path().join("instances/one/memory");
        std::fs::create_dir_all(&memory).unwrap();
        std::fs::write(memory.join("PHOTO.PNG"), b"png").unwrap();
        let mut state = AppState::new(crate::config::Config::default()).await;
        state.workspace_dir = ws.path().to_owned();
        state.vector_store =
            std::sync::Arc::new(crate::services::vector::VectorStore::connect(ws.path()).await);

        let response = serve_memory_file_inner(&state, "one", "PHOTO.PNG")
            .await
            .unwrap();
        assert_eq!(
            response.headers()[axum::http::header::CONTENT_TYPE],
            "image/png"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn http_memory_symlink_never_serves_outside_bytes() {
        use std::os::unix::fs::symlink;

        let ws = tempfile::tempdir().unwrap();
        let memory = ws.path().join("instances/one/memory");
        std::fs::create_dir_all(&memory).unwrap();
        let outside = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(outside.path(), b"outside sentinel").unwrap();
        symlink(outside.path(), memory.join("leak.txt")).unwrap();
        let mut state = AppState::new(crate::config::Config::default()).await;
        state.workspace_dir = ws.path().to_owned();
        state.vector_store =
            std::sync::Arc::new(crate::services::vector::VectorStore::connect(ws.path()).await);

        assert_eq!(
            serve_memory_file_inner(&state, "one", "leak.txt")
                .await
                .unwrap_err(),
            StatusCode::NOT_FOUND
        );
    }

    #[tokio::test]
    async fn media_search_metadata_resolves_root_and_nested_files_through_public_endpoint() {
        use axum::{
            body::{Body, to_bytes},
            http::Request,
        };
        use tower::ServiceExt;
        let mock = MockServer::new(vec![(200, response(vec![1., 0., 0.])); 6]).await;
        let ws = tempfile::tempdir().unwrap();
        let dir = ws.path().join("instances/one/memory");
        std::fs::create_dir_all(dir.join("documents")).unwrap();
        for path in [
            "photo.png",
            "documents/report #1.pdf",
            "clip.mp4",
            "voice.mp3",
        ] {
            std::fs::write(dir.join(path), [0xff, 0x81]).unwrap();
            crate::services::media_text::write(&dir, path, "Orion description").unwrap();
        }
        let mut cfg = crate::config::Config::default();
        cfg.auth_token = "test-token".into();
        cfg.public_url = "https://memory.example".into();
        let mut state = AppState::new(cfg).await;
        state.workspace_dir = ws.path().to_owned();
        state.vector_store = std::sync::Arc::new(
            crate::services::vector::VectorStore::connect_with_config(ws.path(), &mock.config)
                .await,
        );
        state
            .vector_store
            .backfill_text_memories(ws.path(), "one")
            .await
            .unwrap();
        let app = router()
            .merge(public_memory_router())
            .merge(crate::routes::resources::router())
            .with_state(state.clone());
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/instances/one/memory/search?q=Orion")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let results: serde_json::Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 100_000).await.unwrap())
                .unwrap();
        assert_eq!(results.as_array().unwrap().len(), 4);
        for result in results.as_array().unwrap() {
            assert!(
                result["source_type"]
                    .as_str()
                    .unwrap()
                    .starts_with("media_")
            );
            let url = result["media_url"].as_str().unwrap();
            assert!(url.starts_with("/resources/browser/memory/one/"), "{url}");
            let response = app
                .clone()
                .oneshot(Request::builder().uri(url).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK, "{url}");
            assert_eq!(
                to_bytes(response.into_body(), 100).await.unwrap().as_ref(),
                [0xff, 0x81]
            );
        }
        let unauthorized = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/public/memory/one/photo.png")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
        state.config.write().await.public_url.clear();
        let Json(local) = search_memory(
            State(state.clone()),
            Path("one".into()),
            axum::extract::Query(SearchQuery {
                q: "Orion".into(),
                limit: 10,
            }),
        )
        .await
        .unwrap();
        assert!(local.as_array().unwrap().iter().all(|r| {
            r["media_url"]
                .as_str()
                .unwrap()
                .starts_with("/resources/browser/memory/one/")
        }));
        assert_eq!(
            delete_memory_file(
                State(state.clone()),
                Path(("one".into(), "photo.png".into()))
            )
            .await,
            StatusCode::OK
        );
        assert!(!dir.join("photo.png").exists());
        assert!(!dir.join("photo.png.md").exists());
        assert_eq!(
            state.vector_store.list_all("one", 10).await.unwrap().len(),
            3
        );
    }
}
