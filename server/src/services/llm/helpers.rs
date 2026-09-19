use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use crate::domain::chat::{ChatMessage, ChatRole, MessageKind};

use super::types::{ContentBlock, DocumentSource, HistoryEntry, ImageSource, Message};

// ═══════════════════════════════════════════════════════════════════════════
// Rate limit retry
// ═══════════════════════════════════════════════════════════════════════════

const MAX_RETRIES: u32 = 3;
const INITIAL_BACKOFF_MS: u64 = 2000;

fn is_rate_limit_error(msg: &str) -> bool {
    msg.contains("429")
        || msg.contains("rate_limit")
        || msg.contains("Too Many Requests")
        || msg.contains("529")
        || msg.contains("overloaded")
}

pub(crate) async fn retry_on_rate_limit<F, Fut, T>(f: F) -> anyhow::Result<T>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<T>>,
{
    let mut attempt = 0;
    loop {
        match f().await {
            Ok(v) => return Ok(v),
            Err(e) if attempt < MAX_RETRIES && is_rate_limit_error(&e.to_string()) => {
                attempt += 1;
                let delay = INITIAL_BACKOFF_MS * 2u64.pow(attempt - 1);
                log::warn!("Rate limited, retrying in {delay}ms (attempt {attempt}/{MAX_RETRIES})");
                tokio::time::sleep(Duration::from_millis(delay)).await;
            }
            Err(e) => return Err(e),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Real input token cache — populated from Anthropic API responses
// ═══════════════════════════════════════════════════════════════════════════

static REAL_INPUT_TOKENS: Mutex<Option<std::collections::HashMap<String, u64>>> = Mutex::new(None);

/// Cache the real input token count from an Anthropic API response.
pub(crate) fn cache_real_input_tokens(instance_slug: &str, chat_id: &str, tokens: u64) {
    let key = format!("{instance_slug}/{chat_id}");
    let mut guard = REAL_INPUT_TOKENS.lock().unwrap();
    guard
        .get_or_insert_with(std::collections::HashMap::new)
        .insert(key, tokens);
}

/// Retrieve the last real input token count for a given instance/chat.
pub fn get_real_input_tokens(instance_slug: &str, chat_id: &str) -> Option<u64> {
    let key = format!("{instance_slug}/{chat_id}");
    REAL_INPUT_TOKENS
        .lock()
        .unwrap()
        .as_ref()?
        .get(&key)
        .copied()
}

// ═══════════════════════════════════════════════════════════════════════════
// Helpers
// ═══════════════════════════════════════════════════════════════════════════

pub const DEFAULT_ONBOARDING_PROMPT: &str = "\
you are a quiet, thoughtful companion. you speak in lowercase, keep your \
responses short and gentle — one or two sentences at most. you listen more \
than you speak. you're warm but not overbearing. this is a safe, intimate space.";

/// Short summary of a tool use for display.
pub(crate) fn tool_use_summary(name: &str, input: &serde_json::Value) -> String {
    // Extract first meaningful field value for a one-line summary
    if let Some(obj) = input.as_object() {
        if obj.contains_key("command") {
            return format!("{name}: command");
        }
        if obj.contains_key("url") {
            return format!("{name}: URL");
        }
        for key in &["query", "path", "content", "name", "message"] {
            if let Some(val) = obj.get(*key) {
                let owned = val.to_string();
                let s = val.as_str().unwrap_or(&owned);
                let truncated = if s.len() > 80 {
                    let end = s.floor_char_boundary(80);
                    format!("{}…", &s[..end])
                } else {
                    s.to_string()
                };
                return format!("{name}: {truncated}");
            }
        }
    }
    name.to_string()
}

/// Merge timestamps from old entries into a new message list from the LLM.
/// Old entries that match by position keep their ts/id; new entries get fresh values.
/// Strip injected context blocks from user messages before saving to history.
/// Removes [current time: ...] and [system: auto-recalled memories ...] blocks.
pub(crate) fn strip_context_blocks(msg: &Message) -> Message {
    match msg {
        Message::User { content } => {
            let cleaned: Vec<ContentBlock> = content
                .iter()
                .filter(|b| {
                    if let ContentBlock::Text { text } = b {
                        !text.starts_with("[current time:")
                            && !text.starts_with("[system: auto-recalled")
                    } else {
                        true
                    }
                })
                .cloned()
                .collect();
            Message::User {
                content: if cleaned.is_empty() {
                    content.clone()
                } else {
                    cleaned
                },
            }
        }
        other => other.clone(),
    }
}

/// Convert HistoryEntry slice to ChatMessage vec for UI display.
pub fn history_to_chat_messages(entries: &[HistoryEntry]) -> Vec<ChatMessage> {
    let mut out = Vec::new();
    let mut counter = 0u64;
    let mut seen_ids = std::collections::HashSet::new();

    for entry in entries {
        let ts = entry.ts.clone().unwrap_or_else(|| "0".to_string());
        let base_id = entry.id.clone().unwrap_or_else(|| {
            counter += 1;
            format!("h_{counter}")
        });

        let (role, blocks) = match &entry.message {
            Message::User { content } => (ChatRole::User, content),
            Message::Assistant { content } => (ChatRole::Assistant, content),
        };

        let mut block_idx = 0u32;
        for block in blocks {
            let mut block_id = if block_idx == 0 {
                base_id.clone()
            } else {
                format!("{base_id}_{block_idx}")
            };
            block_idx += 1;

            // Ensure uniqueness — append suffix if ID was already emitted
            if !seen_ids.insert(block_id.clone()) {
                let mut dedup = 2u32;
                loop {
                    let candidate = format!("{block_id}_d{dedup}");
                    if seen_ids.insert(candidate.clone()) {
                        block_id = candidate;
                        break;
                    }
                    dedup += 1;
                }
            }

            match block {
                ContentBlock::Text { text } => {
                    if text.is_empty() {
                        continue;
                    }
                    out.push(ChatMessage {
                        id: block_id,
                        role: role.clone(),
                        content: text.clone(),
                        created_at: ts.clone(),
                        kind: MessageKind::Message,
                        tool_name: None,
                        mcp_app_html: None,
                        mcp_app_input: None,
                        model: if role == ChatRole::Assistant {
                            entry.model.clone()
                        } else {
                            None
                        },
                    });
                }
                ContentBlock::ToolCall {
                    name,
                    arguments: input,
                    ..
                } => {
                    let summary = tool_use_summary(name, input);
                    out.push(ChatMessage {
                        id: block_id,
                        role: ChatRole::Assistant,
                        content: summary,
                        created_at: ts.clone(),
                        kind: MessageKind::ToolCall,
                        tool_name: Some(name.clone()),
                        mcp_app_html: entry.mcp_app_html.clone(),
                        mcp_app_input: entry.mcp_app_input.clone(),
                        model: None,
                    });
                }
                ContentBlock::ToolOutput { content, .. } => {
                    let text = match content {
                        super::types::ToolOutputContent::Text(s) => s.clone(),
                        other => other.to_string(),
                    };
                    out.push(ChatMessage {
                        id: block_id,
                        role: ChatRole::Assistant,
                        content: text,
                        created_at: ts.clone(),
                        kind: MessageKind::ToolOutput,
                        tool_name: None,
                        mcp_app_html: None,
                        mcp_app_input: None,
                        model: None,
                    });
                }
                ContentBlock::ContextSummary { content }
                | ContentBlock::LegacyContextSummary {
                    summary: content, ..
                } => {
                    out.push(ChatMessage {
                        id: block_id,
                        role: ChatRole::Assistant,
                        content: content.clone(),
                        created_at: ts.clone(),
                        kind: MessageKind::Compaction,
                        tool_name: None,
                        mcp_app_html: None,
                        mcp_app_input: None,
                        model: None,
                    });
                }
                ContentBlock::Unknown(val) | ContentBlock::ProviderData { data: val, .. } => {
                    // Server tool blocks (web_search, code_execution) — render like regular tools
                    let block_type = val["type"].as_str().unwrap_or("");
                    if block_type == "server_tool_use" {
                        let tool_name = val["name"].as_str().unwrap_or("server_tool");
                        let summary = match tool_name {
                            "web_search" => {
                                let q = val["input"]["query"].as_str().unwrap_or("");
                                if q.is_empty() {
                                    "searching the web".into()
                                } else {
                                    format!("web search: {q}")
                                }
                            }
                            "web_fetch" => {
                                let u = val["input"]["url"].as_str().unwrap_or("");
                                if u.is_empty() {
                                    "fetching web page".into()
                                } else {
                                    format!("fetching {u}")
                                }
                            }
                            "bash_code_execution" | "code_execution" => {
                                "executing code".to_string()
                            }
                            "text_editor_code_execution" => "editing file".to_string(),
                            other => format!("{other}"),
                        };
                        out.push(ChatMessage {
                            id: block_id,
                            role: ChatRole::Assistant,
                            content: summary,
                            created_at: ts.clone(),
                            kind: MessageKind::ToolCall,
                            tool_name: Some(tool_name.to_string()),
                            mcp_app_html: None,
                            mcp_app_input: None,
                            model: None,
                        });
                    } else if block_type.ends_with("_tool_result") {
                        let mut output = String::new();

                        // Web search results — show titles and URLs
                        if block_type == "web_search_tool_result" {
                            if let Some(results) = val["content"].as_array() {
                                for r in results {
                                    let title = r["title"].as_str().unwrap_or("");
                                    let url = r["url"].as_str().unwrap_or("");
                                    if !title.is_empty() {
                                        output.push_str(&format!("- {title}"));
                                        if !url.is_empty() {
                                            output.push_str(&format!(" ({url})"));
                                        }
                                        output.push('\n');
                                    }
                                }
                            }
                        }

                        // Code execution results — show stdout/stderr
                        if output.is_empty() {
                            let stdout = val["content"]["stdout"].as_str().unwrap_or("");
                            let stderr = val["content"]["stderr"].as_str().unwrap_or("");
                            if !stdout.is_empty() {
                                output.push_str(stdout);
                            }
                            if !stderr.is_empty() {
                                if !output.is_empty() {
                                    output.push('\n');
                                }
                                output.push_str(stderr);
                            }
                        }

                        if output.is_empty() {
                            // Skip empty results entirely (encrypted results, etc.)
                            continue;
                        }

                        let truncated: String = output.chars().take(2000).collect();
                        out.push(ChatMessage {
                            id: block_id,
                            role: ChatRole::Assistant,
                            content: truncated,
                            created_at: ts.clone(),
                            kind: MessageKind::ToolOutput,
                            tool_name: None,
                            mcp_app_html: None,
                            mcp_app_input: None,
                            model: None,
                        });
                    }
                    // Other unknown blocks (container_upload, etc.) — skip
                }
                // Image, Document — skip for UI
                _ => {}
            }
        }
    }
    out
}

/// Refresh only explicitly persisted local resource identities before a new turn.
/// URL-shaped strings, including legacy unsigned paths, carry no authority.
pub fn refresh_resource_messages(
    messages: &mut [Message],
    base: &str,
    slug: &str,
    resources: &crate::services::resource_access::ResourceAccess,
) {
    fn renew_block(
        block: &mut ContentBlock,
        base: &str,
        slug: &str,
        resources: &crate::services::resource_access::ResourceAccess,
    ) {
        let (url, provenance) = match block {
            ContentBlock::Image {
                source: ImageSource::Url { url },
                resource_provenance,
            }
            | ContentBlock::Document {
                source: DocumentSource::Url { url },
                resource_provenance,
            } => (url, resource_provenance),
            ContentBlock::ToolOutput {
                content: super::types::ToolOutputContent::Blocks(blocks),
                ..
            } => {
                for block in blocks {
                    renew_block(block, base, slug, resources);
                }
                return;
            }
            _ => return,
        };
        let Some(resource) = provenance
            .as_ref()
            .and_then(|value| value.capability_resource(slug))
        else {
            return;
        };
        if let Ok(fresh) = resources.url(
            base,
            slug,
            resource,
            crate::services::resource_capability::CapabilityAudience::ModelProvider,
        ) {
            *url = fresh;
        }
    }

    for message in messages {
        let blocks = match message {
            Message::User { content } | Message::Assistant { content } => content,
        };
        for block in blocks {
            renew_block(block, base, slug, resources);
        }
    }
}

/// Build a multimodal Message from text + file attachments.
/// Files are referenced via public URL so the LLM provider can fetch them directly.
/// Falls back to inline text for text files when no public URL is configured.
pub fn build_multimodal_prompt(
    text: &str,
    workspace_dir: &Path,
    instance_slug: &str,
    public_url: &str,
    resources: &crate::services::resource_access::ResourceAccess,
    media_store: &crate::services::media_text::MediaStore,
) -> Message {
    let re = regex::Regex::new(r"\[attached:\s*(.+?)\s*\(([^)]+)\)\]").unwrap();

    let mut contents: Vec<ContentBlock> = Vec::new();

    // Images first (with labels) — Claude performs best with images before text
    let caps: Vec<_> = re.captures_iter(text).collect();
    let num_images = caps
        .iter()
        .filter(|c| {
            let uid = &c[2];
            media_store
                .upload_descriptor(instance_slug, uid)
                .ok()
                .map(|m| m.mime_type.starts_with("image/"))
                .unwrap_or(false)
        })
        .count();
    let mut image_idx = 0;

    for cap in &caps {
        let name = &cap[1];
        let upload_id = &cap[2];

        let meta = match media_store.upload_descriptor(instance_slug, upload_id) {
            Ok(meta) => meta,
            Err(error) => {
                log::warn!("attachment {upload_id} is unavailable: {error}");
                continue;
            }
        };

        if meta.mime_type.starts_with("image/") {
            image_idx += 1;
            if num_images > 1 {
                contents.push(ContentBlock::text(&format!("Image {image_idx} ({name}):")));
            }
            if !public_url.is_empty() {
                let url = crate::services::tools::public_file_url(
                    public_url,
                    instance_slug,
                    upload_id,
                    resources,
                );
                contents.push(ContentBlock::Image {
                    source: ImageSource::Url { url: url.clone() },
                    resource_provenance: Some(super::types::ResourceProvenance::uploaded_file(
                        instance_slug,
                        upload_id,
                    )),
                });
                log::info!("attached image (url): {name} ({url})");
            } else {
                log::warn!("image {name}: no public URL configured, skipping");
                contents.push(ContentBlock::text(format!(
                    "[image: {name} — no public URL configured]"
                )));
            }
        } else if meta.mime_type == "application/pdf" {
            if !public_url.is_empty() {
                let url = crate::services::tools::public_file_url(
                    public_url,
                    instance_slug,
                    upload_id,
                    resources,
                );
                contents.push(ContentBlock::Document {
                    source: DocumentSource::Url { url: url.clone() },
                    resource_provenance: Some(super::types::ResourceProvenance::uploaded_file(
                        instance_slug,
                        upload_id,
                    )),
                });
                log::info!("attached PDF (url): {name} ({url})");
            } else {
                log::warn!("PDF {name}: no public URL configured, skipping");
                contents.push(ContentBlock::text(format!(
                    "[PDF: {name} — no public URL configured]"
                )));
            }
        } else if meta.mime_type.starts_with("text/") || meta.mime_type == "application/json" {
            let bytes = match media_store.read_upload_inline(instance_slug, upload_id) {
                Ok((_, bytes)) => bytes,
                Err(error) => {
                    log::warn!("failed to read bounded text attachment {upload_id}: {error}");
                    contents.push(ContentBlock::text(format!(
                        "[text file: {name} — unavailable for inline reading: {error}]"
                    )));
                    continue;
                }
            };
            let text_content = String::from_utf8_lossy(&bytes);
            let truncated: String = text_content.chars().take(10_000).collect();
            contents.push(ContentBlock::text(format!(
                "\n--- {name} ---\n{truncated}\n---"
            )));
            log::info!(
                "attached text file (inline): {name} ({} bytes)",
                bytes.len()
            );
        } else if meta.mime_type == "application/zip" {
            match crate::services::uploads::extract_zip(workspace_dir, instance_slug, upload_id) {
                Ok((extract_dir, files)) => {
                    let mut summary = format!(
                        "\n--- ZIP extracted: {name} ---\n\
                         path: {}\n\
                         {} files:\n",
                        extract_dir.display(),
                        files.len()
                    );
                    for (i, f) in files.iter().enumerate() {
                        if i >= 50 {
                            summary.push_str(&format!("... and {} more files\n", files.len() - 50));
                            break;
                        }
                        summary.push_str(&format!("  {f}\n"));
                    }
                    summary.push_str("---\nUse read_file, write_file, list_files, and run_command with the path above to work with this project.");
                    contents.push(ContentBlock::text(summary));
                    log::info!(
                        "extracted zip: {name} → {} ({} files)",
                        extract_dir.display(),
                        files.len()
                    );
                }
                Err(e) => {
                    contents.push(ContentBlock::text(format!(
                        "[zip: {name} — extraction failed: {e}]"
                    )));
                    log::warn!("failed to extract zip {name}: {e}");
                }
            }
        } else if meta.mime_type.starts_with("video/") || meta.mime_type.starts_with("audio/") {
            // Preserve attachment metadata; video analysis is available via watch_video.
            let kind = if meta.mime_type.starts_with("video/") {
                "video"
            } else {
                "audio"
            };
            let size_mb = meta.size as f64 / (1024.0 * 1024.0);
            let mime = &meta.mime_type;
            let mut description = format!("[{kind}: {name} — {mime}, {size_mb:.1} MB]");
            if kind == "video" {
                description.push_str(
                    "\nto analyze this video, call watch_video for the attached upload.\n\
                    IMPORTANT: in the prompt field, include ALL context you know about this file — \
                    filename, what the user said about it, where it's from, etc. \
                    this context helps the model give a much better analysis.",
                );
            }
            contents.push(ContentBlock::text(description));
            log::info!(
                "attached {kind}: {name} ({}, {size_mb:.1} MB)",
                meta.mime_type
            );
        } else {
            contents.push(ContentBlock::text(format!(
                "[file: {name} — {}, {} bytes, binary format]",
                meta.mime_type, meta.size
            )));
        }
    }

    // Text goes after images (Anthropic best practice: images before text)
    let clean_text = re.replace_all(text, "").trim().to_string();
    if !clean_text.is_empty() {
        contents.push(ContentBlock::text(&clean_text));
    }

    if contents.is_empty() {
        contents.push(ContentBlock::text(text));
    }

    Message::User { content: contents }
}

pub fn load_system_prompt(workspace_dir: &Path, instance_slug: &str) -> String {
    let soul = crate::services::soul::read_soul(workspace_dir, instance_slug);
    if soul.exists && !soul.content.trim().is_empty() {
        soul.content
    } else {
        DEFAULT_ONBOARDING_PROMPT.to_string()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_typed_attachment_provenance_is_renewed_for_the_provider() {
        let resources = crate::services::resource_access::ResourceAccess::new("control-secret");
        let mut messages = vec![super::Message::User { content: vec![
            super::ContentBlock::Image { source: super::ImageSource::Url { url: "https://self.test/resources/browser/files/moon/id?cap=expired".into() }, resource_provenance: None },
            super::ContentBlock::Document { source: super::DocumentSource::Url { url: "https://self.test/public/memory/moon/Folder/R%C3%A9sum%C3%A9.pdf?token=control-secret".into() }, resource_provenance: None },
            super::ContentBlock::Image { source: super::ImageSource::Url { url: "/resources/browser/files/moon/secret".into() }, resource_provenance: None },
            super::ContentBlock::Text { text: "arbitrary /resources/browser/files/moon/secret and /public/files/moon/secret".into() },
        ] }];
        super::refresh_resource_messages(&mut messages, "https://self.test", "moon", &resources);
        let text = serde_json::to_string(&messages).unwrap();
        assert!(text.contains("cap=expired"));
        assert!(text.contains("token=control-secret"));
        assert_eq!(
            text.matches("/resources/browser/files/moon/secret").count(),
            2
        );
        assert!(text.contains("/public/files/moon/secret"));
    }

    #[test]
    fn typed_tool_provenance_roundtrips_and_renews_across_generation_rotation() {
        let resources = crate::services::resource_access::ResourceAccess::new("old-token");
        let output = serde_json::json!([{
            "type": "image",
            "source": { "type": "url", "url": "stale" },
            "resource_provenance": {
                "kind": "uploaded_file",
                "version": 1,
                "slug": "moon",
                "id": "upload_1.png"
            }
        }]);
        let message = super::Message::User {
            content: vec![super::ContentBlock::tool_output(
                "tool-call-1".into(),
                output.to_string(),
                true,
            )],
        };
        let encoded = serde_json::to_string(&message).unwrap();
        assert!(!encoded.contains("old-token"));
        let mut messages = vec![serde_json::from_str(&encoded).unwrap()];
        super::refresh_resource_messages(&mut messages, "https://self.test", "moon", &resources);
        let first = serde_json::to_string(&messages).unwrap();
        resources.replace("new-token");
        super::refresh_resource_messages(&mut messages, "https://self.test", "moon", &resources);
        let second = serde_json::to_string(&messages).unwrap();
        assert_ne!(first, second);
        assert!(second.contains("resource_provenance"));
        assert!(second.contains("/resources/model-provider/files/moon/upload_1.png?cap="));
    }

    #[test]
    fn untrusted_tool_provenance_cannot_renew() {
        let resources = crate::services::resource_access::ResourceAccess::new("control-token");
        let output = serde_json::json!([{
            "type": "image",
            "source": { "type": "url", "url": "https://attacker.invalid/forged" },
            "resource_provenance": {
                "kind": "uploaded_file", "version": 1, "slug": "moon", "id": "guessed"
            }
        }]);
        let mut messages = vec![super::Message::User {
            content: vec![super::ContentBlock::tool_output(
                "malicious".into(),
                output.to_string(),
                false,
            )],
        }];

        super::refresh_resource_messages(&mut messages, "https://self.test", "moon", &resources);
        let encoded = serde_json::to_string(&messages).unwrap();
        assert!(encoded.contains("https://attacker.invalid/forged"));
        assert!(!encoded.contains("resource_provenance"));
        assert!(!encoded.contains("/resources/model-provider/files/moon/guessed"));
    }

    use super::{ContentBlock, Message, build_multimodal_prompt};
    use crate::services::uploads::save_upload;

    #[test]
    fn media_attachment_prompts_preserve_metadata_and_video_guidance() {
        let workspace =
            std::env::temp_dir().join(format!("nolune-media-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&workspace).unwrap();
        let media_store = crate::services::media_text::MediaStore::open(&workspace).unwrap();
        for (name, mime, kind) in [
            ("voice.mp3", "audio/mpeg", "audio"),
            ("voice.wav", "audio/wav", "audio"),
            ("voice.ogg", "audio/ogg", "audio"),
            ("voice.m4a", "audio/mp4", "audio"),
            ("clip.mp4", "video/mp4", "video"),
        ] {
            let upload = save_upload(&workspace, "test", name, b"media fixture").unwrap();
            let message = build_multimodal_prompt(
                &format!("Please review [attached: {name} ({})]", upload.id),
                &workspace,
                "test",
                "",
                &crate::services::resource_access::ResourceAccess::new(""),
                &media_store,
            );
            let Message::User { content } = message else {
                panic!("expected user message")
            };
            let ContentBlock::Text { text } = &content[0] else {
                panic!("expected attachment metadata")
            };
            assert!(text.contains(&format!("[{kind}: {name} — {mime},")));
            assert!(!text.contains("local path:"));
            assert!(!text.contains("listen_music"));
            assert_eq!(text.contains("call watch_video"), kind == "video");
            let ContentBlock::Text { text } = content.last().unwrap() else {
                panic!("expected user text")
            };
            assert_eq!(text, "Please review");
            assert_eq!(
                media_store
                    .read_upload_inline("test", &upload.id)
                    .unwrap()
                    .1,
                b"media fixture"
            );
        }
        std::fs::remove_dir_all(workspace).unwrap();
    }

    #[test]
    fn large_url_backed_image_uses_metadata_without_reading_blob() {
        let workspace = tempfile::tempdir().unwrap();
        let upload = save_upload(workspace.path(), "test", "large.png", b"x").unwrap();
        let uploads = workspace.path().join("instances/test/uploads");
        let large_size = 128_u64 * 1024 * 1024;
        std::fs::OpenOptions::new()
            .write(true)
            .open(uploads.join(&upload.stored_name))
            .unwrap()
            .set_len(large_size)
            .unwrap();
        let metadata_path = uploads.join(format!("{}.json", upload.id));
        let mut metadata: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&metadata_path).unwrap()).unwrap();
        metadata["size"] = serde_json::json!(large_size);
        std::fs::write(&metadata_path, serde_json::to_vec(&metadata).unwrap()).unwrap();
        let media_store = crate::services::media_text::MediaStore::open(workspace.path()).unwrap();

        let message = build_multimodal_prompt(
            &format!("[attached: large.png ({})]", upload.id),
            workspace.path(),
            "test",
            "https://public.invalid",
            &crate::services::resource_access::ResourceAccess::new("control-token"),
            &media_store,
        );
        let serialized = serde_json::to_string(&message).unwrap();
        assert!(serialized.contains("/resources/model-provider/files/test/"));
        assert!(serialized.contains("resource_provenance"));
    }
}
