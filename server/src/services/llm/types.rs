// ═══════════════════════════════════════════════════════════════════════════
// Canonical conversation types. Persisted tags remain stable for old histories.
// Only provider adapters translate these into API payloads.
// ═══════════════════════════════════════════════════════════════════════════

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(tag = "role")]
pub enum Message {
    #[serde(rename = "user")]
    User { content: Vec<ContentBlock> },
    #[serde(rename = "assistant")]
    Assistant { content: Vec<ContentBlock> },
}

impl Message {
    pub fn user(text: impl Into<String>) -> Self {
        Message::User {
            content: vec![ContentBlock::text(text)],
        }
    }

    pub fn assistant(text: impl Into<String>) -> Self {
        Message::Assistant {
            content: vec![ContentBlock::text(text)],
        }
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type")]
pub enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image")]
    Image {
        source: ImageSource,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        resource_provenance: Option<ResourceProvenance>,
    },
    #[serde(rename = "document")]
    Document {
        source: DocumentSource,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        resource_provenance: Option<ResourceProvenance>,
    },
    #[serde(rename = "tool_use")]
    ToolCall {
        id: String,
        name: String,
        #[serde(rename = "input", alias = "arguments")]
        arguments: serde_json::Value,
    },
    #[serde(rename = "tool_result")]
    ToolOutput {
        #[serde(rename = "tool_use_id", alias = "call_id")]
        call_id: String,
        content: ToolOutputContent,
    },
    #[serde(rename = "compaction")]
    ContextSummary { content: String },
    /// Opaque continuation metadata is scoped to the originating provider.
    #[serde(rename = "provider_data")]
    ProviderData {
        provider: crate::config::LlmProvider,
        data: serde_json::Value,
    },
    /// Older histories used the key "summary"; retain that spelling on save.
    #[serde(untagged)]
    LegacyContextSummary {
        #[serde(rename = "type")]
        kind: SummaryTag,
        summary: String,
    },
    /// Legacy catch-all for unknown content block types (e.g. from newer API versions).
    /// Preserves the raw JSON so it can be serialized back without data loss.
    #[serde(untagged)]
    Unknown(serde_json::Value),
}

// Strict input-only schema for tool-produced multimodal blocks. The persisted
// `ContentBlock` schema remains permissive so old/unknown history round-trips,
// but tool output must not silently discard domain fields through Serde's
// default unknown-field behavior.
#[derive(serde::Deserialize)]
#[serde(tag = "type", deny_unknown_fields)]
enum StrictToolOutputBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image")]
    Image {
        source: StrictSource,
        #[serde(default)]
        resource_provenance: Option<ResourceProvenance>,
    },
    #[serde(rename = "document")]
    Document {
        source: StrictSource,
        #[serde(default)]
        resource_provenance: Option<ResourceProvenance>,
    },
}

#[derive(serde::Deserialize)]
#[serde(tag = "type", deny_unknown_fields)]
enum StrictSource {
    #[serde(rename = "base64")]
    Base64 { media_type: String, data: String },
    #[serde(rename = "url")]
    Url { url: String },
}

impl StrictSource {
    fn into_image(self) -> ImageSource {
        match self {
            Self::Base64 { media_type, data } => ImageSource::Base64 { media_type, data },
            Self::Url { url } => ImageSource::Url { url },
        }
    }

    fn into_document(self) -> DocumentSource {
        match self {
            Self::Base64 { media_type, data } => DocumentSource::Base64 { media_type, data },
            Self::Url { url } => DocumentSource::Url { url },
        }
    }
}

impl StrictToolOutputBlock {
    fn into_content(self, trusted_resource_provenance: bool) -> ContentBlock {
        match self {
            Self::Text { text } => ContentBlock::Text { text },
            Self::Image {
                source,
                resource_provenance,
            } => ContentBlock::Image {
                source: source.into_image(),
                resource_provenance: trusted_resource_provenance
                    .then_some(resource_provenance)
                    .flatten(),
            },
            Self::Document {
                source,
                resource_provenance,
            } => ContentBlock::Document {
                source: source.into_document(),
                resource_provenance: trusted_resource_provenance
                    .then_some(resource_provenance)
                    .flatten(),
            },
        }
    }
}

impl ContentBlock {
    pub fn text(text: impl Into<String>) -> Self {
        ContentBlock::Text { text: text.into() }
    }

    pub fn tool_output(
        call_id: String,
        content: String,
        trusted_resource_provenance: bool,
    ) -> Self {
        // ToolDyn blanket impl wraps String output via serde_json::to_string,
        // which adds JSON quotes: `[...]` becomes `"[...]"`. Unwrap that layer.
        let inner = if content.starts_with('"') && content.ends_with('"') {
            serde_json::from_str::<String>(&content).unwrap_or(content)
        } else {
            content
        };

        // Only fully valid multimodal blocks are interpreted structurally.
        // `ContentBlock::Unknown` deliberately accepts arbitrary JSON for history
        // compatibility, so every decoded element must also be one of the block
        // kinds providers permit inside tool output. Ordinary domain objects such
        // as `[{"type":"expense","amount":1}]` remain exact text.
        let content = serde_json::from_str::<Vec<StrictToolOutputBlock>>(&inner)
            .ok()
            .filter(|blocks| !blocks.is_empty())
            .map(|blocks| {
                blocks
                    .into_iter()
                    .map(|block| block.into_content(trusted_resource_provenance))
                    .collect()
            })
            .map(ToolOutputContent::Blocks)
            .unwrap_or(ToolOutputContent::Text(inner));
        ContentBlock::ToolOutput { call_id, content }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// HistoryEntry — wraps Message with timestamps/IDs for unified history
// ═══════════════════════════════════════════════════════════════════════════

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct HistoryEntry {
    #[serde(flatten)]
    pub message: Message,
    /// Timestamp in millis since epoch (as string).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ts: Option<String>,
    /// Stable ID for client dedup.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// HTML content for MCP App rendering.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_app_html: Option<String>,
    /// Tool input JSON for MCP App rendering.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_app_input: Option<String>,
    /// Model name used to generate this message (assistant only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

impl HistoryEntry {
    /// Wrap a Message with timestamp and ID.
    pub fn new(message: Message, ts: String, id: String) -> Self {
        Self {
            message,
            ts: Some(ts),
            id: Some(id),
            mcp_app_html: None,
            mcp_app_input: None,
            model: None,
        }
    }

    /// Extract just the Messages from a slice of entries.
    pub fn to_messages(entries: &[HistoryEntry]) -> Vec<Message> {
        entries.iter().map(|e| e.message.clone()).collect()
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type")]
pub enum ImageSource {
    #[serde(rename = "base64")]
    Base64 { media_type: String, data: String },
    #[serde(rename = "url")]
    Url { url: String },
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type")]
pub enum DocumentSource {
    #[serde(rename = "base64")]
    Base64 { media_type: String, data: String },
    #[serde(rename = "url")]
    Url { url: String },
}

/// Stable, non-authorizing identity for a persisted local resource. Capability
/// URLs are deliberately excluded so histories survive signing-key rotation.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ResourceProvenance {
    UploadedFile {
        version: u8,
        slug: String,
        id: String,
    },
    MemoryPath {
        version: u8,
        slug: String,
        path: String,
    },
}

impl ResourceProvenance {
    pub fn uploaded_file(slug: impl Into<String>, id: impl Into<String>) -> Self {
        Self::UploadedFile {
            version: 1,
            slug: slug.into(),
            id: id.into(),
        }
    }

    pub(crate) fn capability_resource(
        &self,
        expected_slug: &str,
    ) -> Option<crate::services::resource_capability::CapabilityResource> {
        use crate::services::resource_capability::CapabilityResource;
        match self {
            Self::UploadedFile {
                version: 1,
                slug,
                id,
            } if slug == expected_slug => CapabilityResource::uploaded_file(id.clone()).ok(),
            Self::MemoryPath {
                version: 1,
                slug,
                path,
            } if slug == expected_slug => CapabilityResource::memory(path.clone()).ok(),
            _ => None,
        }
    }
}

/// Result of a tool-using LLM call.
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct ToolChatResult {
    pub text: String,
    /// Full message history including tool call/result entries.
    pub rig_history: Option<Vec<Message>>,
    /// The message ID used during streaming (so the saved message can reuse it).
    pub message_id: Option<String>,
    /// Cost-normalized token units across all turns (existing usage accounting).
    pub tokens_used: u64,
}

// ═══════════════════════════════════════════════════════════════════════════
// LlmBackend — configured facade over provider adapters
// ═══════════════════════════════════════════════════════════════════════════

pub(crate) const ANTHROPIC_BASE_URL: &str = "https://api.anthropic.com";
pub(crate) const OPENAI_BASE_URL: &str = "https://api.openai.com";

#[derive(Clone)]
pub struct LlmBackend {
    pub profile: crate::config::ProviderProfile,
    pub http: reqwest::Client,
    pub api_key: String,
    pub model: String,
    /// Base URL for the selected adapter.
    pub base_url: String,
    /// Provider identity; unsupported legacy selections remain typed setup errors.
    pub provider: crate::config::LlmProvider,
}

#[derive(Debug)]
pub(crate) struct ToolCall {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) arguments: serde_json::Value,
}

/// Canonical result of a completion or streaming turn.
#[derive(Debug)]
pub struct LlmResponse {
    pub(crate) text: String,
    pub(crate) tool_calls: Vec<ToolCall>,
    pub(crate) stop_reason: super::contract::StopReason,
    pub(crate) usage: super::contract::Usage,
    pub(crate) tokens_used: u64,
    /// Content blocks in the order they arrived from the API.
    /// Preserves interleaving of text, server_tool_use, and server_tool_result.
    pub(crate) ordered_content: Vec<ContentBlock>,
}

/// Typed text or multimodal output. Untagged serde preserves historical JSON.
#[derive(Clone, Debug, serde::Serialize)]
#[serde(untagged)]
pub enum ToolOutputContent {
    Text(String),
    Blocks(Vec<ContentBlock>),
    /// Preserve unusual legacy payloads without interpreting them as content blocks.
    Legacy(serde_json::Value),
}

impl<'de> serde::Deserialize<'de> for ToolOutputContent {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        if let serde_json::Value::String(text) = value {
            return Ok(Self::Text(text));
        }

        if let Ok(blocks) = serde_json::from_value::<Vec<StrictToolOutputBlock>>(value.clone()) {
            if !blocks.is_empty() {
                return Ok(Self::Blocks(
                    blocks
                        .into_iter()
                        // Persisted history has already crossed the trusted
                        // execution boundary. Preserve typed provenance so it
                        // can renew after a signing-key rotation or restart.
                        .map(|block| block.into_content(true))
                        .collect(),
                ));
            }
        }

        Ok(Self::Legacy(value))
    }
}
impl ToolOutputContent {
    pub fn as_str(&self) -> Option<&str> {
        if let Self::Text(s) = self {
            Some(s)
        } else {
            None
        }
    }
}
impl std::fmt::Display for ToolOutputContent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Text(s) => f.write_str(s),
            _ => write!(
                f,
                "{}",
                serde_json::to_string(self).map_err(|_| std::fmt::Error)?
            ),
        }
    }
}

impl ToolCall {
    pub(crate) fn required_string(
        value: &serde_json::Value,
        key: &str,
    ) -> Result<String, super::contract::LlmError> {
        value[key]
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .map(str::to_owned)
            .ok_or_else(|| {
                super::contract::LlmError::InvalidResponse(format!("missing or empty tool {key}"))
            })
    }
    pub(crate) fn validate_arguments(
        value: &serde_json::Value,
    ) -> Result<(), super::contract::LlmError> {
        if value.is_object() {
            Ok(())
        } else {
            Err(super::contract::LlmError::InvalidResponse(
                "tool arguments must be a JSON object".into(),
            ))
        }
    }
    pub(crate) fn parse_arguments(
        value: &str,
    ) -> Result<serde_json::Value, super::contract::LlmError> {
        let value = serde_json::from_str(value).map_err(|e| {
            super::contract::LlmError::InvalidResponse(format!("invalid tool arguments: {e}"))
        })?;
        Self::validate_arguments(&value)?;
        Ok(value)
    }
    pub(crate) fn from_json_strings(
        value: &serde_json::Value,
        id: &str,
        args: &str,
    ) -> Result<Self, super::contract::LlmError> {
        Ok(Self {
            id: Self::required_string(value, id)?,
            name: Self::required_string(value, "name")?,
            arguments: Self::parse_arguments(&Self::required_string(value, args)?)?,
        })
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub enum SummaryTag {
    #[serde(rename = "compaction")]
    Summary,
}
