//! LanceDB vector store — embedded vector search for semantic memory.

use std::path::Path;
use std::sync::Arc;

use arrow_array::{
    Array, FixedSizeListArray, Float32Array, Int32Array, Int64Array, RecordBatch, StringArray,
    types::Float32Type,
};
use arrow_schema::{DataType, Field, Schema};
use futures::TryStreamExt;
use lancedb::query::{ExecutableQuery, QueryBase};
use uuid::Uuid;

use super::embedding;

/// UUID v5 namespace for deterministic point IDs.
const NS: Uuid = Uuid::from_bytes([
    0x6b, 0x6f, 0x6c, 0x6c, 0x79, 0x2d, 0x76, 0x65, 0x63, 0x74, 0x6f, 0x72, 0x2d, 0x6e, 0x73, 0x21,
]);

pub struct VectorStore {
    db: lancedb::Connection,
}

#[derive(Debug, Clone)]
pub struct VectorSearchResult {
    pub path: String,
    pub source_type: String,
    pub content_preview: String,
    pub score: f32,
    pub upload_id: Option<String>,
}

/// Arrow schema for memory vectors.
fn table_schema() -> Arc<Schema> {
    let dim = embedding::output_dim() as i32;
    Arc::new(Schema::new(vec![
        Field::new("id", DataType::Utf8, false),
        Field::new("source_type", DataType::Utf8, false),
        Field::new("path", DataType::Utf8, false),
        Field::new("chunk_index", DataType::Int32, false),
        Field::new("content_preview", DataType::Utf8, false),
        Field::new("timestamp", DataType::Int64, false),
        Field::new("upload_id", DataType::Utf8, true),
        Field::new(
            "vector",
            DataType::FixedSizeList(Arc::new(Field::new("item", DataType::Float32, true)), dim),
            false,
        ),
    ]))
}

fn table_name(slug: &str) -> String {
    format!("memories_{slug}")
}

fn point_id(source_type: &str, path: &str, chunk_index: u32) -> String {
    Uuid::new_v5(
        &NS,
        format!("{source_type}:{path}:{chunk_index}").as_bytes(),
    )
    .to_string()
}

/// Read a string column value from a RecordBatch.
fn col_str(batch: &RecordBatch, col: &str, row: usize) -> String {
    batch
        .column_by_name(col)
        .and_then(|c| c.as_any().downcast_ref::<StringArray>())
        .map(|a| {
            if a.is_valid(row) {
                a.value(row).to_string()
            } else {
                String::new()
            }
        })
        .unwrap_or_default()
}

fn col_str_opt(batch: &RecordBatch, col: &str, row: usize) -> Option<String> {
    batch
        .column_by_name(col)
        .and_then(|c| c.as_any().downcast_ref::<StringArray>())
        .and_then(|a| {
            if a.is_valid(row) {
                let s = a.value(row);
                if s.is_empty() {
                    None
                } else {
                    Some(s.to_string())
                }
            } else {
                None
            }
        })
}

fn extract_results(batches: &[RecordBatch], has_distance: bool) -> Vec<VectorSearchResult> {
    let mut out = Vec::new();
    for batch in batches {
        for row in 0..batch.num_rows() {
            let score = if has_distance {
                let dist = batch
                    .column_by_name("_distance")
                    .and_then(|c| c.as_any().downcast_ref::<Float32Array>())
                    .map(|a| a.value(row))
                    .unwrap_or(1.0);
                1.0 - dist // cosine distance -> similarity
            } else {
                0.0
            };
            out.push(VectorSearchResult {
                path: col_str(batch, "path", row),
                source_type: col_str(batch, "source_type", row),
                content_preview: col_str(batch, "content_preview", row),
                score,
                upload_id: col_str_opt(batch, "upload_id", row),
            });
        }
    }
    out
}

/// Build a RecordBatch for inserting into a memory table.
fn make_batch(
    ids: Vec<String>,
    source_types: Vec<String>,
    paths: Vec<String>,
    chunk_indices: Vec<i32>,
    previews: Vec<String>,
    timestamps: Vec<i64>,
    upload_ids: Vec<Option<String>>,
    vectors: Vec<Vec<f32>>,
) -> Result<RecordBatch, String> {
    let dim = embedding::output_dim() as i32;
    let vecs: Vec<Option<Vec<Option<f32>>>> = vectors
        .into_iter()
        .map(|v| Some(v.into_iter().map(Some).collect()))
        .collect();
    let uid_refs: Vec<Option<&str>> = upload_ids.iter().map(|o| o.as_deref()).collect();

    RecordBatch::try_new(
        table_schema(),
        vec![
            Arc::new(StringArray::from_iter_values(&ids)),
            Arc::new(StringArray::from_iter_values(&source_types)),
            Arc::new(StringArray::from_iter_values(&paths)),
            Arc::new(Int32Array::from(chunk_indices)),
            Arc::new(StringArray::from_iter_values(&previews)),
            Arc::new(Int64Array::from(timestamps)),
            Arc::new(StringArray::from(uid_refs)),
            Arc::new(FixedSizeListArray::from_iter_primitive::<Float32Type, _, _>(vecs, dim)),
        ],
    )
    .map_err(|e| format!("arrow batch: {e}"))
}

impl VectorStore {
    /// Open (or create) a LanceDB database in the workspace directory.
    pub async fn connect(data_dir: &Path) -> Self {
        let db_path = data_dir.join("lancedb");
        std::fs::create_dir_all(&db_path).ok();
        let path_str = db_path.to_str().unwrap();
        let db = lancedb::connect(path_str)
            .execute()
            .await
            .unwrap_or_else(|e| panic!("failed to open LanceDB at {path_str}: {e}"));
        log::info!("[vector] opened LanceDB at {path_str}");
        Self { db }
    }

    async fn open_table(&self, name: &str) -> Result<lancedb::Table, String> {
        self.db
            .open_table(name)
            .execute()
            .await
            .map_err(|e| format!("lancedb open_table: {e}"))
    }

    /// Drop and recreate the table (reset all vectors).
    pub async fn reset_collection(&self, instance_slug: &str) -> Result<(), String> {
        let name = table_name(instance_slug);
        let tables = self
            .db
            .table_names()
            .execute()
            .await
            .map_err(|e| format!("lancedb: {e}"))?;
        if tables.contains(&name) {
            self.db
                .drop_table(&name, &Vec::<String>::new())
                .await
                .map_err(|e| format!("lancedb drop: {e}"))?;
            log::info!("[vector] dropped table {name}");
        }
        self.ensure_collection(instance_slug).await
    }

    /// Ensure the table exists, create if not.
    pub async fn ensure_collection(&self, instance_slug: &str) -> Result<(), String> {
        let name = table_name(instance_slug);
        let tables = self
            .db
            .table_names()
            .execute()
            .await
            .map_err(|e| format!("lancedb: {e}"))?;
        if !tables.contains(&name) {
            self.db
                .create_empty_table(&name, table_schema())
                .execute()
                .await
                .map_err(|e| format!("lancedb create: {e}"))?;
            log::info!("[vector] created table {name}");
        }
        Ok(())
    }

    /// Upsert text memory chunks.
    pub async fn upsert_text_memory(
        &self,
        instance_slug: &str,
        path: &str,
        chunks: Vec<(String, Vec<f32>)>,
    ) -> Result<(), String> {
        let table = self.open_table(&table_name(instance_slug)).await?;

        // Delete existing chunks for this path
        let escaped = path.replace('\'', "''");
        table.delete(&format!("path = '{escaped}'")).await.ok();

        if chunks.is_empty() {
            return Ok(());
        }

        let n = chunks.len();
        let ts = chrono::Utc::now().timestamp_millis();
        let mut ids = Vec::with_capacity(n);
        let mut src = Vec::with_capacity(n);
        let mut pths = Vec::with_capacity(n);
        let mut cidx = Vec::with_capacity(n);
        let mut prev = Vec::with_capacity(n);
        let mut tss = Vec::with_capacity(n);
        let mut uids: Vec<Option<String>> = Vec::with_capacity(n);
        let mut vecs = Vec::with_capacity(n);

        for (i, (text, vector)) in chunks.into_iter().enumerate() {
            ids.push(point_id("text_memory", path, i as u32));
            src.push("text_memory".to_string());
            pths.push(path.to_string());
            cidx.push(i as i32);
            prev.push(text.chars().take(500).collect());
            tss.push(ts);
            uids.push(None);
            vecs.push(vector);
        }

        let batch = make_batch(ids, src, pths, cidx, prev, tss, uids, vecs)?;
        table
            .add(vec![batch])
            .execute()
            .await
            .map_err(|e| format!("lancedb add: {e}"))?;
        Ok(())
    }

    /// Upsert a media embedding (image, video, audio).
    pub async fn upsert_media(
        &self,
        instance_slug: &str,
        upload_id: &str,
        source_type: &str,
        _mime_type: &str,
        _original_name: &str,
        content_preview: &str,
        vector: Vec<f32>,
    ) -> Result<(), String> {
        let table = self.open_table(&table_name(instance_slug)).await?;

        let escaped = upload_id.replace('\'', "''");
        table.delete(&format!("path = '{escaped}'")).await.ok(); // might not exist yet

        let batch = make_batch(
            vec![point_id(source_type, upload_id, 0)],
            vec![source_type.to_string()],
            vec![upload_id.to_string()],
            vec![0],
            vec![content_preview.chars().take(500).collect()],
            vec![chrono::Utc::now().timestamp_millis()],
            vec![Some(upload_id.to_string())],
            vec![vector],
        )?;
        table
            .add(vec![batch])
            .execute()
            .await
            .map_err(|e| format!("lancedb add: {e}"))?;
        Ok(())
    }

    /// Delete all points matching a path value.
    pub async fn delete_by_path(&self, instance_slug: &str, path: &str) -> Result<(), String> {
        let table = self.open_table(&table_name(instance_slug)).await?;
        let escaped = path.replace('\'', "''");
        table
            .delete(&format!("path = '{escaped}'"))
            .await
            .map(|_| ())
            .map_err(|e| format!("lancedb delete: {e}"))
    }

    /// Search for similar vectors.
    pub async fn search(
        &self,
        instance_slug: &str,
        query_vector: Vec<f32>,
        limit: usize,
    ) -> Result<Vec<VectorSearchResult>, String> {
        let table = self.open_table(&table_name(instance_slug)).await?;

        let count = table.count_rows(None).await.unwrap_or(0);
        if count == 0 {
            return Ok(vec![]);
        }

        let batches = table
            .vector_search(query_vector)
            .map_err(|e| format!("lancedb search: {e}"))?
            .distance_type(lancedb::DistanceType::Cosine)
            .limit(limit)
            .execute()
            .await
            .map_err(|e| format!("lancedb execute: {e}"))?
            .try_collect::<Vec<RecordBatch>>()
            .await
            .map_err(|e| format!("lancedb collect: {e}"))?;

        Ok(extract_results(&batches, true))
    }

    /// List all points with metadata (for debugging).
    pub async fn list_all(
        &self,
        instance_slug: &str,
        limit: usize,
    ) -> Result<Vec<VectorSearchResult>, String> {
        let table = self.open_table(&table_name(instance_slug)).await?;

        let batches = table
            .query()
            .limit(limit)
            .execute()
            .await
            .map_err(|e| format!("lancedb query: {e}"))?
            .try_collect::<Vec<RecordBatch>>()
            .await
            .map_err(|e| format!("lancedb collect: {e}"))?;

        Ok(extract_results(&batches, false))
    }

    /// Backfill all memories (text + media) into LanceDB.
    pub async fn backfill_text_memories(
        &self,
        workspace_dir: &Path,
        instance_slug: &str,
        google_ai_key: &str,
    ) -> Result<usize, String> {
        use super::memory;

        self.ensure_collection(instance_slug).await?;

        let entries = memory::scan_library(workspace_dir, instance_slug);
        let mut count = 0;

        for entry in &entries {
            let file_path = workspace_dir
                .join("instances")
                .join(instance_slug)
                .join("memory")
                .join(&entry.path);

            let ext = file_path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();
            let is_image = matches!(
                ext.as_str(),
                "jpg" | "jpeg" | "png" | "webp" | "gif" | "svg"
            );
            let is_media =
                is_image || matches!(ext.as_str(), "pdf" | "mp4" | "mov" | "mp3" | "wav");

            if is_media {
                let bytes = match std::fs::read(&file_path) {
                    Ok(b) => b,
                    Err(_) => continue,
                };
                if bytes.len() > 20 * 1024 * 1024 {
                    continue;
                }

                let mime_type = match ext.as_str() {
                    "jpg" | "jpeg" => "image/jpeg",
                    "png" => "image/png",
                    "webp" => "image/webp",
                    "gif" => "image/gif",
                    "svg" => "image/svg+xml",
                    "pdf" => "application/pdf",
                    "mp4" => "video/mp4",
                    "mov" => "video/quicktime",
                    "mp3" => "audio/mpeg",
                    "wav" => "audio/wav",
                    _ => "application/octet-stream",
                };

                let source_type = if is_image {
                    "media_image"
                } else if mime_type.starts_with("video/") {
                    "media_video"
                } else if mime_type.starts_with("audio/") {
                    "media_audio"
                } else {
                    "media_document"
                };

                let desc = &entry.path;
                let embed_result = if is_image {
                    embedding::embed_text_and_image(google_ai_key, desc, &bytes, mime_type).await
                } else {
                    embedding::embed_media(google_ai_key, &bytes, mime_type).await
                };

                match embed_result {
                    Ok(vec) => {
                        if let Err(e) = self
                            .upsert_media(
                                instance_slug,
                                &entry.path,
                                source_type,
                                mime_type,
                                &entry.path,
                                desc,
                                vec,
                            )
                            .await
                        {
                            log::warn!(
                                "[vector] backfill media upsert failed for {}: {e}",
                                entry.path
                            );
                        }
                        count += 1;
                    }
                    Err(e) => {
                        log::warn!(
                            "[vector] backfill media embed error for {}: {e}",
                            entry.path
                        )
                    }
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            } else {
                let content = match std::fs::read_to_string(&file_path) {
                    Ok(c) => c,
                    Err(_) => continue,
                };

                let chunks = chunk_text(&content);
                let mut chunk_vectors = Vec::new();

                for chunk in &chunks {
                    match embedding::embed_text(
                        google_ai_key,
                        chunk,
                        embedding::TaskType::RetrievalDocument,
                    )
                    .await
                    {
                        Ok(vec) => chunk_vectors.push((chunk.clone(), vec)),
                        Err(e) => {
                            log::warn!("[vector] backfill embed error for {}: {e}", entry.path);
                            continue;
                        }
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                }

                if !chunk_vectors.is_empty() {
                    self.upsert_text_memory(instance_slug, &entry.path, chunk_vectors)
                        .await?;
                    count += chunks.len();
                }
            }
        }

        Ok(count)
    }
}

/// Chunk text into ~600 byte paragraphs (matching BM25 chunking in memory.rs).
pub fn chunk_text(text: &str) -> Vec<String> {
    if text.trim().is_empty() {
        return vec![];
    }
    if text.len() < 800 {
        return vec![text.to_string()];
    }

    let mut chunks = Vec::new();
    let mut current = String::new();

    for line in text.lines() {
        if current.len() + line.len() > 600 && !current.is_empty() {
            chunks.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push('\n');
        }
        current.push_str(line);
    }

    if !current.is_empty() {
        chunks.push(current);
    }

    chunks.retain(|c| !c.trim().is_empty());
    chunks
}
