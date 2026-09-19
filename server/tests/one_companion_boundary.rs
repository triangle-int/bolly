//! Structural guard for #103: production code may enumerate `instances/`
//! only through `services::companion`, and the storage layout is documented.

#[path = "../test-support/source_scan.rs"]
mod source_scan;

use source_scan::without_cfg_test_items;
use std::{
    fs,
    path::{Path, PathBuf},
};

fn rust_files(root: &Path) -> Vec<PathBuf> {
    fn visit(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                out.push(path);
            }
        }
    }
    let mut files = Vec::new();
    visit(root, &mut files);
    files.sort();
    files
}

#[test]
fn only_the_companion_service_enumerates_instance_directories() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let allowed = "server/src/services/companion.rs";
    let forbidden = [
        "instances_dir",
        "read_instances(",
        ".instance_slugs()",
        "count_directories(&instances",
    ];

    let mut violations = Vec::new();
    for path in rust_files(&repo.join("server/src")) {
        let relative = path
            .strip_prefix(repo)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        if relative == allowed {
            continue;
        }
        let production = without_cfg_test_items(&fs::read_to_string(&path).unwrap());
        for token in forbidden {
            if production.contains(token) {
                violations.push(format!("{relative} contains {token:?}"));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "instance enumeration outside {allowed}:\n{}",
        violations.join("\n")
    );
}

#[test]
fn companion_storage_format_is_documented_for_import_work() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let doc = fs::read_to_string(repo.join("docs/companion-storage.md"))
        .expect("docs/companion-storage.md must describe the one-companion layout");
    for required in [
        "instances/companion/",
        "companion.json",
        "format_version",
        "unknown_companion",
        "companion_format_unsupported",
        "#74",
    ] {
        assert!(
            doc.contains(required),
            "storage doc is missing {required:?}"
        );
    }
}

#[test]
fn multi_instance_selection_surfaces_are_absent() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();

    for removed in [
        "server/src/domain/instance.rs",
        "client/src/lib/stores/instances.svelte.ts",
        "client/src/lib/components/home/InstanceCard.svelte",
        "client/src/lib/components/layout/Sidebar.svelte",
    ] {
        assert!(
            !repo.join(removed).exists(),
            "multi-instance source still exists: {removed}"
        );
    }

    let mut violations = Vec::new();
    for path in rust_files(&repo.join("server/src")) {
        let relative = path
            .strip_prefix(repo)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let production = without_cfg_test_items(&fs::read_to_string(&path).unwrap());
        for token in [
            "InstanceDiscovered",
            "InstanceSummary",
            "discover_instance",
            "list_instances",
            "delete_instance",
            "\"/api/instances\"",
        ] {
            if production.contains(token) {
                violations.push(format!("{relative} contains {token:?}"));
            }
        }
    }

    fn client_files(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                client_files(&path, out);
            } else if path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| matches!(ext, "ts" | "js" | "svelte"))
            {
                out.push(path);
            }
        }
    }
    let mut files = Vec::new();
    client_files(&repo.join("client/src"), &mut files);
    files.sort();
    for path in files {
        let relative = path
            .strip_prefix(repo)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let source = fs::read_to_string(&path).unwrap();
        for token in [
            "instance_discovered",
            "InstanceSummary",
            "fetchInstances",
            "deleteInstance",
            "getInstances",
            "setInstances",
            "selectInstance",
            "\"/api/instances\"",
            "All companions",
            "Delete companion",
            "Create your companion",
        ] {
            if source.contains(token) {
                violations.push(format!("{relative} contains {token:?}"));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "companion creation, deletion, or switching surfaces remain:\n{}",
        violations.join("\n")
    );
}
