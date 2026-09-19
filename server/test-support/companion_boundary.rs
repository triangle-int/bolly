//! Router-level tests for the one-companion storage boundary (#103).
//!
//! Included from `app/router.rs`. Every request goes through `build_router`,
//! so these tests exercise the real auth and companion middleware stack.

use super::*;
use crate::{
    domain::companion::{CANONICAL_SLUG, IDENTITY_FILE, STORAGE_FORMAT_VERSION},
    services::{companion, machine_registry::MachineInfo},
};
use axum::{
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use std::{fs, path::PathBuf, sync::Arc};
use tower::ServiceExt;

const TOKEN: &str = "issue-103-control-token";
const MAX_BODY: usize = 64 * 1024 * 1024;

struct Harness {
    workspace: tempfile::TempDir,
    state: AppState,
}

async fn harness() -> Harness {
    let workspace = tempfile::tempdir().unwrap();
    let config = crate::config::Config {
        auth_token: TOKEN.into(),
        ..Default::default()
    };
    let mut state = AppState::new(config).await;
    state.workspace_dir = workspace.path().to_owned();
    state.vector_store =
        Arc::new(crate::services::vector::VectorStore::connect(workspace.path()).await);
    Harness { workspace, state }
}

impl Harness {
    fn instances(&self) -> PathBuf {
        self.workspace.path().join("instances")
    }

    fn companion(&self) -> PathBuf {
        self.instances().join(CANONICAL_SLUG)
    }

    fn instance_dirs(&self) -> Vec<String> {
        let Ok(entries) = fs::read_dir(self.instances()) else {
            return Vec::new();
        };
        let mut names = entries
            .filter_map(Result::ok)
            .filter(|entry| entry.path().is_dir())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        names.sort();
        names
    }

    fn seed_obsolete(&self, slug: &str) {
        let dir = self.instances().join(slug);
        fs::create_dir_all(dir.join("memory")).unwrap();
        fs::write(dir.join("soul.md"), format!("soul of {slug}")).unwrap();
        fs::write(dir.join("memory/facts.md"), "- likes tea").unwrap();
    }

    fn assert_obsolete_untouched(&self, slug: &str) {
        let dir = self.instances().join(slug);
        assert_eq!(
            fs::read_to_string(dir.join("soul.md")).unwrap(),
            format!("soul of {slug}"),
            "{slug}: soul.md changed"
        );
        assert_eq!(
            fs::read_to_string(dir.join("memory/facts.md")).unwrap(),
            "- likes tea",
            "{slug}: memory changed"
        );
        assert!(
            !dir.join(IDENTITY_FILE).exists(),
            "{slug}: must never receive an identity marker"
        );
    }

    async fn send(
        &self,
        method: Method,
        uri: &str,
        body: Option<serde_json::Value>,
    ) -> (StatusCode, Vec<u8>) {
        let mut request = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"));
        let body = match body {
            Some(json) => {
                request = request.header(header::CONTENT_TYPE, "application/json");
                Body::from(serde_json::to_vec(&json).unwrap())
            }
            None => Body::empty(),
        };
        let response = build_router(self.state.clone(), None)
            .oneshot(request.body(body).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), MAX_BODY)
            .await
            .unwrap();
        (status, bytes.to_vec())
    }

    async fn json(
        &self,
        method: Method,
        uri: &str,
        body: Option<serde_json::Value>,
    ) -> (StatusCode, serde_json::Value) {
        let (status, bytes) = self.send(method, uri, body).await;
        let value = serde_json::from_slice(&bytes).unwrap_or_else(|error| {
            panic!(
                "{uri}: expected JSON body, got {error}: {}",
                String::from_utf8_lossy(&bytes)
            )
        });
        (status, value)
    }
}

#[tokio::test]
async fn companion_context_and_meta_report_only_the_canonical_companion() {
    let h = harness().await;

    let (status, context) = h.json(Method::GET, "/api/companion", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        context,
        serde_json::json!({
            "slug": CANONICAL_SLUG,
            "exists": false,
            "companion_name": "",
            "soul_exists": false,
        })
    );

    h.seed_obsolete("alice");
    h.seed_obsolete("bob");
    let (_, context) = h.json(Method::GET, "/api/companion", None).await;
    assert_eq!(
        context["exists"], false,
        "obsolete directories never stand in for the companion"
    );
    assert_eq!(context["slug"], CANONICAL_SLUG);
    let (_, meta) = h.json(Method::GET, "/api/meta", None).await;
    assert_eq!(meta["instances_count"], 0);
    assert_eq!(meta["companion_slug"], CANONICAL_SLUG);

    companion::ensure_identity(h.workspace.path()).unwrap();
    let (_, context) = h.json(Method::GET, "/api/companion", None).await;
    assert_eq!(context["exists"], true);
    assert_eq!(context["slug"], CANONICAL_SLUG);
    let (_, meta) = h.json(Method::GET, "/api/meta", None).await;
    assert_eq!(meta["instances_count"], 1);

    h.assert_obsolete_untouched("alice");
    h.assert_obsolete_untouched("bob");
}

#[tokio::test]
async fn multi_instance_routes_are_gone_and_unknown_api_paths_are_404_json() {
    let h = harness().await;
    companion::ensure_identity(h.workspace.path()).unwrap();
    fs::write(h.companion().join("soul.md"), "keep me").unwrap();

    for (method, uri) in [
        (Method::GET, "/api/instances"),
        (Method::DELETE, "/api/instances/companion"),
        (Method::GET, "/api/instances/companion"),
        (Method::GET, "/api/no-such-route"),
        (Method::DELETE, "/api/instances/alice"),
    ] {
        let label = format!("{method} {uri}");
        let (status, value) = h.json(method, uri, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{label}");
        assert_eq!(value["error"], "not_found", "{label}");
    }

    assert_eq!(
        fs::read_to_string(h.companion().join("soul.md")).unwrap(),
        "keep me",
        "a removed delete route must not delete anything"
    );
    assert_eq!(h.instance_dirs(), vec![CANONICAL_SLUG]);

    // Authentication still runs before the API 404 fallback.
    let response = build_router(h.state.clone(), None)
        .oneshot(
            Request::builder()
                .uri("/api/instances")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn foreign_slugs_fail_closed_on_every_surface_without_side_effects() {
    let h = harness().await;
    h.seed_obsolete("alice");

    let json = |value: serde_json::Value| Some(value);
    let cases: Vec<(Method, &str, Option<serde_json::Value>)> = vec![
        (
            Method::PUT,
            "/api/instances/alice/companion-name",
            json(serde_json::json!({"name": "Alice"})),
        ),
        (
            Method::PUT,
            "/api/instances/luna/companion-name",
            json(serde_json::json!({"name": "Luna"})),
        ),
        (
            Method::PUT,
            "/api/instances/alice/soul",
            json(serde_json::json!({"content": "rewritten"})),
        ),
        (
            Method::PUT,
            "/api/instances/luna/timezone",
            json(serde_json::json!({"timezone": "UTC"})),
        ),
        (Method::GET, "/api/instances/alice/soul", None),
        (Method::GET, "/api/instances/alice/memory", None),
        (Method::GET, "/api/instances/alice/export", None),
        (Method::GET, "/api/instances/alice/scheduled", None),
        (Method::POST, "/api/instances/alice/machine-hello", None),
        (Method::GET, "/api/chat/alice/chats", None),
        (
            Method::POST,
            "/api/chat",
            json(serde_json::json!({"instance_slug": "alice", "content": "hi"})),
        ),
        (
            Method::POST,
            "/api/chat",
            json(serde_json::json!({"instance_slug": "Companion", "content": "hi"})),
        ),
        (Method::GET, "/api/instances/Companion/soul", None),
        (
            Method::GET,
            "/public/memory/alice/sky.png?token=anything",
            None,
        ),
        (Method::GET, "/public/files/alice/upload-1", None),
    ];

    for (method, uri, body) in cases {
        let label = format!("{method} {uri}");
        let (status, bytes) = h.send(method, uri, body).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{label}");
        let value: serde_json::Value = serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| panic!("{label}: {}", String::from_utf8_lossy(&bytes)));
        assert_eq!(value["error"], "unknown_companion", "{label}");
    }

    assert_eq!(
        h.instance_dirs(),
        vec!["alice"],
        "no directory created or removed"
    );
    h.assert_obsolete_untouched("alice");
    assert!(!h.instances().join("luna").exists());
    assert!(
        !h.companion().exists(),
        "foreign requests never create the companion"
    );
}

#[tokio::test]
async fn canonical_reads_open_nothing_and_canonical_writes_create_the_one_companion() {
    let h = harness().await;
    let soul_uri = format!("/api/instances/{CANONICAL_SLUG}/soul");

    let (status, soul) = h.json(Method::GET, &soul_uri, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(soul["exists"], false);
    assert!(
        h.instance_dirs().is_empty(),
        "GET must not create the companion"
    );

    let (status, _) = h
        .send(
            Method::PUT,
            &format!("/api/instances/{CANONICAL_SLUG}/companion-name"),
            Some(serde_json::json!({"name": "Luna"})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        companion::read_identity(h.workspace.path()),
        Ok(Some(
            crate::domain::companion::CompanionIdentity::canonical()
        )),
        "first write creates the identity marker"
    );

    let (status, _) = h
        .send(
            Method::PUT,
            &soul_uri,
            Some(serde_json::json!({"content": "calm and curious"})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    let (_, context) = h.json(Method::GET, "/api/companion", None).await;
    assert_eq!(context["slug"], CANONICAL_SLUG);
    assert_eq!(context["exists"], true);
    assert_eq!(context["companion_name"], "Luna");
    assert_eq!(context["soul_exists"], true);
    assert_eq!(h.instance_dirs(), vec![CANONICAL_SLUG]);
}

#[tokio::test]
async fn unsupported_identity_marker_fails_closed_for_reads_and_writes() {
    let h = harness().await;
    fs::create_dir_all(h.companion()).unwrap();
    let raw = format!(
        r#"{{"format_version":{},"slug":"{CANONICAL_SLUG}"}}"#,
        STORAGE_FORMAT_VERSION + 1
    );
    fs::write(h.companion().join(IDENTITY_FILE), &raw).unwrap();

    for (method, uri, body) in [
        (Method::GET, "/api/companion".to_owned(), None),
        (
            Method::GET,
            format!("/api/instances/{CANONICAL_SLUG}/soul"),
            None,
        ),
        (
            Method::PUT,
            format!("/api/instances/{CANONICAL_SLUG}/companion-name"),
            Some(serde_json::json!({"name": "Luna"})),
        ),
    ] {
        let label = format!("{method} {uri}");
        let (status, value) = h.json(method, &uri, body).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{label}");
        assert_eq!(value["error"], "companion_format_unsupported", "{label}");
    }

    assert_eq!(
        fs::read_to_string(h.companion().join(IDENTITY_FILE)).unwrap(),
        raw,
        "marker must not be rewritten"
    );
    assert!(!h.companion().join("project_state.json").exists());
}

#[tokio::test]
async fn every_persisted_subsystem_is_owned_by_the_canonical_companion() {
    let h = harness().await;
    let ws = h.workspace.path();
    let api = |suffix: &str| format!("/api/instances/{CANONICAL_SLUG}/{suffix}");

    // Settings.
    let (status, _) = h
        .send(
            Method::PUT,
            &api("companion-name"),
            Some(serde_json::json!({"name": "Luna"})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = h
        .send(
            Method::PUT,
            &api("timezone"),
            Some(serde_json::json!({"timezone": "Europe/Berlin"})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    // Identity / soul.
    let (status, _) = h
        .send(
            Method::PUT,
            &api("soul"),
            Some(serde_json::json!({"content": "calm and curious"})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    // History.
    crate::services::chat::save_user_message(ws, CANONICAL_SLUG, "default", "hello there").unwrap();

    // Memory.
    let media = h.state.vector_store.media_store();
    media
        .write_instance_text(CANONICAL_SLUG, "memory/notes/tea.md", "- likes oolong")
        .unwrap();
    let (status, memory) = h.json(Method::GET, &api("memory"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        memory.to_string().contains("tea.md"),
        "memory listing must see the canonical library: {memory}"
    );

    // Scheduler.
    let scheduled_dir = companion::companion_dir(ws).join("scheduled");
    fs::create_dir_all(&scheduled_dir).unwrap();
    let task = crate::services::tools::ScheduledTask {
        id: "task-1".into(),
        task: "water the plants".into(),
        deliver_at: 0,
        created_at: 0,
    };
    fs::write(
        scheduled_dir.join("task-1.json"),
        serde_json::to_string(&task).unwrap(),
    )
    .unwrap();
    let due = crate::services::scheduler::due_scheduled_tasks(ws, 1);
    assert_eq!(due.len(), 1);
    assert!(due[0].0.starts_with(companion::companion_dir(ws)));
    let (status, scheduled) = h.json(Method::GET, &api("scheduled"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(scheduled.to_string().contains("water the plants"));

    // Machines: a registration carrying a foreign slug is bound to the canonical companion.
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    h.state
        .machine_registry
        .register(
            MachineInfo {
                machine_id: "mac-mini".into(),
                os: "macos".into(),
                hostname: "studio".into(),
                screen_width: 1920,
                screen_height: 1080,
                last_seen: 0,
                instance_slug: Some("alice".into()),
            },
            tx,
        )
        .await;
    let machines = h.state.machine_registry.list().await;
    assert_eq!(machines.len(), 1);
    assert_eq!(machines[0].instance_slug.as_deref(), Some(CANONICAL_SLUG));

    // Everything above landed under one directory.
    assert_eq!(h.instance_dirs(), vec![CANONICAL_SLUG]);
    let dir = companion::companion_dir(ws);
    for relative in [
        IDENTITY_FILE,
        "soul.md",
        "project_state.json",
        "memory/notes/tea.md",
        "scheduled/task-1.json",
    ] {
        assert!(dir.join(relative).is_file(), "missing {relative}");
    }
    assert!(
        dir.join("chats").is_dir(),
        "history lives under the companion"
    );

    // Export archive is rooted at the canonical slug.
    let (status, archive) = h.send(Method::GET, &api("export"), None).await;
    assert_eq!(status, StatusCode::OK);
    let archive_path = h.workspace.path().join("export.tar.gz");
    fs::write(&archive_path, &archive).unwrap();
    let listing = std::process::Command::new("tar")
        .arg("-tzf")
        .arg(&archive_path)
        .output()
        .unwrap();
    assert!(
        listing.status.success(),
        "{}",
        String::from_utf8_lossy(&listing.stderr)
    );
    let entries = String::from_utf8_lossy(&listing.stdout);
    let prefix = format!("{CANONICAL_SLUG}/");
    for entry in entries.lines().filter(|line| !line.is_empty()) {
        assert!(
            entry == CANONICAL_SLUG || entry.starts_with(&prefix),
            "archive entry outside the companion root: {entry}"
        );
    }
    assert!(entries.contains(&format!("{prefix}{IDENTITY_FILE}")));
    assert!(entries.contains(&format!("{prefix}soul.md")));
}
