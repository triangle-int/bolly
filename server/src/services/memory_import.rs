//! Memory import pipeline: bulk-import external data into the memory library.
//!
//! Two-stage process:
//! 1. **Extraction** (Batch API, Haiku): parse uploaded files, extract personal facts
//! 2. **Organization** (single Sonnet call): deduplicate, structure, write to memory

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

// Memory import always uses Anthropic API directly (batch endpoint)
const CHEAP_MODEL: &str = "claude-haiku-4-5-20251001";
const FAST_MODEL: &str = "claude-sonnet-4-6";
use crate::domain::events::ServerEvent;
use crate::services::{memory, vector::VectorStore};

// ── Types ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportStage {
    Parsing,
    Extracting,
    Organizing,
    Writing,
    Done,
    Error,
}

/// A chunk of data to send through the extraction batch.
#[derive(Debug)]
struct ImportChunk {
    id: String,
    content: String,
}

/// A structured memory write operation produced by the organization stage.
#[derive(Debug, Deserialize)]
struct MemoryOp {
    path: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct BatchResponse {
    id: String,
    processing_status: String,
    #[serde(default)]
    results_url: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct BatchResultLine {
    custom_id: String,
    result: BatchResultPayload,
}

#[derive(Debug, Deserialize)]
struct BatchResultPayload {
    #[serde(rename = "type")]
    result_type: String,
    message: Option<BatchResultMessage>,
}

#[derive(Debug, Deserialize)]
struct BatchResultMessage {
    content: Vec<BatchContentBlock>,
}

#[derive(Debug, Deserialize)]
struct BatchContentBlock {
    #[serde(rename = "type")]
    block_type: String,
    #[serde(default)]
    text: String,
}

// ── Public entry point ───────────────────────────────────────────────────────

/// Run the full import pipeline in the background.
pub fn spawn_import(
    http: reqwest::Client,
    api_key: String,
    workspace_dir: PathBuf,
    instance_slug: String,
    upload_dir: PathBuf,
    events: broadcast::Sender<ServerEvent>,
    vector_store: Arc<VectorStore>,
) {
    tokio::spawn(async move {
        match run_import(
            &http,
            &api_key,
            &workspace_dir,
            &instance_slug,
            &upload_dir,
            &events,
            &vector_store,
        )
        .await
        {
            Ok(count) => {
                emit(
                    &events,
                    &instance_slug,
                    ImportStage::Done,
                    &format!("imported {count} memories"),
                );
            }
            Err(e) => {
                emit(
                    &events,
                    &instance_slug,
                    ImportStage::Error,
                    &format!("import failed: {e}"),
                );
            }
        }
    });
}

fn emit(events: &broadcast::Sender<ServerEvent>, slug: &str, stage: ImportStage, detail: &str) {
    let _ = events.send(ServerEvent::ImportProgress {
        instance_slug: slug.to_string(),
        stage,
        detail: detail.to_string(),
    });
    log::info!("[import] {slug} — {detail}");
}

async fn run_import(
    http: &reqwest::Client,
    api_key: &str,
    workspace_dir: &Path,
    instance_slug: &str,
    upload_dir: &Path,
    events: &broadcast::Sender<ServerEvent>,
    vector_store: &VectorStore,
) -> anyhow::Result<usize> {
    // ── Stage 0: Parse uploaded files into chunks ──
    emit(
        events,
        instance_slug,
        ImportStage::Parsing,
        "parsing uploaded files...",
    );
    let chunks = parse_upload_dir(upload_dir)?;
    if chunks.is_empty() {
        anyhow::bail!("no data found to import");
    }
    emit(
        events,
        instance_slug,
        ImportStage::Parsing,
        &format!("found {} chunks to process", chunks.len()),
    );

    // ── Stage 1: Batch extraction (Haiku) ──
    emit(
        events,
        instance_slug,
        ImportStage::Extracting,
        &format!("sending {} chunks to extraction...", chunks.len()),
    );
    let batch_id = create_extraction_batch(http, api_key, &chunks).await?;
    emit(
        events,
        instance_slug,
        ImportStage::Extracting,
        &format!("batch {batch_id} created, waiting..."),
    );

    let facts = poll_and_collect(http, api_key, &batch_id, events, instance_slug).await?;
    emit(
        events,
        instance_slug,
        ImportStage::Extracting,
        &format!("extracted {} raw facts", facts.len()),
    );

    if facts.is_empty() {
        anyhow::bail!("no facts extracted from data");
    }

    // ── Stage 2: Organization (single Sonnet call) ──
    emit(
        events,
        instance_slug,
        ImportStage::Organizing,
        "organizing and deduplicating...",
    );
    let existing_catalog = memory::build_library_catalog(workspace_dir, instance_slug);
    let ops = organize_facts(http, api_key, &facts, &existing_catalog).await?;
    emit(
        events,
        instance_slug,
        ImportStage::Organizing,
        &format!("{} memory files to write", ops.len()),
    );

    // ── Stage 3: Write to memory + index ──
    emit(
        events,
        instance_slug,
        ImportStage::Writing,
        "writing memories...",
    );
    let count = ops.len();
    write_import_memories(workspace_dir, instance_slug, &ops, vector_store).await?;

    // Invalidate catalog cache so the new memories show up in context
    memory::invalidate_frozen_catalog(instance_slug);

    // Cleanup uploaded import files
    let _ = std::fs::remove_dir_all(upload_dir);

    Ok(count)
}

/// Persist source files even when derived semantic indexing is unavailable.
async fn write_import_memories(
    workspace: &Path,
    slug: &str,
    ops: &[MemoryOp],
    store: &VectorStore,
) -> anyhow::Result<()> {
    let memory_dir = workspace.join("instances").join(slug).join("memory");
    for op in ops {
        let path = memory_dir.join(&op.path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, &op.content)?;
        memory::embed_memory_file(store, slug, &op.path, &op.content).await;
    }
    Ok(())
}

// ── Parsing ──────────────────────────────────────────────────────────────────

fn parse_upload_dir(dir: &Path) -> anyhow::Result<Vec<ImportChunk>> {
    let mut chunks = Vec::new();

    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() {
            // Recurse into subdirectories
            let sub = parse_upload_dir(&path)?;
            chunks.extend(sub);
            continue;
        }

        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");

        match ext {
            "json" => {
                let raw = std::fs::read_to_string(&path)?;
                let parsed = parse_json_file(name, &raw);
                chunks.extend(parsed);
            }
            "md" | "txt" | "csv" | "log" => {
                let content = std::fs::read_to_string(&path)?;
                if !content.trim().is_empty() {
                    // Split large text files into ~8K char chunks
                    for (i, chunk) in chunk_text_large(&content, 8000).into_iter().enumerate() {
                        chunks.push(ImportChunk {
                            id: format!("{name}-{i}"),
                            content: chunk,
                        });
                    }
                }
            }
            _ => {
                log::info!("[import] skipping unsupported file: {name}");
            }
        }
    }

    Ok(chunks)
}

/// Parse a JSON file — handles Claude export format and generic JSON.
fn parse_json_file(name: &str, raw: &str) -> Vec<ImportChunk> {
    let mut chunks = Vec::new();

    // Try parsing as array first
    if let Ok(arr) = serde_json::from_str::<Vec<serde_json::Value>>(raw) {
        match name {
            "conversations.json" => {
                for (i, conv) in arr.iter().enumerate() {
                    let conv_name = conv["name"].as_str().unwrap_or("untitled");
                    let messages = conv["chat_messages"]
                        .as_array()
                        .or_else(|| conv["messages"].as_array());

                    if let Some(msgs) = messages {
                        let text = format_conversation(conv_name, msgs);
                        if !text.trim().is_empty() {
                            chunks.push(ImportChunk {
                                id: format!("conv-{i}"),
                                content: text,
                            });
                        }
                    }
                }
            }
            "memories.json" => {
                for (i, mem) in arr.iter().enumerate() {
                    // Claude's memories.json has a `conversations_memory` field
                    let text = mem["conversations_memory"]
                        .as_str()
                        .or_else(|| mem["content"].as_str())
                        .or_else(|| mem["text"].as_str())
                        .unwrap_or("");
                    if !text.trim().is_empty() {
                        for (j, chunk) in chunk_text_large(text, 8000).into_iter().enumerate() {
                            chunks.push(ImportChunk {
                                id: format!("memory-{i}-{j}"),
                                content: chunk,
                            });
                        }
                    }
                }
            }
            "projects.json" => {
                for (i, proj) in arr.iter().enumerate() {
                    let proj_name = proj["name"].as_str().unwrap_or("untitled");
                    let instructions = proj["instructions"]
                        .as_str()
                        .or_else(|| proj["description"].as_str())
                        .unwrap_or("");
                    if !instructions.trim().is_empty() {
                        chunks.push(ImportChunk {
                            id: format!("project-{i}"),
                            content: format!("Project: {proj_name}\n\n{instructions}"),
                        });
                    }
                }
            }
            _ => {
                // Generic JSON array — stringify each item
                for (i, item) in arr.iter().enumerate() {
                    let text = serde_json::to_string_pretty(item).unwrap_or_default();
                    if text.len() > 50 {
                        chunks.push(ImportChunk {
                            id: format!("{name}-{i}"),
                            content: text,
                        });
                    }
                }
            }
        }
    } else if let Ok(obj) = serde_json::from_str::<serde_json::Value>(raw) {
        // Single JSON object
        let text = serde_json::to_string_pretty(&obj).unwrap_or_default();
        if text.len() > 50 {
            for (i, chunk) in chunk_text_large(&text, 8000).into_iter().enumerate() {
                chunks.push(ImportChunk {
                    id: format!("{name}-{i}"),
                    content: chunk,
                });
            }
        }
    }

    chunks
}

fn format_conversation(name: &str, messages: &[serde_json::Value]) -> String {
    let mut out = format!("Conversation: {name}\n\n");
    let mut total_len = out.len();
    const MAX_CONV_LEN: usize = 12_000; // Keep conversations under 12K chars

    for msg in messages {
        let sender = msg["sender"].as_str().unwrap_or("unknown");
        let text = msg["text"]
            .as_str()
            .or_else(|| {
                msg["content"].as_array().and_then(|blocks| {
                    blocks
                        .iter()
                        .find(|b| b["type"].as_str() == Some("text"))
                        .and_then(|b| b["text"].as_str())
                })
            })
            .unwrap_or("");

        if text.is_empty() {
            continue;
        }

        let line = format!("{sender}: {text}\n\n");
        total_len += line.len();
        if total_len > MAX_CONV_LEN {
            out.push_str("[...truncated...]\n");
            break;
        }
        out.push_str(&line);
    }

    out
}

fn chunk_text_large(text: &str, max_chars: usize) -> Vec<String> {
    if text.len() <= max_chars {
        return vec![text.to_string()];
    }
    text.as_bytes()
        .chunks(max_chars)
        .map(|c| String::from_utf8_lossy(c).to_string())
        .collect()
}

// ── Batch API ────────────────────────────────────────────────────────────────

const EXTRACTION_SYSTEM: &str = "\
You are a memory extraction agent. Given a conversation or document, extract ALL personal facts, \
preferences, important moments, relationships, and context about the user. \
Focus on information that would help a companion AI know them better.

Output a bullet list. Each bullet is one discrete fact. Be specific and concise. \
Skip purely technical/code-related content unless it reveals something personal \
(e.g. \"works with Rust\" is personal, but a code snippet is not). \
If the content has nothing personal, output: SKIP";

async fn create_extraction_batch(
    http: &reqwest::Client,
    api_key: &str,
    chunks: &[ImportChunk],
) -> anyhow::Result<String> {
    let requests: Vec<serde_json::Value> = chunks
        .iter()
        .map(|chunk| {
            serde_json::json!({
                "custom_id": chunk.id,
                "params": {
                    "model": CHEAP_MODEL,
                    "max_tokens": 2048,
                    "system": EXTRACTION_SYSTEM,
                    "messages": [{"role": "user", "content": chunk.content}]
                }
            })
        })
        .collect();

    let resp = http
        .post("https://api.anthropic.com/v1/messages/batches")
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&serde_json::json!({ "requests": requests }))
        .send()
        .await?;

    let status = resp.status();
    let body = resp.text().await?;

    if !status.is_success() {
        anyhow::bail!("batch create failed ({status}): {body}");
    }

    let batch: BatchResponse = serde_json::from_str(&body)?;
    Ok(batch.id)
}

async fn poll_and_collect(
    http: &reqwest::Client,
    api_key: &str,
    batch_id: &str,
    events: &broadcast::Sender<ServerEvent>,
    instance_slug: &str,
) -> anyhow::Result<Vec<String>> {
    loop {
        tokio::time::sleep(Duration::from_secs(10)).await;

        let resp = http
            .get(format!(
                "https://api.anthropic.com/v1/messages/batches/{batch_id}"
            ))
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01")
            .send()
            .await?;

        let body = resp.text().await?;
        let batch: BatchResponse = serde_json::from_str(&body)?;

        match batch.processing_status.as_str() {
            "ended" => {
                let results_url = batch
                    .results_url
                    .ok_or_else(|| anyhow::anyhow!("batch ended but no results_url"))?;
                return download_results(http, api_key, &results_url).await;
            }
            "failed" | "expired" | "canceled" => {
                anyhow::bail!("batch {}: {}", batch_id, batch.processing_status);
            }
            status => {
                emit(
                    events,
                    instance_slug,
                    ImportStage::Extracting,
                    &format!("batch status: {status}..."),
                );
            }
        }
    }
}

async fn download_results(
    http: &reqwest::Client,
    api_key: &str,
    results_url: &str,
) -> anyhow::Result<Vec<String>> {
    let resp = http
        .get(results_url)
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .send()
        .await?;

    let body = resp.text().await?;
    let mut facts = Vec::new();

    for line in body.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let result: BatchResultLine = match serde_json::from_str(line) {
            Ok(r) => r,
            Err(e) => {
                log::warn!("[import] failed to parse batch result line: {e}");
                continue;
            }
        };

        if result.result.result_type != "succeeded" {
            continue;
        }
        if let Some(msg) = result.result.message {
            for block in msg.content {
                if block.block_type == "text" && !block.text.contains("SKIP") {
                    facts.push(block.text);
                }
            }
        }
    }

    Ok(facts)
}

// ── Organization (Sonnet) ────────────────────────────────────────────────────

const ORGANIZE_SYSTEM: &str = "\
You are a memory organizer for an AI companion. You receive a list of raw facts \
extracted from a user's chat history and existing data.

Your job:
1. Deduplicate — merge facts that say the same thing
2. Organize into a clean folder structure for the companion's memory library
3. Output a JSON array of write operations

Use these folder conventions:
- about/ — who the user is (name, location, role, personality)
- preferences/ — likes, dislikes, habits, routines
- relationships/ — people the user mentions (family, friends, colleagues)
- work/ — projects, skills, career
- interests/ — hobbies, topics they care about
- moments/ — significant events, milestones, experiences
- context/ — ongoing situations, current state of life

Each file should be a focused markdown document about ONE topic. \
Use descriptive filenames like `about/background.md`, `work/current-projects.md`, etc.

Output ONLY a JSON array (no markdown fences), each element: {\"path\": \"...\", \"content\": \"...\"}

Here is the existing memory structure (to avoid duplicates):\n";

async fn organize_facts(
    http: &reqwest::Client,
    api_key: &str,
    facts: &[String],
    existing_catalog: &str,
) -> anyhow::Result<Vec<MemoryOp>> {
    let all_facts = facts.join("\n\n---\n\n");

    // Truncate if too large for a single call
    let facts_text = if all_facts.len() > 150_000 {
        all_facts[..150_000].to_string()
    } else {
        all_facts
    };

    let system = format!("{ORGANIZE_SYSTEM}{existing_catalog}");

    let req = serde_json::json!({
        "model": FAST_MODEL,
        "max_tokens": 16384,
        "system": system,
        "messages": [{
            "role": "user",
            "content": format!("Here are the extracted facts to organize:\n\n{facts_text}")
        }]
    });

    let resp = http
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&req)
        .send()
        .await?;

    let status = resp.status();
    let body = resp.text().await?;

    if !status.is_success() {
        anyhow::bail!("organization call failed ({status}): {body}");
    }

    let resp_json: serde_json::Value = serde_json::from_str(&body)?;
    let text = resp_json["content"][0]["text"].as_str().unwrap_or("");

    // Parse JSON from response — handle possible markdown fences
    let json_text = text
        .trim()
        .strip_prefix("```json")
        .unwrap_or(text.trim())
        .strip_prefix("```")
        .unwrap_or(text.trim())
        .strip_suffix("```")
        .unwrap_or(text.trim())
        .trim();

    let ops: Vec<MemoryOp> = serde_json::from_str(json_text).map_err(|e| {
        anyhow::anyhow!(
            "failed to parse organization output: {e}\nraw: {}",
            &json_text[..json_text.len().min(500)]
        )
    })?;

    Ok(ops)
}

#[cfg(test)]
mod embedding_import_tests {
    use super::*;
    #[tokio::test]
    async fn embedding_import_writes_without_key_and_uses_configured_backend() {
        use crate::services::embedding::tests::{MockServer, response};
        let workspace = tempfile::tempdir().unwrap();
        let ops = vec![MemoryOp {
            path: "about/stars.md".into(),
            content: "Orion nebula".into(),
        }];
        let store = VectorStore::connect(workspace.path()).await;
        write_import_memories(workspace.path(), "one", &ops, &store)
            .await
            .unwrap();
        assert!(
            std::fs::read_to_string(workspace.path().join("instances/one/memory/about/stars.md"))
                .unwrap()
                .contains("Orion")
        );
        let mock = MockServer::new(vec![(200, response(vec![1., 0., 0.]))]).await;
        let store = VectorStore::connect_with_config(workspace.path(), &mock.config).await;
        write_import_memories(workspace.path(), "one", &ops, &store)
            .await
            .unwrap();
        assert_eq!(
            store.list_all("one", 10).await.unwrap()[0].path,
            "about/stars.md"
        );
        assert_eq!(mock.requests.lock().unwrap().len(), 1);
    }
}
