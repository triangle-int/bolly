//! Versioned local vector search for derived semantic memory.

use super::{
    embedding,
    vector_index::{Record, Store},
};
use std::collections::HashMap;
use std::path::Path;

pub struct VectorStore {
    store: std::sync::Arc<Store>,
    keywords: std::sync::Arc<super::keyword_search::KeywordStore>,
    media: std::sync::Arc<super::media_text::MediaStore>,
    embedding: embedding::EmbeddingService,
    lifecycle: std::sync::Mutex<HashMap<String, std::sync::Arc<tokio::sync::Mutex<()>>>>,
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
        Self::connect_with_config(data_dir, &crate::config::Config::default()).await
    }

    pub async fn connect_with_config(data_dir: &Path, config: &crate::config::Config) -> Self {
        let settings = config.embedding.clone();
        std::fs::create_dir_all(data_dir).expect("failed to create workspace root");
        let media = std::sync::Arc::new(
            super::media_text::MediaStore::open(data_dir)
                .expect("failed to open persistent workspace capability"),
        );
        let data_dir = data_dir.to_owned();
        let store = tokio::task::spawn_blocking(move || {
            Store::with_endpoint(
                &data_dir,
                &settings.provider,
                &settings.model,
                settings.dimensions,
                &settings.base_url,
            )
        })
        .await
        .expect("vector store initialization task panicked");
        Self {
            store: std::sync::Arc::new(store),
            keywords: std::sync::Arc::new(super::keyword_search::KeywordStore::new(media.clone())),
            media,
            embedding: embedding::EmbeddingService::from_config(config),
            lifecycle: std::sync::Mutex::new(HashMap::new()),
        }
    }

    pub fn media_store(&self) -> std::sync::Arc<super::media_text::MediaStore> {
        self.media.clone()
    }

    fn lifecycle_lock(&self, slug: &str) -> std::sync::Arc<tokio::sync::Mutex<()>> {
        let mut locks = self
            .lifecycle
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        locks
            .entry(slug.to_owned())
            .or_insert_with(|| std::sync::Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    }

    /// Replace raw media, its owner-bound representation, and derived search
    /// state while holding one companion lifecycle gate through indexing.
    pub async fn replace_media_from_upload(
        &self,
        slug: &str,
        path: &str,
        upload_id: &str,
        content: &str,
    ) -> Result<Option<String>, String> {
        let _guard = self.lifecycle_lock(slug).lock_owned().await;
        self.keywords.invalidate(slug);
        self.delete_by_path_no_lifecycle(slug, path).await?;

        let media = self.media.clone();
        let publish_slug = slug.to_owned();
        let publish_path = path.to_owned();
        let publish_upload = upload_id.to_owned();
        tokio::task::spawn_blocking(move || {
            media.publish_upload_owner(&publish_slug, &publish_path, &publish_upload)
        })
        .await
        .map_err(|error| format!("media publication task failed: {error}"))?
        .map_err(|error| error.to_string())?;

        let media = self.media.clone();
        let write_slug = slug.to_owned();
        let write_path = path.to_owned();
        let write_content = content.to_owned();
        let persisted = tokio::task::spawn_blocking(move || {
            media.write(&write_slug, &write_path, &write_content)
        })
        .await
        .map_err(|error| format!("representation write task failed: {error}"))?;
        if let Err(error) = persisted {
            self.delete_by_path_no_lifecycle(slug, path).await?;
            return Ok(Some(format!("could not persist representation: {error}")));
        }
        if content.trim().is_empty() {
            return Ok(Some("media text representation is empty".into()));
        }
        match self.index_media_text_no_lifecycle(slug, path).await {
            Ok(()) => Ok(None),
            Err(error) => Ok(Some(error)),
        }
    }

    pub async fn edit_media_text(
        &self,
        slug: &str,
        path: &str,
        content: &str,
        append: bool,
    ) -> Result<(), String> {
        let _guard = self.lifecycle_lock(slug).lock_owned().await;
        let content = if append {
            let media = self.media.clone();
            let read_slug = slug.to_owned();
            let read_path = path.to_owned();
            let mut existing =
                tokio::task::spawn_blocking(move || media.read(&read_slug, &read_path))
                    .await
                    .map_err(|error| format!("representation read task failed: {error}"))??;
            if !existing.is_empty() && !existing.ends_with('\n') {
                existing.push('\n');
            }
            existing.push_str(content);
            existing
        } else {
            content.to_owned()
        };
        let media = self.media.clone();
        let write_slug = slug.to_owned();
        let write_path = path.to_owned();
        tokio::task::spawn_blocking(move || media.write(&write_slug, &write_path, &content))
            .await
            .map_err(|error| format!("representation write task failed: {error}"))?
            .map_err(|error| error.to_string())?;
        self.index_media_text_no_lifecycle(slug, path).await
    }

    pub async fn delete_memory(&self, slug: &str, path: &str) -> Result<(), String> {
        let _guard = self.lifecycle_lock(slug).lock_owned().await;
        self.keywords.invalidate(slug);
        let media = self.media.clone();
        let delete_slug = slug.to_owned();
        let delete_path = path.to_owned();
        let filesystem =
            tokio::task::spawn_blocking(move || media.remove(&delete_slug, &delete_path))
                .await
                .map_err(|error| format!("memory delete task failed: {error}"))?;
        let derived = self.delete_by_path_no_lifecycle(slug, path).await;
        match (filesystem, derived) {
            (Ok(()), Ok(())) => Ok(()),
            (filesystem, derived) => {
                let mut errors = Vec::new();
                if let Err(error) = filesystem {
                    errors.push(format!("filesystem cleanup failed: {error}"));
                }
                if let Err(error) = derived {
                    errors.push(format!("derived cleanup failed: {error}"));
                }
                Err(errors.join("; "))
            }
        }
    }

    pub async fn write_text_memory(
        &self,
        slug: &str,
        path: &str,
        content: &str,
        append: bool,
    ) -> Result<String, String> {
        let _guard = self.lifecycle_lock(slug).lock_owned().await;
        let media = self.media.clone();
        let read_slug = slug.to_owned();
        let read_path = path.to_owned();
        let existing =
            tokio::task::spawn_blocking(move || media.read_memory_text(&read_slug, &read_path))
                .await
                .map_err(|error| format!("memory read task failed: {error}"))?;
        let existing = match existing {
            Ok(content) => Some(content),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(format!("memory read failed: {error}")),
        };
        let body = if append {
            let mut body = existing
                .as_deref()
                .map(crate::services::memory::parse_frontmatter)
                .map(|(_, body)| body.to_owned())
                .unwrap_or_default();
            if !body.is_empty() && !body.ends_with('\n') {
                body.push('\n');
            }
            body.push_str(content);
            body
        } else {
            content.to_owned()
        };
        let stamped = crate::services::memory::stamp_content(&body, existing.as_deref());
        let media = self.media.clone();
        let write_slug = slug.to_owned();
        let write_path = path.to_owned();
        let write_content = stamped.clone();
        tokio::task::spawn_blocking(move || {
            media.write_memory_text(&write_slug, &write_path, &write_content)
        })
        .await
        .map_err(|error| format!("memory write task failed: {error}"))?
        .map_err(|error| error.to_string())?;
        self.keywords.invalidate(slug);
        if let Err(error) = self
            .index_document(slug, path, &stamped, "text_memory", None)
            .await
        {
            log::warn!("[memory] semantic indexing unavailable; memory saved: {error}");
        }
        Ok(stamped)
    }

    pub fn embedding_status(&self) -> serde_json::Value {
        self.embedding.status()
    }

    pub fn embedding_needs_restart(&self, config: &crate::config::Config) -> bool {
        self.embedding.needs_restart(config)
    }

    /// Embed and replace a complete document under the same lock as strict backfill.
    pub async fn index_text(&self, slug: &str, path: &str, content: &str) -> Result<(), String> {
        if let Some(owner) = super::media_text::media_path(path) {
            return self.index_media_text(slug, owner).await;
        }
        if super::media_text::source_type(path).is_some() {
            return self.index_media_text(slug, path).await;
        }
        self.index_document(slug, path, content, "text_memory", None)
            .await
    }

    /// Embed persisted media text in the same provider/model space as text memories.
    /// No bytes or caller-supplied vectors are accepted at this boundary.
    pub async fn index_media_text(&self, slug: &str, path: &str) -> Result<(), String> {
        let _guard = self.lifecycle_lock(slug).lock_owned().await;
        self.index_media_text_no_lifecycle(slug, path).await
    }

    async fn index_media_text_no_lifecycle(&self, slug: &str, path: &str) -> Result<(), String> {
        let source_type = super::media_text::source_type(path).ok_or("unsupported media path")?;
        self.index_document(slug, path, "", source_type, Some(path))
            .await
    }

    async fn index_document(
        &self,
        slug: &str,
        path: &str,
        content: &str,
        source_type: &str,
        upload_id: Option<&str>,
    ) -> Result<(), String> {
        self.keywords.invalidate(slug);
        let mutation = self.store.mutation_lock(slug);
        let _guard = mutation.lock_owned().await;
        let result = async {
            let representation;
            let content = if source_type.starts_with("media_") {
                let media = self.media.clone();
                let slug = slug.to_owned();
                let path = path.to_owned();
                representation = tokio::task::spawn_blocking(move || media.read(&slug, &path))
                    .await
                    .map_err(|error| format!("representation read task failed: {error}"))??;
                representation.as_str()
            } else {
                content
            };
            self.embedding.ensure_configured()?;
            let mut records = Vec::new();
            for (i, text) in chunk_text(content).into_iter().enumerate() {
                let vector = self.embedding.document(&text).await?;
                records.push(Record {
                    path: path.into(),
                    source_type: source_type.into(),
                    chunk_index: i as u32,
                    content_preview: text.chars().take(500).collect(),
                    upload_id: upload_id.map(str::to_owned),
                    vector,
                });
            }
            let path = path.to_owned();
            self.read(slug, move |store, slug| {
                store.replace(&slug, &path, records)
            })
            .await
        }
        .await;
        if result.is_err() {
            let path = path.to_owned();
            self.read(slug, move |store, slug| store.invalidate_path(&slug, &path))
                .await?;
        }
        result
    }

    /// BM25 is always usable, including after a provider or index failure.
    pub async fn search_text(
        &self,
        slug: &str,
        query: &str,
        limit: usize,
    ) -> Vec<VectorSearchResult> {
        self.hybrid_search(slug, query, limit, None).await
    }

    /// Preserve the chat RAG relevance threshold without discarding keyword hits.
    pub async fn search_context(
        &self,
        slug: &str,
        query: &str,
        limit: usize,
    ) -> Vec<VectorSearchResult> {
        self.hybrid_search(slug, query, limit, Some(0.3)).await
    }

    async fn hybrid_search(
        &self,
        slug: &str,
        query: &str,
        limit: usize,
        minimum_semantic_score: Option<f32>,
    ) -> Vec<VectorSearchResult> {
        let mut results = match self.embedding.query(query).await {
            Ok(vector) => match self.search(slug, vector, limit).await {
                Ok(hits) => hits
                    .into_iter()
                    .filter(|hit| minimum_semantic_score.is_none_or(|minimum| hit.score > minimum))
                    .collect(),
                Err(error) => {
                    log::debug!("[memory] semantic search unavailable: {error}");
                    Vec::new()
                }
            },
            Err(error) => {
                log::debug!("[memory] BM25 fallback: {error}");
                Vec::new()
            }
        };
        let keywords = self.keywords.clone();
        let companion = slug.to_owned();
        let query = query.to_owned();
        let keywords = tokio::task::spawn_blocking(move || {
            if !keywords.has_index(&companion) {
                keywords.reindex(&companion);
            }
            keywords.search(&companion, &query, limit)
        })
        .await
        .unwrap_or_default();
        for hit in keywords {
            if !results.iter().any(|r| r.path == hit.path) {
                results.push(hit);
            }
        }
        results.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then_with(|| a.path.cmp(&b.path))
        });
        results.truncate(limit);
        results
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
        self.keywords.invalidate(instance_slug);
        self.mutate(instance_slug, |store, slug| store.reset(&slug))
            .await
    }

    pub async fn needs_backfill(&self, instance_slug: &str) -> Result<bool, String> {
        self.mutate(instance_slug, |store, slug| store.needs_backfill(&slug))
            .await
    }

    #[cfg(test)]
    pub(crate) async fn upsert_text_memory(
        &self,
        instance_slug: &str,
        path: &str,
        chunks: Vec<(String, Vec<f32>)>,
    ) -> Result<(), String> {
        self.keywords.invalidate(instance_slug);
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

    pub async fn delete_by_path(&self, instance_slug: &str, path: &str) -> Result<(), String> {
        let _guard = self.lifecycle_lock(instance_slug).lock_owned().await;
        self.delete_by_path_no_lifecycle(instance_slug, path).await
    }

    async fn delete_by_path_no_lifecycle(
        &self,
        instance_slug: &str,
        path: &str,
    ) -> Result<(), String> {
        self.keywords.invalidate(instance_slug);
        let path = path.to_owned();
        self.mutate(instance_slug, move |store, slug| {
            if let Some(owner) = super::media_text::media_path(&path) {
                store.invalidate_path(&slug, owner)
            } else if super::media_text::source_type(&path).is_some() {
                store.invalidate_path(&slug, &path)
            } else {
                store.delete(&slug, &path)
            }
        })
        .await
    }

    #[cfg(test)]
    pub(crate) fn inject_next_persist_failure(&self) {
        self.store.inject_next_persist_failure();
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

    /// Build one candidate from text memories and persisted media representations.
    /// Provider failures leave the committed index intact; missing media text stays retryable.
    pub async fn backfill_text_memories(
        &self,
        _workspace_dir: &Path,
        instance_slug: &str,
    ) -> Result<usize, String> {
        let _lifecycle_guard = self.lifecycle_lock(instance_slug).lock_owned().await;
        let mutation = self.store.mutation_lock(instance_slug);
        let _guard = mutation.lock_owned().await;
        let slug = instance_slug.to_owned();
        let scan_slug = slug.clone();
        let scan_media = self.media.clone();
        let entries = tokio::task::spawn_blocking(move || scan_media.memory_files(&scan_slug))
            .await
            .map_err(|error| format!("backfill scan task failed: {error}"))??;
        let mut records = Vec::new();
        let mut count = 0;
        let mut skipped_media = false;
        let mut provider_checked = false;

        for path in entries {
            let source_type = super::media_text::source_type(&path);
            let content = if source_type.is_some() {
                let media_path = path.clone();
                let media = self.media.clone();
                let media_slug = slug.clone();
                match tokio::task::spawn_blocking(move || media.read(&media_slug, &media_path))
                    .await
                    .map_err(|error| format!("backfill representation task failed: {error}"))?
                {
                    Ok(content) => content,
                    Err(_) => {
                        skipped_media = true;
                        log::info!(
                            "[backfill] semantic indexing pending for {}: missing or unreadable representation",
                            path
                        );
                        continue;
                    }
                }
            } else {
                let display_path = path.clone();
                let media = self.media.clone();
                let text_slug = slug.clone();
                tokio::task::spawn_blocking(move || {
                    media.read_memory_text(&text_slug, &display_path)
                })
                .await
                .map_err(|error| format!("backfill read task failed: {error}"))?
                .map_err(|error| format!("backfill read {path}: {error}"))?
            };
            if !provider_checked {
                self.embedding.ensure_configured()?;
                provider_checked = true;
            }
            for (chunk_index, chunk) in chunk_text(&content).into_iter().enumerate() {
                let vector = self
                    .embedding
                    .document(&chunk)
                    .await
                    .map_err(|error| format!("backfill embed {path}: {error}"))?;
                records.push(Record {
                    path: path.clone(),
                    source_type: source_type.unwrap_or("text_memory").into(),
                    chunk_index: chunk_index as u32,
                    content_preview: chunk.chars().take(500).collect(),
                    upload_id: source_type.map(|_| path.clone()),
                    vector,
                });
                count += 1;
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        }

        let store = self.store.clone();
        tokio::task::spawn_blocking(move || store.commit_backfill(&slug, records, !skipped_media))
            .await
            .map_err(|error| format!("backfill commit task failed: {error}"))??;
        self.keywords.invalidate(instance_slug);
        Ok(count)
    }

    #[cfg(test)]
    fn keyword_reindex_count(&self) -> usize {
        self.keywords.reindex_count()
    }
}

/// Chunk text into ~600 byte paragraphs (matching BM25 chunking in memory.rs).
pub fn chunk_text(text: &str) -> Vec<String> {
    if text.trim().is_empty() {
        return vec![];
    }
    if text.len() <= 600 {
        return vec![text.to_string()];
    }

    let mut chunks = Vec::new();
    let mut current = String::new();

    for line in text.lines() {
        if line.len() > 600 {
            if !current.is_empty() {
                chunks.push(std::mem::take(&mut current));
            }
            let mut remainder = line;
            while remainder.len() > 600 {
                let mut split = 600;
                while !remainder.is_char_boundary(split) {
                    split -= 1;
                }
                chunks.push(remainder[..split].to_owned());
                remainder = &remainder[split..];
            }
            if !remainder.is_empty() {
                current.push_str(remainder);
            }
            continue;
        }
        if !current.is_empty() && current.len() + 1 + line.len() > 600 {
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

    fn upload(workspace: &Path, name: &str, bytes: &[u8]) {
        let uploads = workspace.join("instances/one/uploads");
        std::fs::create_dir_all(&uploads).unwrap();
        std::fs::write(uploads.join(name), bytes).unwrap();
        std::fs::write(
            uploads.join(format!("{name}.json")),
            serde_json::json!({"stored_name": name}).to_string(),
        )
        .unwrap();
    }

    #[tokio::test]
    async fn append_refuses_invalid_utf8_without_changing_existing_bytes() {
        let workspace = tempfile::tempdir().unwrap();
        let memory = workspace.path().join("instances/one/memory");
        std::fs::create_dir_all(&memory).unwrap();
        let original = b"valid prefix\xffexisting suffix";
        std::fs::write(memory.join("note.md"), original).unwrap();
        let store = VectorStore::connect(workspace.path()).await;

        let error = store
            .write_text_memory("one", "note.md", "appended", true)
            .await
            .unwrap_err();

        assert!(
            error.contains("UTF-8") || error.contains("utf-8"),
            "{error}"
        );
        assert_eq!(std::fs::read(memory.join("note.md")).unwrap(), original);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn append_refuses_symlink_read_failure_without_changing_outside_bytes() {
        use std::os::unix::fs::symlink;

        let workspace = tempfile::tempdir().unwrap();
        let memory = workspace.path().join("instances/one/memory");
        std::fs::create_dir_all(&memory).unwrap();
        let outside = tempfile::NamedTempFile::new().unwrap();
        let original = b"outside sentinel";
        std::fs::write(outside.path(), original).unwrap();
        symlink(outside.path(), memory.join("note.md")).unwrap();
        let store = VectorStore::connect(workspace.path()).await;

        assert!(
            store
                .write_text_memory("one", "note.md", "appended", true)
                .await
                .is_err()
        );
        assert!(memory.join("note.md").is_symlink());
        assert_eq!(std::fs::read(outside.path()).unwrap(), original);
    }

    #[tokio::test]
    async fn concurrent_media_replacements_publish_matching_owner_and_sidecar() {
        use super::super::embedding::tests::{MockServer, response};
        let mock = MockServer::new(vec![(200, response(vec![1., 0., 0.])); 2]).await;
        let workspace = tempfile::tempdir().unwrap();
        upload(workspace.path(), "upload-a", b"owner A");
        upload(workspace.path(), "upload-b", b"owner B");
        let store = std::sync::Arc::new(
            VectorStore::connect_with_config(workspace.path(), &mock.config).await,
        );
        let gate = store.lifecycle_lock("one");
        let guard = gate.lock_owned().await;

        let first = {
            let store = store.clone();
            tokio::spawn(async move {
                store
                    .replace_media_from_upload("one", "photo.png", "upload-a", "text A")
                    .await
            })
        };
        tokio::task::yield_now().await;
        let second = {
            let store = store.clone();
            tokio::spawn(async move {
                store
                    .replace_media_from_upload("one", "photo.png", "upload-b", "text B")
                    .await
            })
        };
        tokio::task::yield_now().await;
        assert!(!first.is_finished() && !second.is_finished());
        drop(guard);
        first.await.unwrap().unwrap();
        second.await.unwrap().unwrap();

        assert_eq!(
            store
                .media_store()
                .read_memory_file("one", "photo.png", 100)
                .unwrap(),
            b"owner B"
        );
        assert_eq!(
            store.media_store().read("one", "photo.png").unwrap(),
            "text B"
        );
        assert_eq!(
            store.list_all("one", 10).await.unwrap()[0].content_preview,
            "text B"
        );
    }

    #[tokio::test]
    async fn media_replace_then_delete_is_ordered_as_one_lifecycle() {
        use super::super::embedding::tests::{MockServer, response};
        let mock = MockServer::new(vec![(200, response(vec![1., 0., 0.]))]).await;
        let workspace = tempfile::tempdir().unwrap();
        upload(workspace.path(), "upload-a", b"owner A");
        let store = std::sync::Arc::new(
            VectorStore::connect_with_config(workspace.path(), &mock.config).await,
        );
        let guard = store.lifecycle_lock("one").lock_owned().await;
        let replace = {
            let store = store.clone();
            tokio::spawn(async move {
                store
                    .replace_media_from_upload("one", "photo.png", "upload-a", "text A")
                    .await
            })
        };
        tokio::task::yield_now().await;
        let delete = {
            let store = store.clone();
            tokio::spawn(async move { store.delete_memory("one", "photo.png").await })
        };
        drop(guard);
        replace.await.unwrap().unwrap();
        delete.await.unwrap().unwrap();

        assert!(
            store
                .media_store()
                .read_memory_file("one", "photo.png", 100)
                .is_err()
        );
        assert!(store.media_store().read("one", "photo.png").is_err());
        assert!(store.list_all("one", 10).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn media_sidecar_edit_then_replace_has_one_consistent_winner() {
        use super::super::embedding::tests::{MockServer, response};
        let mock = MockServer::new(vec![(200, response(vec![1., 0., 0.])); 3]).await;
        let workspace = tempfile::tempdir().unwrap();
        upload(workspace.path(), "upload-a", b"owner A");
        upload(workspace.path(), "upload-b", b"owner B");
        let store = std::sync::Arc::new(
            VectorStore::connect_with_config(workspace.path(), &mock.config).await,
        );
        store
            .replace_media_from_upload("one", "photo.png", "upload-a", "text A")
            .await
            .unwrap();
        let guard = store.lifecycle_lock("one").lock_owned().await;
        let edit = {
            let store = store.clone();
            tokio::spawn(async move {
                store
                    .edit_media_text("one", "photo.png", "edited A", false)
                    .await
            })
        };
        tokio::task::yield_now().await;
        let replace = {
            let store = store.clone();
            tokio::spawn(async move {
                store
                    .replace_media_from_upload("one", "photo.png", "upload-b", "text B")
                    .await
            })
        };
        drop(guard);
        edit.await.unwrap().unwrap();
        replace.await.unwrap().unwrap();

        assert_eq!(
            store
                .media_store()
                .read_memory_file("one", "photo.png", 100)
                .unwrap(),
            b"owner B"
        );
        assert_eq!(
            store.media_store().read("one", "photo.png").unwrap(),
            "text B"
        );
        assert_eq!(
            store.list_all("one", 10).await.unwrap()[0].content_preview,
            "text B"
        );
    }

    #[tokio::test]
    async fn media_backfill_pairs_all_types_and_commits_media_only_and_mixed_corpora() {
        use super::super::embedding::tests::{MockServer, response};
        for mixed in [false, true] {
            let mock = MockServer::new(vec![(200, response(vec![1., 0., 0.])); 5]).await;
            let ws = tempfile::tempdir().unwrap();
            let dir = ws.path().join("instances/one/memory");
            std::fs::create_dir_all(&dir).unwrap();
            for path in ["photo.png", "paper.pdf", "clip.mov", "voice.wav"] {
                std::fs::write(dir.join(path), [0xff, 0x81]).unwrap();
                super::super::media_text::write(&dir, path, &format!("representation of {path}"))
                    .unwrap();
            }
            if mixed {
                std::fs::write(dir.join("note.md"), "independent note").unwrap();
            }
            let store = VectorStore::connect_with_config(ws.path(), &mock.config).await;
            let count = 4 + usize::from(mixed);
            assert_eq!(
                store
                    .backfill_text_memories(ws.path(), "one")
                    .await
                    .unwrap(),
                count
            );
            let records = store.list_all("one", 20).await.unwrap();
            assert_eq!(records.len(), count);
            assert!(!store.needs_backfill("one").await.unwrap());
            for record in records.iter().filter(|r| r.path != "note.md") {
                assert_eq!(
                    record.source_type,
                    super::super::media_text::source_type(&record.path).unwrap()
                );
                assert_eq!(record.upload_id.as_deref(), Some(record.path.as_str()));
                assert_eq!(
                    record.content_preview,
                    format!("representation of {}", record.path)
                );
            }
            assert_eq!(mock.requests.lock().unwrap().len(), count);
            assert!(
                mock.requests
                    .lock()
                    .unwrap()
                    .iter()
                    .all(|r| r.2["input"][0].as_str().is_some())
            );
            drop(store);
            let restored = VectorStore::connect_with_config(ws.path(), &mock.config).await;
            assert!(!restored.needs_backfill("one").await.unwrap());
            assert_eq!(restored.list_all("one", 20).await.unwrap().len(), count);
        }
    }

    #[tokio::test]
    async fn media_backfill_missing_unreadable_empty_representations_commit_valid_text_and_retry() {
        use super::super::embedding::tests::{MockServer, response};
        let mock = MockServer::new(vec![(200, response(vec![1., 0., 0.]))]).await;
        let ws = tempfile::tempdir().unwrap();
        let dir = ws.path().join("instances/one/memory");
        std::fs::create_dir_all(&dir).unwrap();
        for path in ["missing.png", "invalid.pdf", "empty.mp3", "directory.mov"] {
            std::fs::write(dir.join(path), [0xff]).unwrap();
        }
        std::fs::write(dir.join("invalid.pdf.md"), [0xff]).unwrap();
        std::fs::write(dir.join("empty.mp3.md"), " \n").unwrap();
        std::fs::create_dir(dir.join("directory.mov.md")).unwrap();
        std::fs::write(dir.join("note.md"), "valid text").unwrap();
        let store = VectorStore::connect_with_config(ws.path(), &mock.config).await;
        assert_eq!(
            store
                .backfill_text_memories(ws.path(), "one")
                .await
                .unwrap(),
            1
        );
        assert_eq!(store.list_all("one", 20).await.unwrap()[0].path, "note.md");
        assert!(store.needs_backfill("one").await.unwrap());
        assert_eq!(mock.requests.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn media_reindex_failure_preserves_committed_index_then_replaces_changed_sources() {
        use super::super::embedding::tests::{MockServer, response};
        let mock = MockServer::new(vec![
            (200, response(vec![1., 0., 0.])),
            (200, response(vec![1., 0., 0.])),
            (200, response(vec![0., 1., 0.])),
            (503, serde_json::json!({"error":"offline"})),
            (200, response(vec![0., 1., 0.])),
            (200, response(vec![0., 1., 0.])),
        ])
        .await;
        let ws = tempfile::tempdir().unwrap();
        let dir = ws.path().join("instances/one/memory");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.md"), "note").unwrap();
        std::fs::write(dir.join("photo.png"), [0xff]).unwrap();
        super::super::media_text::write(&dir, "photo.png", "old sky").unwrap();
        let store = VectorStore::connect_with_config(ws.path(), &mock.config).await;
        store
            .backfill_text_memories(ws.path(), "one")
            .await
            .unwrap();
        let index_path = std::fs::read_dir(ws.path().join("vectors"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let before = std::fs::read(&index_path).unwrap();
        std::fs::write(dir.join("photo.png"), [0x81]).unwrap();
        super::super::media_text::write(&dir, "photo.png", "new mountain").unwrap();
        assert!(
            store
                .backfill_text_memories(ws.path(), "one")
                .await
                .is_err()
        );
        assert_eq!(std::fs::read(&index_path).unwrap(), before);
        assert_eq!(
            store.list_all("one", 10).await.unwrap()[1].content_preview,
            "old sky"
        );
        store
            .backfill_text_memories(ws.path(), "one")
            .await
            .unwrap();
        let records = store.list_all("one", 10).await.unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[1].content_preview, "new mountain");
        assert!(!store.needs_backfill("one").await.unwrap());
    }

    #[tokio::test]
    async fn media_text_entrypoint_never_embeds_raw_textual_media() {
        use super::super::embedding::tests::{MockServer, response};
        let mock = MockServer::new(vec![(200, response(vec![1., 0., 0.]))]).await;
        let ws = tempfile::tempdir().unwrap();
        let dir = ws.path().join("instances/one/memory");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("drawing.svg"), "<svg>raw media markup</svg>").unwrap();
        super::super::media_text::write(&dir, "drawing.svg", "a blue star").unwrap();
        let store = VectorStore::connect_with_config(ws.path(), &mock.config).await;
        store
            .index_text("one", "drawing.svg", "<svg>raw media markup</svg>")
            .await
            .unwrap();
        assert_eq!(
            mock.requests.lock().unwrap()[0].2["input"],
            serde_json::json!(["a blue star"])
        );
        assert_eq!(
            store.list_all("one", 10).await.unwrap()[0].source_type,
            "media_image"
        );
    }
    #[tokio::test]
    async fn digest_mismatch_is_refused_by_vector_and_bm25_indexing() {
        use super::super::embedding::tests::{MockServer, response};
        let mock = MockServer::new(vec![(200, response(vec![1., 0., 0.]))]).await;
        let ws = tempfile::tempdir().unwrap();
        let dir = ws.path().join("instances/one/memory");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("photo.png"), b"old bytes").unwrap();
        super::super::media_text::write(&dir, "photo.png", "Orion sky").unwrap();
        let store = VectorStore::connect_with_config(ws.path(), &mock.config).await;
        store.index_media_text("one", "photo.png").await.unwrap();
        assert_eq!(store.search_text("one", "Orion", 10).await.len(), 1);

        std::fs::write(dir.join("photo.png"), b"replacement bytes").unwrap();
        assert!(store.index_media_text("one", "photo.png").await.is_err());
        assert!(store.list_all("one", 10).await.unwrap().is_empty());
        assert!(store.search_text("one", "Orion", 10).await.is_empty());
        assert!(store.needs_backfill("one").await.unwrap());
        assert_eq!(mock.requests.lock().unwrap().len(), 3);
    }

    #[test]
    fn oversized_unicode_line_is_split_on_utf8_boundaries() {
        let input = "é".repeat(1000);
        let chunks = chunk_text(&input);
        assert!(chunks.len() > 1);
        assert!(chunks.iter().all(|chunk| chunk.len() <= 600));
        assert_eq!(chunks.concat(), input);

        let paragraphs = format!("{}\n{}", "a".repeat(300), "b".repeat(300));
        assert!(
            chunk_text(&paragraphs)
                .iter()
                .all(|chunk| chunk.len() <= 600)
        );
    }

    #[tokio::test]
    async fn embedding_backfill_uses_openai_and_commits_only_complete_corpus() {
        use super::super::embedding::tests::{MockServer, response};
        let mock = MockServer::new(vec![
            (200, response(vec![1., 0., 0.])),
            (200, response(vec![0., 1., 0.])),
            (503, serde_json::json!({"error":"offline"})),
        ])
        .await;
        let workspace = tempfile::tempdir().unwrap();
        let memories = workspace.path().join("instances/one/memory");
        std::fs::create_dir_all(&memories).unwrap();
        std::fs::write(memories.join("note.md"), "Orion nebula").unwrap();
        let store = VectorStore::connect_with_config(workspace.path(), &mock.config).await;
        assert_eq!(
            store
                .backfill_text_memories(workspace.path(), "one")
                .await
                .unwrap(),
            1
        );
        assert!(!store.needs_backfill("one").await.unwrap());
        std::fs::write(memories.join("note.md"), "Orion nebula line\n".repeat(100)).unwrap();
        assert!(
            store
                .backfill_text_memories(workspace.path(), "one")
                .await
                .is_err()
        );
        let records = store.list_all("one", 10).await.unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].content_preview, "Orion nebula");
        assert!(!store.needs_backfill("one").await.unwrap());
        assert_eq!(mock.requests.lock().unwrap().len(), 3);
    }

    #[tokio::test]
    async fn media_only_backfill_skips_embedding_and_stays_incomplete() {
        let workspace = tempfile::tempdir().unwrap();
        let memories = workspace.path().join("instances/one/memory");
        std::fs::create_dir_all(&memories).unwrap();
        std::fs::write(memories.join("photo.png"), b"raw image bytes").unwrap();
        let store = VectorStore::connect(workspace.path()).await;

        assert_eq!(
            store
                .backfill_text_memories(workspace.path(), "one")
                .await
                .unwrap(),
            0
        );
        assert!(store.list_all("one", 10).await.unwrap().is_empty());
        assert!(store.needs_backfill("one").await.unwrap());
    }

    #[tokio::test]
    async fn mixed_backfill_commits_text_without_sending_raw_media_and_stays_incomplete() {
        use super::super::embedding::tests::{MockServer, response};
        let mock = MockServer::new(vec![(200, response(vec![1., 0., 0.]))]).await;
        let workspace = tempfile::tempdir().unwrap();
        let memories = workspace.path().join("instances/one/memory");
        std::fs::create_dir_all(&memories).unwrap();
        std::fs::write(memories.join("note.md"), "Orion nebula").unwrap();
        std::fs::write(memories.join("photo.png"), b"raw image bytes").unwrap();
        let store = VectorStore::connect_with_config(workspace.path(), &mock.config).await;

        assert_eq!(
            store
                .backfill_text_memories(workspace.path(), "one")
                .await
                .unwrap(),
            1
        );
        assert_eq!(store.list_all("one", 10).await.unwrap().len(), 1);
        assert!(store.needs_backfill("one").await.unwrap());
        let requests = mock.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].2["input"], serde_json::json!(["Orion nebula"]));
    }

    #[tokio::test]
    async fn embedding_backfill_without_provider_is_retryable_for_nonempty_corpus() {
        let workspace = tempfile::tempdir().unwrap();
        let memories = workspace.path().join("instances/one/memory");
        std::fs::create_dir_all(&memories).unwrap();
        std::fs::write(memories.join("note.md"), "Orion nebula").unwrap();
        let store = VectorStore::connect(workspace.path()).await;
        assert!(
            store
                .backfill_text_memories(workspace.path(), "one")
                .await
                .is_err()
        );
        assert!(store.needs_backfill("one").await.unwrap());
        assert_eq!(
            store
                .backfill_text_memories(workspace.path(), "empty")
                .await
                .unwrap(),
            0
        );
        assert!(!store.needs_backfill("empty").await.unwrap());
    }

    #[test]
    fn embedding_raw_media_paths_do_not_request_embeddings() {
        for source in [
            include_str!("chat.rs"),
            include_str!("memory.rs"),
            include_str!("tools/memory_tools.rs"),
            include_str!("vector.rs"),
        ] {
            assert!(!source.contains(concat!("embedding::", "embed_text_and_image(")));
            assert!(!source.contains(concat!("embedding::", "embed_media(")));
        }
    }

    #[tokio::test]
    async fn embedding_switching_back_never_reuses_a_stale_completed_corpus() {
        let workspace = tempfile::tempdir().unwrap();
        let initial = crate::config::Config::default();
        let mut changed = initial.clone();
        changed.embedding.model = "other-model".into();
        for (cfg, name) in [(&initial, "old.md"), (&changed, "current.md")] {
            let store = VectorStore::connect_with_config(workspace.path(), cfg).await;
            let mut vector = vec![0.; 768];
            vector[0] = 1.;
            store
                .upsert_text_memory("one", name, vec![(name.into(), vector)])
                .await
                .unwrap();
            store.store.mark_backfilled("one").unwrap();
        }
        let store = VectorStore::connect_with_config(workspace.path(), &initial).await;
        assert!(store.needs_backfill("one").await.unwrap());
        assert!(store.list_all("one", 10).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn embedding_live_write_without_key_invalidates_completed_empty_backfill() {
        let workspace = tempfile::tempdir().unwrap();
        let store = VectorStore::connect(workspace.path()).await;
        store
            .backfill_text_memories(workspace.path(), "one")
            .await
            .unwrap();
        assert!(!store.needs_backfill("one").await.unwrap());
        assert!(
            store
                .index_text("one", "new.md", "new memory")
                .await
                .is_err()
        );
        assert!(store.needs_backfill("one").await.unwrap());
    }

    #[tokio::test]
    async fn embedding_failed_live_update_removes_stale_vectors_and_stays_retryable() {
        use super::super::embedding::tests::{MockServer, response};
        let mock = MockServer::new(vec![
            (200, response(vec![1., 0., 0.])),
            (200, response(vec![0., 1., 0.])),
            (503, serde_json::json!({"error":"offline"})),
        ])
        .await;
        let workspace = tempfile::tempdir().unwrap();
        let store = VectorStore::connect_with_config(workspace.path(), &mock.config).await;
        store
            .index_text("one", "note.md", "old note")
            .await
            .unwrap();
        store.store.mark_backfilled("one").unwrap();
        assert!(
            store
                .index_text("one", "note.md", &"updated note\n".repeat(100))
                .await
                .is_err()
        );
        assert!(store.list_all("one", 10).await.unwrap().is_empty());
        assert!(store.needs_backfill("one").await.unwrap());
    }

    #[tokio::test]
    async fn bm25_cache_reuses_index_and_live_write_invalidates_it() {
        use super::super::embedding::tests::{MockServer, response};
        let mock = MockServer::new(vec![
            (200, response(vec![0., 1., 0.])),
            (200, response(vec![0., 1., 0.])),
            (200, response(vec![1., 0., 0.])),
            (200, response(vec![0., 1., 0.])),
        ])
        .await;
        let workspace = tempfile::tempdir().unwrap();
        let memories = workspace.path().join("instances/one/memory");
        std::fs::create_dir_all(&memories).unwrap();
        std::fs::write(memories.join("old.md"), "Orion nebula").unwrap();
        let store = VectorStore::connect_with_config(workspace.path(), &mock.config).await;

        assert_eq!(store.search_context("one", "Orion", 5).await.len(), 1);
        assert_eq!(store.search_context("one", "Orion", 5).await.len(), 1);
        assert_eq!(store.keyword_reindex_count(), 1);

        std::fs::write(memories.join("new.md"), "Andromeda galaxy").unwrap();
        store
            .index_text("one", "new.md", "Andromeda galaxy")
            .await
            .unwrap();
        let hits = store.search_context("one", "Andromeda", 5).await;
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path, "new.md");
        assert_eq!(hits[0].source_type, "memory");
        assert_eq!(store.keyword_reindex_count(), 2);
    }

    #[tokio::test]
    async fn embedding_context_keeps_bm25_when_semantic_match_is_below_rag_threshold() {
        use super::super::embedding::tests::{MockServer, response};
        let mock = MockServer::new(vec![(200, response(vec![0., 1., 0.]))]).await;
        let workspace = tempfile::tempdir().unwrap();
        let memories = workspace.path().join("instances/one/memory");
        std::fs::create_dir_all(&memories).unwrap();
        std::fs::write(memories.join("note.md"), "Orion nebula").unwrap();
        let store = VectorStore::connect_with_config(workspace.path(), &mock.config).await;
        store
            .upsert_text_memory(
                "one",
                "note.md",
                vec![("Orion nebula".into(), vec![1., 0., 0.])],
            )
            .await
            .unwrap();
        let hits = store.search_context("one", "Orion", 5).await;
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].source_type, "memory");
    }

    async fn metadata_change(change: &str) {
        let workspace = tempfile::tempdir().unwrap();
        let mut cfg = crate::config::Config::default();
        let store = VectorStore::connect_with_config(workspace.path(), &cfg).await;
        let mut vector = vec![0.; 768];
        vector[0] = 1.;
        store
            .upsert_text_memory("one", "old.md", vec![("old".into(), vector)])
            .await
            .unwrap();
        store.store.mark_backfilled("one").unwrap();
        drop(store);
        match change {
            "provider" => cfg.embedding.provider = "openai_compatible".into(),
            "model" => cfg.embedding.model = "another-model".into(),
            "dimensions" => cfg.embedding.dimensions = 3,
            _ => cfg.embedding.base_url = "http://127.0.0.1:1/v1".into(),
        }
        let store = VectorStore::connect_with_config(workspace.path(), &cfg).await;
        assert!(store.needs_backfill("one").await.unwrap(), "{change}");
        assert!(
            store.list_all("one", 10).await.unwrap().is_empty(),
            "{change}"
        );
        let mut vector = vec![0.; cfg.embedding.dimensions as usize];
        vector[0] = 1.;
        store
            .upsert_text_memory("one", "new.md", vec![("new".into(), vector)])
            .await
            .unwrap();
        store.store.mark_backfilled("one").unwrap();
        drop(store);
        let restored = VectorStore::connect_with_config(workspace.path(), &cfg).await;
        assert!(!restored.needs_backfill("one").await.unwrap());
        assert_eq!(
            restored.list_all("one", 10).await.unwrap()[0].path,
            "new.md"
        );
    }
    #[tokio::test]
    async fn embedding_provider_change_rebuilds() {
        metadata_change("provider").await;
    }
    #[tokio::test]
    async fn embedding_model_change_rebuilds() {
        metadata_change("model").await;
    }
    #[tokio::test]
    async fn embedding_dimensions_change_rebuilds() {
        metadata_change("dimensions").await;
    }
    #[tokio::test]
    async fn embedding_endpoint_change_rebuilds() {
        metadata_change("base_url").await;
    }

    #[tokio::test]
    async fn empty_backfill_marks_only_its_companion_complete() {
        let workspace =
            std::env::temp_dir().join(format!("backfill-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&workspace).unwrap();
        let store = VectorStore::connect(&workspace).await;
        assert!(store.needs_backfill("one").await.unwrap());
        assert_eq!(
            store
                .backfill_text_memories(&workspace, "one")
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
                .backfill_text_memories(&workspace, "one")
                .await
                .is_err()
        );
        assert!(store.needs_backfill("one").await.unwrap());
        std::fs::remove_file(memory.join("broken.md")).unwrap();
        assert_eq!(
            store
                .backfill_text_memories(&workspace, "one")
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
            .backfill_text_memories(&workspace, "one")
            .await
            .unwrap();
        let mut vector = vec![0.; crate::config::EmbeddingConfig::default().dimensions as usize];
        vector[0] = 1.;
        store
            .upsert_text_memory("one", "prior.md", vec![("prior".into(), vector)])
            .await
            .unwrap();
        std::fs::create_dir_all(&memory).unwrap();
        std::fs::write(memory.join("broken.md"), [0xff]).unwrap();

        assert!(
            store
                .backfill_text_memories(&workspace, "one")
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
        let mut vector = vec![0.; crate::config::EmbeddingConfig::default().dimensions as usize];
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
    async fn text_boundary_preserves_chunk_metadata_and_deletion() {
        let workspace =
            std::env::temp_dir().join(format!("vector-api-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&workspace).unwrap();
        let store = VectorStore::connect(&workspace).await;
        let mut vector = vec![0.; crate::config::EmbeddingConfig::default().dimensions as usize];
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
        assert_eq!(store.list_all("slug", 10).await.unwrap().len(), 2);
        drop(store);
        let store = VectorStore::connect(&workspace).await;
        let hits = store.search("slug", vector.clone(), 10).await.unwrap();
        assert!(hits[0].upload_id.is_none());
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
