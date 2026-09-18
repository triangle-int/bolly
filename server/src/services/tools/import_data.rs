use std::path::{Path, PathBuf};
use std::sync::Arc;

use schemars::JsonSchema;
use serde::Deserialize;
use tokio::sync::broadcast;

use super::{ToolExecError, openai_schema};
use crate::domain::events::ServerEvent;
use crate::services::{memory_import, tool::*, vector::VectorStore};

pub struct ImportDataTool {
    workspace_dir: PathBuf,
    instance_slug: String,
    http: reqwest::Client,
    api_key: String,
    events: broadcast::Sender<ServerEvent>,
    vector_store: Arc<VectorStore>,
    google_ai_key: String,
}

impl ImportDataTool {
    pub fn new(
        workspace_dir: &Path,
        instance_slug: &str,
        http: reqwest::Client,
        api_key: &str,
        events: broadcast::Sender<ServerEvent>,
        vector_store: Arc<VectorStore>,
        google_ai_key: &str,
    ) -> Self {
        Self {
            workspace_dir: workspace_dir.to_path_buf(),
            instance_slug: instance_slug.to_string(),
            http,
            api_key: api_key.to_string(),
            events,
            vector_store,
            google_ai_key: google_ai_key.to_string(),
        }
    }
}

#[derive(Deserialize, JsonSchema)]
pub struct ImportDataArgs {
    /// Path to a file or directory to import. Can be an uploaded file, a directory
    /// of files, or a specific JSON/text file. The system will parse the data,
    /// extract personal information, and add it to your memory library.
    pub path: String,
}

impl Tool for ImportDataTool {
    const NAME: &'static str = "import_knowledge";
    type Error = ToolExecError;
    type Args = ImportDataArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "import_knowledge".into(),
            description: "Import external data into your memory library. \
                Accepts a path to files (JSON, text, markdown, CSV) — for example a Claude \
                chat export, notes, or any personal data dump. The system uses AI to extract \
                personal facts and organize them into your memory. This runs in the background; \
                progress is shown via notifications."
                .into(),
            parameters: openai_schema::<ImportDataArgs>(),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let source = PathBuf::from(args.path.trim());

        // Resolve upload IDs
        let resolved = if args.path.starts_with("upload_") {
            let uploads_dir = self
                .workspace_dir
                .join("instances")
                .join(&self.instance_slug)
                .join("uploads");
            // Find the actual file
            let mut found = None;
            if let Ok(entries) = std::fs::read_dir(&uploads_dir) {
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if name.starts_with(args.path.trim()) {
                        found = Some(entry.path());
                        break;
                    }
                }
            }
            found.unwrap_or(source)
        } else {
            source
        };

        if !resolved.exists() {
            return Err(ToolExecError(format!(
                "path not found: {}",
                resolved.display()
            )));
        }

        // Copy files to a temp import dir
        let import_dir = self
            .workspace_dir
            .join("instances")
            .join(&self.instance_slug)
            .join(".import_temp");
        let _ = std::fs::remove_dir_all(&import_dir);
        std::fs::create_dir_all(&import_dir)
            .map_err(|e| ToolExecError(format!("failed to create import dir: {e}")))?;

        if resolved.is_dir() {
            // Copy directory contents
            copy_dir_recursive(&resolved, &import_dir)
                .map_err(|e| ToolExecError(format!("failed to copy: {e}")))?;
        } else {
            // Copy single file
            let name = resolved.file_name().unwrap_or_default();
            std::fs::copy(&resolved, import_dir.join(name))
                .map_err(|e| ToolExecError(format!("failed to copy: {e}")))?;
        }

        // Spawn the background import pipeline
        memory_import::spawn_import(
            self.http.clone(),
            self.api_key.clone(),
            self.workspace_dir.clone(),
            self.instance_slug.clone(),
            import_dir,
            self.events.clone(),
            self.vector_store.clone(),
            self.google_ai_key.clone(),
        );

        Ok(
            "import started — I'll process the data in the background and extract \
            personal facts into my memory library. this may take a few minutes \
            depending on the data size."
                .to_string(),
        )
    }
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if src_path.is_dir() {
            std::fs::create_dir_all(&dst_path)?;
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}
