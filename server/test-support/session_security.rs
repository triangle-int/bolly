//! Browser pairing and session tests for #112, run against the full router.

use super::*;
use axum::{
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use tower::ServiceExt;

const API_TOKEN: &str = "issue-112-seeded-api-token";
const HOST: &str = "localhost:26559";
const ORIGIN: &str = "http://localhost:26559";
const MAX_BODY: usize = 64 * 1024;

async fn seeded_state() -> AppState {
    let config = crate::config::Config {
        auth_token: API_TOKEN.into(),
        ..crate::config::Config::default()
    };
    AppState::new(config).await
}

fn request(
    method: Method,
    uri: &str,
    headers: &[(&str, &str)],
    body: Option<String>,
) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(uri);
    let mut has_host = false;
    for (name, value) in headers {
        has_host |= name.eq_ignore_ascii_case("host");
        builder = builder.header(*name, *value);
    }
    if !has_host {
        builder = builder.header(header::HOST, HOST);
    }
    match body {
        Some(body) => builder
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    }
}

async fn send(
    state: &AppState,
    req: Request<Body>,
) -> (StatusCode, axum::http::HeaderMap, serde_json::Value) {
    let response = build_router(state.clone(), None)
        .oneshot(req)
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    for value in headers.values() {
        assert!(
            !String::from_utf8_lossy(value.as_bytes()).contains(API_TOKEN),
            "a response header disclosed the API token"
        );
    }
    let bytes = axum::body::to_bytes(response.into_body(), MAX_BODY)
        .await
        .unwrap();
    assert!(
        !String::from_utf8_lossy(&bytes).contains(API_TOKEN),
        "a response body disclosed the API token"
    );
    let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, headers, json)
}

fn set_cookie(headers: &axum::http::HeaderMap) -> String {
    headers
        .get(header::SET_COOKIE)
        .expect("response sets the session cookie")
        .to_str()
        .unwrap()
        .to_string()
}

fn cookie_pair(set_cookie: &str) -> String {
    set_cookie.split(';').next().unwrap().trim().to_string()
}

async fn mint_code(state: &AppState) -> String {
    let (status, _, body) = send(
        state,
        request(
            Method::POST,
            "/api/session/pairing",
            &[("authorization", &format!("Bearer {API_TOKEN}"))],
            Some(r#"{"source":"cli"}"#.into()),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.get("bound_host").is_none(),
        "API-token codes are not host-bound"
    );
    assert_eq!(body["expires_in_secs"], 300);
    body["code"].as_str().unwrap().to_string()
}

async fn pair(
    state: &AppState,
    code: &str,
    headers: &[(&str, &str)],
) -> (StatusCode, axum::http::HeaderMap, serde_json::Value) {
    send(
        state,
        request(
            Method::POST,
            "/api/session/pair",
            headers,
            Some(serde_json::json!({ "code": code }).to_string()),
        ),
    )
    .await
}

async fn paired_cookie(state: &AppState) -> String {
    let code = mint_code(state).await;
    let (status, headers, body) = pair(
        state,
        &code,
        &[
            ("origin", ORIGIN),
            (
                "user-agent",
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) Safari/605.1.15",
            ),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["session"]["label"], "Safari on macOS");
    assert_eq!(body["session"]["paired_via"], "cli");
    cookie_pair(&set_cookie(&headers))
}

#[tokio::test]
async fn query_string_tokens_no_longer_authenticate_anything() {
    let state = seeded_state().await;
    for uri in [
        format!("/api/meta?token={API_TOKEN}"),
        format!("/api/ws?token={API_TOKEN}"),
        format!("/api/instances/moon/uploads/x/file?token={API_TOKEN}"),
    ] {
        let (status, _, _) = send(&state, request(Method::GET, &uri, &[], None)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{uri}");
    }
    let (status, _, _) = send(
        &state,
        request(
            Method::GET,
            "/api/meta",
            &[("authorization", &format!("Bearer {API_TOKEN}"))],
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the bearer token still works");
    let (status, _, _) = send(
        &state,
        request(
            Method::GET,
            "/api/meta",
            &[("authorization", "Bearer wrong")],
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn pairing_requires_an_owner_and_is_single_use() {
    let state = seeded_state().await;
    let (status, _, _) = send(
        &state,
        request(
            Method::POST,
            "/api/session/pairing",
            &[("origin", ORIGIN)],
            None,
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "anonymous callers cannot mint codes"
    );

    let code = mint_code(&state).await;
    assert_eq!(code.len(), 9);

    let (status, _, body) = pair(&state, &code, &[("origin", "http://evil.example")]).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    let (status, _, _) = pair(&state, &code, &[]).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "no Origin means no pairing");

    let (status, headers, _) = pair(&state, &code, &[("origin", ORIGIN)]).await;
    assert_eq!(status, StatusCode::OK);
    let cookie = set_cookie(&headers);
    assert!(cookie.starts_with("nolune_session="));
    assert!(cookie.contains("HttpOnly"));
    assert!(cookie.contains("SameSite=Strict"));
    assert!(cookie.contains("Path=/"));
    assert!(
        !cookie.contains("Secure"),
        "plain http://localhost must not set Secure"
    );

    let (status, _, body) = pair(&state, &code, &[("origin", ORIGIN)]).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"], "invalid_code");
}

#[tokio::test]
async fn session_cookie_is_bound_to_its_host_and_needs_same_origin_for_writes() {
    let state = seeded_state().await;
    let cookie = paired_cookie(&state).await;

    let (status, _, _) = send(
        &state,
        request(Method::GET, "/api/meta", &[("cookie", &cookie)], None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _, _) = send(
        &state,
        request(
            Method::GET,
            "/api/meta",
            &[("cookie", &cookie), ("host", "192.168.1.20:26559")],
            None,
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "another host cannot replay the cookie"
    );

    let (status, _, _) = send(
        &state,
        request(
            Method::GET,
            "/api/meta",
            &[("cookie", "nolune_session=forged.value")],
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // State-changing requests: cookie alone is not enough.
    let (status, _, _) = send(
        &state,
        request(
            Method::POST,
            "/api/session/pairing",
            &[("cookie", &cookie)],
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "missing Origin is rejected");
    let (status, _, _) = send(
        &state,
        request(
            Method::POST,
            "/api/session/pairing",
            &[("cookie", &cookie), ("origin", "http://evil.example")],
            None,
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "cross-site Origin is rejected"
    );
    let (status, _, body) = send(
        &state,
        request(
            Method::POST,
            "/api/session/pairing",
            &[("cookie", &cookie), ("origin", ORIGIN)],
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["bound_host"], HOST,
        "browser-minted codes bind to the browser's host"
    );
    let browser_code = body["code"].as_str().unwrap().to_string();

    let (status, _, _) = pair(
        &state,
        &browser_code,
        &[
            ("origin", "http://192.168.1.20:26559"),
            ("host", "192.168.1.20:26559"),
        ],
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "a bound code only works on its host"
    );
    let (status, _, body) = pair(&state, &browser_code, &[("origin", ORIGIN)]).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body["session"]["paired_via"]
            .as_str()
            .unwrap()
            .starts_with("browser:")
    );
}

#[tokio::test]
async fn websocket_upgrade_with_a_cookie_requires_the_same_origin() {
    let state = seeded_state().await;
    let cookie = paired_cookie(&state).await;
    let ws_headers = |origin: &'static str| -> Vec<(&'static str, String)> {
        vec![
            ("connection", "upgrade".into()),
            ("upgrade", "websocket".into()),
            ("sec-websocket-version", "13".into()),
            ("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==".into()),
            ("origin", origin.into()),
        ]
    };
    for (origin, expected) in [("http://evil.example", StatusCode::FORBIDDEN)] {
        let mut headers: Vec<(&str, &str)> = vec![("cookie", &cookie)];
        let extra = ws_headers(origin);
        headers.extend(extra.iter().map(|(k, v)| (*k, v.as_str())));
        let (status, _, _) = send(&state, request(Method::GET, "/api/ws", &headers, None)).await;
        assert_eq!(status, expected, "{origin}");
    }
    let mut headers: Vec<(&str, &str)> = vec![("cookie", &cookie)];
    let extra = ws_headers(ORIGIN);
    headers.extend(extra.iter().map(|(k, v)| (*k, v.as_str())));
    let (status, _, _) = send(&state, request(Method::GET, "/api/ws", &headers, None)).await;
    assert!(
        status != StatusCode::UNAUTHORIZED && status != StatusCode::FORBIDDEN,
        "same-origin upgrade passes authentication (got {status})"
    );
}

#[tokio::test]
async fn owners_can_list_and_revoke_paired_browsers() {
    let state = seeded_state().await;
    let first = paired_cookie(&state).await;
    let second = paired_cookie(&state).await;

    let (status, _, body) = send(
        &state,
        request(Method::GET, "/api/session", &[("cookie", &first)], None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["auth"], "session");
    let first_id = body["session"]["id"].as_str().unwrap().to_string();

    let (status, _, body) = send(
        &state,
        request(
            Method::GET,
            "/api/session/devices",
            &[("cookie", &first)],
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let devices = body["devices"].as_array().unwrap();
    assert_eq!(devices.len(), 2);
    assert_eq!(
        devices.iter().filter(|d| d["current"] == true).count(),
        1,
        "exactly one device is the caller"
    );
    let second_id = devices.iter().find(|d| d["current"] == false).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    // The API token sees the same list without a "current" entry.
    let (status, _, body) = send(
        &state,
        request(
            Method::GET,
            "/api/session/devices",
            &[("authorization", &format!("Bearer {API_TOKEN}"))],
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["auth"], "token");
    assert!(
        body["devices"]
            .as_array()
            .unwrap()
            .iter()
            .all(|d| d["current"] == false)
    );

    let (status, headers, _) = send(
        &state,
        request(
            Method::DELETE,
            &format!("/api/session/devices/{second_id}"),
            &[("cookie", &first), ("origin", ORIGIN)],
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(
        headers.get(header::SET_COOKIE).is_none(),
        "revoking another browser keeps this cookie"
    );
    let (status, _, _) = send(
        &state,
        request(Method::GET, "/api/meta", &[("cookie", &second)], None),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "the revoked browser is out immediately"
    );

    let (status, _, _) = send(
        &state,
        request(
            Method::DELETE,
            &format!("/api/session/devices/{second_id}"),
            &[("cookie", &first), ("origin", ORIGIN)],
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, headers, _) = send(
        &state,
        request(
            Method::POST,
            "/api/session/logout",
            &[("cookie", &first), ("origin", ORIGIN)],
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(set_cookie(&headers).contains("Max-Age=0"));
    let (status, _, _) = send(
        &state,
        request(Method::GET, "/api/meta", &[("cookie", &first)], None),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(!state.browser_sessions.is_active(&first_id));
}

#[tokio::test]
async fn api_token_rotation_leaves_browser_sessions_alone() {
    let state = seeded_state().await;
    let cookie = paired_cookie(&state).await;
    // Rotate in memory (the settings route also writes config.toml, which a
    // unit test must not touch).
    state.config.write().await.auth_token = "rotated-token".into();
    let (status, _, _) = send(
        &state,
        request(Method::GET, "/api/meta", &[("cookie", &cookie)], None),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "browser sessions survive token rotation"
    );
    let (status, _, _) = send(
        &state,
        request(
            Method::GET,
            "/api/meta",
            &[("authorization", &format!("Bearer {API_TOKEN}"))],
            None,
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "the old API token is dead"
    );
    let (status, _, _) = send(
        &state,
        request(
            Method::GET,
            "/api/meta",
            &[("authorization", "Bearer rotated-token")],
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(state.browser_sessions.list().len(), 1);
}

#[tokio::test]
async fn wrong_codes_are_rate_limited_without_leaking_anything() {
    let state = seeded_state().await;
    let mut saw_rate_limit = false;
    for attempt in 0..25 {
        let (status, headers, body) = pair(&state, "0000-0000", &[("origin", ORIGIN)]).await;
        if status == StatusCode::TOO_MANY_REQUESTS {
            assert_eq!(body["error"], "rate_limited");
            assert!(headers.get(header::RETRY_AFTER).is_some());
            saw_rate_limit = true;
            break;
        }
        assert_eq!(status, StatusCode::UNAUTHORIZED, "attempt {attempt}");
        assert!(headers.get(header::SET_COOKIE).is_none());
    }
    assert!(saw_rate_limit, "repeated wrong codes must trip the limiter");
}

#[tokio::test]
async fn disabled_auth_needs_no_pairing() {
    let state = AppState::new(crate::config::Config::default()).await;
    let (status, _, body) = send(&state, request(Method::GET, "/api/session", &[], None)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["auth"], "disabled");
    let (status, _, body) = pair(&state, "1234-5678", &[("origin", ORIGIN)]).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "auth_disabled");
    let (status, _, body) = send(
        &state,
        request(
            Method::POST,
            "/api/session/pairing",
            &[("origin", ORIGIN)],
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "auth_disabled");
}

#[tokio::test]
async fn malformed_pair_bodies_are_rejected_cleanly() {
    let state = seeded_state().await;
    for body in ["", "{}", "not json", r#"{"code":123}"#] {
        let (status, _, _) = send(
            &state,
            request(
                Method::POST,
                "/api/session/pair",
                &[("origin", ORIGIN)],
                Some(body.into()),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
    }
}
