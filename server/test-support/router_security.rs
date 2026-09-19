#[path = "source_scan.rs"]
mod source_scan;

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
            "/public/files/companion/missing",
            StatusCode::UNAUTHORIZED,
            None,
        ),
        (
            Method::GET,
            "/public/memory/companion/missing",
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
    let account_routes = format!("/api/instances/companion/google/{}", "accounts");
    let connect_route = format!("/api/instances/companion/google/{}", "connect");
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

#[tokio::test]
async fn browser_capability_issuance_is_authenticated_exact_and_secret_free() {
    let state = seeded_state().await;
    let app = build_router(state, None);
    let uri = "/api/instances/moon/resource-capabilities/memory";
    let body = r#"{"path":"Folder/Résumé 1%.png"}"#;
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("authorization", format!("Bearer {SEEDED_AUTH_TOKEN}"))
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), 8192)
        .await
        .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let url = value["url"].as_str().unwrap();
    assert!(
        url.starts_with("/resources/browser/memory/moon/Folder/R%C3%A9sum%C3%A9%201%25.png?cap=")
    );
    assert!(!url.contains(SEEDED_AUTH_TOKEN));
    for (url, method, expected) in [
        (url.to_string(), "GET", StatusCode::NOT_FOUND),
        (url.to_string(), "HEAD", StatusCode::UNAUTHORIZED),
        (
            url.replace("/browser/", "/model-provider/"),
            "GET",
            StatusCode::UNAUTHORIZED,
        ),
        (url.replace("%C3", "%c3"), "GET", StatusCode::UNAUTHORIZED),
        (
            url.replace("/moon/", "/other/"),
            "GET",
            StatusCode::UNAUTHORIZED,
        ),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(&url)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), expected, "{method} {url}");
    }
}

#[tokio::test]
async fn legacy_http_resource_tokens_are_rejected_before_lookup() {
    let app = build_router(seeded_state().await, None);
    for kind in ["files", "memory"] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/public/{kind}/{}/missing?token={SEEDED_AUTH_TOKEN}",
                        crate::domain::companion::CANONICAL_SLUG
                    ))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
}

#[test]
fn model_resource_producers_never_embed_the_control_token() {
    let resources = crate::services::resource_access::ResourceAccess::new(SEEDED_AUTH_TOKEN);
    let file =
        crate::services::tools::public_file_url("https://example.test", "moon", "a b", &resources);
    let memory = crate::services::tools::public_memory_url(
        "https://example.test",
        "moon",
        "Folder/Résumé 1%.png",
        &resources,
    );
    for url in [file, memory] {
        assert!(!url.contains(SEEDED_AUTH_TOKEN));
        assert!(url.contains("/resources/model-provider/"));
        assert!(url.contains("?cap="));
    }
}

#[tokio::test]
async fn config_replacement_revokes_grants_and_refreshes_live_producers() {
    use crate::services::resource_capability::{CapabilityAudience, CapabilityResource};
    let workspace = tempfile::tempdir().unwrap();
    let mut state = seeded_state().await;
    state.workspace_dir = workspace.path().into();
    let producer = state.resources.clone();
    let app = build_router(state.clone(), None);
    let resource = CapabilityResource::memory("file.png").unwrap();
    let old = producer
        .url(
            "",
            "moon",
            resource.clone(),
            CapabilityAudience::ModelProvider,
        )
        .unwrap();
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/config/server")
                .header("authorization", format!("Bearer {SEEDED_AUTH_TOKEN}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({"auth_token": SEEDED_AUTH_TOKEN}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        state
            .resources
            .verify(
                "moon",
                resource.clone(),
                CapabilityAudience::ModelProvider,
                &old.parse().unwrap(),
                "GET"
            )
            .is_ok()
    );
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/config/server")
                .header("authorization", format!("Bearer {SEEDED_AUTH_TOKEN}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({"auth_token": "rotated-control-token"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        state
            .resources
            .verify(
                "moon",
                resource.clone(),
                CapabilityAudience::ModelProvider,
                &old.parse().unwrap(),
                "GET"
            )
            .is_err()
    );
    let refreshed = producer
        .url(
            "",
            "moon",
            resource.clone(),
            CapabilityAudience::ModelProvider,
        )
        .unwrap();
    assert!(
        state
            .resources
            .verify(
                "moon",
                resource.clone(),
                CapabilityAudience::ModelProvider,
                &refreshed.parse().unwrap(),
                "GET"
            )
            .is_ok()
    );
    let response = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/config/server")
                .header("authorization", "Bearer rotated-control-token")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({"auth_token": ""}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let open = producer
        .url("", "moon", resource.clone(), CapabilityAudience::Browser)
        .unwrap();
    assert!(!open.contains('?'));
    assert!(
        state
            .resources
            .verify(
                "moon",
                resource,
                CapabilityAudience::Browser,
                &open.parse().unwrap(),
                "GET"
            )
            .is_ok()
    );
}

#[tokio::test]
async fn configured_control_token_is_redacted_from_tool_outputs() {
    let _state = seeded_state().await;
    assert!(
        !crate::services::tools::redact_secrets(&format!("tool output: {SEEDED_AUTH_TOKEN}"))
            .contains(SEEDED_AUTH_TOKEN)
    );
}

#[tokio::test]
async fn native_issuance_has_a_fixed_audience_and_bounded_exact_inputs() {
    let state = seeded_state().await;
    let app = build_router(state.clone(), None);
    let uri = "/api/native-relay/instances/moon/resource-capabilities/files";
    for (body, expected) in [
        (
            r#"{"id":"id","audience":"browser"}"#.to_string(),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (r#"{"id":"../id"}"#.to_string(), StatusCode::BAD_REQUEST),
        (
            format!(r#"{{"id":"{}"}}"#, "a".repeat(4096)),
            StatusCode::PAYLOAD_TOO_LARGE,
        ),
        (r#"{"id":"id"}"#.to_string(), StatusCode::OK),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(uri)
                    .header("authorization", format!("Bearer {SEEDED_AUTH_TOKEN}"))
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
        if expected == StatusCode::OK {
            let bytes = axum::body::to_bytes(response.into_body(), 8192)
                .await
                .unwrap();
            let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert!(
                value["url"]
                    .as_str()
                    .unwrap()
                    .starts_with("/resources/native-relay/files/moon/id?cap=")
            );
        }
    }
}

#[tokio::test]
async fn scoped_downloads_preserve_all_media_types_and_never_cache_authorization() {
    use crate::services::resource_capability::{CapabilityAudience as A, CapabilityResource as R};
    let ws = tempfile::tempdir().unwrap();
    let memory = ws.path().join("instances/moon/memory");
    fs::create_dir_all(memory.join("Folder")).unwrap();
    let mut state = seeded_state().await;
    state.workspace_dir = ws.path().into();
    state.vector_store =
        std::sync::Arc::new(crate::services::vector::VectorStore::connect(ws.path()).await);
    let cases = [
        ("png", "image/png"),
        ("pdf", "application/pdf"),
        ("mp3", "audio/mpeg"),
        ("mp4", "video/mp4"),
        ("webm", "video/webm"),
        ("mov", "video/quicktime"),
        ("ogg", "audio/ogg"),
        ("m4a", "audio/mp4"),
        ("flac", "audio/flac"),
        ("aac", "audio/aac"),
        ("avif", "image/avif"),
    ];
    let app = build_router(state.clone(), None);
    for (extension, content_type) in cases {
        for folder in ["", "Folder/"] {
            let path = format!("{folder}Résumé 1%.{extension}");
            fs::write(memory.join(&path), b"media-sentinel").unwrap();
            for audience in [A::Browser, A::ModelProvider, A::NativeRelay] {
                let url = state
                    .resources
                    .url("", "moon", R::memory(&path).unwrap(), audience)
                    .unwrap();
                for _retry in 0..2 {
                    let response = app
                        .clone()
                        .oneshot(Request::builder().uri(&url).body(Body::empty()).unwrap())
                        .await
                        .unwrap();
                    assert_eq!(response.status(), StatusCode::OK, "{path}");
                    assert_eq!(response.headers()["content-type"], content_type);
                    assert_eq!(response.headers()["cache-control"], "private, no-store");
                    assert_eq!(
                        axum::body::to_bytes(response.into_body(), 100)
                            .await
                            .unwrap()
                            .as_ref(),
                        b"media-sentinel"
                    );
                }
            }
        }
    }
    let meta =
        crate::services::uploads::save_upload(ws.path(), "moon", "photo.png", b"upload-sentinel")
            .unwrap();
    let url = state
        .resources
        .url("", "moon", R::uploaded_file(meta.id).unwrap(), A::Browser)
        .unwrap();
    let response = app
        .clone()
        .oneshot(Request::builder().uri(&url).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["cache-control"], "private, no-store");
    let cap = url.split_once("?cap=").unwrap().1;
    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/companion?cap={cap}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn api_control_auth_never_accepts_http_query_tokens() {
    let app = build_router(seeded_state().await, None);
    for path in [
        "/api/meta",
        "/api/companion",
        "/api/instances/moon/export",
        "/api/instances/moon/uploads/id/file",
        "/api/instances/moon/memory/photo.png",
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("{path}?token={SEEDED_AUTH_TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{path}");
    }
}

#[test]
fn captured_server_logs_redact_control_tokens() {
    use log::Log;
    use std::io::Write;
    use std::sync::{Arc, Mutex};
    #[derive(Clone)]
    struct Capture(Arc<Mutex<Vec<u8>>>);
    impl Write for Capture {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    crate::services::tools::register_control_secret(SEEDED_AUTH_TOKEN);
    let capture = Capture(Arc::new(Mutex::new(Vec::new())));
    let logger = env_logger::Builder::new()
        .filter_level(log::LevelFilter::Info)
        .format(crate::app::logging::format_record)
        .target(env_logger::Target::Pipe(Box::new(capture.clone())))
        .build();
    logger.log(
        &log::Record::builder()
            .level(log::Level::Info)
            .args(format_args!("provider echoed {SEEDED_AUTH_TOKEN}"))
            .build(),
    );
    let output = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
    assert!(output.contains("[REDACTED]"));
    assert!(!output.contains(SEEDED_AUTH_TOKEN));
}

#[test]
fn recursive_resource_producer_and_consumer_regression_scan() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    for tree in [
        "server/src",
        "client/src",
        "desktop/src-tauri/src",
        "desktop/src",
        "scripts",
    ] {
        let mut files = Vec::new();
        production_files(&root.join(tree), &mut files);
        for file in files {
            if !matches!(
                file.extension().and_then(|v| v.to_str()),
                Some("rs" | "ts" | "js" | "svelte" | "sh")
            ) || file
                .file_name()
                .unwrap()
                .to_string_lossy()
                .ends_with("_tests.rs")
            {
                continue;
            }
            let source = fs::read_to_string(&file).unwrap();
            let mut source = if file.extension().and_then(|v| v.to_str()) == Some("rs") {
                source_scan::without_cfg_test_items(&source)
            } else {
                source
            };
            // ISSUE-112 is limited to these exact WebSocket handshake implementations.
            if file.ends_with("server/src/app/auth.rs") {
                let start = source.find("    // ISSUE-112:").unwrap();
                let end = source[start..].find("    let provided").unwrap() + start;
                source.replace_range(start..end, "");
                source = source.replace("`?token=<value>`", "WebSocket credential");
            }
            if file.ends_with("client/src/lib/api/client.ts") {
                let start = source.find("// ISSUE-112:").unwrap();
                let end = source[start..].find("\n}").unwrap() + start + 2;
                source.replace_range(start..end, "");
            }
            assert!(
                !source.contains("?token=") && !source.contains("&token="),
                "HTTP query credential in {}",
                file.display()
            );
            if file.starts_with(root.join("server/src/services/tools")) {
                assert!(
                    !source.contains("/uploads/{}/file"),
                    "unscoped tool URL in {}",
                    file.display()
                );
                assert!(
                    !source.contains("NOLUNE_AUTH_TOKEN\").unwrap"),
                    "tool loads control credential in {}",
                    file.display()
                );
            }
        }
    }
}

#[tokio::test]
async fn attachments_and_memory_tools_only_emit_scoped_secret_free_urls() {
    use crate::services::{llm, tool::Tool, tools};
    let ws = tempfile::tempdir().unwrap();
    let memory = ws.path().join("instances/moon/memory");
    fs::create_dir_all(memory.join("Folder")).unwrap();
    let mut state = seeded_state().await;
    state.workspace_dir = ws.path().into();
    state.vector_store =
        std::sync::Arc::new(crate::services::vector::VectorStore::connect(ws.path()).await);
    for name in ["photo.png", "paper.pdf"] {
        let meta =
            crate::services::uploads::save_upload(ws.path(), "moon", name, b"media bytes").unwrap();
        let prompt = llm::build_multimodal_prompt(
            &format!("[attached: {name} ({})]", meta.id),
            ws.path(),
            "moon",
            "https://example.test",
            &state.resources,
            &state.vector_store.media_store(),
        );
        let serialized = serde_json::to_string(&prompt).unwrap();
        assert!(!serialized.contains(SEEDED_AUTH_TOKEN));
        assert!(serialized.contains("/resources/model-provider/files/moon/"));
        assert!(serialized.contains("?cap="));
    }
    let reader = tools::MemoryReadTool::new(
        ws.path(),
        "moon",
        "https://example.test",
        state.vector_store.clone(),
        &state.resources,
    );
    for name in [
        "photo.png",
        "paper.pdf",
        "sound.mp3",
        "clip.mp4",
        "Folder/Résumé 1%.png",
    ] {
        fs::write(memory.join(name), b"media bytes").unwrap();
        let output = reader
            .call(tools::memory_tools::MemoryReadArgs { path: name.into() })
            .await
            .unwrap();
        assert!(!output.contains(SEEDED_AUTH_TOKEN));
        assert!(
            output.contains("/resources/model-provider/memory/moon/"),
            "{output}"
        );
        assert!(output.contains("?cap="));
    }
}

#[tokio::test]
async fn changed_control_token_revokes_old_grants_and_old_api_headers() {
    use crate::services::resource_capability::{CapabilityAudience, CapabilityResource};
    let workspace = tempfile::tempdir().unwrap();
    let mut state = seeded_state().await;
    state.workspace_dir = workspace.path().into();
    let resource = CapabilityResource::uploaded_file("missing").unwrap();
    let old = state
        .resources
        .url("", "moon", resource.clone(), CapabilityAudience::Browser)
        .unwrap();
    let app = build_router(state.clone(), None);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/config/server")
                .header("authorization", format!("Bearer {SEEDED_AUTH_TOKEN}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({"auth_token": "rotated-control-token"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let response = app
        .clone()
        .oneshot(Request::builder().uri(old).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    for (token, expected) in [
        (SEEDED_AUTH_TOKEN, StatusCode::UNAUTHORIZED),
        ("rotated-control-token", StatusCode::OK),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/meta")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
    }
    let fresh = state
        .resources
        .url("", "moon", resource, CapabilityAudience::Browser)
        .unwrap();
    let response = app
        .oneshot(Request::builder().uri(fresh).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn observable_tool_errors_and_activity_redact_control_tokens() {
    use crate::services::{
        tool::{ToolDefinition, ToolDyn, ToolError},
        tools::{ObservableTool, ToolExecError},
    };
    use std::{future::Future, pin::Pin};
    struct FailingTool;
    impl ToolDyn for FailingTool {
        fn name(&self) -> String {
            "read_file".into()
        }
        fn definition(
            &self,
            _: String,
        ) -> Pin<Box<dyn Future<Output = ToolDefinition> + Send + '_>> {
            Box::pin(async { unreachable!() })
        }
        fn call(
            &self,
            _: String,
        ) -> Pin<Box<dyn Future<Output = Result<String, ToolError>> + Send + '_>> {
            Box::pin(async {
                Err(ToolError::ToolCallError(Box::new(ToolExecError(
                    SEEDED_AUTH_TOKEN.into(),
                ))))
            })
        }
    }
    let state = seeded_state().await;
    let mut events = state.events.subscribe();
    let tool = ObservableTool::new(
        Box::new(FailingTool),
        state.events.clone(),
        &state.workspace_dir,
        "moon".into(),
        "default".into(),
        None,
    );
    let error = tool
        .call(serde_json::json!({"path":SEEDED_AUTH_TOKEN}).to_string())
        .await
        .unwrap_err();
    assert!(!error.to_string().contains(SEEDED_AUTH_TOKEN));
    while let Ok(event) = events.try_recv() {
        assert!(
            !serde_json::to_string(&event)
                .unwrap()
                .contains(SEEDED_AUTH_TOKEN)
        );
    }
}

#[tokio::test]
async fn empty_auth_serves_resources_without_minting_capabilities() {
    use crate::services::resource_capability::{CapabilityAudience as A, CapabilityResource as R};
    let ws = tempfile::tempdir().unwrap();
    let mut state = AppState::new(crate::config::Config::default()).await;
    state.workspace_dir = ws.path().into();
    state.vector_store =
        std::sync::Arc::new(crate::services::vector::VectorStore::connect(ws.path()).await);
    let meta = crate::services::uploads::save_upload(ws.path(), "moon", "photo.png", b"open media")
        .unwrap();
    let app = build_router(state.clone(), None);
    for audience in [A::Browser, A::ModelProvider, A::NativeRelay] {
        let url = state
            .resources
            .url("", "moon", R::uploaded_file(&meta.id).unwrap(), audience)
            .unwrap();
        assert!(!url.contains('?'));
        let response = app
            .clone()
            .oneshot(Request::builder().uri(url).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/instances/moon/resource-capabilities/files")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::json!({"id":meta.id}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(!value["url"].as_str().unwrap().contains('?'));
}

#[tokio::test]
async fn browser_memory_issuance_rejects_noncanonical_and_unbounded_inputs() {
    let app = build_router(seeded_state().await, None);
    for path in [
        "../secret".to_string(),
        "/absolute".into(),
        "a//b".into(),
        "e\u{301}.png".into(),
        "a\\b".into(),
        "x".repeat(1025),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/instances/moon/resource-capabilities/memory")
                    .header("authorization", format!("Bearer {SEEDED_AUTH_TOKEN}"))
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::json!({"path":path}).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}
