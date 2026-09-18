use axum::response::IntoResponse;
use axum::{
    Json, Router,
    body::Body,
    extract::{Multipart, Path, State},
    http::StatusCode,
    routing::{delete, get, post, put},
};
use serde::{Deserialize, Serialize};
use std::fs;

use crate::{
    app::state::AppState,
    domain::instance::InstanceSummary,
    domain::memory::MemoryEntry,
    services::{chat, memory, tools, workspace},
};

/// Public memory file route (no auth middleware) — uses ?token= query param.
/// Used by LLM providers (Anthropic) to fetch memory images via URL.
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

async fn list_instances(State(state): State<AppState>) -> Json<Vec<InstanceSummary>> {
    let instances =
        workspace::read_instances(&state.workspace_dir.join("instances")).unwrap_or_default();
    Json(instances)
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
    let stats = tokio::spawn(async move {
        chat::compute_context_stats_async(wd, slug, "default".to_string()).await
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
    let stats = tokio::spawn(async move { chat::compute_context_stats_async(wd, slug, cid).await })
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
    Json(memory::scan_library(&state.workspace_dir, &instance_slug))
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
    let google_ai_key = state.config.read().await.llm.tokens.google_ai.clone();

    let query_vec = crate::services::embedding::embed_text(
        &google_ai_key,
        &params.q,
        crate::services::embedding::TaskType::RetrievalQuery,
    )
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let results = state
        .vector_store
        .search(&instance_slug, query_vec, params.limit)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let memory_dir = state
        .workspace_dir
        .join("instances")
        .join(&instance_slug)
        .join("memory");

    let cfg = state.config.read().await;
    let auth_token = cfg.auth_token.clone();
    let public_url = cfg.public_url.clone();
    drop(cfg);

    let json: Vec<serde_json::Value> = results
        .into_iter()
        .map(|r| {
            let is_media = r.source_type.starts_with("media_");

            let text = if r.content_preview.is_empty() && !is_media {
                std::fs::read_to_string(memory_dir.join(&r.path)).unwrap_or_default()
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
                    let url = if upload_id.contains('/') {
                        if public_url.is_empty() {
                            format!("/api/instances/{instance_slug}/memory/{upload_id}")
                        } else {
                            crate::services::tools::public_memory_url(
                                &public_url,
                                &instance_slug,
                                upload_id,
                                &auth_token,
                            )
                        }
                    } else {
                        if public_url.is_empty() {
                            format!("/api/instances/{instance_slug}/uploads/{upload_id}/file")
                        } else {
                            crate::services::tools::public_file_url(
                                &public_url,
                                &instance_slug,
                                upload_id,
                                &auth_token,
                            )
                        }
                    };
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
    Json(memory::load_graph(&state.workspace_dir, &instance_slug))
}

async fn reindex_memory(
    State(state): State<AppState>,
    Path(instance_slug): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let google_ai_key = state.config.read().await.llm.tokens.google_ai.clone();

    // Reset collection
    state
        .vector_store
        .reset_collection(&instance_slug)
        .await
        .map_err(|e| {
            log::warn!("[reindex] reset failed for {instance_slug}: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    // Backfill in background
    let vs = state.vector_store.clone();
    let ws = state.workspace_dir.clone();
    let slug = instance_slug.clone();
    tokio::spawn(async move {
        match vs.backfill_text_memories(&ws, &slug, &google_ai_key).await {
            Ok(count) => log::info!("[reindex] {slug}: indexed {count} chunks"),
            Err(e) => log::warn!("[reindex] {slug}: failed: {e}"),
        }
    });

    Ok(Json(serde_json::json!({ "status": "reindexing" })))
}

#[derive(Deserialize)]
struct MemoryTokenQuery {
    token: Option<String>,
}

async fn serve_memory_file_public(
    State(state): State<AppState>,
    Path((instance_slug, file_path)): Path<(String, String)>,
    axum::extract::Query(query): axum::extract::Query<MemoryTokenQuery>,
) -> Result<axum::response::Response<axum::body::Body>, StatusCode> {
    let expected = state.config.read().await.auth_token.clone();
    if expected.is_empty() || query.token.as_deref() != Some(&expected) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    serve_memory_file_inner(&state, &instance_slug, &file_path).await
}

async fn read_memory_file(
    State(state): State<AppState>,
    Path((instance_slug, file_path)): Path<(String, String)>,
) -> Result<axum::response::Response<axum::body::Body>, StatusCode> {
    serve_memory_file_inner(&state, &instance_slug, &file_path).await
}

async fn serve_memory_file_inner(
    state: &AppState,
    instance_slug: &str,
    file_path: &str,
) -> Result<axum::response::Response<axum::body::Body>, StatusCode> {
    if file_path.contains("..") || file_path.starts_with('/') {
        return Err(StatusCode::BAD_REQUEST);
    }
    let full_path = state
        .workspace_dir
        .join("instances")
        .join(instance_slug)
        .join("memory")
        .join(file_path);

    let bytes = tokio::fs::read(&full_path)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;

    let content_type = match full_path.extension().and_then(|e| e.to_str()) {
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("png") => "image/png",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        Some("mp4") => "video/mp4",
        Some("mp3") => "audio/mpeg",
        Some("wav") => "audio/wav",
        Some("pdf") => "application/pdf",
        Some("md" | "txt") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    };

    let is_media = content_type.starts_with("image/")
        || content_type.starts_with("video/")
        || content_type.starts_with("audio/");
    let filename = full_path
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
        .header(axum::http::header::CACHE_CONTROL, "public, max-age=86400")
        .body(axum::body::Body::from(bytes))
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

async fn delete_memory_file(
    State(state): State<AppState>,
    Path((instance_slug, file_path)): Path<(String, String)>,
) -> StatusCode {
    if file_path.contains("..") || file_path.starts_with('/') {
        return StatusCode::BAD_REQUEST;
    }
    let full_path = state
        .workspace_dir
        .join("instances")
        .join(&instance_slug)
        .join("memory")
        .join(&file_path);
    if !full_path.exists() {
        return StatusCode::NOT_FOUND;
    }
    if std::fs::remove_file(&full_path).is_err() {
        return StatusCode::INTERNAL_SERVER_ERROR;
    }
    // Clean up empty parent dirs
    if let Some(parent) = full_path.parent() {
        let memory_dir = state
            .workspace_dir
            .join("instances")
            .join(&instance_slug)
            .join("memory");
        let _ = memory::cleanup_empty_dirs(parent, &memory_dir);
    }
    // Remove from vector store
    let vs = state.vector_store.clone();
    let slug = instance_slug.clone();
    let path = file_path.clone();
    tokio::spawn(async move {
        if let Err(e) = vs.delete_by_path(&slug, &path).await {
            log::warn!("[delete_memory] vector delete failed: {e}");
        }
    });
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

/// POST /api/instances/{slug}/import — upload a tar.gz to replace instance data.
/// Existing files are overwritten; files not in the archive are kept.
async fn import_instance(
    Path(instance_slug): Path<String>,
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    let instance_dir = state.workspace_dir.join("instances").join(&instance_slug);
    if !instance_dir.is_dir() {
        fs::create_dir_all(&instance_dir).ok();
    }

    // Read the uploaded file
    let field = match multipart.next_field().await {
        Ok(Some(f)) => f,
        _ => return (StatusCode::BAD_REQUEST, "no file uploaded").into_response(),
    };

    let data = match field.bytes().await {
        Ok(b) => b,
        Err(e) => {
            log::error!("[import] failed to read upload: {e}");
            return (StatusCode::BAD_REQUEST, "failed to read upload").into_response();
        }
    };

    // Extract archive into the instance directory
    // Use --strip-components=1 to handle archives that contain the slug as root dir
    // Auto-detect format: try gzip first, fall back to plain tar
    let is_gzip = data.len() >= 2 && data[0] == 0x1f && data[1] == 0x8b;
    let mut child = match tokio::process::Command::new("tar")
        .arg(if is_gzip { "xzf" } else { "xf" })
        .arg("-") // stdin
        .arg("--strip-components=1")
        .arg("-C")
        .arg(&instance_dir)
        .stdin(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            log::error!("[import] failed to spawn tar: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "import failed").into_response();
        }
    };

    // Write data to stdin
    if let Some(mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt;
        if let Err(e) = stdin.write_all(&data).await {
            log::error!("[import] failed to write to tar stdin: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "import failed").into_response();
        }
        drop(stdin);
    }

    match child.wait().await {
        Ok(status) if status.success() => {
            log::info!(
                "[import] imported {} bytes into {instance_slug}",
                data.len()
            );
            // Rebuild memory catalog after import
            memory::rebuild_catalog_snapshot(&state.workspace_dir, &instance_slug);
            memory::invalidate_frozen_catalog(&instance_slug);
            Json(serde_json::json!({ "ok": true })).into_response()
        }
        Ok(_) => (
            StatusCode::BAD_REQUEST,
            "invalid archive — make sure it's a .tar or .tar.gz file",
        )
            .into_response(),
        Err(e) => {
            log::error!("[import] tar failed: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "import failed").into_response()
        }
    }
}
