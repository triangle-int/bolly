use crate::services::tool::{Tool, ToolDefinition};
use schemars::JsonSchema;
use serde::Deserialize;

use super::{ToolExecError, openai_schema};

pub struct ViewImageTool;

#[derive(Deserialize, JsonSchema)]
pub struct ViewImageArgs {
    /// URL of the image to view. Supports jpg, png, gif, webp.
    pub url: String,
}

impl Tool for ViewImageTool {
    const NAME: &'static str = "view_image";
    type Error = ToolExecError;
    type Args = ViewImageArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "view_image".into(),
            description: "View an image from a URL. Claude fetches the image directly — \
                no download needed. Use this when you need to examine an image. \
                The image is NOT shown to the user — to share images, \
                include ![description](url) in your response text."
                .into(),
            parameters: openai_schema::<ViewImageArgs>(),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let url = args.url.trim();
        if url.is_empty() {
            return Err(ToolExecError("url cannot be empty".into()));
        }

        // Quick HEAD request to validate URL is accessible and is an image
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .map_err(|e| ToolExecError(format!("http client error: {e}")))?;

        let resp = client
            .head(url)
            .send()
            .await
            .map_err(|e| ToolExecError(format!("failed to reach image URL: {e}")))?;

        if !resp.status().is_success() {
            return Err(ToolExecError(format!("HTTP {}", resp.status())));
        }

        log::info!("[view_image] URL-based: {url}");

        // Return URL-based image content block — Claude fetches it directly
        Ok(serde_json::to_string(&serde_json::json!([
            {"type": "image", "source": {"type": "url", "url": url}}
        ]))
        .unwrap())
    }
}
