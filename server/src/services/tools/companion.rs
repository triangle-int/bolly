use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::services::tool::{Tool, ToolDefinition};
use schemars::JsonSchema;
use serde::Deserialize;
use tokio::sync::broadcast;

use super::{ToolExecError, openai_schema};
use crate::domain::events::ServerEvent;
use crate::domain::mood::MoodState;

// ---------------------------------------------------------------------------
// Mood state I/O
// ---------------------------------------------------------------------------

pub fn load_mood_state(instance_dir: &Path) -> MoodState {
    let path = instance_dir.join("mood.json");
    match fs::read_to_string(&path) {
        Ok(raw) => serde_json::from_str(&raw).unwrap_or_default(),
        Err(_) => MoodState::default(),
    }
}

pub fn save_mood_state(instance_dir: &Path, state: &MoodState) {
    let path = instance_dir.join("mood.json");
    if let Ok(json) = serde_json::to_string_pretty(state) {
        let _ = fs::write(&path, json);
    }
}

/// Allowed mood values that the client can visualize.
pub const ALLOWED_MOODS: &[&str] = &[
    "calm",
    "curious",
    "excited",
    "warm",
    "happy",
    "joyful",
    "reflective",
    "contemplative",
    "melancholy",
    "melancholic",
    "sad",
    "worried",
    "anxious",
    "playful",
    "mischievous",
    "focused",
    "tired",
    "peaceful",
    "loving",
    "tender",
    "creative",
    "energetic",
    "thoughtful",
    "grateful",
    "nostalgic",
];

// Mood tools removed — mood is now managed by background sentiment
// extraction (Haiku) and heartbeat triage, not by agent tool calls.

// ---------------------------------------------------------------------------
// play_music — control client-side music/ambient playback
// ---------------------------------------------------------------------------

pub struct PlayMusicTool {
    workspace_dir: PathBuf,
    instance_slug: String,
    events: broadcast::Sender<ServerEvent>,
}

impl PlayMusicTool {
    pub fn new(
        workspace_dir: &Path,
        instance_slug: &str,
        events: broadcast::Sender<ServerEvent>,
    ) -> Self {
        Self {
            workspace_dir: workspace_dir.to_path_buf(),
            instance_slug: instance_slug.to_string(),
            events,
        }
    }
}

#[derive(Deserialize, JsonSchema)]
pub struct PlayMusicArgs {
    /// Action: "play", "pause", "set_volume".
    pub action: String,
    /// Track: "ambient", "intro", "loop", or a URL (YouTube, direct audio link).
    /// YouTube URLs are automatically downloaded. Required for "play".
    pub track: Option<String>,
    /// Volume 0.0–1.0. Used with "play" (sets initial volume) and "set_volume".
    pub volume: Option<f64>,
}

impl Tool for PlayMusicTool {
    const NAME: &'static str = "play_music";
    type Error = ToolExecError;
    type Args = PlayMusicArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "play_music".into(),
            description: "Control music/ambient playback in the user's browser. \
                Actions: \"play\" (start a track), \"pause\" (stop all music), \
                \"set_volume\" (change volume of a track). \
                Built-in tracks: \"ambient\", \"intro\", \"loop\". \
                You can also pass a YouTube URL or direct audio URL to play custom audio."
                .into(),
            parameters: openai_schema::<PlayMusicArgs>(),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let action = args.action.to_lowercase();
        match action.as_str() {
            "play" | "pause" | "set_volume" => {}
            _ => {
                return Err(ToolExecError(format!(
                    "unknown action '{action}', use play/pause/set_volume"
                )));
            }
        }
        if action == "play" && args.track.is_none() {
            return Err(ToolExecError("track is required for play action".into()));
        }

        // Resolve track: download external URLs and save to uploads for same-origin playback
        let resolved_track = if action == "play" {
            if let Some(ref track) = args.track {
                let builtin = matches!(track.as_str(), "ambient" | "intro" | "loop");
                if builtin {
                    Some(track.clone())
                } else {
                    // External URL — download and proxy through uploads
                    let local_url = self.download_and_upload(track).await?;
                    Some(local_url)
                }
            } else {
                None
            }
        } else {
            args.track.clone()
        };

        let _ = self.events.send(ServerEvent::MusicControl {
            instance_slug: self.instance_slug.clone(),
            action: action.clone(),
            track: resolved_track.clone(),
            volume: args.volume,
        });
        let msg = match action.as_str() {
            "play" => format!("playing {}", args.track.as_deref().unwrap_or("?")),
            "pause" => "music paused".into(),
            "set_volume" => format!("volume set to {:.0}%", args.volume.unwrap_or(0.5) * 100.0),
            _ => "ok".into(),
        };
        Ok(msg)
    }
}

impl PlayMusicTool {
    /// Download audio from a URL (YouTube or direct) and save to instance uploads.
    /// Returns a local `/api/instances/{slug}/uploads/{id}/file` URL.
    async fn download_and_upload(&self, url: &str) -> Result<String, ToolExecError> {
        use super::media::{MediaType, download_youtube, is_youtube_url};

        let local_path = if is_youtube_url(url) {
            log::info!("[play_music] downloading YouTube audio: {url}");
            download_youtube(url, MediaType::Audio).await?
        } else {
            // Direct URL — download with reqwest
            log::info!("[play_music] downloading audio: {url}");
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(60))
                .build()
                .map_err(|e| ToolExecError(format!("HTTP client error: {e}")))?;
            let response = client
                .get(url)
                .send()
                .await
                .map_err(|e| ToolExecError(format!("download failed: {e}")))?;
            if !response.status().is_success() {
                return Err(ToolExecError(format!(
                    "download failed: HTTP {}",
                    response.status()
                )));
            }
            let bytes = response
                .bytes()
                .await
                .map_err(|e| ToolExecError(format!("failed to read audio: {e}")))?;

            let ext = url
                .rsplit('.')
                .next()
                .filter(|e| ["mp3", "m4a", "ogg", "wav", "flac", "aac", "opus"].contains(e))
                .unwrap_or("mp3");
            let tmp = format!("/tmp/play_music_{}.{ext}", std::process::id());
            std::fs::write(&tmp, &bytes)
                .map_err(|e| ToolExecError(format!("failed to write temp file: {e}")))?;
            tmp
        };

        // Save to uploads
        let bytes = std::fs::read(&local_path)
            .map_err(|e| ToolExecError(format!("failed to read downloaded file: {e}")))?;
        let _ = std::fs::remove_file(&local_path);

        let ext = std::path::Path::new(&local_path)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("mp3");
        let filename = format!("music.{ext}");

        let meta = crate::services::uploads::save_upload(
            &self.workspace_dir,
            &self.instance_slug,
            &filename,
            &bytes,
        )
        .map_err(|e| ToolExecError(format!("failed to save upload: {e}")))?;

        let local_url = format!(
            "/api/instances/{}/uploads/{}/file",
            self.instance_slug, meta.id
        );
        log::info!(
            "[play_music] audio saved → {local_url} ({:.1} MB)",
            bytes.len() as f64 / 1024.0 / 1024.0
        );
        Ok(local_url)
    }
}

// ---------------------------------------------------------------------------
// set_voice — temporarily change the ElevenLabs voice ID (in-memory only)
// ---------------------------------------------------------------------------

use std::collections::HashMap;
use std::sync::Mutex;

/// Temporary voice overrides — cleared on server restart or context clear.
static VOICE_OVERRIDES: std::sync::OnceLock<Mutex<HashMap<String, String>>> =
    std::sync::OnceLock::new();

fn voice_overrides() -> &'static Mutex<HashMap<String, String>> {
    VOICE_OVERRIDES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Get the temporary voice override for an instance, if set.
pub fn get_voice_override(instance_slug: &str) -> Option<String> {
    let map = voice_overrides().lock().unwrap_or_else(|e| e.into_inner());
    map.get(instance_slug).cloned()
}

/// Clear the temporary voice override for an instance (e.g. on context clear).
pub fn clear_voice_override(instance_slug: &str) {
    let mut map = voice_overrides().lock().unwrap_or_else(|e| e.into_inner());
    map.remove(instance_slug);
}

pub struct SetVoiceTool {
    instance_slug: String,
}

impl SetVoiceTool {
    pub fn new(_workspace_dir: &Path, instance_slug: &str) -> Self {
        Self {
            instance_slug: instance_slug.to_string(),
        }
    }
}

#[derive(Deserialize, JsonSchema)]
pub struct SetVoiceArgs {
    /// ElevenLabs voice ID to use. Pass empty string to reset to default.
    pub voice_id: String,
}

impl Tool for SetVoiceTool {
    const NAME: &'static str = "set_voice";
    type Error = ToolExecError;
    type Args = SetVoiceArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "set_voice".into(),
            description: "Temporarily change the ElevenLabs voice for text-to-speech. \
                All subsequent messages will use this voice until context is cleared \
                or server restarts. Pass empty string to reset to instance default."
                .into(),
            parameters: openai_schema::<SetVoiceArgs>(),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let vid = args.voice_id.trim().to_string();
        let mut map = voice_overrides().lock().unwrap_or_else(|e| e.into_inner());

        if vid.is_empty() {
            map.remove(&self.instance_slug);
            Ok("voice reset to default".into())
        } else {
            map.insert(self.instance_slug.clone(), vid.clone());
            Ok(format!("voice temporarily set to {vid}"))
        }
    }
}

// ---------------------------------------------------------------------------
// edit_soul
// ---------------------------------------------------------------------------

pub struct EditSoulTool {
    soul_path: PathBuf,
}

impl EditSoulTool {
    pub fn new(workspace_dir: &Path, instance_slug: &str) -> Self {
        Self {
            soul_path: workspace_dir
                .join("instances")
                .join(instance_slug)
                .join("soul.md"),
        }
    }
}

/// Arguments for edit_soul tool.
#[derive(Deserialize, JsonSchema)]
pub struct EditSoulArgs {
    /// The full new content of soul.md in markdown format.
    pub content: String,
}

impl Tool for EditSoulTool {
    const NAME: &'static str = "edit_soul";
    type Error = ToolExecError;
    type Args = EditSoulArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "edit_soul".into(),
            description:
                "Rewrite your soul.md (personality/voice definition). Full markdown content.".into(),
            parameters: openai_schema::<EditSoulArgs>(),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        if let Some(parent) = self.soul_path.parent() {
            fs::create_dir_all(parent).map_err(|e| ToolExecError(e.to_string()))?;
        }
        fs::write(&self.soul_path, &args.content).map_err(|e| ToolExecError(e.to_string()))?;
        Ok(
            "soul.md updated. your personality will reflect these changes on the next message."
                .into(),
        )
    }
}
