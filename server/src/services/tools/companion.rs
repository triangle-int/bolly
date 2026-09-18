use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::services::tool::{Tool, ToolDefinition};
use schemars::JsonSchema;
use serde::Deserialize;

use super::{ToolExecError, openai_schema};
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
