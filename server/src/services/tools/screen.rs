use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{ToolExecError, openai_schema};
use crate::services::machine_registry::{AgentToolCall, MachineRegistry};
use crate::services::tool::{Tool, ToolDefinition};

// Screen recording path no longer used — frames are streamed via WebSocket
// and stitched on the server from the ring buffer.

// ═══════════════════════════════════════════════════════════════════════════
// Observations — saved screen recording + analysis
// ═══════════════════════════════════════════════════════════════════════════

#[derive(Serialize, Deserialize, Clone)]
pub struct ScreenObservation {
    pub id: String,
    pub upload_id: String,
    pub machine_id: String,
    pub analysis: String,
    pub created_at: String,
}

pub fn save_observation(workspace_dir: &Path, slug: &str, obs: &ScreenObservation) {
    let dir = workspace_dir
        .join("instances")
        .join(slug)
        .join("observations");
    let _ = fs::create_dir_all(&dir);
    let path = dir.join(format!("{}.json", obs.id));
    if let Ok(json) = serde_json::to_string_pretty(obs) {
        let _ = fs::write(path, json);
    }
}

pub fn list_observations(workspace_dir: &Path, slug: &str) -> Vec<ScreenObservation> {
    let dir = workspace_dir
        .join("instances")
        .join(slug)
        .join("observations");
    let entries = match fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return vec![],
    };
    let mut obs: Vec<ScreenObservation> = entries
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("json"))
        .filter_map(|e| {
            let content = fs::read_to_string(e.path()).ok()?;
            serde_json::from_str(&content).ok()
        })
        .collect();
    obs.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    obs
}

fn unix_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_millis()
}

/// Start screen recording on a specific machine.
pub async fn start_recording_on_machine(registry: &MachineRegistry, machine_id: &str, _os: &str) {
    let start_cmd = AgentToolCall {
        request_id: uuid::Uuid::new_v4().to_string(),
        action: "start_recording".into(),
        params: serde_json::json!({}),
    };
    match registry.execute(machine_id, start_cmd).await {
        Ok(_) => log::info!("[screen] started native recording on '{}'", machine_id),
        Err(e) => log::warn!("[screen] failed to start recording: {e}"),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// collect_screen_recording — stop, upload, restart
// ═══════════════════════════════════════════════════════════════════════════

pub struct CollectScreenRecordingTool {
    registry: MachineRegistry,
    workspace_dir: PathBuf,
    instance_slug: String,
}

impl CollectScreenRecordingTool {
    pub fn new(
        registry: MachineRegistry,
        workspace_dir: &std::path::Path,
        instance_slug: &str,
        _public_url: &str,
        _auth_token: &str,
    ) -> Self {
        Self {
            registry,
            workspace_dir: workspace_dir.to_path_buf(),
            instance_slug: instance_slug.to_string(),
        }
    }
}

#[derive(Deserialize, JsonSchema)]
pub struct CollectScreenRecordingArgs {}

impl Tool for CollectScreenRecordingTool {
    const NAME: &'static str = "collect_screen_recording";
    type Error = ToolExecError;
    type Args = CollectScreenRecordingArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "collect_screen_recording".into(),
            description: "Stop the current screen recording on the user's desktop, \
                upload the video to the server, and start a new recording. \
                Returns the upload_id and URL of the recorded video. \
                Use watch_video on the returned URL to analyze what the user was doing."
                .into(),
            parameters: openai_schema::<CollectScreenRecordingArgs>(),
        }
    }

    async fn call(&self, _args: Self::Args) -> Result<Self::Output, Self::Error> {
        let machines = self.registry.list().await;
        let machine = machines
            .iter()
            .find(|m| m.screen_recording_allowed)
            .ok_or_else(|| ToolExecError("no desktop with screen recording enabled".into()))?;
        let machine_id = machine.machine_id.clone();

        // 1. Take all buffered frames from the server's ring buffer
        let frames = self.registry.take_frames(&machine_id).await;
        if frames.is_empty() {
            return Err(ToolExecError(
                "no frames captured yet — recording may not have started".into(),
            ));
        }

        log::info!(
            "[screen] stitching {} frames from '{}'",
            frames.len(),
            machine_id
        );

        // 2. Write frames to temp dir
        let tmp_dir = format!("/tmp/bolly_frames_{}", std::process::id());
        let _ = std::fs::create_dir_all(&tmp_dir);

        for (i, frame) in frames.iter().enumerate() {
            let path = format!("{tmp_dir}/frame_{i:05}.jpg");
            if let Err(e) = std::fs::write(&path, &frame.jpeg) {
                log::warn!("[screen] failed to write frame {i}: {e}");
            }
        }

        // 3. Stitch into MP4 with ffmpeg
        let output_path = format!("{tmp_dir}/recording.mp4");
        let ffmpeg = tokio::process::Command::new("ffmpeg")
            .args([
                "-y",
                "-framerate",
                "1",
                "-i",
                &format!("{tmp_dir}/frame_%05d.jpg"),
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-crf",
                "28",
                "-pix_fmt",
                "yuv420p",
                "-an",
                &output_path,
            ])
            .output()
            .await
            .map_err(|e| ToolExecError(format!("ffmpeg failed: {e}")))?;

        if !ffmpeg.status.success() {
            let stderr = String::from_utf8_lossy(&ffmpeg.stderr);
            let _ = std::fs::remove_dir_all(&tmp_dir);
            return Err(ToolExecError(format!(
                "ffmpeg stitch failed: {}",
                &stderr[..stderr.len().min(300)]
            )));
        }

        // 4. Save as upload
        let video_bytes = std::fs::read(&output_path)
            .map_err(|e| ToolExecError(format!("failed to read stitched video: {e}")))?;
        let upload_meta = crate::services::uploads::save_upload(
            &self.workspace_dir,
            &self.instance_slug,
            "screen_recording.mp4",
            &video_bytes,
        )
        .map_err(|e| ToolExecError(format!("failed to save upload: {e}")))?;

        // 5. Clean up temp dir
        let _ = std::fs::remove_dir_all(&tmp_dir);

        let upload_id = upload_meta.id.clone();
        let file_path = self
            .workspace_dir
            .join("instances")
            .join(&self.instance_slug)
            .join("uploads")
            .join(&upload_meta.stored_name);

        log::info!(
            "[screen] stitched {} frames → {} ({} bytes)",
            frames.len(),
            upload_id,
            video_bytes.len()
        );

        Ok(format!(
            "Screen recording collected ({} frames, {} seconds).\n\
             upload_id: {upload_id}\n\
             machine_id: {machine_id}\n\n\
             Now use watch_video to analyze what the user was doing.\n\
             File path: {}\n\n\
             After watching, call save_screen_observation with the upload_id, machine_id, and your analysis.",
            frames.len(),
            frames.len(),
            file_path.display()
        ))
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// save_screen_observation — save analysis after watch_video
// ═══════════════════════════════════════════════════════════════════════════

pub struct SaveScreenObservationTool {
    workspace_dir: PathBuf,
    instance_slug: String,
}

impl SaveScreenObservationTool {
    pub fn new(workspace_dir: &std::path::Path, instance_slug: &str) -> Self {
        Self {
            workspace_dir: workspace_dir.to_path_buf(),
            instance_slug: instance_slug.to_string(),
        }
    }
}

#[derive(Deserialize, JsonSchema)]
pub struct SaveScreenObservationArgs {
    /// The upload_id from collect_screen_recording.
    pub upload_id: String,
    /// The machine_id from collect_screen_recording.
    pub machine_id: String,
    /// Your analysis of what the user was doing on screen.
    pub analysis: String,
}

impl Tool for SaveScreenObservationTool {
    const NAME: &'static str = "save_screen_observation";
    type Error = ToolExecError;
    type Args = SaveScreenObservationArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "save_screen_observation".into(),
            description: "Save a screen observation with your analysis. Call this after \
                watch_video to record what you observed on the user's screen."
                .into(),
            parameters: openai_schema::<SaveScreenObservationArgs>(),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let obs = ScreenObservation {
            id: format!("obs_{}", unix_millis()),
            upload_id: args.upload_id,
            machine_id: args.machine_id,
            analysis: args.analysis,
            created_at: unix_millis().to_string(),
        };
        save_observation(&self.workspace_dir, &self.instance_slug, &obs);
        log::info!("[screen] saved observation {}", obs.id);
        Ok(format!("observation saved: {}", obs.id))
    }
}
