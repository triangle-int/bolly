//! Guard for #97: curated skills stay, raw prompt authoring and unreviewed
//! extension surfaces are gone from the default product, and MCP Apps render
//! only inside a documented sandbox.

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
fn raw_skill_authoring_and_unsandboxed_extension_surfaces_are_absent() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();

    assert!(
        !repo
            .join("client/src/lib/components/skills/CreateSkillModal.svelte")
            .exists(),
        "the raw prompt skill-authoring modal still exists"
    );
    assert!(
        repo.join("client/src/lib/extensions/trust.js").exists(),
        "extension trust helpers are missing"
    );

    let mut violations = Vec::new();
    for path in files(&repo.join("server/src"), &["rs"]) {
        let relative = path
            .strip_prefix(repo)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let production = without_cfg_test_items(&fs::read_to_string(&path).unwrap());
        for token in [
            "skill_creator",
            "Skill Creator",
            "fn create_skill",
            "post(create_skill)",
            ".tools_as_dyn()",
            "add_mcp_server: Option",
            "remove_mcp_server: Option",
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
            "CreateSkillModal",
            "createSkill(",
            "New skill",
            "allow-same-origin",
            "allow-popups",
            "allow-top-navigation",
            "doc.write(",
        ] {
            if source.contains(token) {
                violations.push(format!("{relative} contains {token:?}"));
            }
        }
    }

    let viewer =
        fs::read_to_string(repo.join("client/src/lib/components/chat/McpAppViewer.svelte"))
            .unwrap();
    if !viewer.contains("sandbox=\"allow-scripts\"") || !viewer.contains("srcdoc") {
        violations.push(
            "McpAppViewer must render through srcdoc with sandbox=\"allow-scripts\" only".into(),
        );
    }
    let client_api = fs::read_to_string(repo.join("client/src/lib/api/client.ts")).unwrap();
    let settings =
        fs::read_to_string(repo.join("client/src/routes/[slug]/settings/+page.svelte")).unwrap();
    if !client_api.contains("acknowledge_untrusted") || !settings.contains("acknowledgeUntrusted") {
        violations
            .push("Settings must send an explicit acknowledgement for custom MCP servers".into());
    }

    assert!(
        violations.is_empty(),
        "unreviewed extension surface remains:\n{}",
        violations.join("\n")
    );
}

#[test]
fn extension_trust_model_is_documented() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let doc = fs::read_to_string(repo.join("docs/extensions.md"))
        .expect("docs/extensions.md must describe the extension trust model");
    for required in [
        "#97",
        "curated",
        "custom",
        "acknowledge_untrusted",
        "enabled_tools",
        "headers",
        "sandbox",
        "srcdoc",
        "allow-scripts",
        "threat model",
        "skills/",
    ] {
        assert!(
            doc.contains(required),
            "extensions doc is missing {required:?}"
        );
    }
}
