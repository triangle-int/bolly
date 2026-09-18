use axum::{
    Json, Router,
    body::Body,
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode},
    routing::post,
};
use serde::Deserialize;
use std::path::Path;

use crate::{app::state::AppState, config::InstanceConfig};

const DEFAULT_VOICE_ID: &str = "TWutjvRaJqAX89preB4e";

pub fn router() -> Router<AppState> {
    Router::new().route("/api/tts", post(synthesize))
}

#[derive(Debug, Deserialize)]
struct TtsRequest {
    text: String,
    #[serde(default)]
    instance_slug: String,
}

/// Resolve ElevenLabs voice ID: temporary override > instance config > env var > default.
pub fn resolve_voice_id(workspace_dir: &Path, instance_slug: &str) -> String {
    // Check temporary in-memory override first (set by set_voice tool)
    if let Some(override_id) = crate::services::tools::get_voice_override(instance_slug) {
        if !override_id.is_empty() {
            return override_id;
        }
    }
    if !instance_slug.is_empty() {
        let inst = InstanceConfig::load(workspace_dir, instance_slug);
        if !inst.elevenlabs_voice_id.is_empty() {
            return inst.elevenlabs_voice_id;
        }
    }
    std::env::var("ELEVENLABS_VOICE_ID").unwrap_or_else(|_| DEFAULT_VOICE_ID.into())
}

// ── Text preprocessing for ElevenLabs v3 ──────────────────────────────────

/// Strip markdown syntax and prepend a mood-based voice tag.
/// ElevenLabs v3 reads `**word**` literally; tags like `[softly]` set tone.
pub fn prepare_text_for_voice(text: &str, mood: &str) -> String {
    // 1. Strip markdown
    let mut s = text.to_string();

    // Fenced code blocks → just the content
    while let Some(start) = s.find("```") {
        if let Some(end) = s[start + 3..].find("```") {
            let inner = &s[start + 3..start + 3 + end];
            // Drop the language identifier on the first line
            let clean = inner
                .split_once('\n')
                .map(|(_, rest)| rest)
                .unwrap_or(inner);
            s = format!("{}{}{}", &s[..start], clean.trim(), &s[start + 6 + end..]);
        } else {
            break;
        }
    }

    // Bold/italic: **text**, *text*, __text__, _text_
    s = s.replace("***", "").replace("**", "").replace("__", "");
    // Single * and _ only when surrounding a word (avoid breaking contractions)
    let mut out = String::with_capacity(s.len());
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if (c == '*' || c == '_') && i + 1 < chars.len() && chars[i + 1].is_alphanumeric() {
            // Opening marker — skip it
            i += 1;
            continue;
        }
        if (c == '*' || c == '_') && i > 0 && chars[i - 1].is_alphanumeric() {
            // Closing marker — skip it
            i += 1;
            continue;
        }
        out.push(c);
        i += 1;
    }
    s = out;

    // Inline code: `text`
    s = s.replace('`', "");
    // Headings: # ## ### etc
    s = regex::Regex::new(r"(?m)^#{1,6}\s*")
        .unwrap()
        .replace_all(&s, "")
        .into_owned();
    // Blockquotes: > text
    s = regex::Regex::new(r"(?m)^>\s*")
        .unwrap()
        .replace_all(&s, "")
        .into_owned();
    // List markers: - item, * item, 1. item
    s = regex::Regex::new(r"(?m)^[\s]*[-*]\s+")
        .unwrap()
        .replace_all(&s, "")
        .into_owned();
    s = regex::Regex::new(r"(?m)^[\s]*\d+\.\s+")
        .unwrap()
        .replace_all(&s, "")
        .into_owned();
    // Links: [text](url) → text
    s = regex::Regex::new(r"\[([^\]]+)\]\([^)]+\)")
        .unwrap()
        .replace_all(&s, "$1")
        .into_owned();
    // Horizontal rules
    s = regex::Regex::new(r"(?m)^---+\s*$")
        .unwrap()
        .replace_all(&s, "")
        .into_owned();

    // Collapse multiple newlines/spaces
    s = regex::Regex::new(r"\n{3,}")
        .unwrap()
        .replace_all(&s, "\n\n")
        .into_owned();
    let s = s.trim().to_string();

    // 2. Map mood → ElevenLabs v3 voice tag
    let tag = match mood {
        "excited" | "energetic" | "joyful" => "[excitedly]",
        "happy" | "playful" | "mischievous" => "[cheerfully]",
        "calm" | "peaceful" => "[softly]",
        "warm" | "loving" | "tender" => "[warmly]",
        "reflective" | "contemplative" | "focused" => "[thoughtfully]",
        "curious" => "[curiously]",
        "melancholy" | "sad" | "tired" => "[sadly]",
        "worried" | "anxious" => "[nervously]",
        "creative" => "[expressively]",
        _ => "[conversationally]",
    };

    format!("{tag} {s}")
}

/// Synthesize speech and return raw MP3 bytes.
pub async fn synthesize_bytes(
    http: &reqwest::Client,
    api_key: &str,
    voice_id: &str,
    text: &str,
    mood: &str,
) -> Result<Vec<u8>, String> {
    let prepared = prepare_text_for_voice(text, mood);

    let url = format!(
        "https://api.elevenlabs.io/v1/text-to-speech/{}/stream?output_format=mp3_44100_128",
        voice_id
    );

    let body = serde_json::json!({
        "text": prepared,
        "model_id": "eleven_v3",
    });

    let response = http
        .post(&url)
        .header("xi-api-key", api_key)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("ElevenLabs request failed: {e}"))?;

    if !response.status().is_success() {
        let status = response.status().as_u16();
        let err_text = response.text().await.unwrap_or_default();
        return Err(format!("ElevenLabs API error ({status}): {err_text}"));
    }

    response
        .bytes()
        .await
        .map(|b| b.to_vec())
        .map_err(|e| format!("failed to read audio bytes: {e}"))
}

async fn synthesize(
    State(state): State<AppState>,
    Json(request): Json<TtsRequest>,
) -> Result<(HeaderMap, Body), (StatusCode, String)> {
    let api_key = {
        let cfg = state.config.read().await;
        cfg.llm.tokens.elevenlabs.clone()
    };

    if api_key.is_empty() {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "ElevenLabs API key not configured".into(),
        ));
    }
    if request.text.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "text is required".into()));
    }

    let voice_id = resolve_voice_id(&state.workspace_dir, &request.instance_slug);

    // HTTP endpoint doesn't have mood context — use neutral
    let bytes = synthesize_bytes(
        &state.http_client,
        &api_key,
        &voice_id,
        &request.text,
        "calm",
    )
    .await
    .map_err(|e| (StatusCode::BAD_GATEWAY, e))?;

    let mut headers = HeaderMap::new();
    headers.insert("content-type", HeaderValue::from_static("audio/mpeg"));
    headers.insert("cache-control", HeaderValue::from_static("no-cache"));

    Ok((headers, Body::from(bytes)))
}
