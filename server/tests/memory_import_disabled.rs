#[path = "../test-support/source_scan.rs"]
mod source_scan;

use source_scan::without_cfg_test_items;
use std::{
    fs,
    path::{Path, PathBuf},
};

fn production_files(root: &Path, relative: &str, extensions: &[&str]) -> Vec<PathBuf> {
    fn visit(path: &Path, extensions: &[&str], out: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(&path, extensions, out);
            } else if path
                .extension()
                .and_then(|value| value.to_str())
                .is_some_and(|ext| extensions.contains(&ext))
            {
                out.push(path);
            }
        }
    }

    let mut files = Vec::new();
    visit(&root.join(relative), extensions, &mut files);
    files
}

#[test]
fn production_scan_detects_forbidden_code_after_early_cfg_test_item() {
    let source = r##"
const BEFORE: &str = "production 月 #[cfg(test)]";
const RAW_BEFORE: &str = r#"#[cfg(test)] is still text"#;
// A comment containing #[cfg(test)] is not an attribute.
#[cfg(test)]
fn generic_test_fixture<T: Into<Result<(), ImportDataTool>>>() {
    let brace = '}';
    let unsafe_only = ".import_temp";
}
#[cfg(test)]
mod tests {
    const UNSAFE_TEST_FIXTURE: &str = "ImportDataTool .import_temp";
    const BRACES: &str = "{ not structure }";
    const RAW: &str = r#"} still not structure"#;
    /* { nested /* } */ comment } */
}
const AFTER: &str = "ImportDataTool";
"##;

    let production = without_cfg_test_items(source);
    let violations = ["ImportDataTool", ".import_temp"]
        .into_iter()
        .filter(|forbidden| production.contains(forbidden))
        .collect::<Vec<_>>();

    assert!(production.contains("BEFORE"));
    assert!(production.contains("AFTER"));
    assert_eq!(violations, vec!["ImportDataTool"]);
    assert!(!production.contains("UNSAFE_TEST_FIXTURE"));
}

#[test]
fn unsafe_memory_import_pipeline_and_entry_points_are_absent() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();

    for removed in [
        "server/src/services/memory_import.rs",
        "server/src/services/tools/import_data.rs",
    ] {
        assert!(
            !repo.join(removed).exists(),
            "unsafe import source still exists: {removed}"
        );
    }

    let mut violations = Vec::new();
    for path in production_files(repo, "server/src", &["rs"]) {
        let source = fs::read_to_string(&path).unwrap();
        let production = without_cfg_test_items(&source);
        for forbidden in [
            "ImportDataTool",
            "import_knowledge",
            "pub mod import_data",
            ".import_temp",
        ] {
            if production.contains(forbidden) {
                violations.push(format!(
                    "{} contains {forbidden:?}",
                    path.strip_prefix(repo).unwrap().display()
                ));
            }
        }
    }
    for path in production_files(repo, "client/src", &["ts", "svelte"]) {
        let source = fs::read_to_string(&path).unwrap();
        for forbidden in ["importKnowledge", "import_progress", "/memory/import"] {
            if source.contains(forbidden) {
                violations.push(format!(
                    "{} contains {forbidden:?}",
                    path.strip_prefix(repo).unwrap().display()
                ));
            }
        }
    }

    let route = fs::read_to_string(repo.join("server/src/routes/memory_import.rs")).unwrap();
    let production_route = without_cfg_test_items(&route);
    for forbidden in [
        "Multipart",
        "DefaultBodyLimit",
        "create_dir",
        "remove_dir",
        "std::fs",
        ".join(",
        "file_name",
    ] {
        if production_route.contains(forbidden) {
            violations.push(format!(
                "server/src/routes/memory_import.rs contains unsafe operation {forbidden:?}"
            ));
        }
    }

    assert!(
        violations.is_empty(),
        "unsafe memory-import entry points remain:\n{}",
        violations.join("\n")
    );
}
