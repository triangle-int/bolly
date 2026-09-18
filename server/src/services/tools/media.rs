use std::path::{Path, PathBuf};

use crate::services::tool::{Tool, ToolDefinition};
use base64::Engine;
use schemars::JsonSchema;
use serde::Deserialize;

use super::{ToolExecError, openai_schema};

const GEMINI_MODEL: &str = "gemini-2.5-flash";
const MAX_MEDIA_SIZE: u64 = 45 * 1024 * 1024; // 45 MB

// ---------------------------------------------------------------------------
// Shared media context (upload, compress, analyze)
// ---------------------------------------------------------------------------

pub struct MediaContext {
    google_ai_key: String,
    workspace_dir: PathBuf,
    instance_slug: String,
    public_url: String,
    auth_token: String,
}

impl MediaContext {
    pub fn new(
        google_ai_key: &str,
        workspace_dir: &Path,
        instance_slug: &str,
        public_url: &str,
        auth_token: &str,
    ) -> Self {
        Self {
            google_ai_key: google_ai_key.to_string(),
            workspace_dir: workspace_dir.to_path_buf(),
            instance_slug: instance_slug.to_string(),
            public_url: public_url.to_string(),
            auth_token: auth_token.to_string(),
        }
    }

    fn save_and_get_url(&self, name: &str, bytes: &[u8]) -> Result<String, ToolExecError> {
        let meta = crate::services::uploads::save_upload(
            &self.workspace_dir,
            &self.instance_slug,
            name,
            bytes,
        )
        .map_err(|e| ToolExecError(format!("failed to save upload: {e}")))?;

        if self.public_url.is_empty() {
            // No public URL — return local file path for direct Gemini upload
            let path = self
                .workspace_dir
                .join("instances")
                .join(&self.instance_slug)
                .join("uploads")
                .join(&meta.stored_name);
            return Ok(path.display().to_string());
        }

        Ok(super::public_file_url(
            &self.public_url,
            &self.instance_slug,
            &meta.id,
            &self.auth_token,
        ))
    }

    /// Process input → reference for Gemini.
    /// Video: returns a public URL (uploaded).
    /// Audio: returns a local file path (for base64 encoding).
    async fn resolve_media_ref(
        &self,
        input: &str,
        media_type: MediaType,
    ) -> Result<String, ToolExecError> {
        let local_path = Path::new(input);
        let is_local = local_path.exists() && local_path.is_file();

        if is_local {
            let processed = maybe_compress(input, media_type).await?;
            match media_type {
                MediaType::Audio => Ok(processed), // keep local for base64
                MediaType::Video => {
                    let bytes = std::fs::read(&processed)
                        .map_err(|e| ToolExecError(format!("failed to read file: {e}")))?;
                    if processed != input {
                        let _ = std::fs::remove_file(&processed);
                    }
                    let name = local_path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or(media_type.default_filename());
                    self.save_and_get_url(name, &bytes)
                }
            }
        } else if is_youtube_url(input) {
            log::info!("[media] downloading from YouTube...");
            let downloaded = download_youtube(input, media_type).await?;
            let processed = maybe_compress(&downloaded, media_type).await?;
            if processed != downloaded {
                let _ = std::fs::remove_file(&downloaded);
            }

            match media_type {
                MediaType::Audio => {
                    log::info!("[media] YouTube audio ready: {processed}");
                    Ok(processed) // keep local for base64
                }
                MediaType::Video => {
                    let bytes = std::fs::read(&processed)
                        .map_err(|e| ToolExecError(format!("failed to read file: {e}")))?;
                    let _ = std::fs::remove_file(&processed);
                    let name = format!("youtube_{}", media_type.default_filename());
                    let url = self.save_and_get_url(&name, &bytes)?;
                    log::info!(
                        "[media] YouTube video → upload ({:.1} MB)",
                        bytes.len() as f64 / 1024.0 / 1024.0
                    );
                    Ok(url)
                }
            }
        } else {
            Ok(input.to_string())
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum MediaType {
    Video,
    Audio,
}

impl MediaType {
    fn default_filename(self) -> &'static str {
        match self {
            MediaType::Video => "video.mp4",
            MediaType::Audio => "audio.mp3",
        }
    }
    fn yt_dlp_format(self) -> &'static str {
        match self {
            MediaType::Video => "best[ext=mp4][filesize<100M]/best[ext=mp4]/best",
            MediaType::Audio => "bestaudio[ext=m4a]/bestaudio/best",
        }
    }
    fn yt_dlp_ext(self) -> &'static str {
        match self {
            MediaType::Video => "mp4",
            MediaType::Audio => "m4a",
        }
    }
}

// ---------------------------------------------------------------------------
// watch_video tool
// ---------------------------------------------------------------------------

pub struct WatchVideoTool {
    ctx: MediaContext,
}

impl WatchVideoTool {
    pub fn new(
        google_ai_key: &str,
        workspace_dir: &Path,
        instance_slug: &str,
        public_url: &str,
        auth_token: &str,
    ) -> Self {
        Self {
            ctx: MediaContext::new(
                google_ai_key,
                workspace_dir,
                instance_slug,
                public_url,
                auth_token,
            ),
        }
    }
}

#[derive(Deserialize, JsonSchema)]
pub struct WatchVideoArgs {
    /// URL or local file path of the video. Supports YouTube, direct URLs, local paths.
    pub url: String,
    /// What to focus on. Defaults to a general summary.
    pub prompt: Option<String>,
}

impl Tool for WatchVideoTool {
    const NAME: &'static str = "watch_video";
    type Error = ToolExecError;
    type Args = WatchVideoArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "watch_video".into(),
            description: "Analyze a video via Gemini. YouTube URLs, direct links, or local files. Auto-compresses large files.".into(),
            parameters: openai_schema::<WatchVideoArgs>(),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let input = args.url.trim();
        if input.is_empty() {
            return Err(ToolExecError("url/path cannot be empty".into()));
        }
        let prompt = args.prompt.as_deref()
            .unwrap_or("Watch this video carefully and provide a detailed summary. Include key points, any text/code shown, and notable visual elements.");
        let media_ref = self.ctx.resolve_media_ref(input, MediaType::Video).await?;
        analyze_with_gemini(
            &self.ctx.google_ai_key,
            &media_ref,
            prompt,
            MediaType::Video,
        )
        .await
    }
}

// ---------------------------------------------------------------------------
// listen_music tool
// ---------------------------------------------------------------------------

pub struct ListenMusicTool {
    ctx: MediaContext,
}

impl ListenMusicTool {
    pub fn new(
        google_ai_key: &str,
        workspace_dir: &Path,
        instance_slug: &str,
        public_url: &str,
        auth_token: &str,
    ) -> Self {
        Self {
            ctx: MediaContext::new(
                google_ai_key,
                workspace_dir,
                instance_slug,
                public_url,
                auth_token,
            ),
        }
    }
}

#[derive(Deserialize, JsonSchema)]
pub struct ListenMusicArgs {
    /// URL or local file path of the audio. Supports YouTube, Spotify links (audio only), direct URLs, local paths.
    pub url: String,
    /// What to focus on. E.g. "what genre is this?", "transcribe the lyrics", "describe the mood".
    /// Defaults to a general analysis.
    pub prompt: Option<String>,
}

impl Tool for ListenMusicTool {
    const NAME: &'static str = "listen_music";
    type Error = ToolExecError;
    type Args = ListenMusicArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "listen_music".into(),
            description: "Analyze audio/music via Gemini. YouTube URLs, direct links, or local files. Can identify genre, transcribe lyrics, describe mood.".into(),
            parameters: openai_schema::<ListenMusicArgs>(),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let input = args.url.trim();
        if input.is_empty() {
            return Err(ToolExecError("url/path cannot be empty".into()));
        }
        let prompt = args.prompt.as_deref()
            .unwrap_or("Listen to this audio carefully. Describe what you hear: genre, mood, instruments, lyrics (if any), and overall impression.");
        let media_ref = self.ctx.resolve_media_ref(input, MediaType::Audio).await?;
        let result = analyze_with_gemini(
            &self.ctx.google_ai_key,
            &media_ref,
            prompt,
            MediaType::Audio,
        )
        .await;
        // Clean up temp audio file if it was downloaded/compressed
        if media_ref != input && Path::new(&media_ref).exists() {
            let _ = std::fs::remove_file(&media_ref);
        }
        result
    }
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

pub fn is_youtube_url(url: &str) -> bool {
    let lower = url.to_lowercase();
    lower.contains("youtube.com/")
        || lower.contains("youtu.be/")
        || lower.contains("music.youtube.com/")
}

async fn maybe_compress(path: &str, media_type: MediaType) -> Result<String, ToolExecError> {
    let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    if size <= MAX_MEDIA_SIZE {
        return Ok(path.to_string());
    }
    log::info!(
        "[media] compressing {:.1} MB",
        size as f64 / 1024.0 / 1024.0
    );
    compress_media(Path::new(path), media_type).await
}

pub async fn download_youtube(url: &str, media_type: MediaType) -> Result<String, ToolExecError> {
    ensure_ytdlp().await?;
    let ext = media_type.yt_dlp_ext();
    let output_path = format!("/tmp/yt_{}.{ext}", std::process::id());

    let result = tokio::process::Command::new("yt-dlp")
        .args([
            "-f",
            media_type.yt_dlp_format(),
            "--no-playlist",
            "--no-warnings",
            "-o",
            &output_path,
            url,
        ])
        .output()
        .await
        .map_err(|e| ToolExecError(format!("yt-dlp failed: {e}")))?;

    if !result.status.success() {
        let stderr = String::from_utf8_lossy(&result.stderr);
        return Err(ToolExecError(format!(
            "yt-dlp failed: {}",
            stderr.chars().take(300).collect::<String>()
        )));
    }
    if !Path::new(&output_path).exists() {
        return Err(ToolExecError("yt-dlp produced no output file".into()));
    }
    Ok(output_path)
}

pub async fn ensure_ytdlp() -> Result<(), ToolExecError> {
    let check = tokio::process::Command::new("yt-dlp")
        .arg("--version")
        .output()
        .await;
    if check.is_ok() && check.as_ref().unwrap().status.success() {
        return Ok(());
    }

    log::info!("[media] installing yt-dlp...");
    let pipx = tokio::process::Command::new("pipx")
        .args(["install", "yt-dlp"])
        .output()
        .await;
    if pipx.as_ref().map(|o| o.status.success()).unwrap_or(false) {
        return Ok(());
    }

    let install = tokio::process::Command::new("sh")
        .args(["-c", "curl -sL https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp -o /usr/local/bin/yt-dlp && chmod +x /usr/local/bin/yt-dlp"])
        .output().await
        .map_err(|e| ToolExecError(format!("failed to install yt-dlp: {e}")))?;
    if !install.status.success() {
        return Err(ToolExecError(
            "yt-dlp not found and failed to install".into(),
        ));
    }
    Ok(())
}

async fn compress_media(path: &Path, media_type: MediaType) -> Result<String, ToolExecError> {
    let check = tokio::process::Command::new("ffmpeg")
        .arg("-version")
        .output()
        .await;
    if check.is_err() || !check.as_ref().unwrap().status.success() {
        return Err(ToolExecError("ffmpeg is not installed".into()));
    }

    let ext = match media_type {
        MediaType::Video => "mp4",
        MediaType::Audio => "mp3",
    };
    let output_path = format!("/tmp/compressed_{}.{ext}", std::process::id());
    let input = path.display().to_string();

    let args: Vec<&str> = match media_type {
        MediaType::Video => vec![
            "-i",
            &input,
            "-vf",
            "scale=-2:480",
            "-c:v",
            "libx264",
            "-crf",
            "28",
            "-preset",
            "fast",
            "-c:a",
            "aac",
            "-b:a",
            "64k",
            "-movflags",
            "+faststart",
            "-y",
            &output_path,
        ],
        MediaType::Audio => vec![
            "-i",
            &input,
            "-c:a",
            "libmp3lame",
            "-b:a",
            "96k",
            "-y",
            &output_path,
        ],
    };

    let result = tokio::process::Command::new("ffmpeg")
        .args(&args)
        .output()
        .await
        .map_err(|e| ToolExecError(format!("ffmpeg failed: {e}")))?;

    if !result.status.success() {
        let stderr = String::from_utf8_lossy(&result.stderr);
        return Err(ToolExecError(format!(
            "ffmpeg failed: {}",
            stderr.chars().take(300).collect::<String>()
        )));
    }

    let size = std::fs::metadata(&output_path)
        .map(|m| m.len())
        .unwrap_or(0);
    log::info!(
        "[media] compressed to {:.1} MB",
        size as f64 / 1024.0 / 1024.0
    );
    Ok(output_path)
}

/// Analyze media with Gemini via Google AI API (direct, no OpenRouter).
pub(crate) async fn analyze_with_gemini(
    api_key: &str,
    media_ref: &str,
    prompt: &str,
    media_type: MediaType,
) -> Result<String, ToolExecError> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .map_err(|e| ToolExecError(format!("HTTP client error: {e}")))?;

    // Build content parts based on media type
    let mut parts = vec![serde_json::json!({"text": prompt})];

    match media_type {
        MediaType::Video => {
            // Video: upload via Files API first, then reference by file_uri.
            // media_ref is a local path or public URL — read the bytes.
            let bytes = if media_ref.starts_with("http") {
                // Download from URL
                let dl = client
                    .get(media_ref)
                    .send()
                    .await
                    .map_err(|e| ToolExecError(format!("failed to download video: {e}")))?;
                dl.bytes()
                    .await
                    .map_err(|e| ToolExecError(format!("failed to read video bytes: {e}")))?
                    .to_vec()
            } else {
                std::fs::read(media_ref)
                    .map_err(|e| ToolExecError(format!("failed to read video file: {e}")))?
            };

            let file_uri = upload_to_gemini_files(&client, api_key, &bytes, "video/mp4").await?;

            parts.insert(
                0,
                serde_json::json!({
                    "file_data": {
                        "mime_type": "video/mp4",
                        "file_uri": file_uri,
                    }
                }),
            );
        }
        MediaType::Audio => {
            // Audio < 20MB: inline as base64
            let bytes = std::fs::read(media_ref)
                .map_err(|e| ToolExecError(format!("failed to read audio: {e}")))?;
            let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
            let ext = Path::new(media_ref)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("mp3");
            let mime = match ext {
                "mp3" => "audio/mp3",
                "m4a" => "audio/mp4",
                "wav" => "audio/wav",
                "ogg" => "audio/ogg",
                "flac" => "audio/flac",
                _ => "audio/mpeg",
            };
            parts.insert(
                0,
                serde_json::json!({
                    "inline_data": {
                        "mime_type": mime,
                        "data": b64,
                    }
                }),
            );
        }
    }

    let body = serde_json::json!({
        "contents": [{ "parts": parts }],
        "generationConfig": { "maxOutputTokens": 4096 }
    });

    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
        GEMINI_MODEL, api_key
    );

    let response = client
        .post(&url)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| ToolExecError(format!("Google AI request failed: {e}")))?;

    if !response.status().is_success() {
        let status = response.status();
        let err = response.text().await.unwrap_or_default();
        return Err(ToolExecError(format!("Google AI HTTP {status}: {err}")));
    }

    let result: serde_json::Value = response
        .json()
        .await
        .map_err(|e| ToolExecError(format!("failed to parse response: {e}")))?;

    Ok(result["candidates"][0]["content"]["parts"][0]["text"]
        .as_str()
        .unwrap_or("(no response from Gemini)")
        .to_string())
}

/// Upload a file to the Gemini Files API and return the file_uri.
async fn upload_to_gemini_files(
    client: &reqwest::Client,
    api_key: &str,
    bytes: &[u8],
    mime_type: &str,
) -> Result<String, ToolExecError> {
    let size = bytes.len();
    log::info!(
        "[media] uploading {:.1} MB to Gemini Files API",
        size as f64 / 1024.0 / 1024.0
    );

    // Step 1: initiate resumable upload
    let init_url = format!(
        "https://generativelanguage.googleapis.com/upload/v1beta/files?key={}",
        api_key
    );

    let init_res = client
        .post(&init_url)
        .header("X-Goog-Upload-Protocol", "resumable")
        .header("X-Goog-Upload-Command", "start")
        .header("X-Goog-Upload-Header-Content-Length", size.to_string())
        .header("X-Goog-Upload-Header-Content-Type", mime_type)
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({"file": {"display_name": "media_upload"}}))
        .send()
        .await
        .map_err(|e| ToolExecError(format!("Gemini Files init failed: {e}")))?;

    if !init_res.status().is_success() {
        let err = init_res.text().await.unwrap_or_default();
        return Err(ToolExecError(format!("Gemini Files init error: {err}")));
    }

    let upload_url = init_res
        .headers()
        .get("x-goog-upload-url")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| ToolExecError("no upload URL in response".into()))?
        .to_string();

    // Step 2: upload the bytes
    let upload_res = client
        .put(&upload_url)
        .header("Content-Length", size.to_string())
        .header("X-Goog-Upload-Offset", "0")
        .header("X-Goog-Upload-Command", "upload, finalize")
        .body(bytes.to_vec())
        .send()
        .await
        .map_err(|e| ToolExecError(format!("Gemini Files upload failed: {e}")))?;

    if !upload_res.status().is_success() {
        let err = upload_res.text().await.unwrap_or_default();
        return Err(ToolExecError(format!("Gemini Files upload error: {err}")));
    }

    let result: serde_json::Value = upload_res
        .json()
        .await
        .map_err(|e| ToolExecError(format!("failed to parse upload response: {e}")))?;

    let file_uri = result["file"]["uri"]
        .as_str()
        .ok_or_else(|| ToolExecError("no file URI in upload response".into()))?
        .to_string();

    let file_name = result["file"]["name"]
        .as_str()
        .ok_or_else(|| ToolExecError("no file name in upload response".into()))?
        .to_string();

    log::info!("[media] uploaded to Gemini: {file_uri}, waiting for ACTIVE state...");

    // Poll until the file is ACTIVE (video processing takes a few seconds)
    let poll_url = format!(
        "https://generativelanguage.googleapis.com/v1beta/{}?key={}",
        file_name, api_key
    );
    for attempt in 0..30 {
        let poll_res = client
            .get(&poll_url)
            .send()
            .await
            .map_err(|e| ToolExecError(format!("Gemini file poll failed: {e}")))?;
        if poll_res.status().is_success() {
            let info: serde_json::Value = poll_res.json().await.unwrap_or_default();
            let state = info["state"].as_str().unwrap_or("");
            match state {
                "ACTIVE" => {
                    log::info!("[media] file is ACTIVE after {attempt} polls");
                    return Ok(file_uri);
                }
                "FAILED" => {
                    return Err(ToolExecError("Gemini file processing failed".into()));
                }
                _ => {
                    log::info!("[media] file state: {state}, polling... ({attempt}/30)");
                }
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }

    Err(ToolExecError(
        "Gemini file did not become ACTIVE within 60s".into(),
    ))
}
