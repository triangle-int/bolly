//! BM25 keyword search over memory files — complements vector similarity search.

use std::collections::HashMap;
use std::path::Path;
use std::sync::RwLock;

use bm25::SearchEngineBuilder;

use super::vector::VectorSearchResult;

/// Cached BM25 indexes plus a per-companion generation that prevents a stale
/// rebuild from publishing after a concurrent write or delete invalidated it.
pub struct KeywordStore {
    state: RwLock<KeywordState>,
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
    pub fn new() -> Self {
        Self {
            state: RwLock::new(KeywordState::default()),
            #[cfg(test)]
            reindex_count: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// Rebuild the BM25 index for an instance. The scan happens without holding
    /// the state lock; publication succeeds only if no invalidation happened in
    /// the meantime.
    pub fn reindex(&self, workspace_dir: &Path, instance_slug: &str) {
        #[cfg(test)]
        self.reindex_count
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let observed_generation = self.generation(instance_slug);
        let memory_dir = workspace_dir
            .join("instances")
            .join(instance_slug)
            .join("memory");

        let mut meta = Vec::new();
        let mut docs = Vec::new();
        if memory_dir.exists() {
            collect_md_files(&memory_dir, &memory_dir, &mut meta, &mut docs);
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
                Some(VectorSearchResult {
                    path,
                    source_type: "memory".to_string(),
                    content_preview: preview,
                    score: result.score,
                    upload_id: None,
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

/// Recursively collect .md files: meta (path, preview) + full content for corpus.
fn collect_md_files(
    base: &Path,
    dir: &Path,
    meta: &mut Vec<(String, String)>,
    docs: &mut Vec<String>,
) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };

    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            collect_md_files(base, &path, meta, docs);
        } else if path.extension().and_then(|extension| extension.to_str()) == Some("md") {
            let relative = path
                .strip_prefix(base)
                .unwrap_or(&path)
                .to_string_lossy()
                .to_string();
            let content = std::fs::read_to_string(&path).unwrap_or_default();
            if content.trim().is_empty() {
                continue;
            }
            let preview: String = content.chars().take(300).collect();
            meta.push((relative, preview));
            docs.push(content);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_reindex_snapshot_cannot_overtake_invalidation() {
        let store = KeywordStore::new();
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
