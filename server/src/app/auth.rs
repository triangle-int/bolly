//! Request authentication for `/api/*`.
//!
//! Two credentials exist and they never mix:
//!
//! * The server API token, sent as `Authorization: Bearer`. It is the
//!   automation/admin credential used by the CLI, the desktop relay and
//!   machine agents. Browsers never receive it.
//! * A paired browser session, sent as the `nolune_session` cookie (see
//!   [`crate::services::browser_sessions`]). Sessions are bound to the exact
//!   host they were issued on and rotate on a schedule; a rotated cookie is
//!   returned on the same response.
//!
//! Credentials in query strings are not accepted anywhere.

use axum::{
    extract::{Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    middleware::Next,
    response::Response,
};

use super::state::AppState;
use crate::{config::Config, services::browser_sessions::COOKIE_NAME};

/// How the current request was authenticated. Inserted as a request extension
/// so routes can tell a paired browser from the API token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthContext {
    /// `auth_token` is empty, so nothing is protected.
    Disabled,
    /// The server API token.
    ApiToken,
    /// A paired browser session.
    BrowserSession { id: String },
}

pub async fn auth_middleware(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let (expected, secure) = {
        let config = state.config.read().await;
        (
            config.auth_token.clone(),
            secure_cookie(request.headers(), &config),
        )
    };

    if expected.is_empty() {
        request.extensions_mut().insert(AuthContext::Disabled);
        return Ok(next.run(request).await);
    }

    if let Some(bearer) = bearer_token(request.headers()) {
        if constant_time_eq(bearer.as_bytes(), expected.as_bytes()) {
            request.extensions_mut().insert(AuthContext::ApiToken);
            return Ok(next.run(request).await);
        }
        return Err(StatusCode::UNAUTHORIZED);
    }

    let Some(cookie) = cookie_value(request.headers(), COOKIE_NAME) else {
        return Err(StatusCode::UNAUTHORIZED);
    };
    let Some(host) = request_host(request.headers(), &request) else {
        return Err(StatusCode::UNAUTHORIZED);
    };
    let Some(auth) = state.browser_sessions.authenticate(&cookie, &host) else {
        return Err(StatusCode::UNAUTHORIZED);
    };
    // Cookies are attached by the browser, so anything that changes state (or
    // opens a WebSocket) must prove it came from this origin.
    if requires_same_origin(&request) && !same_origin(request.headers(), &host) {
        return Err(StatusCode::FORBIDDEN);
    }

    request
        .extensions_mut()
        .insert(AuthContext::BrowserSession { id: auth.id });
    let mut response = next.run(request).await;
    if let Some((value, max_age)) = auth.rotated {
        response
            .headers_mut()
            .append(header::SET_COOKIE, session_cookie(&value, max_age, secure));
    }
    Ok(response)
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|v| !v.is_empty())
}

/// Value of the cookie called `name`, if present.
pub(crate) fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|line| line.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(key, _)| key.trim() == name)
        .map(|(_, value)| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// The host the browser addressed, lower-cased. Prefers `X-Forwarded-Host`
/// (set by a reverse proxy), then `Host`, then the request authority.
pub(crate) fn request_host(headers: &HeaderMap, request: &Request) -> Option<String> {
    let forwarded = headers
        .get("x-forwarded-host")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(str::trim)
        .filter(|v| !v.is_empty());
    let host = forwarded
        .or_else(|| headers.get(header::HOST).and_then(|v| v.to_str().ok()))
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_ascii_lowercase)
        .or_else(|| {
            request
                .uri()
                .authority()
                .map(|a| a.as_str().to_ascii_lowercase())
        })?;
    (host.len() <= 255 && !host.contains(['/', '\\', ' '])).then_some(host)
}

/// State-changing requests and WebSocket upgrades need same-origin evidence.
pub(crate) fn requires_same_origin(request: &Request) -> bool {
    if !matches!(
        *request.method(),
        Method::GET | Method::HEAD | Method::OPTIONS
    ) {
        return true;
    }
    request
        .headers()
        .get(header::UPGRADE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.eq_ignore_ascii_case("websocket"))
}

/// Browsers send `Origin` on every non-GET request and every WebSocket
/// upgrade. Without it, only an explicit `Sec-Fetch-Site: same-origin`
/// counts. Nothing else is trusted.
pub(crate) fn same_origin(headers: &HeaderMap, host: &str) -> bool {
    if let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
        return origin_authority(origin)
            .is_some_and(|authority| authority.eq_ignore_ascii_case(host));
    }
    matches!(
        headers.get("sec-fetch-site").and_then(|v| v.to_str().ok()),
        Some("same-origin" | "none")
    )
}

fn origin_authority(origin: &str) -> Option<&str> {
    let rest = origin
        .strip_prefix("https://")
        .or_else(|| origin.strip_prefix("http://"))?;
    let authority = rest.split(['/', '?', '#']).next()?;
    (!authority.is_empty()).then_some(authority)
}

/// Whether the browser reached us over TLS, so the cookie may carry `Secure`.
/// Plain-HTTP localhost installs must not set it: Safari drops `Secure`
/// cookies on `http://localhost`.
pub(crate) fn secure_cookie(headers: &HeaderMap, config: &Config) -> bool {
    if headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .is_some_and(|v| v.trim().eq_ignore_ascii_case("https"))
    {
        return true;
    }
    let Some(public_authority) = origin_authority(&config.public_url) else {
        return false;
    };
    config.public_url.starts_with("https://")
        && headers
            .get(header::HOST)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|host| host.trim().eq_ignore_ascii_case(public_authority))
}

pub(crate) fn session_cookie(value: &str, max_age: u64, secure: bool) -> HeaderValue {
    let mut cookie =
        format!("{COOKIE_NAME}={value}; Path=/; Max-Age={max_age}; HttpOnly; SameSite=Strict");
    if secure {
        cookie.push_str("; Secure");
    }
    HeaderValue::from_str(&cookie).expect("session cookie is ASCII")
}

pub(crate) fn clear_session_cookie(secure: bool) -> HeaderValue {
    session_cookie("", 0, secure)
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.append(
                header::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                HeaderValue::from_str(value).unwrap(),
            );
        }
        map
    }

    #[test]
    fn origin_must_match_the_addressed_host() {
        let host = "localhost:26559";
        assert!(same_origin(
            &headers(&[("origin", "http://localhost:26559")]),
            host
        ));
        assert!(same_origin(
            &headers(&[("origin", "http://LOCALHOST:26559")]),
            host
        ));
        assert!(!same_origin(
            &headers(&[("origin", "http://evil.example")]),
            host
        ));
        assert!(!same_origin(&headers(&[("origin", "null")]), host));
        assert!(!same_origin(
            &headers(&[("origin", "http://localhost:26559.evil.example")]),
            host
        ));
        assert!(same_origin(
            &headers(&[("sec-fetch-site", "same-origin")]),
            host
        ));
        assert!(!same_origin(
            &headers(&[("sec-fetch-site", "cross-site")]),
            host
        ));
        assert!(!same_origin(&headers(&[]), host));
        assert!(
            !same_origin(
                &headers(&[
                    ("origin", "http://evil.example"),
                    ("sec-fetch-site", "same-origin")
                ]),
                host
            ),
            "an explicit Origin wins over Sec-Fetch-Site"
        );
    }

    #[test]
    fn cookie_parsing_picks_the_named_cookie_only() {
        let map = headers(&[("cookie", "other=1; nolune_session=abc.def ; last=2")]);
        assert_eq!(cookie_value(&map, COOKIE_NAME).as_deref(), Some("abc.def"));
        assert_eq!(cookie_value(&map, "missing"), None);
        let empty = headers(&[("cookie", "nolune_session=")]);
        assert_eq!(cookie_value(&empty, COOKIE_NAME), None);
    }

    #[test]
    fn secure_flag_follows_tls_evidence_only() {
        let mut config = Config {
            public_url: "https://nolune.example".into(),
            ..Config::default()
        };
        assert!(secure_cookie(
            &headers(&[("host", "nolune.example")]),
            &config
        ));
        assert!(!secure_cookie(
            &headers(&[("host", "localhost:26559")]),
            &config
        ));
        assert!(secure_cookie(
            &headers(&[("host", "localhost:26559"), ("x-forwarded-proto", "https")]),
            &config
        ));
        config.public_url = "http://localhost:26559".into();
        assert!(!secure_cookie(
            &headers(&[("host", "localhost:26559")]),
            &config
        ));
    }

    #[test]
    fn cookie_attributes_are_strict() {
        let cookie = session_cookie("id.secret", 42, true);
        assert_eq!(
            cookie.to_str().unwrap(),
            "nolune_session=id.secret; Path=/; Max-Age=42; HttpOnly; SameSite=Strict; Secure"
        );
        assert_eq!(
            clear_session_cookie(false).to_str().unwrap(),
            "nolune_session=; Path=/; Max-Age=0; HttpOnly; SameSite=Strict"
        );
    }
}
