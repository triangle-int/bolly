use super::*;
use axum::{
    body::Body,
    http::{Method, Request, StatusCode, header::CONTENT_TYPE},
};
use std::{fs, path::Path};
use tower::ServiceExt;

const SEEDED_AUTH_TOKEN: &str = "issue-88-seeded-control-token";
const MAX_PUBLIC_RESPONSE_BYTES: usize = 1024 * 1024;

fn production_files(root: &Path, files: &mut Vec<PathBuf>) {
    if root.is_file() {
        files.push(root.to_path_buf());
        return;
    }
    for entry in fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            if matches!(
                path.file_name().and_then(|name| name.to_str()),
                Some("tests" | "test-support")
            ) {
                continue;
            }
            production_files(&path, files);
        } else {
            files.push(path);
        }
    }
}

async fn assert_public_response(
    state: &AppState,
    static_dir: &Path,
    method: Method,
    uri: &str,
    expected_status: StatusCode,
    expected_content_type: Option<&str>,
) -> Vec<u8> {
    let is_auth_post = method == Method::POST && uri == "/auth";
    let body = if is_auth_post {
        Body::from(format!(r#"{{"token":"{SEEDED_AUTH_TOKEN}"}}"#))
    } else {
        Body::empty()
    };
    let mut request = Request::builder().method(method).uri(uri);
    if is_auth_post {
        request = request.header(CONTENT_TYPE, "application/json");
    }
    let response = build_router(state.clone(), Some(static_dir.to_path_buf()))
        .oneshot(request.body(body).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), expected_status, "{uri}");
    assert_eq!(
        response
            .headers()
            .get(CONTENT_TYPE)
            .map(|value| value.to_str().unwrap()),
        expected_content_type,
        "content type for {uri}",
    );
    for value in response.headers().values() {
        assert!(
            !String::from_utf8_lossy(value.as_bytes()).contains(SEEDED_AUTH_TOKEN),
            "response header for {uri} disclosed the configured auth token",
        );
    }

    let body = axum::body::to_bytes(response.into_body(), MAX_PUBLIC_RESPONSE_BYTES)
        .await
        .unwrap();
    assert!(
        !String::from_utf8_lossy(&body).contains(SEEDED_AUTH_TOKEN),
        "response body for {uri} disclosed the configured auth token",
    );
    body.to_vec()
}

async fn seeded_state() -> AppState {
    let mut config = crate::config::Config::default();
    config.auth_token = SEEDED_AUTH_TOKEN.into();
    AppState::new(config).await
}

fn seeded_static_dir() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("assets")).unwrap();
    fs::write(
        dir.path().join("index.html"),
        "<!doctype html><title>issue 88 test SPA</title><main>safe-index</main>",
    )
    .unwrap();
    fs::write(
        dir.path().join("assets/app.js"),
        "globalThis.__issue88StaticAsset = 'safe-asset';",
    )
    .unwrap();
    dir
}

#[tokio::test]
async fn public_routes_have_exact_semantics_and_never_disclose_the_auth_token() {
    let state = seeded_state().await;
    let static_dir = seeded_static_dir();

    let cases = [
        (
            Method::GET,
            "/healthz",
            StatusCode::OK,
            Some("application/json"),
        ),
        (Method::GET, "/", StatusCode::OK, Some("text/html")),
        (
            Method::GET,
            "/assets/app.js",
            StatusCode::OK,
            Some("text/javascript"),
        ),
        (
            Method::GET,
            "/some/spa/route",
            StatusCode::NOT_FOUND,
            Some("text/html"),
        ),
        (
            Method::GET,
            "/auth?token=issue-88-seeded-control-token",
            StatusCode::NOT_FOUND,
            Some("text/html"),
        ),
        (Method::POST, "/auth", StatusCode::METHOD_NOT_ALLOWED, None),
        (
            Method::GET,
            "/manifest.webmanifest",
            StatusCode::NOT_FOUND,
            Some("text/html"),
        ),
        (
            Method::GET,
            "/public/files/moon/missing",
            StatusCode::UNAUTHORIZED,
            None,
        ),
        (
            Method::GET,
            "/public/memory/moon/missing",
            StatusCode::UNAUTHORIZED,
            None,
        ),
    ];

    for (method, uri, status, content_type) in cases {
        let body =
            assert_public_response(&state, static_dir.path(), method, uri, status, content_type)
                .await;
        if matches!(
            uri,
            "/" | "/some/spa/route"
                | "/auth?token=issue-88-seeded-control-token"
                | "/manifest.webmanifest"
        ) {
            assert!(
                String::from_utf8_lossy(&body).contains("safe-index"),
                "{uri}"
            );
        }
    }
}

#[tokio::test]
async fn every_file_in_the_served_static_tree_is_token_free() {
    let state = seeded_state().await;
    let static_dir = seeded_static_dir();
    let mut files = Vec::new();
    production_files(static_dir.path(), &mut files);
    files.sort();
    assert_eq!(files.len(), 2);

    for path in files {
        let relative = path.strip_prefix(static_dir.path()).unwrap();
        let uri = format!("/{}", relative.to_string_lossy());
        assert_public_response(
            &state,
            static_dir.path(),
            Method::GET,
            &uri,
            StatusCode::OK,
            if uri.ends_with(".html") {
                Some("text/html")
            } else {
                Some("text/javascript")
            },
        )
        .await;
    }
}

#[tokio::test]
async fn legacy_browser_cookies_never_authenticate_api_requests() {
    let state = seeded_state().await;

    for name in ["nolune_token", "bolly_token", "bolly_auth_token"] {
        let response = build_router(state.clone(), None)
            .oneshot(
                Request::builder()
                    .uri("/api/meta")
                    .header("cookie", format!("{name}={SEEDED_AUTH_TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{name}");
    }
}

#[test]
fn production_sources_and_checked_in_build_have_no_tokenized_browser_bootstrap() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let mut files = Vec::new();
    for root in [
        repo.join("server/src"),
        repo.join("client/src"),
        repo.join("client/static"),
        repo.join("client/vite.config.ts"),
        repo.join("client/build"),
        repo.join("desktop/src"),
        repo.join("desktop/src-tauri/src"),
        repo.join("scripts"),
    ] {
        production_files(&root, &mut files);
    }

    for path in files {
        let Ok(source) = fs::read_to_string(&path) else {
            continue;
        };
        for forbidden in [
            "/auth?token=",
            "start_url",
            "manifest.webmanifest",
            "apple-mobile-web-app-capable",
        ] {
            assert!(
                !source.contains(forbidden),
                "{} still contains forbidden PWA bootstrap source {forbidden:?}",
                path.display(),
            );
        }
    }
}

#[tokio::test]
async fn removed_google_workspace_routes_are_not_in_api_router() {
    let state = AppState::new(crate::config::Config::default()).await;
    let account_routes = format!("/api/instances/moon/google/{}", "accounts");
    let connect_route = format!("/api/instances/moon/google/{}", "connect");
    let disconnect_route = format!("{account_routes}/user@example.com");
    for (method, uri) in [
        ("GET", account_routes),
        ("GET", connect_route),
        ("DELETE", disconnect_route),
    ] {
        let response = api_router(&state)
            .with_state(state.clone())
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(&uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{uri}");
    }
}

#[tokio::test]
async fn standalone_api_has_no_managed_status_or_usage_route() {
    let state = AppState::new(crate::config::Config::default()).await;
    let status = api_router(&state)
        .with_state(state.clone())
        .oneshot(
            Request::builder()
                .uri("/api/config/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(status.status(), StatusCode::OK);
    let body = axum::body::to_bytes(status.into_body(), MAX_PUBLIC_RESPONSE_BYTES)
        .await
        .unwrap();
    let status: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(status.get("is_managed").is_none());

    let usage = api_router(&state)
        .with_state(state)
        .oneshot(
            Request::builder()
                .uri("/api/usage")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(usage.status(), StatusCode::NOT_FOUND);
}
