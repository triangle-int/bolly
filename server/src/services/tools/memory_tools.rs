#[cfg(test)]
use std::fs;
use std::{path::Path, sync::Arc};

use crate::services::tool::{Tool, ToolDefinition};
use crate::services::vector::VectorStore;
use schemars::JsonSchema;
use serde::Deserialize;

use super::{ToolExecError, openai_schema};

// ---------------------------------------------------------------------------
// memory_write — create or update a memory file
// ---------------------------------------------------------------------------

pub struct MemoryWriteTool {
    instance_slug: String,
    vector_store: Arc<VectorStore>,
}

impl MemoryWriteTool {
    pub fn new(_workspace_dir: &Path, instance_slug: &str, vector_store: Arc<VectorStore>) -> Self {
        Self {
            instance_slug: instance_slug.to_string(),
            vector_store,
        }
    }
}

#[derive(Deserialize, JsonSchema)]
pub struct MemoryWriteArgs {
    /// Path within the memory library (e.g. "about/basics.md", "documents/schedule.pdf", "moments/sunset.jpg").
    /// Folders will be created automatically. For text files must end with .md.
    /// For uploaded files, use the original extension (.jpg, .png, .pdf, .mp4, .mp3).
    pub path: String,
    /// Content to write. For media, supply a useful description, extracted text, or transcript
    /// based on your analysis. It is persisted beside the original and enables semantic search.
    /// Without it the original is saved, but semantic indexing remains pending.
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
                Include a useful description, extracted text, or transcript in content for semantic retrieval. \
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

            let pending = self
                .vector_store
                .replace_media_from_upload(
                    &self.instance_slug,
                    &clean_path,
                    upload_id,
                    &args.content,
                )
                .await
                .map_err(ToolExecError)?;
            if let Some(error) = pending {
                return Ok(format!(
                    "saved {clean_path}; semantic indexing pending: {error}"
                ));
            }
            return Ok(format!("saved {clean_path}"));
        }

        // Text file mode
        let clean_path = sanitize_path(&args.path);
        if clean_path.is_empty() {
            return Err(ToolExecError("invalid path".into()));
        }

        // Editing a reserved media sidecar must preserve its versioned owner binding.
        if let Some(owner) = crate::services::media_text::media_path(&clean_path) {
            self.vector_store
                .edit_media_text(
                    &self.instance_slug,
                    owner,
                    &args.content,
                    args.mode == "append",
                )
                .await
                .map_err(ToolExecError)?;
            return Ok(format!("wrote {clean_path}"));
        }
        self.vector_store
            .write_text_memory(
                &self.instance_slug,
                &clean_path,
                &args.content,
                args.mode == "append",
            )
            .await
            .map_err(ToolExecError)?;

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
    instance_slug: String,
    media: Arc<crate::services::media_text::MediaStore>,
    public_url: String,
    auth_token: String,
}

impl MemoryReadTool {
    pub fn new(
        _workspace_dir: &Path,
        instance_slug: &str,
        public_url: &str,
        vector_store: Arc<VectorStore>,
    ) -> Self {
        Self {
            instance_slug: instance_slug.to_string(),
            media: vector_store.media_store(),
            public_url: public_url.to_string(),
            auth_token: std::env::var("NOLUNE_AUTH_TOKEN").unwrap_or_default(),
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
        let clean_path = args.path.trim().trim_matches('/');
        let metadata = if clean_path.is_empty() {
            None
        } else {
            Some(
                self.media
                    .memory_metadata(&self.instance_slug, clean_path)
                    .map_err(|error| ToolExecError(error.to_string()))?,
            )
        };
        if clean_path.is_empty() || metadata.is_some_and(|metadata| metadata.is_dir) {
            // List directory contents
            let items = self
                .media
                .list_memory_dir(
                    &self.instance_slug,
                    (!clean_path.is_empty()).then_some(clean_path),
                )
                .map_err(|error| ToolExecError(error.to_string()))?
                .into_iter()
                .map(|entry| {
                    if entry.is_dir {
                        format!("{}/", entry.name)
                    } else {
                        entry.name
                    }
                })
                .collect::<Vec<_>>();
            if items.is_empty() {
                Ok("(empty folder)".into())
            } else {
                Ok(items.join("\n"))
            }
        } else if metadata.is_some_and(|metadata| metadata.is_file) {
            let ext = Path::new(clean_path)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            let ext = ext.as_str();
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
                let size = metadata.map_or(0, |metadata| metadata.len);
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
                self.media
                    .read_memory_text(&self.instance_slug, clean_path)
                    .map_err(|error| ToolExecError(error.to_string()))
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
    media: Arc<crate::services::media_text::MediaStore>,
    instance_slug: String,
}

impl MemoryListTool {
    pub fn new(_workspace_dir: &Path, instance_slug: &str, vector_store: Arc<VectorStore>) -> Self {
        Self {
            media: vector_store.media_store(),
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
        let entries = crate::services::memory::scan_library(&self.media, &self.instance_slug);

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
    instance_slug: String,
    vector_store: Arc<VectorStore>,
}

impl MemoryForgetTool {
    pub fn new(workspace_dir: &Path, instance_slug: &str, vector_store: Arc<VectorStore>) -> Self {
        let _ = workspace_dir;
        Self {
            instance_slug: instance_slug.to_string(),
            vector_store,
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

        // Paths always reconcile filesystem and derived state, even when either
        // side is already absent from a previous partial attempt.
        if target.contains('/')
            || target.ends_with(".md")
            || crate::services::media_text::source_type(target).is_some()
        {
            let clean = target;
            if let Err(error) = self
                .vector_store
                .delete_memory(&self.instance_slug, clean)
                .await
            {
                return Err(ToolExecError(format!(
                    "cleanup incomplete for {clean}: {error}; retry memory_forget with this exact path",
                )));
            }
            return Ok(format!("reconciled {clean}"));
        }

        // Otherwise, search and delete matching files
        let media = self.vector_store.media_store();
        let query = target.to_ascii_lowercase();
        let words = query.split_whitespace().collect::<Vec<_>>();
        let mut matches = Vec::new();
        for entry in crate::services::memory::scan_library(&media, &self.instance_slug) {
            let content = if crate::services::media_text::source_type(&entry.path).is_some() {
                media
                    .read(&self.instance_slug, &entry.path)
                    .unwrap_or_default()
            } else {
                media
                    .read_memory_text(&self.instance_slug, &entry.path)
                    .unwrap_or_default()
            };
            let combined = format!("{} {content}", entry.path).to_ascii_lowercase();
            if words.iter().any(|word| combined.contains(word)) {
                matches.push(entry.path);
            }
        }
        if matches.is_empty() {
            return Ok(format!("no memories matched \"{target}\""));
        }

        let mut failures = Vec::new();
        for path in &matches {
            if let Err(error) = self
                .vector_store
                .delete_memory(&self.instance_slug, path)
                .await
            {
                failures.push(format!("{path} ({error})"));
            }
        }
        if !failures.is_empty() {
            return Err(ToolExecError(format!(
                "cleanup incomplete for {}; retry memory_forget with this exact path",
                failures.join(", ")
            )));
        }
        Ok(format!(
            "deleted {} memory file(s) matching \"{target}\"",
            matches.len()
        ))
    }
}

// ---------------------------------------------------------------------------
// memory_search — BM25-style semantic search across memory files
// ---------------------------------------------------------------------------

pub struct MemorySearchTool {
    instance_slug: String,
    vector_store: Arc<VectorStore>,
    public_url: String,
    auth_token: String,
}

impl MemorySearchTool {
    pub fn new(
        _workspace_dir: &Path,
        instance_slug: &str,
        vector_store: Arc<VectorStore>,
        public_url: &str,
    ) -> Self {
        let auth_token = std::env::var("NOLUNE_AUTH_TOKEN").unwrap_or_default();
        Self {
            instance_slug: instance_slug.to_string(),
            vector_store,
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

        let results = self
            .vector_store
            .search_text(&self.instance_slug, query, limit)
            .await;

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
                if let Some(url) = super::media_result_url(
                    &self.public_url,
                    &self.instance_slug,
                    r,
                    &self.auth_token,
                ) {
                    output.push_str(&format!("media: {url}\n\n"));
                }
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
            if let Some(url) =
                super::media_result_url(&self.public_url, &self.instance_slug, r, &self.auth_token)
            {
                text_buf.push_str(&format!("media: {url}\n\n"));
            }

            if r.source_type == "media_image" && !self.public_url.is_empty() {
                if let Some(upload_id) = &r.upload_id {
                    // Flush text before image
                    if !text_buf.trim().is_empty() {
                        blocks.push(serde_json::json!({"type": "text", "text": text_buf.trim()}));
                        text_buf.clear();
                    }
                    // Memory-originated images use /public/memory/, uploads use /public/files/
                    let url = if upload_id == &r.path || upload_id.contains('/') {
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

fn sanitize_media_path(path: &str) -> String {
    let path = path.trim();
    let parts: Vec<&str> = path.split('/').collect();
    if parts
        .iter()
        .any(|p| p.is_empty() || *p == ".." || p.starts_with('.'))
    {
        return String::new();
    }
    let result = parts.join("/");
    if crate::services::media_text::source_type(&result).is_some() {
        result
    } else {
        // Don't silently rename — return empty to signal unsupported format
        String::new()
    }
}

// ---------------------------------------------------------------------------
// memory_connect — create/remove connections in the memory graph
// ---------------------------------------------------------------------------

pub struct MemoryConnectTool {
    media: Arc<crate::services::media_text::MediaStore>,
    instance_slug: String,
}

impl MemoryConnectTool {
    pub fn new(instance_slug: &str, vector_store: Arc<VectorStore>) -> Self {
        Self {
            media: vector_store.media_store(),
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
                let added =
                    memory::add_edge(&self.media, &self.instance_slug, &args.path_a, &args.path_b);
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
                let mut graph = memory::load_graph(&self.media, &self.instance_slug);
                let edge = if args.path_a <= args.path_b {
                    [args.path_a.clone(), args.path_b.clone()]
                } else {
                    [args.path_b.clone(), args.path_a.clone()]
                };
                let before = graph.edges.len();
                graph.edges.retain(|e| *e != edge);
                if graph.edges.len() != before {
                    memory::save_graph(&self.media, &self.instance_slug, &graph);
                    Ok(format!("disconnected: {} <-> {}", args.path_a, args.path_b))
                } else {
                    Ok(format!(
                        "no connection found between {} and {}",
                        args.path_a, args.path_b
                    ))
                }
            }
            "neighbors" => {
                let graph = memory::load_graph(&self.media, &self.instance_slug);
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

#[cfg(test)]
mod embedding_fallback_tests {
    use super::*;

    #[tokio::test]
    async fn missing_key_keeps_memory_write_and_search_tools_functional() {
        let workspace = tempfile::tempdir().unwrap();
        let store = Arc::new(VectorStore::connect(workspace.path()).await);
        // The default embedding backend is absent; BM25 still sees live writes.
        assert_eq!(store.embedding_status()["status"], "unavailable");
        let writer = MemoryWriteTool::new(workspace.path(), "one", store.clone());
        let saved = writer
            .call(MemoryWriteArgs {
                path: "preferences.md".into(),
                content: "Favorite constellation is Orion".into(),
                mode: "write".into(),
                upload_id: None,
            })
            .await
            .unwrap();
        assert!(saved.contains("wrote"));
        assert!(
            fs::read_to_string(workspace.path().join("instances/one/memory/preferences.md"))
                .unwrap()
                .contains("Orion")
        );
        let hits = store.search_text("one", "Orion", 5).await;
        assert_eq!(hits[0].path, "preferences.md");
        let search = MemorySearchTool::new(workspace.path(), "one", store, "");
        assert!(
            search
                .call(MemorySearchArgs {
                    query: "Orion".into(),
                    limit: None
                })
                .await
                .unwrap()
                .contains("Orion")
        );
    }

    #[tokio::test]
    async fn query_forget_removes_files_vector_records_and_cached_bm25_hits() {
        let workspace = tempfile::tempdir().unwrap();
        let memory = workspace.path().join("instances/one/memory");
        fs::create_dir_all(&memory).unwrap();
        fs::write(memory.join("stars.md"), "Orion nebula").unwrap();
        let store = Arc::new(VectorStore::connect(workspace.path()).await);
        let mut vector = vec![0.; 768];
        vector[0] = 1.;
        store
            .upsert_text_memory("one", "stars.md", vec![("Orion nebula".into(), vector)])
            .await
            .unwrap();
        assert_eq!(store.search_text("one", "Orion", 5).await.len(), 1);

        let forget = MemoryForgetTool::new(workspace.path(), "one", store.clone());
        let result = forget
            .call(MemoryForgetArgs {
                target: "Orion".into(),
            })
            .await
            .unwrap();
        assert!(result.contains("deleted 1"));
        assert!(!memory.join("stars.md").exists());
        assert!(store.search_text("one", "Orion", 5).await.is_empty());
        assert!(store.list_all("one", 10).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn direct_forget_reconciles_stale_vector_when_source_is_missing() {
        let workspace = tempfile::tempdir().unwrap();
        let store = Arc::new(VectorStore::connect(workspace.path()).await);
        let mut vector = vec![0.; 768];
        vector[0] = 1.;
        store
            .upsert_text_memory("one", "photo.png", vec![("stale".into(), vector)])
            .await
            .unwrap();

        let forget = MemoryForgetTool::new(workspace.path(), "one", store.clone());
        let output = forget
            .call(MemoryForgetArgs {
                target: "photo.png".into(),
            })
            .await
            .unwrap();

        assert!(output.contains("reconciled photo.png"), "{output}");
        assert!(store.list_all("one", 10).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn direct_forget_attempts_owner_and_vector_after_sidecar_remove_failure() {
        let workspace = tempfile::tempdir().unwrap();
        let memory = workspace.path().join("instances/one/memory");
        fs::create_dir_all(memory.join("photo.png.md")).unwrap();
        fs::write(memory.join("photo.png"), b"owner").unwrap();
        let store = Arc::new(VectorStore::connect(workspace.path()).await);
        let mut vector = vec![0.; 768];
        vector[0] = 1.;
        store
            .upsert_text_memory("one", "photo.png", vec![("stale".into(), vector)])
            .await
            .unwrap();

        let forget = MemoryForgetTool::new(workspace.path(), "one", store.clone());
        let error = forget
            .call(MemoryForgetArgs {
                target: "photo.png".into(),
            })
            .await
            .unwrap_err();

        assert!(error.to_string().contains("photo.png"));
        assert!(!memory.join("photo.png").exists());
        assert!(store.list_all("one", 10).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn direct_forget_retries_vector_delete_after_persistence_failure() {
        let workspace = tempfile::tempdir().unwrap();
        let memory = workspace.path().join("instances/one/memory");
        fs::create_dir_all(&memory).unwrap();
        fs::write(memory.join("note.md"), "Orion").unwrap();
        let store = Arc::new(VectorStore::connect(workspace.path()).await);
        let mut vector = vec![0.; 768];
        vector[0] = 1.;
        store
            .upsert_text_memory("one", "note.md", vec![("Orion".into(), vector)])
            .await
            .unwrap();
        store.inject_next_persist_failure();

        let forget = MemoryForgetTool::new(workspace.path(), "one", store.clone());
        let first = forget
            .call(MemoryForgetArgs {
                target: "note.md".into(),
            })
            .await;
        assert!(first.unwrap_err().to_string().contains("note.md"));
        assert!(!memory.join("note.md").exists());
        assert_eq!(store.list_all("one", 10).await.unwrap().len(), 1);

        forget
            .call(MemoryForgetArgs {
                target: "note.md".into(),
            })
            .await
            .unwrap();
        assert!(store.list_all("one", 10).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn query_forget_reports_the_exact_path_for_derived_delete_failure() {
        let workspace = tempfile::tempdir().unwrap();
        let memory = workspace.path().join("instances/one/memory");
        fs::create_dir_all(&memory).unwrap();
        fs::write(memory.join("stars.md"), "Orion").unwrap();
        let store = Arc::new(VectorStore::connect(workspace.path()).await);
        let mut vector = vec![0.; 768];
        vector[0] = 1.;
        store
            .upsert_text_memory("one", "stars.md", vec![("Orion".into(), vector)])
            .await
            .unwrap();
        store.inject_next_persist_failure();

        let forget = MemoryForgetTool::new(workspace.path(), "one", store);
        let error = forget
            .call(MemoryForgetArgs {
                target: "Orion".into(),
            })
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("stars.md"), "{error}");
        assert!(
            error.contains("retry memory_forget with this exact path"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn provider_outage_keeps_live_writes_and_bm25_functional() {
        use crate::services::embedding::tests::MockServer;
        for status in [401, 503] {
            let mock = MockServer::new(vec![
                (status, serde_json::json!({"error":"mock-secret"}));
                3
            ])
            .await;
            let workspace = tempfile::tempdir().unwrap();
            let store =
                Arc::new(VectorStore::connect_with_config(workspace.path(), &mock.config).await);
            let writer = MemoryWriteTool::new(workspace.path(), "one", store.clone());
            writer
                .call(MemoryWriteArgs {
                    path: "star.md".into(),
                    content: "Orion nebula".into(),
                    mode: "write".into(),
                    upload_id: None,
                })
                .await
                .unwrap();
            assert_eq!(store.embedding_status()["status"], "unavailable");
            assert_eq!(
                store.search_text("one", "Orion", 5).await[0].path,
                "star.md"
            );
            assert!(store.list_all("one", 10).await.unwrap().is_empty());
        }
    }
}

#[cfg(test)]
mod media_tests {
    use super::*;
    use crate::services::embedding::tests::{MockServer, response};

    fn upload(workspace: &Path) {
        let dir = workspace.join("instances/one/uploads");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("raw.png"), [0xff, 0x00, 0x81]).unwrap();
        fs::write(dir.join("upload_test.json"), r#"{"stored_name":"raw.png"}"#).unwrap();
    }

    #[tokio::test]
    async fn media_live_write_indexes_persisted_representation_under_real_path() {
        let mock = MockServer::new(vec![(200, response(vec![1., 0., 0.]))]).await;
        let ws = tempfile::tempdir().unwrap();
        upload(ws.path());
        let store = Arc::new(VectorStore::connect_with_config(ws.path(), &mock.config).await);
        let writer = MemoryWriteTool::new(ws.path(), "one", store.clone());
        writer
            .call(MemoryWriteArgs {
                path: "moments/photo.png".into(),
                content: "Orion above snowy mountains".into(),
                mode: "write".into(),
                upload_id: Some("upload_test".into()),
            })
            .await
            .unwrap();
        let dir = ws.path().join("instances/one/memory");
        assert_eq!(
            fs::read(dir.join("moments/photo.png")).unwrap(),
            [0xff, 0x00, 0x81]
        );
        assert_eq!(
            crate::services::media_text::read(&dir, "moments/photo.png").unwrap(),
            "Orion above snowy mountains"
        );
        let records = store.list_all("one", 10).await.unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].path, "moments/photo.png");
        assert_eq!(records[0].source_type, "media_image");
        assert_eq!(records[0].content_preview, "Orion above snowy mountains");
        assert_eq!(records[0].upload_id.as_deref(), Some("moments/photo.png"));
        assert_eq!(
            mock.requests.lock().unwrap()[0].2["input"],
            serde_json::json!(["Orion above snowy mountains"])
        );
    }
    #[tokio::test]
    async fn media_missing_representation_is_nonfatal_and_invalidates_completion() {
        let ws = tempfile::tempdir().unwrap();
        upload(ws.path());
        let store = Arc::new(VectorStore::connect(ws.path()).await);
        store
            .backfill_text_memories(ws.path(), "one")
            .await
            .unwrap();
        assert!(!store.needs_backfill("one").await.unwrap());
        let writer = MemoryWriteTool::new(ws.path(), "one", store.clone());
        let output = writer
            .call(MemoryWriteArgs {
                path: "photo.png".into(),
                content: "  ".into(),
                mode: "write".into(),
                upload_id: Some("upload_test".into()),
            })
            .await
            .unwrap();
        assert!(output.contains("semantic indexing pending"), "{output}");
        assert!(ws.path().join("instances/one/memory/photo.png").is_file());
        assert!(!ws.path().join("instances/one/memory/photo.png.md").exists());
        assert!(store.needs_backfill("one").await.unwrap());
        assert!(store.list_all("one", 10).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn media_sidecar_edits_replace_owner_and_forget_cleans_both() {
        for target in ["photo.png", "Orion"] {
            let mock = MockServer::new(vec![(200, response(vec![1., 0., 0.])); 2]).await;
            let ws = tempfile::tempdir().unwrap();
            upload(ws.path());
            let store = Arc::new(VectorStore::connect_with_config(ws.path(), &mock.config).await);
            let writer = MemoryWriteTool::new(ws.path(), "one", store.clone());
            writer
                .call(MemoryWriteArgs {
                    path: "photo.png".into(),
                    content: "old sky".into(),
                    mode: "write".into(),
                    upload_id: Some("upload_test".into()),
                })
                .await
                .unwrap();
            writer
                .call(MemoryWriteArgs {
                    path: "photo.png.md".into(),
                    content: "Orion nebula".into(),
                    mode: "write".into(),
                    upload_id: None,
                })
                .await
                .unwrap();
            let records = store.list_all("one", 10).await.unwrap();
            assert_eq!(records.len(), 1);
            assert_eq!(records[0].path, "photo.png");
            assert_eq!(records[0].content_preview, "Orion nebula");
            let forget = MemoryForgetTool::new(ws.path(), "one", store.clone());
            let result = forget
                .call(MemoryForgetArgs {
                    target: target.into(),
                })
                .await
                .unwrap();
            let message = if target == "photo.png" {
                "reconciled"
            } else {
                "deleted"
            };
            assert!(result.contains(message), "{result}");
            let dir = ws.path().join("instances/one/memory");
            assert!(!dir.join("photo.png").exists());
            assert!(!dir.join("photo.png.md").exists());
            assert!(store.list_all("one", 10).await.unwrap().is_empty());
        }
    }

    #[tokio::test]
    async fn media_sidecar_deletion_invalidates_owner_and_retains_raw_file() {
        let mock = MockServer::new(vec![(200, response(vec![1., 0., 0.]))]).await;
        let ws = tempfile::tempdir().unwrap();
        upload(ws.path());
        let store = Arc::new(VectorStore::connect_with_config(ws.path(), &mock.config).await);
        store
            .backfill_text_memories(ws.path(), "one")
            .await
            .unwrap();
        let writer = MemoryWriteTool::new(ws.path(), "one", store.clone());
        writer
            .call(MemoryWriteArgs {
                path: "photo.png".into(),
                content: "sky".into(),
                mode: "write".into(),
                upload_id: Some("upload_test".into()),
            })
            .await
            .unwrap();
        let forget = MemoryForgetTool::new(ws.path(), "one", store.clone());
        forget
            .call(MemoryForgetArgs {
                target: "photo.png.md".into(),
            })
            .await
            .unwrap();
        assert!(ws.path().join("instances/one/memory/photo.png").exists());
        assert!(!ws.path().join("instances/one/memory/photo.png.md").exists());
        assert!(store.list_all("one", 10).await.unwrap().is_empty());
        assert!(store.needs_backfill("one").await.unwrap());
    }

    #[tokio::test]
    async fn media_overwrite_without_text_removes_previous_representation() {
        let mock = MockServer::new(vec![(200, response(vec![1., 0., 0.]))]).await;
        let ws = tempfile::tempdir().unwrap();
        upload(ws.path());
        let store = Arc::new(VectorStore::connect_with_config(ws.path(), &mock.config).await);
        let writer = MemoryWriteTool::new(ws.path(), "one", store.clone());
        for content in ["old sky", ""] {
            writer
                .call(MemoryWriteArgs {
                    path: "photo.png".into(),
                    content: content.into(),
                    mode: "write".into(),
                    upload_id: Some("upload_test".into()),
                })
                .await
                .unwrap();
        }
        assert!(!ws.path().join("instances/one/memory/photo.png.md").exists());
        assert!(store.list_all("one", 10).await.unwrap().is_empty());
        assert!(store.needs_backfill("one").await.unwrap());
        assert_eq!(mock.requests.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn media_search_tool_links_root_image_document_audio_and_video() {
        let ws = tempfile::tempdir().unwrap();
        let dir = ws.path().join("instances/one/memory");
        fs::create_dir_all(&dir).unwrap();
        for path in ["photo.png", "paper.pdf", "voice.mp3", "clip.mp4"] {
            fs::write(dir.join(path), [0xff]).unwrap();
            crate::services::media_text::write(&dir, path, "Orion").unwrap();
        }
        let store = Arc::new(VectorStore::connect(ws.path()).await);
        let search = MemorySearchTool::new(ws.path(), "one", store, "https://memory.example");
        let output = search
            .call(MemorySearchArgs {
                query: "Orion".into(),
                limit: Some(10),
            })
            .await
            .unwrap();
        for path in ["photo.png", "paper.pdf", "voice.mp3", "clip.mp4"] {
            assert!(
                output.contains(&format!("/public/memory/one/{path}")),
                "{output}"
            );
        }
    }
    #[tokio::test]
    async fn media_replacement_with_sidecar_persistence_failure_cannot_reuse_old_text() {
        let mock = MockServer::new(vec![(200, response(vec![1., 0., 0.]))]).await;
        let ws = tempfile::tempdir().unwrap();
        upload(ws.path());
        let store = Arc::new(VectorStore::connect_with_config(ws.path(), &mock.config).await);
        let writer = MemoryWriteTool::new(ws.path(), "one", store.clone());
        writer
            .call(MemoryWriteArgs {
                path: "photo.png".into(),
                content: "old sky".into(),
                mode: "write".into(),
                upload_id: Some("upload_test".into()),
            })
            .await
            .unwrap();

        fs::write(
            ws.path().join("instances/one/uploads/raw.png"),
            b"replacement bytes",
        )
        .unwrap();
        store.media_store().inject_next_write_failure();
        let output = writer
            .call(MemoryWriteArgs {
                path: "photo.png".into(),
                content: "new sky".into(),
                mode: "write".into(),
                upload_id: Some("upload_test".into()),
            })
            .await
            .unwrap();

        assert!(output.contains("semantic indexing pending"), "{output}");
        let memory = ws.path().join("instances/one/memory");
        assert!(
            crate::services::media_text::read(&memory, "photo.png")
                .unwrap_err()
                .contains("digest")
        );
        assert!(store.list_all("one", 10).await.unwrap().is_empty());
        assert!(store.search_text("one", "old", 10).await.is_empty());
    }

    #[tokio::test]
    async fn media_write_provider_outage_reports_pending_and_preserves_representation() {
        let mock = MockServer::new(vec![(503, serde_json::json!({"error":"offline"}))]).await;
        let ws = tempfile::tempdir().unwrap();
        upload(ws.path());
        let store = Arc::new(VectorStore::connect_with_config(ws.path(), &mock.config).await);
        let writer = MemoryWriteTool::new(ws.path(), "one", store.clone());
        let output = writer
            .call(MemoryWriteArgs {
                path: "photo.png".into(),
                content: "Orion sky".into(),
                mode: "write".into(),
                upload_id: Some("upload_test".into()),
            })
            .await
            .unwrap();
        assert!(output.contains("semantic indexing pending"));
        assert!(output.contains("HTTP 503"), "{output}");
        assert_eq!(
            crate::services::media_text::read(
                &ws.path().join("instances/one/memory"),
                "photo.png",
            )
            .unwrap(),
            "Orion sky"
        );
        assert!(ws.path().join("instances/one/memory/photo.png").is_file());
        assert!(store.needs_backfill("one").await.unwrap());
        assert!(store.list_all("one", 10).await.unwrap().is_empty());
    }
}
