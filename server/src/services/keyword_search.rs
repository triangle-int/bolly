//! BM25 keyword search over memory files — complements vector similarity search.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use bm25::SearchEngineBuilder;

use super::vector::VectorSearchResult;

/// Cached BM25 indexes plus a per-companion generation that prevents a stale
/// rebuild from publishing after a concurrent write or delete invalidated it.
pub struct KeywordStore {
    state: RwLock<KeywordState>,
    media: Arc<super::media_text::MediaStore>,
    #[cfg(test)]
    reindex_count: std::sync::atomic::AtomicUsize,
}

#[derive(Default)]
struct KeywordState {
    indices: HashMap<String, IndexData>,
    generations: HashMap<String, u64>,
}

struct IndexData {
    engine: Option<bm25::SearchEngine<u32>>,
    /// doc_id (u32) → (relative_path, content_preview)
    meta: Vec<(String, String)>,
}

impl KeywordStore {
    pub fn new(media: Arc<super::media_text::MediaStore>) -> Self {
        Self {
            state: RwLock::new(KeywordState::default()),
            media,
            #[cfg(test)]
            reindex_count: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// Rebuild the BM25 index for an instance. The scan happens without holding
    /// the state lock; publication succeeds only if no invalidation happened in
    /// the meantime.
    pub fn reindex(&self, instance_slug: &str) {
        #[cfg(test)]
        self.reindex_count
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let observed_generation = self.generation(instance_slug);
        let mut meta = Vec::new();
        let mut docs = Vec::new();
        if let Ok(paths) = self.media.memory_files(instance_slug) {
            for path in paths {
                let content = if super::media_text::source_type(&path).is_some() {
                    let Ok(content) = self.media.read(instance_slug, &path) else {
                        continue;
                    };
                    content
                } else {
                    let Ok(content) = self.media.read_memory_text(instance_slug, &path) else {
                        continue;
                    };
                    content
                };
                if content.trim().is_empty() {
                    continue;
                }
                meta.push((path, content.chars().take(300).collect()));
                docs.push(content);
            }
        }

        let doc_count = docs.len();
        let engine = (!docs.is_empty()).then(|| {
            SearchEngineBuilder::<u32>::with_corpus(
                bm25::LanguageMode::Fixed(bm25::Language::English),
                docs,
            )
            .build()
        });
        if self.commit_if_current(
            instance_slug,
            observed_generation,
            IndexData { engine, meta },
        ) {
            log::info!("[bm25] indexed {doc_count} memory files for {instance_slug}");
        } else {
            log::debug!("[bm25] discarded stale rebuild for {instance_slug}");
        }
    }

    /// Search the cached BM25 index. An empty corpus is cached explicitly.
    pub fn search(
        &self,
        instance_slug: &str,
        query: &str,
        limit: usize,
    ) -> Vec<VectorSearchResult> {
        let state = self.state.read().unwrap_or_else(|error| error.into_inner());
        let Some(index) = state.indices.get(instance_slug) else {
            return Vec::new();
        };
        let Some(engine) = &index.engine else {
            return Vec::new();
        };

        engine
            .search(query, limit)
            .into_iter()
            .filter_map(|result| {
                let doc_id = result.document.id as usize;
                let (path, preview) = index.meta.get(doc_id)?.clone();
                let media_type = super::media_text::source_type(&path);
                Some(VectorSearchResult {
                    upload_id: media_type.map(|_| path.clone()),
                    path,
                    source_type: media_type.unwrap_or("memory").to_string(),
                    content_preview: preview,
                    score: result.score,
                })
            })
            .collect()
    }

    pub fn has_index(&self, instance_slug: &str) -> bool {
        self.state
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .indices
            .contains_key(instance_slug)
    }

    #[cfg(test)]
    pub fn reindex_count(&self) -> usize {
        self.reindex_count
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Invalidate both the current index and every rebuild that observed the
    /// prior generation.
    pub fn invalidate(&self, instance_slug: &str) {
        let mut state = self
            .state
            .write()
            .unwrap_or_else(|error| error.into_inner());
        let generation = state
            .generations
            .entry(instance_slug.to_owned())
            .or_default();
        *generation = generation.wrapping_add(1);
        state.indices.remove(instance_slug);
    }

    fn generation(&self, instance_slug: &str) -> u64 {
        self.state
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .generations
            .get(instance_slug)
            .copied()
            .unwrap_or(0)
    }

    fn commit_if_current(
        &self,
        instance_slug: &str,
        observed_generation: u64,
        index: IndexData,
    ) -> bool {
        let mut state = self
            .state
            .write()
            .unwrap_or_else(|error| error.into_inner());
        if state.generations.get(instance_slug).copied().unwrap_or(0) != observed_generation {
            return false;
        }
        state.indices.insert(instance_slug.to_owned(), index);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_sidecar_keyword_hits_belong_to_media_and_orphans_are_excluded() {
        let ws = tempfile::tempdir().unwrap();
        let dir = ws.path().join("instances/one/memory");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("photo.png"), [0xff]).unwrap();
        super::super::media_text::write(&dir, "photo.png", "Orion mountain").unwrap();
        std::fs::write(dir.join("orphan.pdf.md"), "Orion mountain").unwrap();
        let store = KeywordStore::new(Arc::new(
            super::super::media_text::MediaStore::open(ws.path()).unwrap(),
        ));
        store.reindex("one");
        let hits = store.search("one", "Orion", 10);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path, "photo.png");
        assert_eq!(hits[0].source_type, "media_image");
        assert_eq!(hits[0].upload_id.as_deref(), Some("photo.png"));
    }

    #[test]
    fn stale_reindex_snapshot_cannot_overtake_invalidation() {
        let workspace = tempfile::tempdir().unwrap();
        let store = KeywordStore::new(Arc::new(
            super::super::media_text::MediaStore::open(workspace.path()).unwrap(),
        ));
        let observed_generation = store.generation("one");
        store.invalidate("one");
        assert!(!store.commit_if_current(
            "one",
            observed_generation,
            IndexData {
                engine: None,
                meta: Vec::new(),
            },
        ));
        assert!(!store.has_index("one"));
    }
}
