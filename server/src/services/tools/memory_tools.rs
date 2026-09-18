use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use crate::services::tool::{Tool, ToolDefinition};
use crate::services::vector::VectorStore;
use schemars::JsonSchema;
use serde::Deserialize;

use super::{ToolExecError, openai_schema};

// ---------------------------------------------------------------------------
// memory_write — create or update a memory file
// ---------------------------------------------------------------------------

pub struct MemoryWriteTool {
    memory_dir: PathBuf,
    uploads_dir: PathBuf,
    instance_slug: String,
    vector_store: Arc<VectorStore>,
    google_ai_key: String,
}

impl MemoryWriteTool {
    pub fn new(
        workspace_dir: &Path,
        instance_slug: &str,
        vector_store: Arc<VectorStore>,
        google_ai_key: &str,
    ) -> Self {
        Self {
            memory_dir: workspace_dir
                .join("instances")
                .join(instance_slug)
                .join("memory"),
            uploads_dir: workspace_dir
                .join("instances")
                .join(instance_slug)
                .join("uploads"),
            instance_slug: instance_slug.to_string(),
            vector_store,
            google_ai_key: google_ai_key.to_string(),
        }
    }
}

#[derive(Deserialize, JsonSchema)]
pub struct MemoryWriteArgs {
    /// Path within the memory library (e.g. "about/basics.md", "documents/schedule.pdf", "moments/sunset.jpg").
    /// Folders will be created automatically. For text files must end with .md.
    /// For uploaded files, use the original extension (.jpg, .png, .pdf, .mp4, .mp3).
    pub path: String,
    /// Content to write. For text files: the memory content. For uploaded files: optional description.
    #[serde(default)]
    pub content: String,
    /// "write" (default) to create/replace, or "append" to add to existing file.
    #[serde(default = "default_write_mode")]
    pub mode: String,
    /// Upload ID of a file to save as a memory (e.g. "upload_1234567890").
    /// When provided, the file is copied from uploads to the memory library.
    /// Works for any uploaded file: images, PDFs, videos, audio.
    /// IMPORTANT: when the user asks to save an uploaded file to memory, always use
    /// this field with the upload ID — do NOT read the file and convert to markdown.
    #[serde(default, alias = "image_upload_id")]
    pub upload_id: Option<String>,
}

fn default_write_mode() -> String {
    "write".to_string()
}

impl Tool for MemoryWriteTool {
    const NAME: &'static str = "memory_write";
    type Error = ToolExecError;
    type Args = MemoryWriteArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "memory_write".into(),
            description: "Create or update a memory file. Organize by folder (about/, preferences/, moments/, etc). \
                Files in pinned/ are always loaded into your context — use for triggers, rituals, critical references. \
                Can save uploaded files: set upload_id to the upload ID and path with the right extension. \
                IMPORTANT: when the user asks to save an uploaded file (image, PDF, video, audio), \
                always preserve the original file — use upload_id, do NOT convert to markdown. \
                Supported: images (.jpg .png .webp .gif), documents (.pdf), video (.mp4 .mov), audio (.mp3 .wav). \
                Example: documents/schedule.pdf, moments/sunset.jpg, recordings/voice-note.mp3".into(),
            parameters: openai_schema::<MemoryWriteArgs>(),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        // Upload save mode (images, PDFs, video, audio)
        if let Some(upload_id) = &args.upload_id {
            let clean_path = sanitize_media_path(&args.path);
            if clean_path.is_empty() {
                return Err(ToolExecError("invalid file path".into()));
            }

            // Find the uploaded file
            let meta_path = self.uploads_dir.join(format!("{upload_id}.json"));
            let meta_str = fs::read_to_string(&meta_path).map_err(|e| {
                log::warn!(
                    "memory_write: upload meta not found at {}: {e}",
                    meta_path.display()
                );
                ToolExecError(format!(
                    "upload {upload_id} not found (path: {})",
                    meta_path.display()
                ))
            })?;
            let meta: serde_json::Value = serde_json::from_str(&meta_str)
                .map_err(|_| ToolExecError("invalid upload metadata".into()))?;
            let stored_name = meta["stored_name"]
                .as_str()
                .ok_or_else(|| ToolExecError("missing stored_name".into()))?;
            let mime_type = meta["mime_type"].as_str().unwrap_or("image/jpeg");

            let src = self.uploads_dir.join(stored_name);
            let dst = self.memory_dir.join(&clean_path);

            if let Some(parent) = dst.parent() {
                fs::create_dir_all(parent).map_err(|e| ToolExecError(e.to_string()))?;
            }
            fs::copy(&src, &dst).map_err(|e| ToolExecError(e.to_string()))?;

            // Embed file into vector store
            if let Ok(bytes) = fs::read(&dst) {
                if bytes.len() < 20 * 1024 * 1024 {
                    let is_image = mime_type.starts_with("image/");
                    let source_type = if is_image {
                        "media_image"
                    } else if mime_type.starts_with("video/") {
                        "media_video"
                    } else if mime_type.starts_with("audio/") {
                        "media_audio"
                    } else {
                        "media_document"
                    };

                    let desc = if args.content.is_empty() {
                        clean_path.clone()
                    } else {
                        args.content.clone()
                    };

                    // Embed with text description so text queries can find images
                    let embed_result = if is_image {
                        crate::services::embedding::embed_text_and_image(
                            &self.google_ai_key,
                            &desc,
                            &bytes,
                            mime_type,
                        )
                        .await
                    } else {
                        crate::services::embedding::embed_media(
                            &self.google_ai_key,
                            &bytes,
                            mime_type,
                        )
                        .await
                    };

                    match embed_result {
                        Ok(vec) => {
                            if let Err(e) = self
                                .vector_store
                                .upsert_media(
                                    &self.instance_slug,
                                    &clean_path,
                                    source_type,
                                    mime_type,
                                    &clean_path,
                                    &desc,
                                    vec,
                                )
                                .await
                            {
                                log::warn!("[memory_write] vector upsert failed: {e}");
                            }
                        }
                        Err(e) => log::warn!("[memory_write] embed failed: {e}"),
                    }
                }
            }

            // If content provided, save it as a description sidecar
            if !args.content.is_empty() {
                let desc_path = format!("{clean_path}.md");
                let desc_full = self.memory_dir.join(&desc_path);
                let _ = fs::write(&desc_full, &args.content);
                embed_memory_to_vector(
                    &self.vector_store,
                    &self.google_ai_key,
                    &self.instance_slug,
                    &desc_path,
                    &args.content,
                )
                .await;
            }

            return Ok(format!("saved {clean_path}"));
        }

        // Text file mode
        let clean_path = sanitize_path(&args.path);
        if clean_path.is_empty() {
            return Err(ToolExecError("invalid path".into()));
        }

        let full_path = self.memory_dir.join(&clean_path);
        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent).map_err(|e| ToolExecError(e.to_string()))?;
        }

        let final_content = match args.mode.as_str() {
            "append" => {
                let existing = fs::read_to_string(&full_path).unwrap_or_default();
                let (_, body) = crate::services::memory::parse_frontmatter(&existing);
                let mut new_body = body.to_string();
                if !new_body.ends_with('\n') && !new_body.is_empty() {
                    new_body.push('\n');
                }
                new_body.push_str(&args.content);
                let stamped = crate::services::memory::stamp_content(&new_body, Some(&existing));
                fs::write(&full_path, &stamped).map_err(|e| ToolExecError(e.to_string()))?;
                stamped
            }
            _ => {
                let existing = fs::read_to_string(&full_path).ok();
                let stamped =
                    crate::services::memory::stamp_content(&args.content, existing.as_deref());
                fs::write(&full_path, &stamped).map_err(|e| ToolExecError(e.to_string()))?;
                stamped
            }
        };

        embed_memory_to_vector(
            &self.vector_store,
            &self.google_ai_key,
            &self.instance_slug,
            &clean_path,
            &final_content,
        )
        .await;

        Ok(format!(
            "{} {clean_path}",
            if args.mode == "append" {
                "appended to"
            } else {
                "wrote"
            }
        ))
    }
}

// ---------------------------------------------------------------------------
// memory_read — read a memory file or folder listing
// ---------------------------------------------------------------------------

pub struct MemoryReadTool {
    memory_dir: PathBuf,
    instance_slug: String,
    public_url: String,
    auth_token: String,
}

impl MemoryReadTool {
    pub fn new(workspace_dir: &Path, instance_slug: &str, public_url: &str) -> Self {
        Self {
            memory_dir: workspace_dir
                .join("instances")
                .join(instance_slug)
                .join("memory"),
            instance_slug: instance_slug.to_string(),
            public_url: public_url.to_string(),
            auth_token: std::env::var("BOLLY_AUTH_TOKEN").unwrap_or_default(),
        }
    }
}

#[derive(Deserialize, JsonSchema)]
pub struct MemoryReadArgs {
    /// Path to read — a file path (e.g. "about/basics.md") returns its content,
    /// a folder path (e.g. "about/") lists its contents.
    pub path: String,
}

impl Tool for MemoryReadTool {
    const NAME: &'static str = "memory_read";
    type Error = ToolExecError;
    type Args = MemoryReadArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "memory_read".into(),
            description: "Read a memory file or list folder contents.".into(),
            parameters: openai_schema::<MemoryReadArgs>(),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let clean_path = args.path.trim().trim_start_matches('/');
        let full_path = self.memory_dir.join(clean_path);

        // Prevent traversal
        if !full_path.starts_with(&self.memory_dir) {
            return Err(ToolExecError("invalid path".into()));
        }

        if full_path.is_dir() {
            // List directory contents
            let entries = fs::read_dir(&full_path).map_err(|e| ToolExecError(e.to_string()))?;
            let mut items = Vec::new();
            for entry in entries.filter_map(Result::ok) {
                let name = entry.file_name().to_string_lossy().to_string();
                let is_dir = entry.path().is_dir();
                items.push(if is_dir { format!("{name}/") } else { name });
            }
            items.sort();
            if items.is_empty() {
                Ok("(empty folder)".into())
            } else {
                Ok(items.join("\n"))
            }
        } else if full_path.exists() {
            let ext = full_path.extension().and_then(|e| e.to_str()).unwrap_or("");
            let is_image = matches!(ext, "jpg" | "jpeg" | "png" | "gif" | "webp" | "svg");
            let is_pdf = ext == "pdf";
            let is_media = is_image || is_pdf || matches!(ext, "mp4" | "mov" | "mp3" | "wav");

            if (is_image || is_pdf) && !self.public_url.is_empty() {
                let url = super::public_memory_url(
                    &self.public_url,
                    &self.instance_slug,
                    &clean_path,
                    &self.auth_token,
                );
                let block_type = if is_image { "image" } else { "document" };
                let blocks = serde_json::json!([
                    {"type": "text", "text": format!("memory file: {clean_path}")},
                    {"type": block_type, "source": {"type": "url", "url": url}},
                ]);
                Ok(serde_json::to_string(&blocks).unwrap())
            } else if is_media {
                // Audio/video — return metadata only (LLM can't inline these)
                let size = fs::metadata(&full_path).map(|m| m.len()).unwrap_or(0);
                let kind = match ext {
                    "mp4" | "mov" => "video",
                    "mp3" | "wav" => "audio",
                    _ => "file",
                };
                let mut out = format!("[{kind}: {clean_path}, {:.1} KB]", size as f64 / 1024.0);
                if !self.public_url.is_empty() {
                    let url = format!(
                        "{}/public/memory/{}/{}?token={}",
                        self.public_url, self.instance_slug, clean_path, self.auth_token,
                    );
                    out.push_str(&format!("\ndownload: {url}"));
                }
                Ok(out)
            } else {
                fs::read_to_string(&full_path).map_err(|e| ToolExecError(e.to_string()))
            }
        } else {
            Err(ToolExecError(format!("not found: {clean_path}")))
        }
    }
}

// ---------------------------------------------------------------------------
// memory_list — browse the full library structure
// ---------------------------------------------------------------------------

pub struct MemoryListTool {
    workspace_dir: PathBuf,
    instance_slug: String,
}

impl MemoryListTool {
    pub fn new(workspace_dir: &Path, instance_slug: &str) -> Self {
        Self {
            workspace_dir: workspace_dir.to_path_buf(),
            instance_slug: instance_slug.to_string(),
        }
    }
}

#[derive(Deserialize, JsonSchema)]
pub struct MemoryListArgs {
    /// Optional: filter by folder prefix (e.g. "moments/"). Omit to list everything.
    #[serde(default)]
    pub prefix: String,
}

impl Tool for MemoryListTool {
    const NAME: &'static str = "memory_list";
    type Error = ToolExecError;
    type Args = MemoryListArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "memory_list".into(),
            description: "List all memory files with summaries. Optional folder filter.".into(),
            parameters: openai_schema::<MemoryListArgs>(),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let entries =
            crate::services::memory::scan_library(&self.workspace_dir, &self.instance_slug);

        if entries.is_empty() {
            return Ok("(empty library — no memories yet)".into());
        }

        let prefix = args.prefix.trim().trim_start_matches('/');
        let filtered: Vec<_> = if prefix.is_empty() {
            entries
        } else {
            entries
                .into_iter()
                .filter(|e| e.path.starts_with(prefix))
                .collect()
        };

        if filtered.is_empty() {
            return Ok(format!("no memories under \"{prefix}\""));
        }

        let mut result = String::new();
        for entry in &filtered {
            result.push_str(&format!("{} — {}\n", entry.path, entry.summary));
        }
        Ok(result)
    }
}

// ---------------------------------------------------------------------------
// memory_forget — delete a memory file
// ---------------------------------------------------------------------------

pub struct MemoryForgetTool {
    memory_dir: PathBuf,
    instance_slug: String,
    vector_store: Arc<VectorStore>,
    #[allow(dead_code)]
    google_ai_key: String,
}

impl MemoryForgetTool {
    pub fn new(
        workspace_dir: &Path,
        instance_slug: &str,
        vector_store: Arc<VectorStore>,
        google_ai_key: &str,
    ) -> Self {
        Self {
            memory_dir: workspace_dir
                .join("instances")
                .join(instance_slug)
                .join("memory"),
            instance_slug: instance_slug.to_string(),
            vector_store,
            google_ai_key: google_ai_key.to_string(),
        }
    }
}

#[derive(Deserialize, JsonSchema)]
pub struct MemoryForgetArgs {
    /// Path of the memory file to delete (e.g. "about/old-job.md").
    /// Or a search query — all files containing this text will be listed for confirmation.
    pub target: String,
}

impl Tool for MemoryForgetTool {
    const NAME: &'static str = "memory_forget";
    type Error = ToolExecError;
    type Args = MemoryForgetArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "memory_forget".into(),
            description: "Delete a memory file by path.".into(),
            parameters: openai_schema::<MemoryForgetArgs>(),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let target = args.target.trim();

        // If it looks like a path (contains / or ends with .md), try direct delete
        if target.contains('/') || target.ends_with(".md") {
            let clean = target.trim_start_matches('/');
            let full_path = self.memory_dir.join(clean);

            if !full_path.starts_with(&self.memory_dir) {
                return Err(ToolExecError("invalid path".into()));
            }

            if full_path.exists() {
                fs::remove_file(&full_path).map_err(|e| ToolExecError(e.to_string()))?;
                if let Some(parent) = full_path.parent() {
                    let _ = cleanup_empty_dirs(parent, &self.memory_dir);
                }
                if let Err(e) = self
                    .vector_store
                    .delete_by_path(&self.instance_slug, clean)
                    .await
                {
                    log::warn!("[memory_forget] vector delete failed: {e}");
                }
                return Ok(format!("deleted {clean}"));
            }
        }

        // Otherwise, search and delete matching files
        let workspace_dir = self
            .memory_dir
            .parent()
            .and_then(|p| p.parent())
            .and_then(|p| p.parent())
            .unwrap_or(&self.memory_dir);
        let instance_slug = self
            .memory_dir
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|s| s.to_str())
            .unwrap_or("");

        let removed =
            crate::services::memory::forget_memories(workspace_dir, instance_slug, target);
        if removed == 0 {
            Ok(format!("no memories matched \"{target}\""))
        } else {
            Ok(format!(
                "deleted {removed} memory file(s) matching \"{target}\""
            ))
        }
    }
}

// ---------------------------------------------------------------------------
// memory_search — BM25-style semantic search across memory files
// ---------------------------------------------------------------------------

pub struct MemorySearchTool {
    instance_slug: String,
    vector_store: Arc<VectorStore>,
    google_ai_key: String,
    public_url: String,
    auth_token: String,
}

impl MemorySearchTool {
    pub fn new(
        _workspace_dir: &Path,
        instance_slug: &str,
        vector_store: Arc<VectorStore>,
        google_ai_key: &str,
        public_url: &str,
    ) -> Self {
        let auth_token = std::env::var("BOLLY_AUTH_TOKEN").unwrap_or_default();
        Self {
            instance_slug: instance_slug.to_string(),
            vector_store,
            google_ai_key: google_ai_key.to_string(),
            public_url: public_url.to_string(),
            auth_token,
        }
    }
}

#[derive(Deserialize, JsonSchema)]
pub struct MemorySearchArgs {
    /// Natural language search query. Can be a question, keywords, or a topic.
    /// Examples: "what does the user do for work", "music preferences", "that bug we discussed"
    pub query: String,
    /// Maximum number of results to return. Default: 5.
    pub limit: Option<usize>,
}

impl Tool for MemorySearchTool {
    const NAME: &'static str = "memory_search";
    type Error = ToolExecError;
    type Args = MemorySearchArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "memory_search".into(),
            description: "Search the memory library using natural language. \
                Finds relevant memories by matching words and concepts across all files. \
                Large files are searched at chunk level for precise results. \
                Use this instead of memory_list when looking for something specific."
                .into(),
            parameters: openai_schema::<MemorySearchArgs>(),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let query = args.query.trim();
        if query.is_empty() {
            return Err(ToolExecError("query cannot be empty".into()));
        }
        let limit = args.limit.unwrap_or(5).min(20);

        let query_vec = crate::services::embedding::embed_text(
            &self.google_ai_key,
            query,
            crate::services::embedding::TaskType::RetrievalQuery,
        )
        .await
        .map_err(|e| ToolExecError(format!("embed query failed: {e}")))?;

        let results = self
            .vector_store
            .search(&self.instance_slug, query_vec, limit)
            .await
            .map_err(|e| ToolExecError(format!("vector search failed: {e}")))?;

        if results.is_empty() {
            return Ok(format!("no memories matched \"{query}\""));
        }

        let has_images = results
            .iter()
            .any(|r| r.source_type == "media_image" && !self.public_url.is_empty());

        if !has_images {
            // Text-only results — return plain string
            let mut output = format!("found {} relevant memories:\n\n", results.len());
            for (i, r) in results.iter().enumerate() {
                let preview = r.content_preview.trim();
                output.push_str(&format!(
                    "--- [{}/{} · {} · score: {:.4}] ---\n{preview}\n\n",
                    i + 1,
                    results.len(),
                    r.path,
                    r.score,
                ));
            }
            return Ok(output);
        }

        // Mixed text + image results — return as content block array
        let mut blocks: Vec<serde_json::Value> = Vec::new();
        let mut text_buf = format!("found {} relevant memories:\n\n", results.len());

        for (i, r) in results.iter().enumerate() {
            let preview = r.content_preview.trim();
            text_buf.push_str(&format!(
                "--- [{}/{} · {} · score: {:.4}] ---\n{preview}\n\n",
                i + 1,
                results.len(),
                r.path,
                r.score,
            ));

            if r.source_type == "media_image" && !self.public_url.is_empty() {
                if let Some(upload_id) = &r.upload_id {
                    // Flush text before image
                    if !text_buf.trim().is_empty() {
                        blocks.push(serde_json::json!({"type": "text", "text": text_buf.trim()}));
                        text_buf.clear();
                    }
                    // Memory-originated images use /public/memory/, uploads use /public/files/
                    let url = if upload_id.contains('/') {
                        super::public_memory_url(
                            &self.public_url,
                            &self.instance_slug,
                            upload_id,
                            &self.auth_token,
                        )
                    } else {
                        super::public_file_url(
                            &self.public_url,
                            &self.instance_slug,
                            upload_id,
                            &self.auth_token,
                        )
                    };
                    blocks.push(
                        serde_json::json!({"type": "image", "source": {"type": "url", "url": url}}),
                    );
                }
            }
        }
        if !text_buf.trim().is_empty() {
            blocks.push(serde_json::json!({"type": "text", "text": text_buf.trim()}));
        }

        Ok(serde_json::to_string(&serde_json::Value::Array(blocks)).unwrap())
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn sanitize_path(path: &str) -> String {
    let path = path.trim().trim_start_matches('/');
    let parts: Vec<&str> = path.split('/').collect();
    if parts
        .iter()
        .any(|p| p.is_empty() || *p == ".." || p.starts_with('.'))
    {
        return String::new();
    }
    let result = parts.join("/");
    if !result.ends_with(".md") {
        format!("{result}.md")
    } else {
        result
    }
}

/// Sanitize path for image files — allows image extensions instead of forcing .md.
/// Allowed file extensions for memory storage.
/// Matches Gemini Embedding 2 supported formats + common web formats.
const ALLOWED_MEDIA_EXTS: &[&str] = &[
    // Images (Gemini: PNG, JPEG; also allow web formats)
    ".jpg", ".jpeg", ".png", ".webp", ".gif", ".svg", // Documents (Gemini: PDF)
    ".pdf", // Video (Gemini: MP4, MOV with H264/H265/AV1/VP9)
    ".mp4", ".mov", // Audio (Gemini: MP3, WAV)
    ".mp3", ".wav",
];

fn sanitize_media_path(path: &str) -> String {
    let path = path.trim().trim_start_matches('/');
    let parts: Vec<&str> = path.split('/').collect();
    if parts
        .iter()
        .any(|p| p.is_empty() || *p == ".." || p.starts_with('.'))
    {
        return String::new();
    }
    let result = parts.join("/");
    let lower = result.to_lowercase();
    if ALLOWED_MEDIA_EXTS.iter().any(|ext| lower.ends_with(ext)) {
        result
    } else {
        // Don't silently rename — return empty to signal unsupported format
        String::new()
    }
}

fn cleanup_empty_dirs(dir: &Path, base: &Path) -> std::io::Result<()> {
    if dir == base || !dir.starts_with(base) {
        return Ok(());
    }
    if dir.is_dir() {
        let is_empty = fs::read_dir(dir)?.next().is_none();
        if is_empty {
            fs::remove_dir(dir)?;
            if let Some(parent) = dir.parent() {
                cleanup_empty_dirs(parent, base)?;
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// memory_connect — create/remove connections in the memory graph
// ---------------------------------------------------------------------------

pub struct MemoryConnectTool {
    workspace_dir: PathBuf,
    instance_slug: String,
}

impl MemoryConnectTool {
    pub fn new(workspace_dir: &Path, instance_slug: &str) -> Self {
        Self {
            workspace_dir: workspace_dir.to_path_buf(),
            instance_slug: instance_slug.to_string(),
        }
    }
}

#[derive(Deserialize, JsonSchema)]
pub struct MemoryConnectArgs {
    /// Action: "connect" to add an edge, "disconnect" to remove, "neighbors" to list connections.
    action: String,
    /// First memory path (e.g. "about/work.md").
    path_a: String,
    /// Second memory path (required for connect/disconnect, ignored for neighbors).
    #[serde(default)]
    path_b: String,
}

impl Tool for MemoryConnectTool {
    const NAME: &'static str = "memory_connect";
    type Error = ToolExecError;
    type Args = MemoryConnectArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "memory_connect".into(),
            description: "Manage the memory graph — connect related memories, disconnect them, or list neighbors. \
                           The graph is undirected: connecting A to B also connects B to A.".into(),
            parameters: openai_schema::<MemoryConnectArgs>(),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, ToolExecError> {
        use crate::services::memory;

        match args.action.as_str() {
            "connect" => {
                if args.path_b.is_empty() {
                    return Ok("error: path_b is required for connect".into());
                }
                let added = memory::add_edge(
                    &self.workspace_dir,
                    &self.instance_slug,
                    &args.path_a,
                    &args.path_b,
                );
                if added {
                    Ok(format!("connected: {} <-> {}", args.path_a, args.path_b))
                } else {
                    Ok(format!(
                        "already connected: {} <-> {}",
                        args.path_a, args.path_b
                    ))
                }
            }
            "disconnect" => {
                if args.path_b.is_empty() {
                    return Ok("error: path_b is required for disconnect".into());
                }
                let mut graph = memory::load_graph(&self.workspace_dir, &self.instance_slug);
                let edge = if args.path_a <= args.path_b {
                    [args.path_a.clone(), args.path_b.clone()]
                } else {
                    [args.path_b.clone(), args.path_a.clone()]
                };
                let before = graph.edges.len();
                graph.edges.retain(|e| *e != edge);
                if graph.edges.len() != before {
                    memory::save_graph(&self.workspace_dir, &self.instance_slug, &graph);
                    Ok(format!("disconnected: {} <-> {}", args.path_a, args.path_b))
                } else {
                    Ok(format!(
                        "no connection found between {} and {}",
                        args.path_a, args.path_b
                    ))
                }
            }
            "neighbors" => {
                let graph = memory::load_graph(&self.workspace_dir, &self.instance_slug);
                let neighbors = memory::get_neighbors(&graph, &args.path_a);
                if neighbors.is_empty() {
                    Ok(format!("{} has no connections", args.path_a))
                } else {
                    Ok(format!(
                        "{} is connected to:\n{}",
                        args.path_a,
                        neighbors
                            .iter()
                            .map(|n| format!("- {n}"))
                            .collect::<Vec<_>>()
                            .join("\n")
                    ))
                }
            }
            _ => Ok(format!(
                "unknown action: {}. use connect, disconnect, or neighbors",
                args.action
            )),
        }
    }
}

/// Embed a memory file into the vector store.
async fn embed_memory_to_vector(
    vector_store: &VectorStore,
    google_ai_key: &str,
    instance_slug: &str,
    path: &str,
    content: &str,
) {
    use crate::services::{embedding, vector};

    let chunks = vector::chunk_text(content);
    let mut chunk_vectors = Vec::new();

    for chunk in &chunks {
        match embedding::embed_text(google_ai_key, chunk, embedding::TaskType::RetrievalDocument)
            .await
        {
            Ok(vec) => chunk_vectors.push((chunk.clone(), vec)),
            Err(e) => {
                log::warn!("[memory_tool] embed error for {path}: {e}");
                return;
            }
        }
    }

    if let Err(e) = vector_store
        .upsert_text_memory(instance_slug, path, chunk_vectors)
        .await
    {
        log::warn!("[memory_tool] vector upsert failed for {path}: {e}");
    }
}
