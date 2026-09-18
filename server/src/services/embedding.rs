//! Gemini Embedding 2 client — generates embeddings for text, images, video, and audio.

use base64::Engine;
use serde::{Deserialize, Serialize};

const EMBED_URL: &str = "https://generativelanguage.googleapis.com/v1beta/models/gemini-embedding-2-preview:embedContent";

/// Default output dimensionality (MRL-truncated from 3072 for efficiency).
const OUTPUT_DIM: u32 = 768;

/// Task type for embedding — affects how the model optimizes the vector.
#[derive(Debug, Clone, Copy)]
pub enum TaskType {
    /// Use when indexing documents/memories for later retrieval.
    RetrievalDocument,
    /// Use when embedding a search query.
    RetrievalQuery,
}

impl TaskType {
    fn as_str(&self) -> &'static str {
        match self {
            TaskType::RetrievalDocument => "RETRIEVAL_DOCUMENT",
            TaskType::RetrievalQuery => "RETRIEVAL_QUERY",
        }
    }
}

// --- Request types ---

#[derive(Serialize)]
struct EmbedRequest {
    content: Content,
    #[serde(rename = "taskType")]
    task_type: String,
    output_dimensionality: u32,
}

#[derive(Serialize)]
struct Content {
    parts: Vec<Part>,
}

#[derive(Serialize)]
#[serde(untagged)]
enum Part {
    Text { text: String },
    InlineData { inline_data: InlineData },
}

#[derive(Serialize)]
struct InlineData {
    mime_type: String,
    data: String, // base64
}

// --- Response types ---

#[derive(Deserialize)]
struct EmbedResponse {
    #[serde(default)]
    embeddings: Vec<Embedding>,
    // Single-content response uses `embedding` (singular)
    #[serde(default)]
    embedding: Option<Embedding>,
    #[serde(default)]
    error: Option<ApiError>,
}

#[derive(Deserialize, Clone)]
struct Embedding {
    values: Vec<f32>,
}

#[derive(Deserialize)]
struct ApiError {
    message: String,
}

/// Shared HTTP client — reuse across calls.
static HTTP: std::sync::LazyLock<reqwest::Client> = std::sync::LazyLock::new(reqwest::Client::new);

/// Embed a text string.
pub async fn embed_text(
    api_key: &str,
    text: &str,
    task_type: TaskType,
) -> Result<Vec<f32>, String> {
    let req = EmbedRequest {
        content: Content {
            parts: vec![Part::Text {
                text: text.to_string(),
            }],
        },
        task_type: task_type.as_str().to_string(),
        output_dimensionality: OUTPUT_DIM,
    };
    send(api_key, &req).await
}

/// Embed any media (video, audio, PDF) as raw bytes.
pub async fn embed_media(api_key: &str, bytes: &[u8], mime_type: &str) -> Result<Vec<f32>, String> {
    let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
    let req = EmbedRequest {
        content: Content {
            parts: vec![Part::InlineData {
                inline_data: InlineData {
                    mime_type: mime_type.to_string(),
                    data: b64,
                },
            }],
        },
        task_type: TaskType::RetrievalDocument.as_str().to_string(),
        output_dimensionality: OUTPUT_DIM,
    };
    send(api_key, &req).await
}

/// Embed text + image together (aggregated single embedding).
pub async fn embed_text_and_image(
    api_key: &str,
    text: &str,
    image_bytes: &[u8],
    mime_type: &str,
) -> Result<Vec<f32>, String> {
    let b64 = base64::engine::general_purpose::STANDARD.encode(image_bytes);
    let req = EmbedRequest {
        content: Content {
            parts: vec![
                Part::Text {
                    text: text.to_string(),
                },
                Part::InlineData {
                    inline_data: InlineData {
                        mime_type: mime_type.to_string(),
                        data: b64,
                    },
                },
            ],
        },
        task_type: TaskType::RetrievalDocument.as_str().to_string(),
        output_dimensionality: OUTPUT_DIM,
    };
    send(api_key, &req).await
}

/// The configured output dimensionality.
pub const fn output_dim() -> u32 {
    OUTPUT_DIM
}

// --- Internal ---

async fn send(api_key: &str, req: &EmbedRequest) -> Result<Vec<f32>, String> {
    let mut delay = 500u64; // ms

    for attempt in 0..3 {
        let res = HTTP
            .post(EMBED_URL)
            .header("x-goog-api-key", api_key)
            .json(req)
            .send()
            .await
            .map_err(|e| format!("embedding HTTP error: {e}"))?;

        let status = res.status();
        let body = res
            .text()
            .await
            .map_err(|e| format!("embedding response read error: {e}"))?;

        // Retry on 5xx errors
        if status.is_server_error() && attempt < 2 {
            log::warn!(
                "embedding API {status}, retrying in {delay}ms (attempt {}/3)",
                attempt + 1
            );
            tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
            delay *= 2;
            continue;
        }

        if !status.is_success() {
            return Err(format!("embedding API {status}: {body}"));
        }

        let parsed: EmbedResponse =
            serde_json::from_str(&body).map_err(|e| format!("embedding parse error: {e}"))?;

        if let Some(err) = parsed.error {
            return Err(format!("embedding API error: {}", err.message));
        }

        // Response may use `embedding` (singular) or `embeddings` (array)
        if let Some(emb) = parsed.embedding {
            return Ok(emb.values);
        }
        if let Some(emb) = parsed.embeddings.into_iter().next() {
            return Ok(emb.values);
        }

        return Err("embedding API returned no embeddings".to_string());
    }

    Err("embedding API failed after 3 retries".to_string())
}
