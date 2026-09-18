use crate::services::tool::{Tool, ToolDefinition};
use schemars::JsonSchema;
use serde::Deserialize;

use super::{ToolExecError, openai_schema};
use crate::services::google::GoogleClient;

// ---------------------------------------------------------------------------
// list_drive_files
// ---------------------------------------------------------------------------

pub struct ListDriveFilesTool {
    google: GoogleClient,
    instance_slug: String,
}

impl ListDriveFilesTool {
    pub fn new(google: GoogleClient, instance_slug: &str) -> Self {
        Self {
            google,
            instance_slug: instance_slug.to_string(),
        }
    }
}

#[derive(Deserialize, JsonSchema)]
pub struct ListDriveFilesArgs {
    /// Search query in Google Drive query syntax (e.g. "name contains 'report'"). Optional.
    pub query: Option<String>,
    /// Folder ID to list files from. Optional.
    pub folder_id: Option<String>,
    /// Number of files to return (default 10, max 50).
    #[serde(default = "default_file_count")]
    pub count: u32,
    /// Email address of the Google account to use. Leave empty to use default.
    pub account: Option<String>,
}

fn default_file_count() -> u32 {
    10
}

impl Tool for ListDriveFilesTool {
    const NAME: &'static str = "list_drive_files";
    type Error = ToolExecError;
    type Args = ListDriveFilesArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "list_drive_files".into(),
            description: "List Google Drive files. Supports search queries and folder filtering."
                .into(),
            parameters: openai_schema::<ListDriveFilesArgs>(),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let (token, _) = self
            .google
            .access_token(&self.instance_slug, args.account.as_deref())
            .await
            .map_err(|e| ToolExecError(e))?;

        let count = args.count.min(50).max(1);
        let client = reqwest::Client::new();

        let mut q_parts: Vec<String> = vec!["trashed = false".into()];
        if let Some(query) = &args.query {
            q_parts.push(query.clone());
        }
        if let Some(folder_id) = &args.folder_id {
            q_parts.push(format!("'{folder_id}' in parents"));
        }
        let q = q_parts.join(" and ");

        let res = client
            .get("https://www.googleapis.com/drive/v3/files")
            .header("Authorization", format!("Bearer {token}"))
            .query(&[
                ("q", q.as_str()),
                ("pageSize", &count.to_string()),
                ("fields", "files(id,name,mimeType,modifiedTime,size)"),
                ("orderBy", "modifiedTime desc"),
            ])
            .send()
            .await
            .map_err(|e| ToolExecError(format!("Drive API failed: {e}")))?;

        if !res.status().is_success() {
            let body = res.text().await.unwrap_or_default();
            return Err(ToolExecError(format!("Drive API error: {body}")));
        }

        let data: serde_json::Value = res
            .json()
            .await
            .map_err(|e| ToolExecError(format!("Drive parse failed: {e}")))?;

        let files = match data["files"].as_array() {
            Some(f) if !f.is_empty() => f,
            _ => return Ok("no files found".into()),
        };

        let mut result = String::new();
        for file in files {
            let name = file["name"].as_str().unwrap_or("?");
            let id = file["id"].as_str().unwrap_or("?");
            let mime = file["mimeType"].as_str().unwrap_or("?");
            let modified = file["modifiedTime"].as_str().unwrap_or("?");
            let size = file["size"].as_str().unwrap_or("");
            result.push_str(&format!(
                "- {name} (id: {id}, type: {mime}, modified: {modified}"
            ));
            if !size.is_empty() {
                if let Ok(bytes) = size.parse::<u64>() {
                    let kb = bytes / 1024;
                    result.push_str(&format!(", {kb} KB"));
                }
            }
            result.push_str(")\n");
        }

        Ok(result)
    }
}

// ---------------------------------------------------------------------------
// read_drive_file
// ---------------------------------------------------------------------------

pub struct ReadDriveFileTool {
    google: GoogleClient,
    instance_slug: String,
}

impl ReadDriveFileTool {
    pub fn new(google: GoogleClient, instance_slug: &str) -> Self {
        Self {
            google,
            instance_slug: instance_slug.to_string(),
        }
    }
}

#[derive(Deserialize, JsonSchema)]
pub struct ReadDriveFileArgs {
    /// The file ID from Google Drive (from list_drive_files output).
    pub file_id: String,
    /// Email address of the Google account to use. Leave empty to use default.
    pub account: Option<String>,
}

impl Tool for ReadDriveFileTool {
    const NAME: &'static str = "read_drive_file";
    type Error = ToolExecError;
    type Args = ReadDriveFileArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "read_drive_file".into(),
            description: "Read a Google Drive file. Docs→text, Sheets→CSV, others→raw content."
                .into(),
            parameters: openai_schema::<ReadDriveFileArgs>(),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let (token, _) = self
            .google
            .access_token(&self.instance_slug, args.account.as_deref())
            .await
            .map_err(|e| ToolExecError(e))?;

        let client = reqwest::Client::new();

        // First get file metadata to check mime type
        let meta_res = client
            .get(&format!(
                "https://www.googleapis.com/drive/v3/files/{}",
                args.file_id
            ))
            .header("Authorization", format!("Bearer {token}"))
            .query(&[("fields", "mimeType,name")])
            .send()
            .await
            .map_err(|e| ToolExecError(format!("Drive metadata failed: {e}")))?;

        if !meta_res.status().is_success() {
            let body = meta_res.text().await.unwrap_or_default();
            return Err(ToolExecError(format!("Drive metadata error: {body}")));
        }

        let meta: serde_json::Value = meta_res.json().await.unwrap_or_default();
        let mime = meta["mimeType"].as_str().unwrap_or("");
        let name = meta["name"].as_str().unwrap_or("?");

        // Google Workspace files need export
        let content = if mime == "application/vnd.google-apps.document" {
            // Export Google Doc as plain text
            let res = client
                .get(&format!(
                    "https://www.googleapis.com/drive/v3/files/{}/export",
                    args.file_id
                ))
                .header("Authorization", format!("Bearer {token}"))
                .query(&[("mimeType", "text/plain")])
                .send()
                .await
                .map_err(|e| ToolExecError(format!("Drive export failed: {e}")))?;
            res.text().await.unwrap_or_default()
        } else if mime == "application/vnd.google-apps.spreadsheet" {
            // Export Google Sheet as CSV
            let res = client
                .get(&format!(
                    "https://www.googleapis.com/drive/v3/files/{}/export",
                    args.file_id
                ))
                .header("Authorization", format!("Bearer {token}"))
                .query(&[("mimeType", "text/csv")])
                .send()
                .await
                .map_err(|e| ToolExecError(format!("Drive export failed: {e}")))?;
            res.text().await.unwrap_or_default()
        } else {
            // Download regular file
            let res = client
                .get(&format!(
                    "https://www.googleapis.com/drive/v3/files/{}?alt=media",
                    args.file_id
                ))
                .header("Authorization", format!("Bearer {token}"))
                .send()
                .await
                .map_err(|e| ToolExecError(format!("Drive download failed: {e}")))?;
            res.text().await.unwrap_or_default()
        };

        // Truncate very large files
        let max_chars = 10_000;
        let truncated = if content.len() > max_chars {
            let truncated_str: String = content.chars().take(max_chars).collect();
            format!(
                "{truncated_str}\n\n...(truncated at {max_chars} chars, total: {})",
                content.len()
            )
        } else {
            content
        };

        Ok(format!("--- {name} ---\n{truncated}"))
    }
}

// ---------------------------------------------------------------------------
// upload_drive_file
// ---------------------------------------------------------------------------

pub struct UploadDriveFileTool {
    google: GoogleClient,
    instance_slug: String,
}

impl UploadDriveFileTool {
    pub fn new(google: GoogleClient, instance_slug: &str) -> Self {
        Self {
            google,
            instance_slug: instance_slug.to_string(),
        }
    }
}

#[derive(Deserialize, JsonSchema)]
pub struct UploadDriveFileArgs {
    /// File name (e.g. "report.txt").
    pub name: String,
    /// File content (text).
    pub content: String,
    /// MIME type (e.g. "text/plain", "text/csv"). Defaults to "text/plain".
    pub mime_type: Option<String>,
    /// Folder ID to upload into. Optional.
    pub folder_id: Option<String>,
    /// Email address of the Google account to use. Leave empty to use default.
    pub account: Option<String>,
}

impl Tool for UploadDriveFileTool {
    const NAME: &'static str = "upload_drive_file";
    type Error = ToolExecError;
    type Args = UploadDriveFileArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "upload_drive_file".into(),
            description: "Upload a text file to Google Drive. Optional MIME type and folder ID."
                .into(),
            parameters: openai_schema::<UploadDriveFileArgs>(),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let (token, _) = self
            .google
            .access_token(&self.instance_slug, args.account.as_deref())
            .await
            .map_err(|e| ToolExecError(e))?;

        let mime = args.mime_type.as_deref().unwrap_or("text/plain");

        let mut metadata = serde_json::json!({
            "name": args.name,
            "mimeType": mime,
        });
        if let Some(folder_id) = &args.folder_id {
            metadata["parents"] = serde_json::json!([folder_id]);
        }

        // Use multipart upload
        let boundary = "nolune_upload_boundary";
        let body = format!(
            "--{boundary}\r\n\
             Content-Type: application/json; charset=UTF-8\r\n\r\n\
             {}\r\n\
             --{boundary}\r\n\
             Content-Type: {mime}\r\n\r\n\
             {}\r\n\
             --{boundary}--",
            serde_json::to_string(&metadata).unwrap_or_default(),
            args.content
        );

        let client = reqwest::Client::new();
        let res = client
            .post("https://www.googleapis.com/upload/drive/v3/files?uploadType=multipart")
            .header("Authorization", format!("Bearer {token}"))
            .header(
                "Content-Type",
                format!("multipart/related; boundary={boundary}"),
            )
            .body(body)
            .send()
            .await
            .map_err(|e| ToolExecError(format!("Drive upload failed: {e}")))?;

        if !res.status().is_success() {
            let body = res.text().await.unwrap_or_default();
            return Err(ToolExecError(format!("Drive upload error: {body}")));
        }

        let data: serde_json::Value = res.json().await.unwrap_or_default();
        let id = data["id"].as_str().unwrap_or("?");
        Ok(format!("file '{}' uploaded to Drive (id: {id})", args.name))
    }
}
