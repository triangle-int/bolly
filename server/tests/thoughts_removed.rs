//! Guard for #94: no raw Thoughts store, route, view, or event remains, and
//! proactive actions are explained through activity receipts instead.

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
fn raw_thoughts_store_route_view_and_event_are_absent() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();

    for removed in [
        "server/src/services/thoughts.rs",
        "server/src/domain/thought.rs",
        "server/src/routes/thoughts.rs",
        "client/src/lib/components/thoughts",
        "client/src/routes/[slug]/thoughts",
    ] {
        assert!(
            !repo.join(removed).exists(),
            "raw thoughts source still exists: {removed}"
        );
    }
    for required in [
        "client/src/lib/components/activity/ActivityView.svelte",
        "client/src/routes/[slug]/activity/+page.svelte",
        "client/src/lib/activity/receipts.js",
    ] {
        assert!(
            repo.join(required).exists(),
            "activity receipts UI is missing: {required}"
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
            "save_thought",
            "list_thoughts",
            "HeartbeatThought",
            "thoughts::",
            "mod thoughts",
            "domain::thought",
            "/thoughts\"",
            "inner monologue",
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
            "fetchThoughts",
            "ThoughtsView",
            "heartbeat_thought",
            "interface Thought ",
            "\"thoughts\"",
            "/thoughts",
        ] {
            if source.contains(token) {
                violations.push(format!("{relative} contains {token:?}"));
            }
        }
    }
    // Routine results no longer carry the model's private text.
    let routine =
        fs::read_to_string(repo.join("server/src/services/companion_routine.rs")).unwrap();
    if without_cfg_test_items(&routine).contains("pub response: String") {
        violations.push("companion_routine.rs still returns the raw response".into());
    }

    assert!(
        violations.is_empty(),
        "raw thought surface remains:\n{}",
        violations.join("\n")
    );
}

#[test]
fn activity_receipts_are_documented() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let doc = fs::read_to_string(repo.join("docs/proactive-loop.md")).unwrap();
    for required in ["#94", "activity_updated", "Activity tab", "thoughts/"] {
        assert!(
            doc.contains(required),
            "proactive doc is missing {required:?}"
        );
    }
    let storage = fs::read_to_string(repo.join("docs/companion-storage.md")).unwrap();
    assert!(
        storage.contains("thoughts/"),
        "storage doc must list thoughts/ as retired"
    );
}
