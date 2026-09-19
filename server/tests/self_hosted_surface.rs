use std::{
    fs,
    path::{Path, PathBuf},
};

fn source_files(root: &Path, extensions: &[&str]) -> Vec<PathBuf> {
    fn visit(dir: &Path, extensions: &[&str], files: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(&path, extensions, files);
            } else if path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extensions.contains(&extension))
            {
                files.push(path);
            }
        }
    }

    let mut files = Vec::new();
    visit(root, extensions, &mut files);
    files.sort();
    files
}

fn cfg_test_item_end(source: &str, marker: usize) -> usize {
    let bytes = source.as_bytes();
    let mut index = marker + "#[cfg(test)]".len();
    let mut opening = None;

    while index < bytes.len() {
        match bytes[index] {
            b'{' | b';' | b',' => {
                opening = Some((index, bytes[index]));
                break;
            }
            _ => index += 1,
        }
    }

    let Some((opening_index, delimiter)) = opening else {
        return bytes.len();
    };
    if delimiter != b'{' {
        return opening_index + 1;
    }

    let mut depth = 0usize;
    let mut index = opening_index;
    let mut block_comment_depth = 0usize;
    while index < bytes.len() {
        if block_comment_depth > 0 {
            if bytes.get(index..index + 2) == Some(b"/*") {
                block_comment_depth += 1;
                index += 2;
            } else if bytes.get(index..index + 2) == Some(b"*/") {
                block_comment_depth -= 1;
                index += 2;
            } else {
                index += 1;
            }
            continue;
        }

        if bytes.get(index..index + 2) == Some(b"//") {
            index = source[index..]
                .find('\n')
                .map_or(bytes.len(), |newline| index + newline + 1);
            continue;
        }
        if bytes.get(index..index + 2) == Some(b"/*") {
            block_comment_depth = 1;
            index += 2;
            continue;
        }
        if matches!(bytes[index], b'"' | b'\'') {
            let quote = bytes[index];
            index += 1;
            while index < bytes.len() {
                if bytes[index] == b'\\' {
                    index += 2;
                } else if bytes[index] == quote {
                    index += 1;
                    break;
                } else {
                    index += 1;
                }
            }
            continue;
        }
        if bytes[index] == b'r' {
            let mut cursor = index + 1;
            while bytes.get(cursor) == Some(&b'#') {
                cursor += 1;
            }
            if bytes.get(cursor) == Some(&b'"') {
                let hashes = cursor - index - 1;
                cursor += 1;
                loop {
                    let Some(relative_quote) = source[cursor..].find('"') else {
                        return bytes.len();
                    };
                    cursor += relative_quote + 1;
                    if bytes.get(cursor..cursor + hashes) == Some(&vec![b'#'; hashes]) {
                        index = cursor + hashes;
                        break;
                    }
                }
                continue;
            }
        }

        match bytes[index] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return index + 1;
                }
            }
            _ => {}
        }
        index += 1;
    }
    bytes.len()
}

fn without_cfg_test_items(source: &str) -> String {
    let mut production = String::with_capacity(source.len());
    let mut cursor = 0;
    while let Some(relative_marker) = source[cursor..].find("#[cfg(test)]") {
        let marker = cursor + relative_marker;
        production.push_str(&source[cursor..marker]);
        cursor = cfg_test_item_end(source, marker);
    }
    production.push_str(&source[cursor..]);
    production
}

fn allow_exact(source: &mut String, path: &str, allowed: &str, expected: usize) {
    let actual = source.matches(allowed).count();
    assert_eq!(
        actual, expected,
        "{path}: expected {expected} exact allowed occurrence(s) of {allowed:?}, found {actual}"
    );
    *source = source.replace(allowed, "");
}

#[test]
fn cfg_test_filter_removes_only_test_items() {
    let source = r##"
const BEFORE: &str = "production";
#[cfg(test)]
mod tests {
    const BRACES: &str = "{ not structure }";
    const RAW: &str = r#"} still not structure"#;
    /* { nested /* } */ comment } */
}
const AFTER: &str = "production too";
"##;
    let production = without_cfg_test_items(source);
    assert!(production.contains("BEFORE"));
    assert!(production.contains("AFTER"));
    assert!(!production.contains("BRACES"));
    assert!(!production.contains("RAW"));
}

#[test]
fn managed_control_plane_surfaces_are_absent() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let server_root = repo.join("server/src");
    let client_root = repo.join("client/src");
    let mut sources = Vec::new();

    for path in source_files(&server_root, &["rs"]) {
        let relative = path
            .strip_prefix(repo)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        sources.push((
            relative,
            without_cfg_test_items(&fs::read_to_string(path).unwrap()),
        ));
    }
    for path in source_files(&client_root, &["ts", "svelte"]) {
        let relative = path
            .strip_prefix(repo)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        sources.push((relative, fs::read_to_string(path).unwrap()));
    }

    for (path, source) in &mut sources {
        if path == "server/src/config.rs" {
            allow_exact(
                source,
                path,
                "for key in [\"landing_url\", \"plan\", \"landing_auth_token\"] {",
                1,
            );
            allow_exact(
                source,
                path,
                "for obsolete in [\"landing_url\", \"plan\", \"landing_auth_token\"] {",
                1,
            );
            allow_exact(
                source,
                path,
                "for key in [\"LANDING_URL\", \"FLY_APP_NAME\", \"FLY_MACHINE_ID\"] {",
                1,
            );
            allow_exact(
                source,
                path,
                "ignoring obsolete managed control-plane settings",
                1,
            );
        }
    }

    let forbidden = [
        "landing_url",
        "landing_auth_token",
        "is_managed",
        "FLY_APP_NAME",
        "FLY_MACHINE_ID",
        "Fly.io",
        "fly.io",
        "_api.internal",
        "api.fly.io",
        "/v1/apps/",
        "bollyai.dev",
        "nolune.dev",
        "/api/usage",
        "fetchUsage",
        "UsageBar",
        "Companion plan",
        "upgrade their plan",
        "managed AI companion platform",
        "unique subdomain",
        "RestartMachineTool",
        "restart_machine",
    ];

    let mut violations = Vec::new();
    for (path, source) in &sources {
        for needle in forbidden {
            if source.contains(needle) {
                violations.push(format!("{path} contains {needle:?}"));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "managed control-plane remnants:\n{}",
        violations.join("\n")
    );

    let system_tools = sources
        .iter()
        .find(|(path, _)| path == "server/src/services/tools/system.rs")
        .map(|(_, source)| source)
        .expect("system tool source must be scanned");
    assert!(!system_tools.contains("std::process::exit"));
    assert!(!system_tools.contains("process::exit"));

    assert!(
        !repo.join("server/src/routes/usage.rs").exists(),
        "deleted usage route was restored"
    );
    assert!(
        !repo
            .join("client/src/lib/components/layout/UsageBar.svelte")
            .exists(),
        "deleted UsageBar was restored"
    );
}
