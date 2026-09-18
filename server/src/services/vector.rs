//! Versioned local vector search for derived semantic memory.

use super::{
    embedding,
    vector_index::{Record, Store},
};
use std::path::Path;

// Keep aligned with the existing Google embedding endpoint. Provider migration is separate.
const EMBEDDING_PROVIDER: &str = "google";
const EMBEDDING_MODEL: &str = "gemini-embedding-2-preview";

pub struct VectorStore {
    store: std::sync::Arc<Store>,
}

#[derive(Debug, Clone)]
pub struct VectorSearchResult {
    pub path: String,
    pub source_type: String,
    pub content_preview: String,
    pub score: f32,
    pub upload_id: Option<String>,
}

fn result(record: Record, score: f32) -> VectorSearchResult {
    VectorSearchResult {
        path: record.path,
        source_type: record.source_type,
        content_preview: record.content_preview,
        score,
        upload_id: record.upload_id,
    }
}

impl VectorStore {
    pub async fn connect(data_dir: &Path) -> Self {
        let data_dir = data_dir.to_owned();
        let store = tokio::task::spawn_blocking(move || {
            Store::new(
                &data_dir,
                EMBEDDING_PROVIDER,
                EMBEDDING_MODEL,
                embedding::output_dim(),
            )
        })
        .await
        .expect("vector store initialization task panicked");
        Self {
            store: std::sync::Arc::new(store),
        }
    }

    async fn mutate<T: Send + 'static>(
        &self,
        instance_slug: &str,
        operation: impl FnOnce(std::sync::Arc<Store>, String) -> Result<T, String> + Send + 'static,
    ) -> Result<T, String> {
        let mutation = self.store.mutation_lock(instance_slug);
        let _guard = mutation.lock_owned().await;
        let store = self.store.clone();
        let slug = instance_slug.to_owned();
        tokio::task::spawn_blocking(move || operation(store, slug))
            .await
            .map_err(|error| format!("vector blocking task failed: {error}"))?
    }

    async fn read<T: Send + 'static>(
        &self,
        instance_slug: &str,
        operation: impl FnOnce(std::sync::Arc<Store>, String) -> Result<T, String> + Send + 'static,
    ) -> Result<T, String> {
        let store = self.store.clone();
        let slug = instance_slug.to_owned();
        tokio::task::spawn_blocking(move || operation(store, slug))
            .await
            .map_err(|error| format!("vector blocking task failed: {error}"))?
    }

    pub async fn ensure_collection(&self, instance_slug: &str) -> Result<(), String> {
        self.mutate(instance_slug, |store, slug| store.ensure(&slug))
            .await
    }

    pub async fn reset_collection(&self, instance_slug: &str) -> Result<(), String> {
        self.mutate(instance_slug, |store, slug| store.reset(&slug))
            .await
    }

    pub async fn needs_backfill(&self, instance_slug: &str) -> Result<bool, String> {
        self.mutate(instance_slug, |store, slug| store.needs_backfill(&slug))
            .await
    }

    pub async fn upsert_text_memory(
        &self,
        instance_slug: &str,
        path: &str,
        chunks: Vec<(String, Vec<f32>)>,
    ) -> Result<(), String> {
        let path = path.to_owned();
        let records = chunks
            .into_iter()
            .enumerate()
            .map(|(i, (text, vector))| Record {
                path: path.clone(),
                source_type: "text_memory".into(),
                chunk_index: i as u32,
                content_preview: text.chars().take(500).collect(),
                upload_id: None,
                vector,
            })
            .collect();
        self.mutate(instance_slug, move |store, slug| {
            store.replace(&slug, &path, records)
        })
        .await
    }

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
        let upload_id = upload_id.to_owned();
        let record = Record {
            path: upload_id.clone(),
            source_type: source_type.to_owned(),
            chunk_index: 0,
            content_preview: content_preview.chars().take(500).collect(),
            upload_id: Some(upload_id.clone()),
            vector,
        };
        self.mutate(instance_slug, move |store, slug| {
            store.replace(&slug, &upload_id, vec![record])
        })
        .await
    }

    pub async fn delete_by_path(&self, instance_slug: &str, path: &str) -> Result<(), String> {
        let path = path.to_owned();
        self.mutate(instance_slug, move |store, slug| store.delete(&slug, &path))
            .await
    }

    pub async fn search(
        &self,
        instance_slug: &str,
        query_vector: Vec<f32>,
        limit: usize,
    ) -> Result<Vec<VectorSearchResult>, String> {
        self.read(instance_slug, move |store, slug| {
            Ok(store
                .search(&slug, &query_vector, limit)?
                .into_iter()
                .map(|(record, score)| result(record, score))
                .collect())
        })
        .await
    }

    pub async fn list_all(
        &self,
        instance_slug: &str,
        limit: usize,
    ) -> Result<Vec<VectorSearchResult>, String> {
        self.read(instance_slug, move |store, slug| {
            Ok(store
                .list(&slug, limit)?
                .into_iter()
                .map(|record| result(record, 0.))
                .collect())
        })
        .await
    }

    /// Backfill all memories (text + media) into the local index.
    pub async fn backfill_text_memories(
        &self,
        workspace_dir: &Path,
        instance_slug: &str,
        google_ai_key: &str,
    ) -> Result<usize, String> {
        use super::memory;

        let mutation = self.store.mutation_lock(instance_slug);
        let _guard = mutation.lock_owned().await;
        let workspace = workspace_dir.to_owned();
        let slug = instance_slug.to_owned();
        let scan_workspace = workspace.clone();
        let scan_slug = slug.clone();
        let entries = tokio::task::spawn_blocking(move || {
            memory::scan_library_checked(&scan_workspace, &scan_slug)
        })
        .await
        .map_err(|error| format!("backfill scan task failed: {error}"))??;
        let mut records = Vec::new();
        let mut count = 0;

        for entry in entries {
            let file_path = workspace
                .join("instances")
                .join(&slug)
                .join("memory")
                .join(&entry.path);
            let ext = file_path
                .extension()
                .and_then(|extension| extension.to_str())
                .unwrap_or("")
                .to_lowercase();
            let is_image = matches!(
                ext.as_str(),
                "jpg" | "jpeg" | "png" | "webp" | "gif" | "svg"
            );
            let is_media =
                is_image || matches!(ext.as_str(), "pdf" | "mp4" | "mov" | "mp3" | "wav");

            if is_media {
                let display_path = entry.path.clone();
                let bytes = tokio::task::spawn_blocking(move || std::fs::read(&file_path))
                    .await
                    .map_err(|error| format!("backfill read task failed: {error}"))?
                    .map_err(|error| format!("backfill read {display_path}: {error}"))?;
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
                let vector = if is_image {
                    embedding::embed_text_and_image(google_ai_key, &entry.path, &bytes, mime_type)
                        .await
                } else {
                    embedding::embed_media(google_ai_key, &bytes, mime_type).await
                }
                .map_err(|error| format!("backfill embed {}: {error}", entry.path))?;
                records.push(Record {
                    path: entry.path.clone(),
                    source_type: source_type.into(),
                    chunk_index: 0,
                    content_preview: entry.path.chars().take(500).collect(),
                    upload_id: Some(entry.path),
                    vector,
                });
                count += 1;
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            } else {
                let display_path = entry.path.clone();
                let content =
                    tokio::task::spawn_blocking(move || std::fs::read_to_string(&file_path))
                        .await
                        .map_err(|error| format!("backfill read task failed: {error}"))?
                        .map_err(|error| format!("backfill read {display_path}: {error}"))?;
                let chunks = chunk_text(&content);
                for (chunk_index, chunk) in chunks.into_iter().enumerate() {
                    let vector = embedding::embed_text(
                        google_ai_key,
                        &chunk,
                        embedding::TaskType::RetrievalDocument,
                    )
                    .await
                    .map_err(|error| format!("backfill embed {}: {error}", entry.path))?;
                    records.push(Record {
                        path: entry.path.clone(),
                        source_type: "text_memory".into(),
                        chunk_index: chunk_index as u32,
                        content_preview: chunk.chars().take(500).collect(),
                        upload_id: None,
                        vector,
                    });
                    count += 1;
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                }
            }
        }

        let store = self.store.clone();
        tokio::task::spawn_blocking(move || store.commit_backfill(&slug, records))
            .await
            .map_err(|error| format!("backfill commit task failed: {error}"))??;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn empty_backfill_marks_only_its_companion_complete() {
        let workspace =
            std::env::temp_dir().join(format!("backfill-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&workspace).unwrap();
        let store = VectorStore::connect(&workspace).await;
        assert!(store.needs_backfill("one").await.unwrap());
        assert_eq!(
            store
                .backfill_text_memories(&workspace, "one", "")
                .await
                .unwrap(),
            0
        );
        assert!(!store.needs_backfill("one").await.unwrap());
        assert!(store.needs_backfill("two").await.unwrap());
        std::fs::remove_dir_all(workspace).unwrap();
    }
    #[tokio::test]
    async fn failed_backfill_remains_retryable() {
        let workspace =
            std::env::temp_dir().join(format!("backfill-test-{}", uuid::Uuid::new_v4()));
        let memory = workspace.join("instances/one/memory");
        std::fs::create_dir_all(&memory).unwrap();
        std::fs::write(memory.join("broken.md"), [0xff]).unwrap();
        let store = VectorStore::connect(&workspace).await;
        assert!(
            store
                .backfill_text_memories(&workspace, "one", "")
                .await
                .is_err()
        );
        assert!(store.needs_backfill("one").await.unwrap());
        std::fs::write(memory.join("broken.md"), "").unwrap();
        assert_eq!(
            store
                .backfill_text_memories(&workspace, "one", "")
                .await
                .unwrap(),
            0
        );
        assert!(!store.needs_backfill("one").await.unwrap());
        std::fs::remove_dir_all(workspace).unwrap();
    }

    #[tokio::test]
    async fn failed_backfill_preserves_prior_records_and_completion_state() {
        let workspace =
            std::env::temp_dir().join(format!("backfill-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&workspace).unwrap();
        let memory = workspace.join("instances/one/memory");
        let store = VectorStore::connect(&workspace).await;
        store
            .backfill_text_memories(&workspace, "one", "")
            .await
            .unwrap();
        let mut vector = vec![0.; embedding::output_dim() as usize];
        vector[0] = 1.;
        store
            .upsert_text_memory("one", "prior.md", vec![("prior".into(), vector)])
            .await
            .unwrap();
        std::fs::create_dir_all(&memory).unwrap();
        std::fs::write(memory.join("broken.md"), [0xff]).unwrap();

        assert!(
            store
                .backfill_text_memories(&workspace, "one", "")
                .await
                .is_err()
        );
        assert!(!store.needs_backfill("one").await.unwrap());
        assert_eq!(store.list_all("one", 10).await.unwrap()[0].path, "prior.md");
        std::fs::remove_dir_all(workspace).unwrap();
    }

    #[tokio::test]
    async fn mutation_lock_serializes_same_companion_without_blocking_another() {
        let workspace =
            std::env::temp_dir().join(format!("vector-lock-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&workspace).unwrap();
        let store = std::sync::Arc::new(VectorStore::connect(&workspace).await);
        let guard = store.store.mutation_lock("one").lock_owned().await;
        let mut vector = vec![0.; embedding::output_dim() as usize];
        vector[0] = 1.;

        let same = {
            let store = store.clone();
            let vector = vector.clone();
            tokio::spawn(async move {
                store
                    .upsert_text_memory("one", "one.md", vec![("one".into(), vector)])
                    .await
            })
        };
        tokio::task::yield_now().await;
        assert!(!same.is_finished());

        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            store.upsert_text_memory("two", "two.md", vec![("two".into(), vector)]),
        )
        .await
        .unwrap()
        .unwrap();
        drop(guard);
        same.await.unwrap().unwrap();
        std::fs::remove_dir_all(workspace).unwrap();
    }

    #[tokio::test]
    async fn public_boundary_preserves_text_and_media_semantics() {
        let workspace =
            std::env::temp_dir().join(format!("vector-api-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&workspace).unwrap();
        let store = VectorStore::connect(&workspace).await;
        let mut vector = vec![0.; embedding::output_dim() as usize];
        vector[0] = 1.;
        store.ensure_collection("slug").await.unwrap();
        store
            .upsert_text_memory(
                "slug",
                "notes.md",
                vec![
                    ("é".repeat(600), vector.clone()),
                    ("second".into(), vector.clone()),
                ],
            )
            .await
            .unwrap();
        let records = store.list_all("slug", 10).await.unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].content_preview.chars().count(), 500);
        assert_eq!(records[0].source_type, "text_memory");
        assert!(records[0].upload_id.is_none());
        assert_eq!(records[0].score, 0.);
        store
            .upsert_media(
                "slug",
                "notes.md",
                "media_image",
                "image/png",
                "name",
                "preview",
                vector.clone(),
            )
            .await
            .unwrap();
        assert_eq!(store.list_all("slug", 10).await.unwrap().len(), 1);
        drop(store);
        let store = VectorStore::connect(&workspace).await;
        let hits = store.search("slug", vector.clone(), 10).await.unwrap();
        assert_eq!(hits[0].upload_id.as_deref(), Some("notes.md"));
        assert_eq!(hits[0].score, 1.);
        assert!(store.list_all("slug", 0).await.unwrap().is_empty());
        store
            .upsert_text_memory("slug", "notes.md", vec![])
            .await
            .unwrap();
        assert!(store.list_all("slug", 10).await.unwrap().is_empty());
        store
            .upsert_text_memory("slug", "a", vec![("a".into(), vector)])
            .await
            .unwrap();
        store.delete_by_path("slug", "a").await.unwrap();
        store.reset_collection("slug").await.unwrap();
        assert!(store.needs_backfill("slug").await.unwrap());
        std::fs::remove_dir_all(workspace).unwrap();
    }
}
