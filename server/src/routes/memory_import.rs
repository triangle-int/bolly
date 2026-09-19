use axum::{Router, http::StatusCode, response::IntoResponse, routing::post};

use crate::app::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route(
        "/api/instances/{instance_slug}/memory/import",
        post(import_unavailable),
    )
}

/// Kept only as an explicit compatibility response. This handler intentionally
/// has no body, multipart, path, or state extractor, so rejection happens
/// before request payload parsing or filesystem access.
async fn import_unavailable() -> impl IntoResponse {
    (
        StatusCode::NOT_IMPLEMENTED,
        "memory import is unavailable pending a capability-safe import format",
    )
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        convert::Infallible,
        fs,
        path::{Path as FsPath, PathBuf},
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
    };

    use axum::{
        body::{Body, Bytes, to_bytes},
        http::{Request, StatusCode},
    };
    use futures::stream;
    use tower::ServiceExt;

    use super::*;

    async fn test_app(workspace: &FsPath) -> Router {
        let mut config = crate::config::Config::default();
        config.llm.tokens.anthropic = "test-only-key".into();
        let mut state = AppState::new(config).await;
        state.workspace_dir = workspace.to_owned();
        state.vector_store =
            Arc::new(crate::services::vector::VectorStore::connect(workspace).await);
        router().with_state(state)
    }

    fn multipart(boundary: &str, files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut body = Vec::new();
        for (name, contents) in files {
            body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
            body.extend_from_slice(
                format!(
                    "Content-Disposition: form-data; name=\"files\"; filename=\"{name}\"\r\n\r\n"
                )
                .as_bytes(),
            );
            body.extend_from_slice(contents);
            body.extend_from_slice(b"\r\n");
        }
        body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
        body
    }

    fn snapshot(root: &FsPath) -> BTreeMap<PathBuf, Vec<u8>> {
        fn visit(root: &FsPath, dir: &FsPath, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
            if !dir.exists() {
                return;
            }
            for entry in fs::read_dir(dir).unwrap() {
                let entry = entry.unwrap();
                let path = entry.path();
                let relative = path.strip_prefix(root).unwrap().to_owned();
                let kind = entry.file_type().unwrap();
                if kind.is_symlink() {
                    out.insert(relative, b"<symlink>".to_vec());
                } else if kind.is_dir() {
                    out.insert(relative.clone(), b"<directory>".to_vec());
                    visit(root, &path, out);
                } else {
                    out.insert(relative, fs::read(path).unwrap());
                }
            }
        }

        let mut files = BTreeMap::new();
        visit(root, root, &mut files);
        files
    }

    async fn post_multipart(
        app: Router,
        slug: &str,
        boundary: &str,
        body: Vec<u8>,
    ) -> axum::response::Response {
        app.oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/instances/{slug}/memory/import"))
                .header(
                    "content-type",
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn import_is_explicitly_unavailable_before_malicious_filenames_can_mutate_files() {
        let workspace = tempfile::tempdir().unwrap();
        fs::create_dir_all(workspace.path().join("instances/one/memory")).unwrap();
        fs::write(
            workspace.path().join("instances/one/memory/sentinel.md"),
            b"preserve me",
        )
        .unwrap();
        let outside = tempfile::tempdir().unwrap();
        let outside_file = outside.path().join("absolute.txt");
        fs::write(&outside_file, b"outside sentinel").unwrap();
        let before_workspace = snapshot(workspace.path());
        let before_outside = snapshot(outside.path());
        let absolute = outside
            .path()
            .join("absolute.txt")
            .to_string_lossy()
            .into_owned();
        let cases = [
            absolute,
            "../../outside.txt".into(),
            "nested/name.txt".into(),
            "nested\\name.txt".into(),
        ];

        for (index, name) in cases.iter().enumerate() {
            let boundary = format!("disabled-{index}");
            let response = post_multipart(
                test_app(workspace.path()).await,
                "one",
                &boundary,
                multipart(&boundary, &[(name, b"attacker data")]),
            )
            .await;
            assert_eq!(response.status(), StatusCode::NOT_IMPLEMENTED, "{name}");
            let response_body = to_bytes(response.into_body(), 1024).await.unwrap();
            assert!(String::from_utf8_lossy(&response_body).contains("unavailable"));
            assert_eq!(snapshot(workspace.path()), before_workspace, "{name}");
            assert_eq!(snapshot(outside.path()), before_outside, "{name}");
        }
    }

    #[tokio::test]
    async fn duplicate_and_concurrent_imports_are_rejected_without_mutation() {
        let workspace = tempfile::tempdir().unwrap();
        fs::create_dir_all(workspace.path().join("instances/one/memory")).unwrap();
        fs::write(
            workspace.path().join("instances/one/memory/sentinel.md"),
            b"preserve me",
        )
        .unwrap();
        let before = snapshot(workspace.path());
        let app = test_app(workspace.path()).await;
        let boundary = "duplicate-concurrent";
        let body = multipart(boundary, &[("same.txt", b"first"), ("same.txt", b"second")]);
        let requests = (0..8).map(|_| post_multipart(app.clone(), "one", boundary, body.clone()));
        let responses = futures::future::join_all(requests).await;

        assert!(
            responses
                .iter()
                .all(|response| response.status() == StatusCode::NOT_IMPLEMENTED)
        );
        assert_eq!(snapshot(workspace.path()), before);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn import_rejection_does_not_follow_or_remove_existing_symlink() {
        use std::os::unix::fs::symlink;

        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let instance = workspace.path().join("instances/one");
        fs::create_dir_all(&instance).unwrap();
        fs::write(outside.path().join("sentinel"), b"outside sentinel").unwrap();
        symlink(outside.path(), instance.join(".import_temp")).unwrap();
        let before_workspace = snapshot(workspace.path());
        let before_outside = snapshot(outside.path());
        let boundary = "symlink";

        let response = post_multipart(
            test_app(workspace.path()).await,
            "one",
            boundary,
            multipart(boundary, &[("file.txt", b"attacker data")]),
        )
        .await;

        assert_eq!(response.status(), StatusCode::NOT_IMPLEMENTED);
        assert_eq!(snapshot(workspace.path()), before_workspace);
        assert_eq!(snapshot(outside.path()), before_outside);
    }

    #[tokio::test]
    async fn oversized_body_is_rejected_without_being_polled_or_persisted() {
        let workspace = tempfile::tempdir().unwrap();
        let before = snapshot(workspace.path());
        let polled = Arc::new(AtomicBool::new(false));
        let stream_polled = polled.clone();
        let body_stream = stream::once(async move {
            stream_polled.store(true, Ordering::SeqCst);
            Ok::<Bytes, Infallible>(Bytes::from_static(b"body must not be read"))
        });

        let response = test_app(workspace.path())
            .await
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/instances/one/memory/import")
                    .header("content-type", "multipart/form-data; boundary=oversized")
                    .header("content-length", u64::MAX.to_string())
                    .body(Body::from_stream(body_stream))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_IMPLEMENTED);
        assert!(
            !polled.load(Ordering::SeqCst),
            "disabled route consumed request body"
        );
        assert_eq!(snapshot(workspace.path()), before);
    }
}
