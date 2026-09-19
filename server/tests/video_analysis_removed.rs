//! Guard for #91: Nolune core ships no Gemini/YouTube/ffmpeg video-analysis
//! stack and never installs executables at runtime.

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
fn video_analysis_stack_and_runtime_installers_are_absent() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();

    assert!(
        !repo.join("server/src/services/tools/media.rs").exists(),
        "the Gemini/yt-dlp/ffmpeg media tool module still exists"
    );

    let mut violations = Vec::new();

    let server_forbidden = [
        "watch_video",
        "WatchVideoTool",
        "MediaContext",
        "google_ai_key",
        "tokens.google_ai",
        "env::var(\"GOOGLE_AI_API_KEY\")",
        "generativelanguage",
        "gemini-",
        "yt-dlp",
        "ffmpeg",
        "pipx",
        "/usr/local/bin",
    ];
    for path in files(&repo.join("server/src"), &["rs"]) {
        let relative = path
            .strip_prefix(repo)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let production = without_cfg_test_items(&fs::read_to_string(&path).unwrap());
        for token in server_forbidden {
            if production.contains(token) {
                violations.push(format!("{relative} contains {token:?}"));
            }
        }
    }

    let client_forbidden = ["google_ai", "Google AI", "Video analysis", "watch_video"];
    for path in files(&repo.join("client/src"), &["ts", "js", "svelte"]) {
        let relative = path
            .strip_prefix(repo)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let source = fs::read_to_string(&path).unwrap();
        for token in client_forbidden {
            if source.contains(token) {
                violations.push(format!("{relative} contains {token:?}"));
            }
        }
    }

    let installer = fs::read_to_string(repo.join("scripts/install.sh")).unwrap();
    for token in ["GOOGLE_AI", "yt-dlp", "ffmpeg", "media analysis"] {
        if installer.contains(token) {
            violations.push(format!("scripts/install.sh contains {token:?}"));
        }
    }

    assert!(
        violations.is_empty(),
        "video-analysis surface remains:\n{}",
        violations.join("\n")
    );
}

#[test]
fn video_analysis_is_documented_as_a_future_reviewed_skill() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let doc = fs::read_to_string(repo.join("docs/video-analysis.md"))
        .expect("docs/video-analysis.md must explain the removal and the skill path");
    for required in ["#91", "skill", "yt-dlp", "attachment"] {
        assert!(doc.contains(required), "video doc is missing {required:?}");
    }
}
