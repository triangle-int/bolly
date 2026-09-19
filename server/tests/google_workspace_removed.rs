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
fn native_google_workspace_and_token_broker_surfaces_are_absent() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let mut files = production_files(repo, "server/src", &["rs"]);
    files.extend(production_files(repo, "client/src", &["ts", "svelte"]));
    files.extend(production_files(repo, "landing/src", &["ts", "svelte"]));

    let forbidden = [
        "GoogleClient",
        "routes::google",
        "pub mod google;",
        "/google/accounts",
        "/google/connect",
        "api/google-token",
        "fetchGoogleAccounts",
        "getGoogleConnectUrl",
        "disconnectGoogleAccount",
        "gmail.googleapis.com",
        "www.googleapis.com/calendar",
        "www.googleapis.com/drive",
        "list_drive_files",
        "read_drive_file",
        "upload_drive_file",
        "list_events",
        "create_event",
    ];

    let mut violations = Vec::new();
    for path in files {
        let text = fs::read_to_string(&path).unwrap();
        for needle in forbidden {
            if text.contains(needle) {
                violations.push(format!(
                    "{} contains {needle:?}",
                    path.strip_prefix(repo).unwrap().display()
                ));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "native Workspace remnants:\n{}",
        violations.join("\n")
    );

    // Gemini/Google AI video analysis was retired in #91; see
    // tests/video_analysis_removed.rs for that guard.
}
