//! Versioned local vector search for derived semantic memory.

use super::{
    embedding,
    vector_index::{Record, Store},
};
use std::path::Path;

pub struct VectorStore {
    store: std::sync::Arc<Store>,
    keywords: std::sync::Arc<super::keyword_search::KeywordStore>,
    embedding: embedding::EmbeddingService,
    workspace: std::path::PathBuf,
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
        let workspace = data_dir.to_owned();
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
            keywords: std::sync::Arc::new(super::keyword_search::KeywordStore::new()),
            embedding: embedding::EmbeddingService::from_config(config),
            workspace,
        }
    }

    pub fn embedding_status(&self) -> serde_json::Value {
        self.embedding.status()
    }

    pub fn embedding_needs_restart(&self, config: &crate::config::Config) -> bool {
        self.embedding.needs_restart(config)
    }

    /// Embed and replace a complete document under the same lock as strict backfill.
    pub async fn index_text(&self, slug: &str, path: &str, content: &str) -> Result<(), String> {
        self.keywords.invalidate(slug);
        let mutation = self.store.mutation_lock(slug);
        let _guard = mutation.lock_owned().await;
        let result = async {
            self.embedding.ensure_configured()?;
            let mut records = Vec::new();
            for (i, text) in chunk_text(content).into_iter().enumerate() {
                let vector = self.embedding.document(&text).await?;
                records.push(Record {
                    path: path.into(),
                    source_type: "text_memory".into(),
                    chunk_index: i as u32,
                    content_preview: text.chars().take(500).collect(),
                    upload_id: None,
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
        let workspace = self.workspace.clone();
        let keywords = self.keywords.clone();
        let companion = slug.to_owned();
        let query = query.to_owned();
        let keywords = tokio::task::spawn_blocking(move || {
            if !keywords.has_index(&companion) {
                keywords.reindex(&workspace, &companion);
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

    pub async fn upsert_text_memory(
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

    /// Guard the public boundary against accidental cross-space raw-media vectors.
    #[allow(dead_code)]
    pub async fn upsert_media(
        &self,
        _instance_slug: &str,
        _upload_id: &str,
        _source_type: &str,
        _mime_type: &str,
        _original_name: &str,
        _content_preview: &str,
        _vector: Vec<f32>,
    ) -> Result<(), String> {
        Err(
            "raw media semantic indexing is unavailable for text embeddings; file is preserved"
                .into(),
        )
    }

    pub async fn delete_by_path(&self, instance_slug: &str, path: &str) -> Result<(), String> {
        self.keywords.invalidate(instance_slug);
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

    /// Strictly backfill text memories; raw media awaits descriptions/transcripts (#60).
    pub async fn backfill_text_memories(
        &self,
        workspace_dir: &Path,
        instance_slug: &str,
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
        let mut skipped_media = false;
        let mut provider_checked = false;

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
                skipped_media = true;
                log::info!(
                    "[backfill] raw media skipped for {}: text embeddings require a description/transcript",
                    entry.path
                );
                continue;
            } else {
                if !provider_checked {
                    self.embedding.ensure_configured()?;
                    provider_checked = true;
                }
                let display_path = entry.path.clone();
                let content =
                    tokio::task::spawn_blocking(move || std::fs::read_to_string(&file_path))
                        .await
                        .map_err(|error| format!("backfill read task failed: {error}"))?
                        .map_err(|error| format!("backfill read {display_path}: {error}"))?;
                let chunks = chunk_text(&content);
                for (chunk_index, chunk) in chunks.into_iter().enumerate() {
                    let vector = self
                        .embedding
                        .document(&chunk)
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

    #[tokio::test]
    async fn embedding_raw_media_vectors_cannot_enter_text_index() {
        let workspace = tempfile::tempdir().unwrap();
        let store = VectorStore::connect(workspace.path()).await;
        let mut google_vector = vec![0.; 768];
        google_vector[0] = 1.;
        assert!(
            store
                .upsert_media(
                    "one",
                    "photo.png",
                    "media_image",
                    "image/png",
                    "photo",
                    "photo",
                    google_vector
                )
                .await
                .is_err()
        );
        assert!(store.list_all("one", 10).await.unwrap().is_empty());
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
    async fn public_boundary_preserves_text_and_rejects_raw_media() {
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
        assert!(
            store
                .upsert_media(
                    "slug",
                    "notes.md",
                    "media_image",
                    "image/png",
                    "name",
                    "preview",
                    vector.clone()
                )
                .await
                .is_err()
        );
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
