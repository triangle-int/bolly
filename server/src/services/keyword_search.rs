//! BM25 keyword search over memory files — complements vector similarity search.

use std::collections::HashMap;
use std::path::Path;
use std::sync::RwLock;

use bm25::SearchEngineBuilder;

use super::vector::VectorSearchResult;

/// Global BM25 index cache keyed by instance slug.
pub struct KeywordStore {
    indices: RwLock<HashMap<String, IndexData>>,
}

struct IndexData {
    engine: bm25::SearchEngine<u32>,
    /// doc_id (u32) → (relative_path, content_preview)
    meta: Vec<(String, String)>,
}

impl KeywordStore {
    pub fn new() -> Self {
        Self {
            indices: RwLock::new(HashMap::new()),
        }
    }

    /// Rebuild the BM25 index for an instance by scanning its memory directory.
    pub fn reindex(&self, workspace_dir: &Path, instance_slug: &str) {
        let memory_dir = workspace_dir
            .join("instances")
            .join(instance_slug)
            .join("memory");

        if !memory_dir.exists() {
            return;
        }

        let mut meta = Vec::new();
        let mut docs = Vec::new();
        collect_md_files(&memory_dir, &memory_dir, &mut meta, &mut docs);

        if docs.is_empty() {
            return;
        }

        let doc_count = docs.len();
        let engine = SearchEngineBuilder::<u32>::with_corpus(
            bm25::LanguageMode::Fixed(bm25::Language::English),
            docs,
        )
        .build();

        let mut indices = self.indices.write().unwrap_or_else(|e| e.into_inner());
        indices.insert(instance_slug.to_string(), IndexData { engine, meta });
        log::info!("[bm25] indexed {doc_count} memory files for {instance_slug}");
    }

    /// Search the BM25 index. Returns results in the same format as vector search.
    pub fn search(
        &self,
        instance_slug: &str,
        query: &str,
        limit: usize,
    ) -> Vec<VectorSearchResult> {
        // Ensure index exists (lazy reindex)
        let indices = self.indices.read().unwrap_or_else(|e| e.into_inner());
        let index = match indices.get(instance_slug) {
            Some(idx) => idx,
            None => return Vec::new(),
        };

        let results = index.engine.search(query, limit);

        results
            .into_iter()
            .filter_map(|r| {
                let doc_id = r.document.id as usize;
                let (path, preview) = index.meta.get(doc_id)?.clone();
                Some(VectorSearchResult {
                    path,
                    source_type: "memory".to_string(),
                    content_preview: preview,
                    score: r.score,
                    upload_id: None,
                })
            })
            .collect()
    }

    /// Check if an index exists for an instance.
    pub fn has_index(&self, instance_slug: &str) -> bool {
        let indices = self.indices.read().unwrap_or_else(|e| e.into_inner());
        indices.contains_key(instance_slug)
    }

    /// Invalidate the index for an instance (e.g. after memory write/delete).
    #[allow(dead_code)]
    pub fn invalidate(&self, instance_slug: &str) {
        let mut indices = self.indices.write().unwrap_or_else(|e| e.into_inner());
        indices.remove(instance_slug);
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
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            collect_md_files(base, &path, meta, docs);
        } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
            let rel = path
                .strip_prefix(base)
                .unwrap_or(&path)
                .to_string_lossy()
                .to_string();

            let content = std::fs::read_to_string(&path).unwrap_or_default();
            if content.trim().is_empty() {
                continue;
            }
            let preview: String = content.chars().take(300).collect();

            meta.push((rel, preview));
            docs.push(content);
        }
    }
}
