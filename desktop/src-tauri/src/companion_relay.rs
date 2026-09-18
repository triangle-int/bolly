//! A private, ephemeral browser origin. The server token NEVER enters the webview.
//! Cookies cannot isolate ports, and Tauri's set_cookie has no source URL/host-only
//! flag. Instead, only native HTTP/WS requests to the validated upstream get the
//! Bearer token. The browser receives a separate, process-lifetime relay session.
use axum::{
    body::Body,
    extract::{
        ws::{Message as BrowserMessage, WebSocket},
        State, WebSocketUpgrade,
    },
    http::{HeaderMap, Request, Response, StatusCode},
    response::IntoResponse,
    routing::{any, get, post},
    Router,
};
use futures_util::{SinkExt, StreamExt};
use std::sync::{Arc, Mutex};
use tokio::sync::watch;
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};

// The validated upstream is trusted to provide executable app assets. Dynamic
// text still crosses a sanitizer, while this policy confines all browser I/O to
// the ephemeral relay origin so trusted code cannot exfiltrate the native token.
const COMPANION_CSP: &str = "default-src 'self'; connect-src 'self' {relay_ws}; img-src 'self' data: blob:; media-src 'self' data: blob:; font-src 'self' data:; style-src 'self' 'unsafe-inline'; script-src 'self' 'unsafe-inline'; worker-src 'self' blob:; object-src 'none'; base-uri 'none'; form-action 'self'; frame-src 'none'; frame-ancestors 'none'";

fn companion_csp(origin: &str) -> String {
    COMPANION_CSP.replace("{relay_ws}", &origin.replacen("http://", "ws://", 1))
}

#[derive(Clone)]
struct RelayState {
    upstream: url::Url,
    origin: String,
    session: String,
    cookie_name: String,
    token: Arc<Mutex<Option<String>>>,
    client: reqwest::Client,
    stopped: watch::Receiver<bool>,
}

pub struct Relay {
    pub origin: String,
    pub script: String,
    token: Arc<Mutex<Option<String>>>,
    stop: watch::Sender<bool>,
    #[cfg(test)]
    session: String,
}
impl Drop for Relay {
    fn drop(&mut self) {
        self.token.lock().unwrap().take();
        let _ = self.stop.send(true);
    }
}

fn session_cookie(name: &str, session: &str) -> String {
    // Host-only, session-only, HttpOnly; this is NOT the upstream credential.
    format!("{name}={session}; Path=/; HttpOnly; SameSite=Strict")
}

fn is_local_request(headers: &HeaderMap, origin: &str) -> bool {
    let expected_host = origin.strip_prefix("http://").unwrap();
    if headers.get("host").and_then(|v| v.to_str().ok()) != Some(expected_host) {
        return false;
    }
    if let Some(value) = headers.get("origin") {
        if value.to_str().ok() != Some(origin) {
            return false;
        }
    }
    if let Some(value) = headers.get("referer") {
        let Ok(url) = value.to_str().unwrap_or_default().parse::<url::Url>() else {
            return false;
        };
        if url.origin().ascii_serialization() != origin {
            return false;
        }
    }
    !matches!(
        headers.get("sec-fetch-site").and_then(|v| v.to_str().ok()),
        Some("cross-site" | "same-site")
    )
}

fn authorized(headers: &HeaderMap, state: &RelayState) -> bool {
    if *state.stopped.borrow() || !is_local_request(headers, &state.origin) {
        return false;
    }
    let expected = format!("{}={}", state.cookie_name, state.session);
    headers
        .get("cookie")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|cookies| cookies.split(';').any(|c| c.trim() == expected))
}

fn upstream_url(base: &url::Url, uri: &axum::http::Uri) -> Result<url::Url, StatusCode> {
    // Never use Url::join: a //host path must not choose another upstream.
    let mut target = base.clone();
    let path = uri.path();
    // Public media endpoints only accept query-token auth on older servers. Use
    // their existing, middleware-protected equivalents with a Bearer header.
    let mapped = if let Some(rest) = path.strip_prefix("/public/files/") {
        rest.split_once('/')
            .map(|(slug, id)| format!("/api/instances/{slug}/uploads/{id}/file"))
    } else if let Some(rest) = path.strip_prefix("/public/memory/") {
        rest.split_once('/')
            .map(|(slug, file)| format!("/api/instances/{slug}/memory/{file}"))
    } else {
        None
    };
    target.set_path(mapped.as_deref().unwrap_or(path));
    target.set_query(uri.query());
    if target.query_pairs().any(|(key, _)| key == "token") {
        return Err(StatusCode::BAD_REQUEST);
    }
    Ok(target)
}

pub async fn start(upstream: url::Url, token: String) -> Result<Relay, String> {
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .map_err(|_| "Could not start companion relay")?;
    let origin = format!(
        "http://{}",
        listener
            .local_addr()
            .map_err(|_| "Could not start companion relay")?
    );
    let session = uuid::Uuid::new_v4().to_string();
    let cookie_name = format!("bolly_relay_{}", uuid::Uuid::new_v4().simple());
    let token = Arc::new(Mutex::new(Some(token)));
    let (stop, stopped) = watch::channel(false);
    let state = RelayState {
        upstream,
        origin: origin.clone(),
        session: session.clone(),
        cookie_name,
        token: token.clone(),
        stopped,
        client: reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| "Could not start companion relay")?,
    };
    let mut shutdown = stop.subscribe();
    let app = Router::new()
        .route("/__desktop/bootstrap", post(bootstrap))
        .route("/api/ws", get(websocket))
        .fallback(any(forward))
        .with_state(state);
    tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = shutdown.changed().await;
            })
            .await;
    });
    // This secret only authorizes the local relay; it is not the server token.
    let script = format!(
        r#"
        if (location.origin === {origin}) {{
            localStorage.removeItem("bolly_auth_token");
            Object.defineProperty(window, "__BOLLY_DESKTOP_RELAY__", {{value: true}});
            window.__bollyBootstrap = async () => {{
                const response = await fetch("/__desktop/bootstrap", {{method: "POST", headers: {{"X-Bolly-Bootstrap": {session}}}}});
                if (response.ok) location.replace("/");
                else document.body.textContent = "Could not open companion. Return to the dashboard and reconnect.";
            }};
        }}
    "#,
        origin = serde_json::to_string(&origin).unwrap(),
        session = serde_json::to_string(&session).unwrap()
    );
    Ok(Relay {
        origin,
        script,
        token,
        stop,
        #[cfg(test)]
        session,
    })
}

async fn bootstrap(State(state): State<RelayState>, headers: HeaderMap) -> Response<Body> {
    if *state.stopped.borrow()
        || !is_local_request(&headers, &state.origin)
        || headers
            .get("x-bolly-bootstrap")
            .and_then(|v| v.to_str().ok())
            != Some(&state.session)
    {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    Response::builder()
        .status(StatusCode::NO_CONTENT)
        .header(
            "set-cookie",
            session_cookie(&state.cookie_name, &state.session),
        )
        .header("cache-control", "no-store")
        .body(Body::empty())
        .unwrap()
}

async fn forward(State(mut state): State<RelayState>, request: Request<Body>) -> Response<Body> {
    if !authorized(request.headers(), &state) {
        if request.uri().path() == "/"
            && request.method() == "GET"
            && is_local_request(request.headers(), &state.origin)
        {
            return Response::builder()
                .header("content-type", "text/html")
                .header("cache-control", "no-store")
                .header("content-security-policy", companion_csp(&state.origin))
                .header("referrer-policy", "no-referrer")
                .header("x-content-type-options", "nosniff")
                .body(Body::from("<!doctype html><html><body><script>window.__bollyBootstrap?.()</script></body></html>"))
                .unwrap();
        }
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let target = match upstream_url(&state.upstream, request.uri()) {
        Ok(url) => url,
        Err(code) => return code.into_response(),
    };
    let token = match state.token.lock().unwrap().clone() {
        Some(t) => t,
        None => return StatusCode::UNAUTHORIZED.into_response(),
    };
    let (parts, body) = request.into_parts();
    let mut headers = parts.headers;
    for name in [
        "host",
        "cookie",
        "authorization",
        "origin",
        "referer",
        "connection",
        "upgrade",
        "x-bolly-bootstrap",
        "accept-encoding",
    ] {
        headers.remove(name);
    }
    // Preserve streamed uploads/downloads and Range requests, but never follow redirects.
    let pending = state
        .client
        .request(parts.method, target)
        .headers(headers)
        .header("accept-encoding", "identity")
        .bearer_auth(&token)
        .body(reqwest::Body::wrap_stream(body.into_data_stream()))
        .send();
    let response = tokio::select! {
        _ = state.stopped.changed() => return StatusCode::UNAUTHORIZED.into_response(),
        result = pending => match result { Ok(r) => r, Err(_) => return StatusCode::BAD_GATEWAY.into_response() }
    };
    if response.status().is_redirection() {
        return StatusCode::BAD_GATEWAY.into_response();
    }
    if response
        .headers()
        .get("content-encoding")
        .is_some_and(|v| v != "identity")
    {
        return StatusCode::BAD_GATEWAY.into_response();
    }
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    let mime = content_type.split(';').next().unwrap_or_default().trim();
    let binary = (mime.starts_with("image/") && mime != "image/svg+xml")
        || mime.starts_with("audio/")
        || mime.starts_with("video/")
        || matches!(mime, "application/pdf" | "application/zip");
    if !binary && supported_text_content_type(&content_type).is_err() {
        return StatusCode::BAD_GATEWAY.into_response();
    }

    let mut output = Response::builder().status(response.status());
    for (name, value) in response.headers() {
        // Browser credentials, upstream cookie state and cross-origin navigation
        // instructions must not cross the relay boundary.
        if !matches!(
            name.as_str(),
            "set-cookie"
                | "connection"
                | "transfer-encoding"
                | "location"
                | "refresh"
                | "access-control-allow-origin"
                | "access-control-allow-credentials"
                | "content-security-policy"
                | "content-security-policy-report-only"
        ) {
            if redact_secret(name.as_str(), &token) != name.as_str() {
                continue;
            }
            if let Ok(value) = value.to_str() {
                output = output.header(
                    name,
                    sanitize_text(value, &state.upstream, &state.origin, &token),
                );
            }
        }
    }
    output = output
        .header("content-security-policy", companion_csp(&state.origin))
        .header("referrer-policy", "no-referrer")
        .header("x-content-type-options", "nosniff");
    if binary {
        // Preserve media streaming and Range support, retaining enough overlap
        // to redact a raw/mixed-percent secret split across network chunks.
        output.headers_mut().unwrap().remove("content-length");
        let stream = futures_util::stream::unfold(
            (
                response.bytes_stream(),
                state.stopped,
                token,
                Vec::new(),
                false,
            ),
            |(mut stream, mut stopped, token, mut pending, done)| async move {
                if done || *stopped.borrow() {
                    return None;
                }
                let next = tokio::select! {
                    _ = stopped.changed() => return None,
                    next = stream.next() => next,
                };
                match next {
                    Some(Ok(chunk)) => {
                        pending.extend_from_slice(&chunk);
                        let limit = pending.len().saturating_sub(token.len().saturating_mul(3));
                        let (safe, consumed) = redact_prefix(&pending, &token, limit);
                        pending.drain(..consumed);
                        Some((
                            Ok::<_, reqwest::Error>(safe),
                            (stream, stopped, token, pending, false),
                        ))
                    }
                    Some(Err(error)) => Some((Err(error), (stream, stopped, token, pending, true))),
                    None => Some((
                        Ok(redact_bytes(&pending, &token)),
                        (stream, stopped, token, Vec::new(), true),
                    )),
                }
            },
        );
        return output
            .header("cache-control", "no-store")
            .body(Body::from_stream(stream))
            .unwrap();
    }
    for header in ["content-length", "content-encoding", "etag", "content-md5"] {
        output.headers_mut().unwrap().remove(header);
    }
    if mime == "text/event-stream" {
        let body = sanitized_sse_body(response, state.stopped, state.upstream, state.origin, token);
        return output
            .header("cache-control", "no-store")
            .body(body)
            .unwrap();
    }
    let bytes = tokio::select! {
        _ = state.stopped.changed() => return StatusCode::UNAUTHORIZED.into_response(),
        result = response.bytes() => match result { Ok(v) => v, Err(_) => return StatusCode::BAD_GATEWAY.into_response() },
    };
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return StatusCode::BAD_GATEWAY.into_response();
    };
    let Ok(body) = sanitize_browser_text(text, &state.upstream, &state.origin, &token) else {
        return StatusCode::BAD_GATEWAY.into_response();
    };
    output
        .header("cache-control", "no-store")
        .body(Body::from(body))
        .unwrap()
}

async fn websocket(
    State(mut state): State<RelayState>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response<Body> {
    if !authorized(&headers, &state) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let mut url = state.upstream.clone();
    let scheme = if url.scheme() == "https" { "wss" } else { "ws" };
    url.set_scheme(scheme).unwrap();
    url.set_path("/api/ws");
    let Ok(mut request) = url.as_str().into_client_request() else {
        return StatusCode::BAD_GATEWAY.into_response();
    };
    let token = match state.token.lock().unwrap().clone() {
        Some(t) => t,
        None => return StatusCode::UNAUTHORIZED.into_response(),
    };
    let Ok(auth) = format!("Bearer {token}").parse() else {
        return StatusCode::BAD_GATEWAY.into_response();
    };
    request.headers_mut().insert("authorization", auth);
    let remote = tokio::select! {
        _ = state.stopped.changed() => return StatusCode::UNAUTHORIZED.into_response(),
        result = tokio_tungstenite::connect_async(request) => match result { Ok((socket, _)) => socket, Err(_) => return StatusCode::BAD_GATEWAY.into_response() }
    };
    ws.on_upgrade(move |browser| pipe(browser, remote, state))
}

async fn pipe(
    mut browser: WebSocket,
    mut remote: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    mut state: RelayState,
) {
    let mut stop = state.stopped.clone();
    let exchange = async move {
        loop {
            tokio::select! {
                _ = state.stopped.changed() => break,
                frame = browser.next() => {
                    let Some(Ok(frame)) = frame else { break };
                    let frame = match frame {
                        BrowserMessage::Text(v) => Message::Text(v.to_string().into()),
                        BrowserMessage::Binary(v) => Message::Binary(v),
                        BrowserMessage::Ping(v) => Message::Ping(v),
                        BrowserMessage::Pong(v) => Message::Pong(v),
                        BrowserMessage::Close(_) => break,
                    };
                    if remote.send(frame).await.is_err() { break; }
                },
                frame = remote.next() => {
                    let Some(Ok(frame)) = frame else { break };
                    let frame = match frame {
                        Message::Text(v) => {
                            let token = state.token.lock().unwrap().clone().unwrap_or_default();
                            let Ok(text) = sanitize_browser_text(
                                &v,
                                &state.upstream,
                                &state.origin,
                                &token,
                            ) else {
                                break;
                            };
                            BrowserMessage::Text(text.into())
                        },
                        Message::Binary(v) => {
                            let token = state.token.lock().unwrap().clone().unwrap_or_default();
                            let bytes = match std::str::from_utf8(&v) {
                                Ok(text) => {
                                    let Ok(text) = sanitize_browser_text(
                                        text,
                                        &state.upstream,
                                        &state.origin,
                                        &token,
                                    ) else {
                                        break;
                                    };
                                    text.into_bytes().into()
                                },
                                Err(_) => redact_bytes(&v, &token).into(),
                            };
                            BrowserMessage::Binary(bytes)
                        },
                        Message::Ping(v) => BrowserMessage::Ping(redact_bytes(&v, &state.token.lock().unwrap().clone().unwrap_or_default()).into()),
                        Message::Pong(v) => BrowserMessage::Pong(redact_bytes(&v, &state.token.lock().unwrap().clone().unwrap_or_default()).into()),
                        Message::Close(_) => break,
                        Message::Frame(_) => continue,
                    };
                    if browser.send(frame).await.is_err() { break; }
                }
            }
        }
    };
    tokio::select! {
        _ = stop.changed() => {},
        _ = exchange => {},
    }
}

// Match the exact secret with any mixture of literal bytes and percent-encoded
// bytes (including lower-case escapes). Never decode/re-encode unrelated text.
pub(crate) fn redact_secret(text: &str, token: &str) -> String {
    String::from_utf8(redact_bytes(text.as_bytes(), token)).expect("redaction preserves UTF-8")
}

fn redact_bytes(bytes: &[u8], token: &str) -> Vec<u8> {
    redact_prefix(bytes, token, bytes.len()).0
}

fn redact_prefix(bytes: &[u8], token: &str, limit: usize) -> (Vec<u8>, usize) {
    if token.is_empty() {
        return (bytes[..limit].to_vec(), limit);
    }
    // The marker must itself be secret-free, and its boundaries must not
    // accidentally recreate a secret by joining surrounding output.
    let marker = if !token.contains(['[', ']']) && !"[redacted]".contains(token) {
        "[redacted]".to_owned()
    } else {
        (0xfffd..=0x10ffff)
            .filter_map(char::from_u32)
            .find(|c| !token.contains(*c))
            .unwrap()
            .to_string()
    };
    let mut result = Vec::new();
    let mut i = 0;
    while i < limit {
        if bytes[i] != token.as_bytes()[0] && bytes[i] != b'%' {
            result.push(bytes[i]);
            i += 1;
            continue;
        }
        let mut positions = vec![i];
        for expected in token.bytes() {
            let mut next = Vec::new();
            for end in positions {
                if bytes.get(end) == Some(&expected) {
                    next.push(end + 1);
                }
                if bytes.get(end) == Some(&b'%') && end + 2 < bytes.len() {
                    let hex = |b: u8| (b as char).to_digit(16);
                    if let (Some(a), Some(b)) = (hex(bytes[end + 1]), hex(bytes[end + 2])) {
                        if (a * 16 + b) as u8 == expected {
                            next.push(end + 3);
                        }
                    }
                }
            }
            next.sort_unstable();
            next.dedup();
            positions = next;
            if positions.is_empty() {
                break;
            }
        }
        if let Some(end) = positions.into_iter().max() {
            result.extend_from_slice(marker.as_bytes());
            i = end;
        } else {
            result.push(bytes[i]);
            i += 1;
        }
    }
    (result, i)
}

fn rewrite_urls(text: &str, upstream: &url::Url, origin: &str, token: &str) -> String {
    // URL delimiters cover Markdown, HTML, prose and JSON string boundaries.
    text.split_inclusive(|c: char| {
        c.is_whitespace()
            || matches!(
                c,
                '"' | '\'' | '<' | '>' | '(' | ')' | '[' | ']' | '{' | '}' | '`'
            )
    })
    .map(|part| {
        let candidate = part.trim_end_matches(|c: char| {
            c.is_whitespace()
                || matches!(
                    c,
                    '"' | '\'' | '<' | '>' | '(' | ')' | '[' | ']' | '{' | '}' | '`' | ',' | '.'
                )
        });
        let suffix = &part[candidate.len()..];
        if !(candidate.starts_with("http://")
            || candidate.starts_with("https://")
            || candidate.starts_with('/'))
        {
            return part.to_owned();
        }
        let Ok(mut url) = upstream.join(candidate) else {
            return part.to_owned();
        };
        let media =
            url.path().starts_with("/public/files/") || url.path().starts_with("/public/memory/");
        // Public-origin links may differ from the connection origin. A
        // token-bearing public media URL can safely map to our fixed
        // upstream route; we never request that URL's foreign origin.
        let authenticated_media =
            !token.is_empty() && url.query_pairs().any(|(k, v)| k == "token" && v == token);
        if media && (url.origin() == upstream.origin() || authenticated_media) {
            let params: Vec<_> = url
                .query_pairs()
                .filter(|(k, _)| k != "token")
                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                .collect();
            url.set_query(None);
            if !params.is_empty() {
                url.query_pairs_mut().extend_pairs(params);
            }
            format!("{}{}{}", origin, &url[url::Position::BeforePath..], suffix)
        } else {
            part.to_owned()
        }
    })
    .collect()
}

fn sanitize_value(value: &mut serde_json::Value, upstream: &url::Url, origin: &str, token: &str) {
    match value {
        serde_json::Value::String(text) => *text = sanitize_text(text, upstream, origin, token),
        serde_json::Value::Array(values) => {
            for value in values {
                sanitize_value(value, upstream, origin, token);
            }
        }
        serde_json::Value::Object(values) => {
            let old = std::mem::take(values);
            for (key, mut value) in old {
                sanitize_value(&mut value, upstream, origin, token);
                values.insert(sanitize_text(&key, upstream, origin, token), value);
            }
        }
        _ => {}
    }
}

fn sanitize_text(text: &str, upstream: &url::Url, origin: &str, token: &str) -> String {
    let rewritten = if let Ok(mut value) = serde_json::from_str::<serde_json::Value>(text) {
        sanitize_value(&mut value, upstream, origin, token);
        value.to_string()
    } else {
        rewrite_urls(text, upstream, origin, token)
    };
    redact_secret(&rewritten, token)
}

fn supported_text_content_type(content_type: &str) -> Result<(), ()> {
    let lower = content_type.to_ascii_lowercase();
    let mime = lower.split(';').next().unwrap_or_default().trim();
    if mime.is_empty() {
        return Ok(());
    }
    let textual = mime.starts_with("text/")
        || mime == "application/json"
        || mime.ends_with("+json")
        || mime == "application/javascript"
        || mime == "application/xml"
        || mime.ends_with("+xml");
    if !textual {
        return Err(());
    }
    for parameter in lower.split(';').skip(1) {
        if let Some(charset) = parameter.trim().strip_prefix("charset=") {
            let charset = charset.trim_matches([' ', '\'', '"']);
            if !matches!(charset, "utf-8" | "utf8" | "us-ascii" | "ascii") {
                return Err(());
            }
        }
    }
    Ok(())
}

fn sanitize_browser_text(
    text: &str,
    upstream: &url::Url,
    origin: &str,
    token: &str,
) -> Result<String, ()> {
    let sanitized = sanitize_text(text, upstream, origin, token);
    // Browsers accept numeric character references without a semicolon, while
    // html_escape intentionally only decodes the unambiguous forms.
    let decoded = decode_numeric_html_entities(&html_escape::decode_html_entities(&sanitized));
    if redact_secret(&decoded, token) != decoded {
        return Err(());
    }
    Ok(sanitized)
}

fn decode_numeric_html_entities(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut output = String::with_capacity(text.len());
    let mut copied = 0;
    let mut i = 0;
    while i + 2 < bytes.len() {
        if bytes[i] != b'&' || bytes[i + 1] != b'#' {
            i += 1;
            continue;
        }
        let (radix, start) = if matches!(bytes.get(i + 2), Some(b'x' | b'X')) {
            (16, i + 3)
        } else {
            (10, i + 2)
        };
        let mut end = start;
        while let Some(byte) = bytes.get(end) {
            let valid = if radix == 16 {
                byte.is_ascii_hexdigit()
            } else {
                byte.is_ascii_digit()
            };
            if !valid {
                break;
            }
            end += 1;
        }
        if end == start {
            i += 1;
            continue;
        }
        let Ok(digits) = std::str::from_utf8(&bytes[start..end]) else {
            i += 1;
            continue;
        };
        let Ok(value) = u32::from_str_radix(digits, radix) else {
            i += 1;
            continue;
        };
        output.push_str(&text[copied..i]);
        output.push(char::from_u32(value).unwrap_or('\u{fffd}'));
        if bytes.get(end) == Some(&b';') {
            end += 1;
        }
        copied = end;
        i = end;
    }
    output.push_str(&text[copied..]);
    output
}

fn sse_delimiter(bytes: &[u8]) -> Option<usize> {
    let mut line_start = 0;
    let mut i = 0;
    while i < bytes.len() {
        let ending = match bytes[i] {
            b'\n' => 1,
            b'\r' if bytes.get(i + 1) == Some(&b'\n') => 2,
            b'\r' => 1,
            _ => {
                i += 1;
                continue;
            }
        };
        let end = i + ending;
        if i == line_start {
            return Some(end);
        }
        line_start = end;
        i = end;
    }
    None
}

fn sanitize_sse_event(
    event: &str,
    upstream: &url::Url,
    origin: &str,
    token: &str,
) -> Result<String, ()> {
    let mut output = String::with_capacity(event.len());
    let bytes = event.as_bytes();
    let mut start = 0;
    while start < bytes.len() {
        let mut end = start;
        while end < bytes.len() && !matches!(bytes[end], b'\r' | b'\n') {
            end += 1;
        }
        let ending_end = if bytes.get(end) == Some(&b'\r') && bytes.get(end + 1) == Some(&b'\n') {
            end + 2
        } else if end < bytes.len() {
            end + 1
        } else {
            end
        };
        let line = &event[start..end];
        if let Some(payload) = line.strip_prefix("data:") {
            output.push_str("data:");
            output.push_str(&sanitize_browser_text(payload, upstream, origin, token)?);
        } else {
            output.push_str(&sanitize_browser_text(line, upstream, origin, token)?);
        }
        output.push_str(&event[end..ending_end]);
        start = ending_end;
    }
    Ok(output)
}

fn sanitized_sse_body(
    response: reqwest::Response,
    stopped: watch::Receiver<bool>,
    upstream: url::Url,
    origin: String,
    token: String,
) -> Body {
    const MAX_EVENT_BYTES: usize = 1024 * 1024;
    let stream = futures_util::stream::unfold(
        (response.bytes_stream(), Vec::<u8>::new(), false),
        move |(mut source, mut pending, done)| {
            let mut stopped = stopped.clone();
            let upstream = upstream.clone();
            let origin = origin.clone();
            let token = token.clone();
            async move {
                if done || *stopped.borrow() {
                    return None;
                }
                loop {
                    if let Some(end) = sse_delimiter(&pending) {
                        if end > MAX_EVENT_BYTES {
                            return Some((
                                Err(std::io::Error::other("SSE event too large")),
                                (source, Vec::new(), true),
                            ));
                        }
                        let event = pending.drain(..end).collect::<Vec<_>>();
                        let output = std::str::from_utf8(&event)
                            .map_err(|_| std::io::Error::other("invalid SSE encoding"))
                            .and_then(|text| {
                                sanitize_sse_event(text, &upstream, &origin, &token)
                                    .map_err(|_| std::io::Error::other("unsafe SSE event"))
                            });
                        return Some((
                            output.map(axum::body::Bytes::from),
                            (source, pending, false),
                        ));
                    }
                    if pending.len() > MAX_EVENT_BYTES {
                        return Some((
                            Err(std::io::Error::other("SSE event too large")),
                            (source, Vec::new(), true),
                        ));
                    }
                    let next = tokio::select! {
                        _ = stopped.changed() => return None,
                        next = source.next() => next,
                    };
                    match next {
                        Some(Ok(chunk)) => pending.extend_from_slice(&chunk),
                        Some(Err(error)) => {
                            return Some((
                                Err(std::io::Error::other(error)),
                                (source, Vec::new(), true),
                            ))
                        }
                        None if pending.is_empty() => return None,
                        None => {
                            let output = std::str::from_utf8(&pending)
                                .map_err(|_| std::io::Error::other("invalid SSE encoding"))
                                .and_then(|text| {
                                    sanitize_sse_event(text, &upstream, &origin, &token)
                                        .map_err(|_| std::io::Error::other("unsafe SSE event"))
                                });
                            return Some((
                                output.map(axum::body::Bytes::from),
                                (source, Vec::new(), true),
                            ));
                        }
                    }
                }
            }
        },
    );
    Body::from_stream(stream)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upstream_scope_includes_scheme_host_and_port_for_every_host_kind() {
        for origin in [
            "http://localhost:3000",
            "http://127.0.0.1:3000",
            "http://[::1]:3000",
            "https://example.org:8443",
        ] {
            let base = crate::connection_url(origin).unwrap();
            for path in [
                "/api/meta",
                "//attacker.invalid/api/meta",
                "/public/files/slug/id",
                "/public/memory/slug/image.png",
            ] {
                let target = upstream_url(&base, &path.parse().unwrap()).unwrap();
                assert_eq!(target.origin(), base.origin());
                assert!(target.query().is_none());
            }
            assert!(upstream_url(&base, &"/api/meta?token=secret".parse().unwrap()).is_err());
            assert_eq!(
                upstream_url(&base, &"/public/files/slug/id".parse().unwrap())
                    .unwrap()
                    .path(),
                "/api/instances/slug/uploads/id/file"
            );
            assert_eq!(
                upstream_url(&base, &"/public/memory/slug/a%20b.png".parse().unwrap())
                    .unwrap()
                    .path(),
                "/api/instances/slug/memory/a%20b.png"
            );
        }
    }

    #[test]
    fn relay_cookie_is_host_only_http_only_and_never_contains_upstream_token() {
        let cookie = session_cookie("bolly_relay_random", "random-session");
        assert_eq!(
            cookie,
            "bolly_relay_random=random-session; Path=/; HttpOnly; SameSite=Strict"
        );
        assert!(!cookie.contains("Domain="));
        assert!(!cookie.contains("Max-Age="));
        assert!(!cookie.contains("Expires="));
    }

    #[test]
    fn sibling_ports_hosts_and_cross_origin_requests_cannot_use_relay_session() {
        let origin = "http://127.0.0.1:31234";
        let mut headers = HeaderMap::new();
        headers.insert("host", "127.0.0.1:31234".parse().unwrap());
        assert!(is_local_request(&headers, origin));
        for other in [
            "http://127.0.0.1:31235",
            "http://localhost:31234",
            "https://127.0.0.1:31234",
            "https://example.org",
        ] {
            headers.insert("origin", other.parse().unwrap());
            assert!(!is_local_request(&headers, origin));
            headers.remove("origin");
            headers.insert("referer", format!("{other}/").parse().unwrap());
            assert!(!is_local_request(&headers, origin));
            headers.remove("referer");
        }
        headers.insert("sec-fetch-site", "same-site".parse().unwrap());
        assert!(!is_local_request(&headers, origin));
        headers.remove("sec-fetch-site");
        headers.insert("host", "evil.invalid:31234".parse().unwrap());
        assert!(!is_local_request(&headers, origin));
    }

    #[test]
    fn generated_media_links_are_localized_without_query_credentials() {
        let base = crate::connection_url("https://example.org:8443").unwrap();
        let mut value = serde_json::json!({"images": [
            "https://example.org:8443/public/memory/a/test.png?token=long-lived&size=2",
            "/public/files/a/b?token=long-lived",
            "https://other.example/public/files/a/b",
        ]});
        sanitize_value(&mut value, &base, "http://127.0.0.1:1234", "long-lived");
        assert_eq!(
            value["images"][0],
            "http://127.0.0.1:1234/public/memory/a/test.png?size=2"
        );
        assert_eq!(value["images"][1], "http://127.0.0.1:1234/public/files/a/b");
        assert_eq!(value["images"][2], "https://other.example/public/files/a/b");
    }

    #[test]
    fn sanitizes_embedded_markdown_nested_json_encoded_and_foreign_urls() {
        let base = crate::connection_url("https://private.example:8443").unwrap();
        let origin = "http://127.0.0.1:1234";
        let value = serde_json::json!({"nested": [{"text":
            "See ![image](https://private.example:8443/public/files/a/b?token=TOP_SECRET&size=2) and [public](https://public.example/public/memory/a/b.png?token=TOP%5fSECRET). Foreign https://foreign.example/path?x=TOP%5FSECRET and malformed TOP_SECRET"
        }], "TOP_SECRET": "{\"deep\":\"TOP_SECRET\"}"});
        let output = sanitize_text(&value.to_string(), &base, origin, "TOP_SECRET");
        assert!(!output.contains("TOP_SECRET"));
        assert!(!output.contains("TOP%5"));
        assert!(output.contains("http://127.0.0.1:1234/public/files/a/b?size=2"));
        assert!(output.contains("http://127.0.0.1:1234/public/memory/a/b.png"));
        assert!(output.contains("https://foreign.example/path?x=[redacted]"));
        assert!(serde_json::from_str::<serde_json::Value>(&output).is_ok());
        for token in ["TOP_SECRET", "a+b/c?d=e&f%", "%25x", "%x", "токен"] {
            let encoded: String = token.bytes().map(|b| format!("%{b:02x}")).collect();
            let text = format!("prose {token} and {encoded}");
            let result = sanitize_text(&text, &base, origin, token);
            assert_eq!(result, "prose [redacted] and [redacted]");
        }
        assert_eq!(
            sanitize_text(r#"{"x":"TOP\u005fSECRET"}"#, &base, origin, "TOP_SECRET"),
            r#"{"x":"[redacted]"}"#
        );
    }

    #[test]
    fn browser_text_policy_rejects_utf16_and_entity_reconstruction() {
        let base = crate::connection_url("https://private.example").unwrap();
        let origin = "http://127.0.0.1:1234";
        assert!(supported_text_content_type("text/html; charset=utf-16le").is_err());
        assert!(supported_text_content_type("application/json; charset=UTF-16").is_err());
        assert!(supported_text_content_type("text/html; charset=utf-8").is_ok());
        assert!(
            sanitize_browser_text("<p>TOP&#95;SECRET</p>", &base, origin, "TOP_SECRET").is_err()
        );
        assert!(
            sanitize_browser_text("<p>TOP&lowbar;SECRET</p>", &base, origin, "TOP_SECRET").is_err()
        );
        for text in [
            "<p>TOP&#95SECRET</p>",
            "<p>TOP&#x5fSECRET</p>",
            "<p>TOP&#X5FSECRET</p>",
        ] {
            assert!(sanitize_browser_text(text, &base, origin, "TOP_SECRET").is_err());
        }
    }

    #[test]
    fn sse_accepts_all_line_endings_and_sanitizes_data_as_json() {
        let base = crate::connection_url("https://private.example").unwrap();
        let origin = "http://127.0.0.1:1234";
        for event in [
            "data: {\"text\":\"TOP\\u005fSECRET\"}\n\n",
            "data: {\"text\":\"TOP\\u005fSECRET\"}\r\r",
            "data: {\"text\":\"TOP\\u005fSECRET\"}\r\n\r\n",
            "data: {\"text\":\"TOP\\u005fSECRET\"}\r\n\r",
        ] {
            assert_eq!(sse_delimiter(event.as_bytes()), Some(event.len()));
            let output = sanitize_sse_event(event, &base, origin, "TOP_SECRET").unwrap();
            assert!(!output.contains("TOP_SECRET"));
            assert!(!output.contains("u005f"));
            assert!(output.contains("[redacted]"));
        }
        assert!(sanitize_sse_event(
            "data: {\"text\":\"TOP&#95SECRET\"}\n\n",
            &base,
            origin,
            "TOP_SECRET"
        )
        .is_err());
    }

    #[test]
    fn streaming_redaction_covers_every_chunk_boundary() {
        let source = b"binary prefix TOP%5fSECRET then TOP_SECRET suffix";
        for chunk_size in 1..source.len() {
            let mut pending = Vec::new();
            let mut output = Vec::new();
            for chunk in source.chunks(chunk_size) {
                pending.extend_from_slice(chunk);
                let limit = pending.len().saturating_sub("TOP_SECRET".len() * 3);
                let (safe, consumed) = redact_prefix(&pending, "TOP_SECRET", limit);
                output.extend(safe);
                pending.drain(..consumed);
            }
            output.extend(redact_bytes(&pending, "TOP_SECRET"));
            assert_eq!(output, b"binary prefix [redacted] then [redacted] suffix");
        }
        for token in ["redacted", "[", "]", "a[", "%25x"] {
            assert!(!redact_secret(&format!("{token}{token}"), token).contains(token));
        }
    }

    async fn fake_upstream() -> (url::Url, tokio::task::JoinHandle<()>) {
        async fn endpoint(request: Request<Body>) -> Response<Body> {
            assert_eq!(
                request.headers().get("authorization").unwrap(),
                "Bearer long-lived"
            );
            assert!(!request.headers().contains_key("cookie"));
            assert!(!request.headers().contains_key("x-bolly-bootstrap"));
            assert!(request.uri().query().is_none());
            if request.uri().path() == "/redirect" {
                return Response::builder()
                    .status(302)
                    .header("location", "http://127.0.0.1:1/stolen")
                    .body(Body::empty())
                    .unwrap();
            }
            if request.uri().path() == "/text" {
                return Response::builder().header("content-type", "text/markdown")
                    .header("x-test", "long-lived")
                    .body(Body::from("![x](/public/files/a/b?token=long%2dlived) prose long-lived foreign https://other.invalid/?token=long%2Dlived")).unwrap();
            }
            if request.uri().path() == "/utf16" {
                return Response::builder()
                    .header("content-type", "text/html; charset=utf-16le")
                    .body(Body::from(
                        "long-lived"
                            .encode_utf16()
                            .flat_map(u16::to_le_bytes)
                            .collect::<Vec<_>>(),
                    ))
                    .unwrap();
            }
            if request.uri().path() == "/entities" {
                return Response::builder()
                    .header("content-type", "text/html; charset=utf-8")
                    .body(Body::from("<p>long&#45;lived</p>"))
                    .unwrap();
            }
            if request.uri().path() == "/events" {
                let chunks = futures_util::stream::iter([
                    Ok::<_, std::convert::Infallible>("data: first\n\n"),
                    Ok("data: long-"),
                    Ok("lived\n\n"),
                    Ok("data: last\n\n"),
                ]);
                return Response::builder()
                    .header("content-type", "text/event-stream; charset=utf-8")
                    .body(Body::from_stream(chunks))
                    .unwrap();
            }
            Response::builder()
                .header("content-type", "text/plain; charset=utf-8")
                .header("set-cookie", "bolly_token=long-lived")
                .body(Body::from(request.uri().path().to_owned()))
                .unwrap()
        }
        async fn socket(headers: HeaderMap, ws: WebSocketUpgrade) -> Response<Body> {
            assert_eq!(headers.get("authorization").unwrap(), "Bearer long-lived");
            assert!(!headers.contains_key("cookie"));
            ws.on_upgrade(|mut socket| async move {
                while let Some(Ok(frame)) = socket.next().await {
                    if socket.send(frame).await.is_err() {
                        break;
                    }
                }
            })
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap())
            .parse()
            .unwrap();
        let app = Router::new()
            .route("/api/ws", get(socket))
            .fallback(any(endpoint));
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (url, task)
    }

    async fn session(client: &reqwest::Client, relay: &Relay) -> String {
        let response = client
            .post(format!("{}/__desktop/bootstrap", relay.origin))
            .header("origin", &relay.origin)
            .header("x-bolly-bootstrap", &relay.session)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        response.headers()["set-cookie"]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string()
    }

    #[tokio::test]
    async fn relay_bootstrap_http_media_redirects_and_revocation() {
        let (upstream, task) = fake_upstream().await;
        let relay = start(upstream, "long-lived".into()).await.unwrap();
        assert!(!relay.script.contains("long-lived"));
        let client = reqwest::Client::new();
        let unauth = client
            .get(format!("{}/api/meta", relay.origin))
            .send()
            .await
            .unwrap();
        assert_eq!(unauth.status(), StatusCode::UNAUTHORIZED);
        let bootstrap = client
            .get(format!("{}/", relay.origin))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert!(bootstrap.contains("__bollyBootstrap"));
        assert!(!bootstrap.contains(&relay.session));
        let cookie = session(&client, &relay).await;
        assert!(!cookie.contains("long-lived"));
        for (path, target) in [
            ("/api/meta", "/api/meta"),
            ("/public/files/a/b", "/api/instances/a/uploads/b/file"),
            (
                "/public/memory/a/image.png",
                "/api/instances/a/memory/image.png",
            ),
        ] {
            let response = client
                .get(format!("{}{path}", relay.origin))
                .header("cookie", &cookie)
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert!(!response.headers().contains_key("set-cookie"));
            assert_eq!(response.text().await.unwrap(), target);
        }
        let text_response = client
            .get(format!("{}/text", relay.origin))
            .header("cookie", &cookie)
            .send()
            .await
            .unwrap();
        assert_eq!(text_response.headers()["x-test"], "[redacted]");
        let csp = text_response.headers()["content-security-policy"]
            .to_str()
            .unwrap();
        assert!(csp.contains("connect-src 'self'"));
        assert!(csp.contains(&relay.origin.replacen("http://", "ws://", 1)));
        assert!(!csp.contains("https:"));
        assert!(csp.contains("form-action 'self'"));
        assert!(csp.contains("object-src 'none'"));
        let text = text_response.text().await.unwrap();
        assert!(!text.contains("long-lived"));
        assert!(!text.contains("long%"));
        assert!(text.contains(&format!("![x]({}/public/files/a/b)", relay.origin)));
        for path in ["/utf16", "/entities"] {
            let response = client
                .get(format!("{}{path}", relay.origin))
                .header("cookie", &cookie)
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        }
        let mut events = client
            .get(format!("{}/events", relay.origin))
            .header("cookie", &cookie)
            .send()
            .await
            .unwrap()
            .bytes_stream();
        let first = tokio::time::timeout(std::time::Duration::from_secs(1), events.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(first, "data: first\n\n");
        let mut remainder = Vec::new();
        while let Some(chunk) = events.next().await {
            remainder.extend_from_slice(&chunk.unwrap());
        }
        let remainder = String::from_utf8(remainder).unwrap();
        assert!(!remainder.contains("long-lived"));
        assert!(remainder.contains("[redacted]"));
        assert!(remainder.contains("data: last"));
        let denied = client
            .get(format!("{}/api/meta", relay.origin))
            .header("cookie", &cookie)
            .header("origin", "http://127.0.0.1:1")
            .send()
            .await
            .unwrap();
        assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);
        let redirect = client
            .get(format!("{}/redirect", relay.origin))
            .header("cookie", &cookie)
            .send()
            .await
            .unwrap();
        assert_eq!(redirect.status(), StatusCode::BAD_GATEWAY);
        let token = relay.token.clone();
        let origin = relay.origin.clone();
        drop(relay);
        assert!(token.lock().unwrap().is_none());
        if let Ok(response) = client
            .get(format!("{origin}/api/meta"))
            .header("cookie", &cookie)
            .send()
            .await
        {
            assert_ne!(response.status(), StatusCode::OK);
        }
        task.abort();
    }

    #[tokio::test]
    async fn websocket_has_no_url_token_and_disconnect_terminates_it() {
        async fn connect(
            relay: &Relay,
            cookie: &str,
        ) -> tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        > {
            let mut request = format!("{}/api/ws", relay.origin.replacen("http:", "ws:", 1))
                .into_client_request()
                .unwrap();
            assert!(request.uri().query().is_none());
            request
                .headers_mut()
                .insert("cookie", cookie.parse().unwrap());
            request
                .headers_mut()
                .insert("origin", relay.origin.parse().unwrap());
            tokio_tungstenite::connect_async(request).await.unwrap().0
        }

        let (upstream, task) = fake_upstream().await;
        let relay = start(upstream, "long-lived".into()).await.unwrap();
        let cookie = session(&reqwest::Client::new(), &relay).await;
        let mut socket = connect(&relay, &cookie).await;
        socket.send(Message::Text("hello".into())).await.unwrap();
        assert_eq!(
            socket.next().await.unwrap().unwrap(),
            Message::Text("hello".into())
        );
        for text in [
            "Markdown ![x](/public/files/a/b?token=long-lived) https://foreign.invalid/?token=long%2dlived",
            r#"{"nested":[{"text":"![x](https://public.example/public/files/a/b?token=long-lived)"}]}"#,
            "long-lived",
        ] {
            socket.send(Message::Text(text.into())).await.unwrap();
            let Message::Text(output) = socket.next().await.unwrap().unwrap() else { panic!("expected text") };
            assert!(!output.contains("long-lived"));
            assert!(!output.contains("long%"));
        }
        socket
            .send(Message::Text(r#"{"text":"long&#45lived"}"#.into()))
            .await
            .unwrap();
        let unsafe_closed = tokio::time::timeout(std::time::Duration::from_secs(2), socket.next())
            .await
            .unwrap();
        assert!(matches!(
            unsafe_closed,
            None | Some(Err(_)) | Some(Ok(Message::Close(_)))
        ));

        let mut socket = connect(&relay, &cookie).await;
        socket.send(Message::Text("again".into())).await.unwrap();
        assert_eq!(
            socket.next().await.unwrap().unwrap(),
            Message::Text("again".into())
        );
        drop(relay);
        let closed = tokio::time::timeout(std::time::Duration::from_secs(2), socket.next())
            .await
            .unwrap();
        assert!(matches!(
            closed,
            None | Some(Err(_)) | Some(Ok(Message::Close(_)))
        ));
        task.abort();
    }
}
