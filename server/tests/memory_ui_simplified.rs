//! Guard for #96: the Memory surface is a searchable library, not a graph or
//! vector-debug console, and derived-index recovery needs no button.

#[path = "../test-support/source_scan.rs"]
mod source_scan;

use source_scan::without_cfg_test_items;
use std::{
    fs,
    path::{Path, PathBuf},
};

fn files(root: &Path, extensions: &[&str]) -> Vec<PathBuf> {
    fn visit(dir: &Path, extensions: &[&str], out: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(&path, extensions, out);
            } else if path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| extensions.contains(&ext))
            {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    visit(root, extensions, &mut out);
    out.sort();
    out
}

#[test]
fn memory_map_vector_debug_and_manual_reindex_are_absent() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();

    assert!(
        !repo
            .join("client/src/lib/components/memory/MemoryMapView.svelte")
            .exists(),
        "the circle-packing / force-graph memory map still exists"
    );
    for required in [
        "client/src/lib/components/memory/MemoryLibraryView.svelte",
        "client/src/lib/memory/library.js",
    ] {
        assert!(
            repo.join(required).exists(),
            "memory library UI is missing: {required}"
        );
    }

    let mut violations = Vec::new();
    for path in files(&repo.join("server/src"), &["rs"]) {
        let relative = path
            .strip_prefix(repo)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let production = without_cfg_test_items(&fs::read_to_string(&path).unwrap());
        for token in [
            "list_vectors",
            "reindex_memory",
            "/memory/vectors\"",
            "/memory/reindex\"",
        ] {
            if production.contains(token) {
                violations.push(format!("{relative} contains {token:?}"));
            }
        }
    }
    for path in files(&repo.join("client/src"), &["ts", "js", "svelte"]) {
        let relative = path
            .strip_prefix(repo)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let source = fs::read_to_string(&path).unwrap();
        for token in [
            "fetchVectors",
            "VectorEntry",
            "reindexMemory",
            "/memory/vectors",
            "/memory/reindex",
            "graphMode",
            "packCircles",
            "force-directed",
            "debugOpen",
            "score.toFixed",
            "content_preview",
        ] {
            if source.contains(token) {
                violations.push(format!("{relative} contains {token:?}"));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "memory graph/debug surface remains:\n{}",
        violations.join("\n")
    );
}

#[test]
fn derived_index_recovery_is_documented_as_automatic() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let doc = fs::read_to_string(repo.join("docs/companion-storage.md")).unwrap();
    for required in [
        "#96",
        "vectors/",
        "automatic",
        "needs_backfill",
        "no button",
    ] {
        assert!(
            doc.contains(required),
            "storage doc is missing {required:?}"
        );
    }
}
