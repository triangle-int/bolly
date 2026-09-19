//! Explicit browser pairing and revocable session authentication (#112).
//!
//! Browsers never receive the server API token. Instead an owner creates a
//! short-lived, single-use pairing code (from the CLI, the desktop app, or an
//! already-paired browser), the new browser exchanges that code for a session,
//! and the session travels in an `HttpOnly` cookie. Sessions are bound to the
//! exact host they were issued on, rotate periodically, expire after idle and
//! absolute lifetimes, and can be listed and revoked individually.
//!
//! Only SHA-256 hashes of session secrets and pairing codes are kept in memory
//! or on disk. Nothing in this module logs a secret.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Name of the browser session cookie.
pub const COOKIE_NAME: &str = "nolune_session";
/// A pairing code must be redeemed within this window.
pub const CHALLENGE_TTL_SECS: u64 = 5 * 60;
/// Wrong guesses allowed per pairing code before it is destroyed.
pub const MAX_CHALLENGE_ATTEMPTS: u32 = 5;
/// Outstanding pairing codes kept at once; creating more evicts the oldest.
pub const MAX_PENDING_CHALLENGES: usize = 8;
/// A session that is not used for this long expires.
pub const SESSION_IDLE_SECS: u64 = 30 * 24 * 60 * 60;
/// A session never outlives this, however active it is.
pub const SESSION_ABSOLUTE_SECS: u64 = 90 * 24 * 60 * 60;
/// The cookie secret is replaced once it is older than this.
pub const SESSION_ROTATE_SECS: u64 = 24 * 60 * 60;
/// After rotation the previous secret still works briefly so that requests
/// already in flight with the old cookie do not fail.
pub const ROTATION_GRACE_SECS: u64 = 60;
/// Failed pairing attempts are counted over this sliding window.
pub const FAILURE_WINDOW_SECS: u64 = 10 * 60;
/// Once this many attempts fail inside the window, pairing is refused until
/// the window drains.
pub const MAX_FAILURES_PER_WINDOW: u32 = 20;

const STORE_VERSION: u32 = 1;
const CODE_DIGITS: usize = 8;
const CODE_HASH_DOMAIN: &[u8] = b"nolune/browser-pairing-code/v1\0";
const SECRET_HASH_DOMAIN: &[u8] = b"nolune/browser-session-secret/v1\0";

pub(crate) trait Clock: Send + Sync {
    fn now(&self) -> u64;
}

struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct StoredSession {
    id: String,
    secret_hash: String,
    #[serde(default)]
    previous_hash: Option<String>,
    #[serde(default)]
    previous_valid_until: u64,
    host: String,
    label: String,
    paired_via: String,
    created_at: u64,
    last_seen_at: u64,
    rotated_at: u64,
}

impl StoredSession {
    fn expires_at(&self) -> u64 {
        (self.last_seen_at + SESSION_IDLE_SECS).min(self.created_at + SESSION_ABSOLUTE_SECS)
    }

    fn is_expired(&self, now: u64) -> bool {
        now >= self.expires_at()
    }
}

#[derive(Serialize, Deserialize)]
struct StoreFile {
    version: u32,
    sessions: Vec<StoredSession>,
}

struct Challenge {
    id: String,
    code_hash: [u8; 32],
    created_at: u64,
    expires_at: u64,
    attempts: u32,
    /// Host the code was created on by a browser session, if any. The
    /// redeeming browser must use the same host. Codes created by the API
    /// token (CLI, desktop app) are not host-bound.
    bound_host: Option<String>,
    created_by: String,
}

#[derive(Default)]
struct Inner {
    sessions: Vec<StoredSession>,
    challenges: Vec<Challenge>,
    failures: Vec<u64>,
}

/// Public view of a paired browser.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SessionSummary {
    pub id: String,
    pub label: String,
    pub paired_via: String,
    pub host: String,
    pub created_at: u64,
    pub last_seen_at: u64,
    pub expires_at: u64,
}

/// A freshly created pairing code. `code` is shown to the owner exactly once.
#[derive(Clone, Debug, Serialize)]
pub struct PairingChallenge {
    pub id: String,
    pub code: String,
    pub expires_at: u64,
    pub expires_in_secs: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bound_host: Option<String>,
}

/// A session issued by a successful pairing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IssuedSession {
    pub cookie_value: String,
    pub max_age_secs: u64,
    pub summary: SessionSummary,
}

/// Result of validating a cookie on a request.
#[derive(Clone, Debug)]
pub struct Authenticated {
    pub id: String,
    /// Set when the secret was rotated during this request. The new cookie
    /// value and its remaining lifetime must be sent back to the browser.
    pub rotated: Option<(String, u64)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PairingError {
    /// The code is wrong, expired, already used, or bound to another host.
    Invalid,
    /// Too many failed attempts recently; try again later.
    RateLimited,
}

pub struct BrowserSessionStore {
    inner: Mutex<Inner>,
    path: Mutex<Option<PathBuf>>,
    clock: Arc<dyn Clock>,
}

impl Default for BrowserSessionStore {
    fn default() -> Self {
        Self::new()
    }
}

impl BrowserSessionStore {
    /// An in-memory store. Call [`attach_storage`](Self::attach_storage) to persist.
    pub fn new() -> Self {
        Self::with_clock(Arc::new(SystemClock))
    }

    pub(crate) fn with_clock(clock: Arc<dyn Clock>) -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
            path: Mutex::new(None),
            clock,
        }
    }

    /// Load persisted sessions from `path` and keep writing there.
    pub fn attach_storage(&self, path: PathBuf) {
        let loaded = match std::fs::read_to_string(&path) {
            Ok(raw) => match serde_json::from_str::<StoreFile>(&raw) {
                Ok(file) if file.version == STORE_VERSION => file.sessions,
                Ok(file) => {
                    log::warn!(
                        "browser session store {} has unsupported version {}; starting empty",
                        path.display(),
                        file.version
                    );
                    Vec::new()
                }
                Err(error) => {
                    log::warn!(
                        "browser session store {} is unreadable ({error}); starting empty",
                        path.display()
                    );
                    Vec::new()
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => {
                log::warn!(
                    "browser session store {} cannot be read ({error}); starting empty",
                    path.display()
                );
                Vec::new()
            }
        };
        let now = self.clock.now();
        {
            let mut inner = self.inner.lock().unwrap();
            let before = loaded.len();
            inner.sessions = loaded
                .into_iter()
                .filter(|session| !session.is_expired(now))
                .collect();
            let dropped = before - inner.sessions.len();
            if dropped > 0 {
                log::info!("[sessions] dropped {dropped} expired browser session(s) on load");
            }
        }
        *self.path.lock().unwrap() = Some(path);
        self.persist();
    }

    fn persist(&self) {
        let path = self.path.lock().unwrap().clone();
        let Some(path) = path else { return };
        let file = {
            let inner = self.inner.lock().unwrap();
            StoreFile {
                version: STORE_VERSION,
                sessions: inner.sessions.clone(),
            }
        };
        if let Err(error) = write_private(&path, &file) {
            log::warn!(
                "[sessions] could not persist browser sessions to {}: {error}",
                path.display()
            );
        }
    }

    /// Create a pairing code. `bound_host` is the host of the creating browser
    /// session; pass `None` for codes created with the API token.
    pub fn create_challenge(
        &self,
        bound_host: Option<String>,
        created_by: &str,
    ) -> PairingChallenge {
        let now = self.clock.now();
        let mut inner = self.inner.lock().unwrap();
        inner
            .challenges
            .retain(|challenge| challenge.expires_at > now);
        while inner.challenges.len() >= MAX_PENDING_CHALLENGES {
            inner.challenges.remove(0);
        }
        let taken: HashSet<[u8; 32]> = inner.challenges.iter().map(|c| c.code_hash).collect();
        let (code, code_hash) = loop {
            let code = random_digits(CODE_DIGITS);
            let hash = hash_code(&code);
            if !taken.contains(&hash) {
                break (code, hash);
            }
        };
        let id = hex(&random_bytes(8));
        let expires_at = now + CHALLENGE_TTL_SECS;
        inner.challenges.push(Challenge {
            id: id.clone(),
            code_hash,
            created_at: now,
            expires_at,
            attempts: 0,
            bound_host: bound_host.clone(),
            created_by: created_by.to_string(),
        });
        log::info!(
            "[sessions] pairing code {id} created by {created_by}{}",
            match &bound_host {
                Some(host) => format!(" for host {host}"),
                None => String::new(),
            }
        );
        PairingChallenge {
            id,
            code: format_code(&code),
            expires_at,
            expires_in_secs: CHALLENGE_TTL_SECS,
            bound_host,
        }
    }

    /// Redeem a pairing code from a browser on `host` and issue a session.
    pub fn confirm_challenge(
        &self,
        code: &str,
        host: &str,
        label: &str,
    ) -> Result<IssuedSession, PairingError> {
        let now = self.clock.now();
        let normalized = normalize_code(code);
        let mut inner = self.inner.lock().unwrap();
        inner.failures.retain(|at| at + FAILURE_WINDOW_SECS > now);
        if inner.failures.len() as u32 >= MAX_FAILURES_PER_WINDOW {
            log::warn!("[sessions] pairing refused: too many failed attempts");
            return Err(PairingError::RateLimited);
        }
        inner
            .challenges
            .retain(|challenge| challenge.expires_at > now);

        let matched = normalized.as_deref().and_then(|digits| {
            let hash = hash_code(digits);
            inner
                .challenges
                .iter()
                .position(|challenge| constant_time_eq(&challenge.code_hash, &hash))
        });

        let Some(index) = matched else {
            inner.failures.push(now);
            // Every outstanding code absorbs a wrong guess so a brute force
            // cannot probe them one by one.
            let mut destroyed = 0;
            inner.challenges.retain_mut(|challenge| {
                challenge.attempts += 1;
                let keep = challenge.attempts < MAX_CHALLENGE_ATTEMPTS;
                if !keep {
                    destroyed += 1;
                }
                keep
            });
            if destroyed > 0 {
                log::warn!(
                    "[sessions] destroyed {destroyed} pairing code(s) after repeated wrong guesses"
                );
            }
            return Err(PairingError::Invalid);
        };

        let host_ok = inner.challenges[index]
            .bound_host
            .as_deref()
            .is_none_or(|bound| bound.eq_ignore_ascii_case(host));
        if !host_ok {
            inner.failures.push(now);
            let challenge = &mut inner.challenges[index];
            challenge.attempts += 1;
            let id = challenge.id.clone();
            let bound = challenge.bound_host.clone().unwrap_or_default();
            if challenge.attempts >= MAX_CHALLENGE_ATTEMPTS {
                inner.challenges.remove(index);
            }
            log::warn!(
                "[sessions] pairing code {id} refused: presented on host {host} but bound to {bound}"
            );
            return Err(PairingError::Invalid);
        }

        // Single use: the code is gone before the session exists.
        let challenge = inner.challenges.remove(index);
        let secret = random_bytes(32);
        let secret_encoded = URL_SAFE_NO_PAD.encode(secret);
        let id = hex(&random_bytes(8));
        let session = StoredSession {
            id: id.clone(),
            secret_hash: hash_secret(&secret_encoded),
            previous_hash: None,
            previous_valid_until: 0,
            host: host.to_string(),
            label: label.to_string(),
            paired_via: challenge.created_by.clone(),
            created_at: now,
            last_seen_at: now,
            rotated_at: now,
        };
        let summary = summarize(&session);
        inner.sessions.retain(|s| !s.is_expired(now));
        inner.sessions.push(session);
        drop(inner);
        self.persist();
        log::info!(
            "[sessions] browser {id} paired on host {host} via code {} (created by {}, {}s after creation)",
            challenge.id,
            challenge.created_by,
            now.saturating_sub(challenge.created_at)
        );
        Ok(IssuedSession {
            cookie_value: format!("{id}.{secret_encoded}"),
            max_age_secs: summary.expires_at.saturating_sub(now),
            summary,
        })
    }

    /// Validate a cookie value presented on `host`. Touches the session and
    /// rotates its secret when due.
    pub fn authenticate(&self, cookie_value: &str, host: &str) -> Option<Authenticated> {
        let (id, secret) = cookie_value.split_once('.')?;
        if id.is_empty() || secret.is_empty() || cookie_value.len() > 256 {
            return None;
        }
        let presented = hash_secret(secret);
        let now = self.clock.now();
        let mut persist = false;
        let result = {
            let mut inner = self.inner.lock().unwrap();
            let index = inner.sessions.iter().position(|s| s.id == id)?;
            let session = &mut inner.sessions[index];
            if session.is_expired(now) {
                inner.sessions.remove(index);
                drop(inner);
                self.persist();
                return None;
            }
            if !session.host.eq_ignore_ascii_case(host) {
                return None;
            }
            let current_ok = constant_time_eq(session.secret_hash.as_bytes(), presented.as_bytes());
            let previous_ok = session.previous_hash.as_deref().is_some_and(|previous| {
                session.previous_valid_until > now
                    && constant_time_eq(previous.as_bytes(), presented.as_bytes())
            });
            if !current_ok && !previous_ok {
                return None;
            }
            // Only refresh the idle clock at most once a minute to keep the
            // persisted file quiet under normal traffic.
            if now.saturating_sub(session.last_seen_at) >= 60 {
                session.last_seen_at = now;
                persist = true;
            }
            let mut rotated = None;
            if current_ok && now.saturating_sub(session.rotated_at) >= SESSION_ROTATE_SECS {
                let secret = random_bytes(32);
                let secret_encoded = URL_SAFE_NO_PAD.encode(secret);
                session.previous_hash = Some(session.secret_hash.clone());
                session.previous_valid_until = now + ROTATION_GRACE_SECS;
                session.secret_hash = hash_secret(&secret_encoded);
                session.rotated_at = now;
                persist = true;
                rotated = Some((
                    format!("{id}.{secret_encoded}"),
                    session.expires_at().saturating_sub(now),
                ));
            }
            Authenticated {
                id: id.to_string(),
                rotated,
            }
        };
        if persist {
            self.persist();
        }
        Some(result)
    }

    /// Whether a session id still authenticates. Used to close live
    /// WebSockets after revocation.
    pub fn is_active(&self, id: &str) -> bool {
        let now = self.clock.now();
        let inner = self.inner.lock().unwrap();
        inner
            .sessions
            .iter()
            .any(|s| s.id == id && !s.is_expired(now))
    }

    pub fn list(&self) -> Vec<SessionSummary> {
        let now = self.clock.now();
        let inner = self.inner.lock().unwrap();
        let mut sessions: Vec<SessionSummary> = inner
            .sessions
            .iter()
            .filter(|s| !s.is_expired(now))
            .map(summarize)
            .collect();
        sessions.sort_by_key(|s| std::cmp::Reverse(s.last_seen_at));
        sessions
    }

    pub fn revoke(&self, id: &str) -> bool {
        let removed = {
            let mut inner = self.inner.lock().unwrap();
            let before = inner.sessions.len();
            inner.sessions.retain(|s| s.id != id);
            before != inner.sessions.len()
        };
        if removed {
            self.persist();
            log::info!("[sessions] browser session {id} revoked");
        }
        removed
    }

    pub fn revoke_all(&self) -> usize {
        let removed = {
            let mut inner = self.inner.lock().unwrap();
            let count = inner.sessions.len();
            inner.sessions.clear();
            count
        };
        if removed > 0 {
            self.persist();
            log::info!("[sessions] revoked all {removed} browser session(s)");
        }
        removed
    }
}

fn summarize(session: &StoredSession) -> SessionSummary {
    SessionSummary {
        id: session.id.clone(),
        label: session.label.clone(),
        paired_via: session.paired_via.clone(),
        host: session.host.clone(),
        created_at: session.created_at,
        last_seen_at: session.last_seen_at,
        expires_at: session.expires_at(),
    }
}

fn write_private(path: &Path, file: &StoreFile) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let raw = serde_json::to_vec_pretty(file)?;
    let tmp = path.with_extension("json.tmp");
    {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut handle = options.open(&tmp)?;
        use std::io::Write;
        handle.write_all(&raw)?;
        handle.sync_all()?;
    }
    std::fs::rename(&tmp, path)
}

fn random_bytes(len: usize) -> Vec<u8> {
    let mut buf = vec![0u8; len];
    getrandom::fill(&mut buf).expect("operating system randomness is unavailable");
    buf
}

/// Uniform decimal digits via rejection sampling.
fn random_digits(len: usize) -> String {
    let mut digits = String::with_capacity(len);
    while digits.len() < len {
        for byte in random_bytes(len) {
            if byte < 250 {
                digits.push(char::from(b'0' + byte % 10));
                if digits.len() == len {
                    break;
                }
            }
        }
    }
    digits
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn hash_code(digits: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(CODE_HASH_DOMAIN);
    hasher.update(digits.as_bytes());
    hasher.finalize().into()
}

fn hash_secret(secret: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(SECRET_HASH_DOMAIN);
    hasher.update(secret.as_bytes());
    URL_SAFE_NO_PAD.encode(hasher.finalize())
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// `1234-5678` presentation of an eight-digit code.
pub fn format_code(digits: &str) -> String {
    match digits.len() {
        CODE_DIGITS => format!("{}-{}", &digits[..4], &digits[4..]),
        _ => digits.to_string(),
    }
}

/// Accepts `1234-5678`, `1234 5678`, or `12345678`; anything else is `None`.
pub fn normalize_code(input: &str) -> Option<String> {
    if input.len() > 64 {
        return None;
    }
    let digits: String = input
        .chars()
        .filter(|c| !matches!(c, ' ' | '-' | '\u{2011}' | '\u{2013}'))
        .collect();
    (digits.len() == CODE_DIGITS && digits.bytes().all(|b| b.is_ascii_digit())).then_some(digits)
}

/// Short human label for a browser derived from its User-Agent header.
pub fn describe_user_agent(user_agent: &str) -> String {
    let browser = if user_agent.contains("Edg/") {
        "Edge"
    } else if user_agent.contains("OPR/") {
        "Opera"
    } else if user_agent.contains("Firefox/") {
        "Firefox"
    } else if user_agent.contains("CriOS/") || user_agent.contains("Chrome/") {
        "Chrome"
    } else if user_agent.contains("Safari/") {
        "Safari"
    } else {
        "Browser"
    };
    let os = if user_agent.contains("iPhone") || user_agent.contains("iPad") {
        Some("iOS")
    } else if user_agent.contains("Android") {
        Some("Android")
    } else if user_agent.contains("Mac OS X") || user_agent.contains("Macintosh") {
        Some("macOS")
    } else if user_agent.contains("Windows") {
        Some("Windows")
    } else if user_agent.contains("Linux") {
        Some("Linux")
    } else {
        None
    };
    match os {
        Some(os) => format!("{browser} on {os}"),
        None => browser.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct MutableClock(AtomicU64);

    impl MutableClock {
        fn advance(&self, secs: u64) {
            self.0.fetch_add(secs, Ordering::SeqCst);
        }
    }

    impl Clock for MutableClock {
        fn now(&self) -> u64 {
            self.0.load(Ordering::SeqCst)
        }
    }

    fn fresh_store() -> (BrowserSessionStore, Arc<MutableClock>) {
        let clock = Arc::new(MutableClock(AtomicU64::new(1_000_000)));
        (BrowserSessionStore::with_clock(clock.clone()), clock)
    }

    #[test]
    fn code_is_single_use_and_issues_a_host_bound_session() {
        let (store, _) = fresh_store();
        let challenge = store.create_challenge(None, "cli");
        assert_eq!(challenge.code.len(), 9);
        assert_eq!(challenge.expires_in_secs, CHALLENGE_TTL_SECS);

        let issued = store
            .confirm_challenge(&challenge.code, "localhost:26559", "Safari on macOS")
            .unwrap();
        assert_eq!(issued.summary.paired_via, "cli");
        assert_eq!(issued.summary.host, "localhost:26559");
        assert_eq!(
            store.confirm_challenge(&challenge.code, "localhost:26559", "again"),
            Err(PairingError::Invalid),
            "a redeemed code must not work twice"
        );

        let auth = store
            .authenticate(&issued.cookie_value, "localhost:26559")
            .expect("cookie authenticates on the issuing host");
        assert_eq!(auth.id, issued.summary.id);
        assert!(auth.rotated.is_none());
        assert!(
            store
                .authenticate(&issued.cookie_value, "192.168.1.5:26559")
                .is_none(),
            "sessions are bound to the exact host"
        );
        assert!(
            store
                .authenticate(&issued.cookie_value.to_uppercase(), "localhost:26559")
                .is_none()
        );
        assert!(store.authenticate("", "localhost:26559").is_none());
    }

    #[test]
    fn codes_accept_common_formatting_and_reject_everything_else() {
        let (store, _) = fresh_store();
        let challenge = store.create_challenge(None, "cli");
        let spaced = challenge.code.replace('-', " ");
        assert!(store.confirm_challenge(&spaced, "h", "l").is_ok());

        let challenge = store.create_challenge(None, "cli");
        let compact = challenge.code.replace('-', "");
        assert!(store.confirm_challenge(&compact, "h", "l").is_ok());

        assert_eq!(normalize_code("1234-567"), None);
        assert_eq!(normalize_code("1234-56789"), None);
        assert_eq!(normalize_code("abcd-efgh"), None);
        assert_eq!(normalize_code(""), None);
    }

    #[test]
    fn wrong_guesses_destroy_codes_and_trip_the_rate_limit() {
        let (store, clock) = fresh_store();
        let challenge = store.create_challenge(None, "cli");
        let wrong = if challenge.code == "0000-0000" {
            "1111-1111"
        } else {
            "0000-0000"
        };
        for _ in 0..MAX_CHALLENGE_ATTEMPTS {
            assert_eq!(
                store.confirm_challenge(wrong, "h", "l"),
                Err(PairingError::Invalid)
            );
        }
        assert_eq!(
            store.confirm_challenge(&challenge.code, "h", "l"),
            Err(PairingError::Invalid),
            "a code guessed at too often is destroyed even when the right code arrives"
        );

        // Five wrong guesses plus the refused correct code make six failures.
        for _ in (MAX_CHALLENGE_ATTEMPTS + 1)..MAX_FAILURES_PER_WINDOW {
            assert_eq!(
                store.confirm_challenge(wrong, "h", "l"),
                Err(PairingError::Invalid)
            );
        }
        let fresh = store.create_challenge(None, "cli");
        assert_eq!(
            store.confirm_challenge(&fresh.code, "h", "l"),
            Err(PairingError::RateLimited),
            "even a valid code is refused while the failure window is saturated"
        );
        clock.advance(FAILURE_WINDOW_SECS);
        let after_window = store.create_challenge(None, "cli");
        assert!(
            store
                .confirm_challenge(&after_window.code, "h", "l")
                .is_ok(),
            "the limit drains with time"
        );
    }

    #[test]
    fn codes_expire_and_pending_codes_are_capped() {
        let (store, clock) = fresh_store();
        let stale = store.create_challenge(None, "cli");
        clock.advance(CHALLENGE_TTL_SECS);
        assert_eq!(
            store.confirm_challenge(&stale.code, "h", "l"),
            Err(PairingError::Invalid)
        );

        let first = store.create_challenge(None, "cli");
        for _ in 0..MAX_PENDING_CHALLENGES {
            store.create_challenge(None, "cli");
        }
        assert_eq!(
            store.confirm_challenge(&first.code, "h", "l"),
            Err(PairingError::Invalid),
            "the oldest code is evicted once the cap is reached"
        );
    }

    #[test]
    fn browser_created_codes_are_bound_to_their_host() {
        let (store, _) = fresh_store();
        let challenge = store.create_challenge(Some("nolune.local:26559".into()), "session:abc");
        assert_eq!(challenge.bound_host.as_deref(), Some("nolune.local:26559"));
        assert_eq!(
            store.confirm_challenge(&challenge.code, "localhost:26559", "l"),
            Err(PairingError::Invalid)
        );
        let issued = store
            .confirm_challenge(&challenge.code, "NOLUNE.local:26559", "l")
            .expect("host comparison is case-insensitive");
        assert_eq!(issued.summary.paired_via, "session:abc");
    }

    #[test]
    fn sessions_rotate_with_a_grace_period_and_expire() {
        let (store, clock) = fresh_store();
        let code = store.create_challenge(None, "cli").code;
        let issued = store.confirm_challenge(&code, "h", "l").unwrap();

        clock.advance(SESSION_ROTATE_SECS);
        let auth = store.authenticate(&issued.cookie_value, "h").unwrap();
        let (rotated, max_age) = auth.rotated.expect("secret rotates after a day");
        assert_ne!(rotated, issued.cookie_value);
        assert!(max_age > 0);
        assert!(
            store.authenticate(&issued.cookie_value, "h").is_some(),
            "the previous secret works inside the grace period"
        );
        assert!(
            store
                .authenticate(&issued.cookie_value, "h")
                .unwrap()
                .rotated
                .is_none(),
            "the previous secret never rotates again"
        );
        clock.advance(ROTATION_GRACE_SECS);
        assert!(
            store.authenticate(&issued.cookie_value, "h").is_none(),
            "the previous secret dies after the grace period"
        );
        assert!(store.authenticate(&rotated, "h").is_some());

        clock.advance(SESSION_IDLE_SECS);
        assert!(
            store.authenticate(&rotated, "h").is_none(),
            "idle sessions expire"
        );
        assert!(store.list().is_empty());
    }

    #[test]
    fn absolute_lifetime_caps_an_active_session() {
        let (store, clock) = fresh_store();
        let code = store.create_challenge(None, "cli").code;
        let mut cookie = store
            .confirm_challenge(&code, "h", "l")
            .unwrap()
            .cookie_value;
        let step = SESSION_IDLE_SECS / 2;
        let mut elapsed = 0;
        while elapsed + step < SESSION_ABSOLUTE_SECS {
            clock.advance(step);
            elapsed += step;
            let auth = store
                .authenticate(&cookie, "h")
                .expect("active session stays alive");
            if let Some((rotated, _)) = auth.rotated {
                cookie = rotated;
            }
        }
        clock.advance(SESSION_ABSOLUTE_SECS - elapsed);
        assert!(store.authenticate(&cookie, "h").is_none());
    }

    #[test]
    fn listing_and_revocation() {
        let (store, _) = fresh_store();
        let a = store
            .confirm_challenge(&store.create_challenge(None, "cli").code, "h", "Safari")
            .unwrap();
        let b = store
            .confirm_challenge(&store.create_challenge(None, "cli").code, "h", "Chrome")
            .unwrap();
        assert_eq!(store.list().len(), 2);
        assert!(store.is_active(&a.summary.id));
        assert!(store.revoke(&a.summary.id));
        assert!(!store.revoke(&a.summary.id));
        assert!(!store.is_active(&a.summary.id));
        assert!(store.authenticate(&a.cookie_value, "h").is_none());
        assert!(store.authenticate(&b.cookie_value, "h").is_some());
        assert_eq!(store.revoke_all(), 1);
        assert!(store.list().is_empty());
    }

    #[test]
    fn persistence_round_trips_without_secrets_and_with_private_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("browser_sessions.json");
        let (store, _) = fresh_store();
        store.attach_storage(path.clone());
        let issued = store
            .confirm_challenge(&store.create_challenge(None, "cli").code, "h", "Safari")
            .unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        let secret = issued.cookie_value.split_once('.').unwrap().1;
        assert!(
            !raw.contains(secret),
            "the cookie secret is never written to disk"
        );
        assert!(raw.contains(&issued.summary.id));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }

        let (reloaded, _) = fresh_store();
        reloaded.attach_storage(path);
        assert!(reloaded.authenticate(&issued.cookie_value, "h").is_some());
        assert_eq!(reloaded.list()[0].label, "Safari");
    }

    #[test]
    fn user_agent_labels_are_short_and_never_echo_the_header() {
        assert_eq!(
            describe_user_agent(
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15"
            ),
            "Safari on macOS"
        );
        assert_eq!(
            describe_user_agent(
                "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) CriOS/118.0 Mobile/15E148 Safari/604.1"
            ),
            "Chrome on iOS"
        );
        assert_eq!(
            describe_user_agent(
                "Mozilla/5.0 (X11; Linux x86_64; rv:120.0) Gecko/20100101 Firefox/120.0"
            ),
            "Firefox on Linux"
        );
        assert_eq!(describe_user_agent("curl/8.4.0"), "Browser");
    }
}
