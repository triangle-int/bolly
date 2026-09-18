use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::services::tool::{Tool, ToolDefinition};
use schemars::JsonSchema;
use serde::Deserialize;

use super::{ToolExecError, openai_schema};

// ---------------------------------------------------------------------------
// read_file
// ---------------------------------------------------------------------------

pub struct ReadFileTool {
    pub(super) instance_dir: PathBuf,
    instance_slug: String,
    public_url: String,
    auth_token: String,
}

impl ReadFileTool {
    pub fn new(workspace_dir: &Path, instance_slug: &str, public_url: &str) -> Self {
        Self {
            instance_dir: workspace_dir.join("instances").join(instance_slug),
            instance_slug: instance_slug.to_string(),
            public_url: public_url.to_string(),
            auth_token: std::env::var("NOLUNE_AUTH_TOKEN").unwrap_or_default(),
        }
    }

    /// Try to extract upload_id from a file path in uploads/ directory.
    fn extract_upload_id(path: &Path) -> Option<String> {
        let path_str = path.to_string_lossy();
        if !path_str.contains("/uploads/") {
            return None;
        }
        let fname = path.file_stem()?.to_str()?;
        Some(fname.trim_end_matches("_blob").to_string())
    }
}

/// Arguments for read_file tool.
#[derive(Deserialize, JsonSchema)]
pub struct ReadFileArgs {
    /// File path. Can be relative to instance directory (e.g. "soul.md") or absolute (e.g. "/Users/timur/projects/app/src/main.rs").
    pub path: String,
    /// Starting line number (1-based). Omit to start from the beginning.
    pub offset: Option<usize>,
    /// Maximum number of lines to read. Omit to read the whole file (up to the size limit).
    pub limit: Option<usize>,
}

impl Tool for ReadFileTool {
    const NAME: &'static str = "read_file";
    type Error = ToolExecError;
    type Args = ReadFileArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "read_file".into(),
            description: "Read a file by relative or absolute path. Use offset/limit for large files (>20k chars truncated).".into(),
            parameters: openai_schema::<ReadFileArgs>(),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let target = if args.path.starts_with('/') {
            PathBuf::from(&args.path)
        } else {
            self.instance_dir.join(&args.path)
        };

        if !target.exists() {
            return Err(ToolExecError(format!(
                "{}: file not found",
                target.display()
            )));
        }

        let ext = target
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        // Image files — return as content block
        if matches!(ext.as_str(), "jpg" | "jpeg" | "png" | "webp" | "gif") {
            // Try URL if in uploads/
            if let Some(upload_id) = Self::extract_upload_id(&target) {
                if !self.public_url.is_empty() {
                    let url = super::public_file_url(
                        &self.public_url,
                        &self.instance_slug,
                        &upload_id,
                        &self.auth_token,
                    );
                    return Ok(serde_json::to_string(&serde_json::json!([
                        {"type": "image", "source": {"type": "url", "url": url}}
                    ]))
                    .unwrap());
                }
            }
            // Fallback — base64
            let bytes = fs::read(&target)
                .map_err(|e| ToolExecError(format!("{}: {e}", target.display())))?;
            if bytes.len() > 5 * 1024 * 1024 {
                return Err(ToolExecError("image too large (>5MB)".into()));
            }
            use base64::Engine;
            let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
            let mime = match ext.as_str() {
                "png" => "image/png",
                "gif" => "image/gif",
                "webp" => "image/webp",
                _ => "image/jpeg",
            };
            return Ok(serde_json::to_string(&serde_json::json!([
                {"type": "image", "source": {"type": "base64", "media_type": mime, "data": b64}}
            ]))
            .unwrap());
        }

        // PDF files — document content block
        if ext == "pdf" {
            // Try URL
            if let Some(upload_id) = Self::extract_upload_id(&target) {
                if !self.public_url.is_empty() {
                    let url = super::public_file_url(
                        &self.public_url,
                        &self.instance_slug,
                        &upload_id,
                        &self.auth_token,
                    );
                    return Ok(serde_json::to_string(&serde_json::json!([
                        {"type": "document", "source": {"type": "url", "url": url}}
                    ]))
                    .unwrap());
                }
            }
            // Fallback — base64
            let bytes = fs::read(&target)
                .map_err(|e| ToolExecError(format!("{}: {e}", target.display())))?;
            if bytes.len() > 32 * 1024 * 1024 {
                return Err(ToolExecError("PDF too large (>32MB)".into()));
            }
            use base64::Engine;
            let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
            return Ok(serde_json::to_string(&serde_json::json!([
                {"type": "document", "source": {"type": "base64", "media_type": "application/pdf", "data": b64}}
            ])).unwrap());
        }

        // Text files — original behavior
        let raw = fs::read_to_string(&target)
            .map_err(|e| ToolExecError(format!("{}: {e}", target.display())))?;

        let total_lines = raw.lines().count();

        let content: String = match (args.offset, args.limit) {
            (Some(off), Some(lim)) => {
                let start = off.saturating_sub(1);
                raw.lines()
                    .skip(start)
                    .take(lim)
                    .collect::<Vec<_>>()
                    .join("\n")
            }
            (Some(off), None) => {
                let start = off.saturating_sub(1);
                raw.lines().skip(start).collect::<Vec<_>>().join("\n")
            }
            (None, Some(lim)) => raw.lines().take(lim).collect::<Vec<_>>().join("\n"),
            (None, None) => raw,
        };

        const MAX_CHARS: usize = 20_000;
        if content.len() > MAX_CHARS {
            let truncated: String = content.chars().take(MAX_CHARS).collect();
            Ok(format!(
                "{truncated}\n\n...(truncated at {MAX_CHARS} chars, total: {} chars, {total_lines} lines — use offset/limit to read specific sections)",
                content.len()
            ))
        } else if args.offset.is_some() || args.limit.is_some() {
            Ok(format!("{content}\n\n({total_lines} lines total in file)"))
        } else {
            Ok(content)
        }
    }
}

// ---------------------------------------------------------------------------
// write_file
// ---------------------------------------------------------------------------

pub struct WriteFileTool {
    instance_dir: PathBuf,
}

impl WriteFileTool {
    pub fn new(workspace_dir: &Path, instance_slug: &str) -> Self {
        Self {
            instance_dir: workspace_dir.join("instances").join(instance_slug),
        }
    }
}

/// Arguments for write_file tool.
#[derive(Deserialize, JsonSchema)]
pub struct WriteFileArgs {
    /// File path. Relative for instance workspace (e.g. "drops/idea.md") or absolute (e.g. "/Users/timur/projects/app/src/main.rs"). Parent directories are created automatically.
    pub path: String,
    /// The full content to write to the file.
    pub content: String,
}

impl Tool for WriteFileTool {
    const NAME: &'static str = "write_file";
    type Error = ToolExecError;
    type Args = WriteFileArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "write_file".into(),
            description: "Write or overwrite a file. Relative or absolute path. Parent dirs created automatically.".into(),
            parameters: openai_schema::<WriteFileArgs>(),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let target = if args.path.starts_with('/') {
            PathBuf::from(&args.path)
        } else {
            self.instance_dir.join(&args.path)
        };

        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| ToolExecError(e.to_string()))?;
        }

        fs::write(&target, &args.content).map_err(|e| ToolExecError(e.to_string()))?;
        Ok(format!(
            "wrote {} bytes to {}",
            args.content.len(),
            args.path
        ))
    }
}

// ---------------------------------------------------------------------------
// edit_file
// ---------------------------------------------------------------------------

pub struct EditFileTool {
    instance_dir: PathBuf,
}

impl EditFileTool {
    pub fn new(workspace_dir: &Path, instance_slug: &str) -> Self {
        Self {
            instance_dir: workspace_dir.join("instances").join(instance_slug),
        }
    }
}

/// Arguments for edit_file tool.
#[derive(Deserialize, JsonSchema)]
pub struct EditFileArgs {
    /// File path. Relative for instance workspace or absolute (starting with /).
    pub path: String,
    /// The exact string to find in the file. Must match exactly (including whitespace and indentation). Must be unique within the file — if it appears more than once, provide more surrounding context to disambiguate.
    pub old_string: String,
    /// The replacement string. Must be different from old_string.
    pub new_string: String,
}

impl Tool for EditFileTool {
    const NAME: &'static str = "edit_file";
    type Error = ToolExecError;
    type Args = EditFileArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "edit_file".into(),
            description: "Edit a file by exact string replacement. old_string must be unique; add context if not.".into(),
            parameters: openai_schema::<EditFileArgs>(),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let target = if args.path.starts_with('/') {
            PathBuf::from(&args.path)
        } else {
            self.instance_dir.join(&args.path)
        };

        if args.old_string == args.new_string {
            return Err(ToolExecError(
                "old_string and new_string are identical".into(),
            ));
        }

        let content = fs::read_to_string(&target)
            .map_err(|e| ToolExecError(format!("{}: {e}", target.display())))?;

        let count = content.matches(&args.old_string).count();
        if count == 0 {
            return Err(ToolExecError(
                "old_string not found in file. Make sure it matches exactly, \
                 including whitespace and indentation. Use read_file to check \
                 the current content."
                    .into(),
            ));
        }
        if count > 1 {
            return Err(ToolExecError(format!(
                "old_string appears {count} times in file. Include more surrounding \
                 context to make it unique."
            )));
        }

        let new_content = content.replacen(&args.old_string, &args.new_string, 1);
        fs::write(&target, &new_content).map_err(|e| ToolExecError(e.to_string()))?;

        let old_lines = args.old_string.lines().count();
        let new_lines = args.new_string.lines().count();
        Ok(format!(
            "edited {} — replaced {old_lines} lines with {new_lines} lines",
            args.path
        ))
    }
}

// ---------------------------------------------------------------------------
// list_files
// ---------------------------------------------------------------------------

pub struct ListFilesTool {
    instance_dir: PathBuf,
}

impl ListFilesTool {
    pub fn new(workspace_dir: &Path, instance_slug: &str) -> Self {
        Self {
            instance_dir: workspace_dir.join("instances").join(instance_slug),
        }
    }
}

/// Arguments for list_files tool.
#[derive(Deserialize, JsonSchema)]
pub struct ListFilesArgs {
    /// Directory path. Absolute (e.g. "/Users/timur/projects/app/src") or relative to instance directory. Omit to list instance root.
    pub path: Option<String>,
}

impl Tool for ListFilesTool {
    const NAME: &'static str = "list_files";
    type Error = ToolExecError;
    type Args = ListFilesArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "list_files".into(),
            description: "List files and directories. Relative or absolute path.".into(),
            parameters: openai_schema::<ListFilesArgs>(),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let target = match &args.path {
            Some(p) if p.starts_with('/') => PathBuf::from(p),
            Some(p) if !p.is_empty() => self.instance_dir.join(p),
            _ => self.instance_dir.clone(),
        };

        if !target.is_dir() {
            return Err(ToolExecError(format!(
                "{} is not a directory",
                args.path.as_deref().unwrap_or(".")
            )));
        }

        let mut entries: Vec<String> = fs::read_dir(&target)
            .map_err(|e| ToolExecError(e.to_string()))?
            .filter_map(Result::ok)
            .map(|entry| {
                let name = entry.file_name().to_string_lossy().to_string();
                if entry.path().is_dir() {
                    format!("{name}/")
                } else {
                    name
                }
            })
            .collect();

        entries.sort();
        Ok(entries.join("\n"))
    }
}

// ---------------------------------------------------------------------------
// upload_file — save a local file as an upload, return public URL
// ---------------------------------------------------------------------------

pub struct UploadFileTool {
    workspace_dir: PathBuf,
    instance_slug: String,
    public_url: String,
    auth_token: String,
}

impl UploadFileTool {
    pub fn new(workspace_dir: &Path, instance_slug: &str, public_url: &str) -> Self {
        Self {
            workspace_dir: workspace_dir.to_path_buf(),
            instance_slug: instance_slug.to_string(),
            public_url: public_url.to_string(),
            auth_token: std::env::var("NOLUNE_AUTH_TOKEN").unwrap_or_default(),
        }
    }
}

#[derive(Deserialize, JsonSchema)]
pub struct UploadFileArgs {
    /// Absolute path to the file on the local filesystem (e.g. "/tmp/video.mp4").
    pub file_path: String,
    /// Optional friendly name for the file. Defaults to the original filename.
    pub name: Option<String>,
}

impl Tool for UploadFileTool {
    const NAME: &'static str = "share_file";
    type Error = ToolExecError;
    type Args = UploadFileArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "share_file".into(),
            description: "Upload a local file and get a public URL. Works with any file size up to 500MB. Use this to share files with the user.".into(),
            parameters: openai_schema::<UploadFileArgs>(),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let path = Path::new(&args.file_path);
        if !path.exists() {
            return Err(ToolExecError(format!("{}: file not found", args.file_path)));
        }
        if !path.is_file() {
            return Err(ToolExecError(format!("{}: not a file", args.file_path)));
        }

        let bytes = fs::read(path)
            .map_err(|e| ToolExecError(format!("failed to read {}: {e}", args.file_path)))?;

        let name = args.name.unwrap_or_else(|| {
            path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("file")
                .to_string()
        });

        let meta = crate::services::uploads::save_upload(
            &self.workspace_dir,
            &self.instance_slug,
            &name,
            &bytes,
        )
        .map_err(|e| ToolExecError(format!("upload failed: {e}")))?;

        if self.public_url.is_empty() {
            return Ok(format!(
                "uploaded as {} ({} bytes) but no public URL configured",
                meta.id,
                bytes.len()
            ));
        }

        let url = super::public_file_url(
            &self.public_url,
            &self.instance_slug,
            &meta.id,
            &self.auth_token,
        );

        let size_mb = bytes.len() as f64 / 1024.0 / 1024.0;
        Ok(format!("{url}\n\nuploaded: {name} ({size_mb:.1} MB)"))
    }
}
