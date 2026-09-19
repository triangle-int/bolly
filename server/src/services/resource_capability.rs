//! Integration contract for #116: each resource route hard-codes its audience, verifies the
//! capability before any filesystem access, and compares the exact canonical path string that
//! was signed. Audience is never accepted from a request, and API/machine routes never invoke
//! this verifier.

use std::{
    collections::HashMap,
    fmt,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use unicode_normalization::UnicodeNormalization;

pub(crate) const CAPABILITY_VERSION: u8 = 1;
pub(crate) const NONCE_BYTES: usize = 32;
const TOKEN_HEADER: &str = "v1";
const KEY_DERIVATION_DOMAIN: &[u8] = b"nolune/scoped-resource-capability/signing-key/v1";
const SIGNING_DOMAIN: &[u8] = b"nolune/scoped-resource-capability/token/v1\0";
const MAX_CONTROL_TOKEN_BYTES: usize = 4096;
const MAX_TOKEN_BYTES: usize = 4096;
const MAX_PAYLOAD_BYTES: usize = 2048;
const MAX_INSTANCE_BYTES: usize = 128;
const MAX_RESOURCE_BYTES: usize = 1024;
pub(crate) const MAX_TTL_SECONDS: u64 = 15 * 60;
const MAX_REPLAY_ENTRIES: usize = 65_536;

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CapabilityMethod {
    Get,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CapabilityAudience {
    Browser,
    ModelProvider,
    NativeRelay,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ReplayPolicy {
    ReusableWithinExpiry,
    SingleUse,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum CapabilityResource {
    UploadedFile { id: String },
    MemoryPath { path: String },
}

impl CapabilityResource {
    pub(crate) fn uploaded_file(id: impl Into<String>) -> Result<Self, CapabilityError> {
        let id = id.into();
        validate_single_component(&id, CapabilityError::InvalidUploadId)?;
        Ok(Self::UploadedFile { id })
    }

    pub(crate) fn memory(path: impl Into<String>) -> Result<Self, CapabilityError> {
        let path = path.into();
        validate_memory_path(&path)?;
        Ok(Self::MemoryPath { path })
    }

    /// Canonical RFC 3986 path encoding for use by internal URL producers.
    /// Memory separators remain separators; every component is encoded independently.
    pub(crate) fn encoded_path(&self) -> String {
        match self {
            Self::UploadedFile { id } => percent_encode_component(id),
            Self::MemoryPath { path } => path
                .split('/')
                .map(percent_encode_component)
                .collect::<Vec<_>>()
                .join("/"),
        }
    }

    fn validate(&self) -> Result<(), CapabilityError> {
        match self {
            Self::UploadedFile { id } => {
                validate_single_component(id, CapabilityError::InvalidUploadId)
            }
            Self::MemoryPath { path } => validate_memory_path(path),
        }
    }
}

pub(crate) struct CapabilityGrant {
    pub(crate) instance_slug: String,
    pub(crate) resource: CapabilityResource,
    pub(crate) method: CapabilityMethod,
    pub(crate) audience: CapabilityAudience,
    pub(crate) expires_at: u64,
    pub(crate) replay: ReplayPolicy,
}

pub(crate) struct CapabilityTarget {
    instance_slug: String,
    resource: CapabilityResource,
    method: CapabilityMethod,
    audience: CapabilityAudience,
}

impl CapabilityTarget {
    pub(crate) fn new(
        instance_slug: impl Into<String>,
        resource: CapabilityResource,
        method: CapabilityMethod,
        audience: CapabilityAudience,
    ) -> Result<Self, CapabilityError> {
        let instance_slug = instance_slug.into();
        validate_instance_slug(&instance_slug)?;
        resource.validate()?;
        Ok(Self {
            instance_slug,
            resource,
            method,
            audience,
        })
    }
}

pub(crate) struct VerifiedCapability {
    pub(crate) version: u8,
    pub(crate) expires_at: u64,
    pub(crate) replay: ReplayPolicy,
}

#[derive(Clone)]
pub(crate) struct CapabilityToken(String);

impl CapabilityToken {
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for CapabilityToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CapabilityToken([REDACTED])")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CapabilityError {
    EmptyControlToken,
    ControlTokenTooLong,
    InvalidInstanceSlug,
    InvalidUploadId,
    InvalidMemoryPath,
    ExpiryNotFuture,
    ExpiryTooFar,
    ReplayCapacity,
    NonceCollision,
    NonceUnavailable,
    MalformedToken,
    InvalidMac,
    UnsupportedVersion,
    WrongMethod,
    WrongAudience,
    WrongInstance,
    WrongResource,
    Expired,
    ReplayStateMissing,
    AlreadyConsumed,
}

impl fmt::Display for CapabilityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::EmptyControlToken => "control token must not be empty",
            Self::ControlTokenTooLong => "control token exceeds the allowed length",
            Self::InvalidInstanceSlug => "invalid canonical instance slug",
            Self::InvalidUploadId => "invalid upload id",
            Self::InvalidMemoryPath => "invalid canonical memory path",
            Self::ExpiryNotFuture => "capability expiry must be in the future",
            Self::ExpiryTooFar => "capability expiry exceeds the maximum lifetime",
            Self::ReplayCapacity => "capability replay store is full",
            Self::NonceCollision => "capability nonce collision",
            Self::NonceUnavailable => "secure capability nonce unavailable",
            Self::MalformedToken => "malformed capability token",
            Self::InvalidMac => "invalid capability authentication",
            Self::UnsupportedVersion => "unsupported capability version",
            Self::WrongMethod => "capability method mismatch",
            Self::WrongAudience => "capability audience mismatch",
            Self::WrongInstance => "capability instance mismatch",
            Self::WrongResource => "capability resource mismatch",
            Self::Expired => "capability expired",
            Self::ReplayStateMissing => "capability replay state unavailable",
            Self::AlreadyConsumed => "single-use capability already consumed",
        })
    }
}

impl std::error::Error for CapabilityError {}

pub(crate) trait Clock: Send + Sync {
    fn now(&self) -> u64;
}

pub(crate) trait NonceSource: Send + Sync {
    fn nonce(&self) -> Result<[u8; NONCE_BYTES], CapabilityError>;
}

struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }
}

struct SecureNonce;

impl NonceSource for SecureNonce {
    fn nonce(&self) -> Result<[u8; NONCE_BYTES], CapabilityError> {
        let mut nonce = [0_u8; NONCE_BYTES];
        getrandom::fill(&mut nonce).map_err(|_| CapabilityError::NonceUnavailable)?;
        Ok(nonce)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CapabilityPayload {
    version: u8,
    instance_slug: String,
    resource: CapabilityResource,
    method: CapabilityMethod,
    audience: CapabilityAudience,
    expires_at: u64,
    nonce: String,
    replay: ReplayPolicy,
}

struct ReplayEntry {
    expires_at: u64,
    consumed: bool,
}

struct ReplayStore {
    entries: HashMap<[u8; NONCE_BYTES], ReplayEntry>,
    capacity: usize,
}

impl ReplayStore {
    fn cleanup(&mut self, now: u64) {
        self.entries.retain(|_, entry| entry.expires_at > now);
    }

    fn reserve(
        &mut self,
        nonce: [u8; NONCE_BYTES],
        expires_at: u64,
        now: u64,
    ) -> Result<(), CapabilityError> {
        self.cleanup(now);
        if self.entries.contains_key(&nonce) {
            return Err(CapabilityError::NonceCollision);
        }
        if self.entries.len() >= self.capacity {
            return Err(CapabilityError::ReplayCapacity);
        }
        self.entries.insert(
            nonce,
            ReplayEntry {
                expires_at,
                consumed: false,
            },
        );
        Ok(())
    }
}

pub(crate) struct CapabilityService {
    signing_key: [u8; 32],
    clock: Arc<dyn Clock>,
    nonce_source: Arc<dyn NonceSource>,
    replay: Mutex<ReplayStore>,
}

impl CapabilityService {
    #[allow(dead_code)]
    pub(crate) fn new(
        control_token: &str,
        replay_capacity: usize,
    ) -> Result<Self, CapabilityError> {
        Self::with_sources(
            control_token,
            Arc::new(SystemClock),
            Arc::new(SecureNonce),
            replay_capacity,
        )
    }

    pub(crate) fn with_sources(
        control_token: &str,
        clock: Arc<dyn Clock>,
        nonce_source: Arc<dyn NonceSource>,
        replay_capacity: usize,
    ) -> Result<Self, CapabilityError> {
        if control_token.is_empty() {
            return Err(CapabilityError::EmptyControlToken);
        }
        if control_token.len() > MAX_CONTROL_TOKEN_BYTES {
            return Err(CapabilityError::ControlTokenTooLong);
        }
        if replay_capacity == 0 || replay_capacity > MAX_REPLAY_ENTRIES {
            return Err(CapabilityError::ReplayCapacity);
        }

        let mut derivation = HmacSha256::new_from_slice(control_token.as_bytes())
            .expect("HMAC accepts keys of any length");
        derivation.update(KEY_DERIVATION_DOMAIN);
        let signing_key: [u8; 32] = derivation.finalize().into_bytes().into();

        Ok(Self {
            signing_key,
            clock,
            nonce_source,
            replay: Mutex::new(ReplayStore {
                entries: HashMap::new(),
                capacity: replay_capacity,
            }),
        })
    }

    pub(crate) fn mint(&self, grant: CapabilityGrant) -> Result<CapabilityToken, CapabilityError> {
        validate_instance_slug(&grant.instance_slug)?;
        grant.resource.validate()?;
        let now = self.clock.now();
        if grant.expires_at <= now {
            return Err(CapabilityError::ExpiryNotFuture);
        }
        if grant.expires_at - now > MAX_TTL_SECONDS {
            return Err(CapabilityError::ExpiryTooFar);
        }

        let nonce = self.nonce_source.nonce()?;
        let payload = CapabilityPayload {
            version: CAPABILITY_VERSION,
            instance_slug: grant.instance_slug,
            resource: grant.resource,
            method: grant.method,
            audience: grant.audience,
            expires_at: grant.expires_at,
            nonce: URL_SAFE_NO_PAD.encode(nonce),
            replay: grant.replay,
        };
        let payload = serde_json::to_vec(&payload).map_err(|_| CapabilityError::MalformedToken)?;
        if payload.len() > MAX_PAYLOAD_BYTES {
            return Err(CapabilityError::MalformedToken);
        }
        let encoded_payload = URL_SAFE_NO_PAD.encode(payload);
        let authenticated = format!("{TOKEN_HEADER}.{encoded_payload}");
        let mac = self.mac(authenticated.as_bytes());
        let token = CapabilityToken(format!("{authenticated}.{}", URL_SAFE_NO_PAD.encode(mac)));
        if token.as_str().len() > MAX_TOKEN_BYTES {
            return Err(CapabilityError::MalformedToken);
        }

        // Nothing after this reservation can fail: successful reservation publishes the
        // already-complete token atomically enough for the in-memory single-use contract.
        if grant.replay == ReplayPolicy::SingleUse {
            self.replay
                .lock()
                .map_err(|_| CapabilityError::ReplayCapacity)?
                .reserve(nonce, grant.expires_at, now)?;
        }
        Ok(token)
    }

    pub(crate) fn verify(
        &self,
        token: &str,
        target: &CapabilityTarget,
    ) -> Result<VerifiedCapability, CapabilityError> {
        self.verify_request_method(token, target, "GET")
    }

    pub(crate) fn verify_request_method(
        &self,
        token: &str,
        target: &CapabilityTarget,
        request_method: &str,
    ) -> Result<VerifiedCapability, CapabilityError> {
        let (header, encoded_payload, encoded_mac) = split_token(token)?;
        let payload_bytes = decode_canonical(encoded_payload, MAX_PAYLOAD_BYTES)?;
        let mac_bytes = decode_canonical(encoded_mac, 32)?;
        if mac_bytes.len() != 32 {
            return Err(CapabilityError::MalformedToken);
        }

        // Authenticate the opaque envelope before parsing or trusting payload fields.
        let authenticated = format!("{header}.{encoded_payload}");
        let mut verifier =
            HmacSha256::new_from_slice(&self.signing_key).expect("HMAC accepts a 32-byte key");
        verifier.update(SIGNING_DOMAIN);
        verifier.update(authenticated.as_bytes());
        verifier
            .verify_slice(&mac_bytes)
            .map_err(|_| CapabilityError::InvalidMac)?;

        if header != TOKEN_HEADER {
            return Err(CapabilityError::UnsupportedVersion);
        }
        let payload: CapabilityPayload =
            serde_json::from_slice(&payload_bytes).map_err(|_| CapabilityError::MalformedToken)?;
        if payload.version != CAPABILITY_VERSION {
            return Err(CapabilityError::UnsupportedVersion);
        }
        if request_method != "GET" || payload.method != target.method {
            return Err(CapabilityError::WrongMethod);
        }
        if payload.audience != target.audience {
            return Err(CapabilityError::WrongAudience);
        }
        validate_instance_slug(&payload.instance_slug)?;
        if payload.instance_slug != target.instance_slug {
            return Err(CapabilityError::WrongInstance);
        }
        payload.resource.validate()?;
        if payload.resource != target.resource {
            return Err(CapabilityError::WrongResource);
        }

        let now = self.clock.now();
        if now >= payload.expires_at {
            return Err(CapabilityError::Expired);
        }

        if payload.replay == ReplayPolicy::ReusableWithinExpiry {
            return Ok(VerifiedCapability {
                version: payload.version,
                expires_at: payload.expires_at,
                replay: payload.replay,
            });
        }

        let nonce = decode_nonce(&payload.nonce)?;

        let mut replay = self
            .replay
            .lock()
            .map_err(|_| CapabilityError::ReplayCapacity)?;
        replay.cleanup(now);
        let entry = replay
            .entries
            .get_mut(&nonce)
            .ok_or(CapabilityError::ReplayStateMissing)?;
        if entry.expires_at != payload.expires_at {
            return Err(CapabilityError::ReplayStateMissing);
        }
        if entry.consumed {
            return Err(CapabilityError::AlreadyConsumed);
        }
        entry.consumed = true;

        Ok(VerifiedCapability {
            version: payload.version,
            expires_at: payload.expires_at,
            replay: payload.replay,
        })
    }

    fn mac(&self, authenticated: &[u8]) -> [u8; 32] {
        let mut signer =
            HmacSha256::new_from_slice(&self.signing_key).expect("HMAC accepts a 32-byte key");
        signer.update(SIGNING_DOMAIN);
        signer.update(authenticated);
        signer.finalize().into_bytes().into()
    }
}

fn split_token(token: &str) -> Result<(&str, &str, &str), CapabilityError> {
    if token.is_empty() || token.len() > MAX_TOKEN_BYTES || !token.is_ascii() {
        return Err(CapabilityError::MalformedToken);
    }
    let mut segments = token.split('.');
    let header = segments.next().ok_or(CapabilityError::MalformedToken)?;
    let payload = segments.next().ok_or(CapabilityError::MalformedToken)?;
    let mac = segments.next().ok_or(CapabilityError::MalformedToken)?;
    if segments.next().is_some()
        || header.is_empty()
        || header.len() > 8
        || payload.is_empty()
        || mac.is_empty()
    {
        return Err(CapabilityError::MalformedToken);
    }
    Ok((header, payload, mac))
}

fn decode_canonical(value: &str, max_decoded: usize) -> Result<Vec<u8>, CapabilityError> {
    if value.contains('=') || !value.bytes().all(is_base64url_byte) {
        return Err(CapabilityError::MalformedToken);
    }
    let max_encoded = max_decoded.saturating_add(2) / 3 * 4;
    if value.len() > max_encoded {
        return Err(CapabilityError::MalformedToken);
    }
    let decoded = URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|_| CapabilityError::MalformedToken)?;
    if decoded.len() > max_decoded || URL_SAFE_NO_PAD.encode(&decoded) != value {
        return Err(CapabilityError::MalformedToken);
    }
    Ok(decoded)
}

fn decode_nonce(encoded: &str) -> Result<[u8; NONCE_BYTES], CapabilityError> {
    let bytes = decode_canonical(encoded, NONCE_BYTES)?;
    bytes
        .try_into()
        .map_err(|_| CapabilityError::MalformedToken)
}

fn is_base64url_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')
}

fn validate_instance_slug(slug: &str) -> Result<(), CapabilityError> {
    if slug.is_empty()
        || slug.len() > MAX_INSTANCE_BYTES
        || !slug
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(CapabilityError::InvalidInstanceSlug);
    }
    Ok(())
}

fn validate_single_component(value: &str, error: CapabilityError) -> Result<(), CapabilityError> {
    if value.is_empty()
        || value.len() > 255
        || matches!(value, "." | "..")
        || value.contains(['/', '\\', '\0'])
        || value.chars().any(char::is_control)
    {
        return Err(error);
    }
    Ok(())
}

fn validate_memory_path(path: &str) -> Result<(), CapabilityError> {
    let bytes = path.as_bytes();
    let has_windows_prefix = bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
    if path.is_empty()
        || path.len() > MAX_RESOURCE_BYTES
        || path.nfc().ne(path.chars())
        || path.starts_with('/')
        || path.ends_with('/')
        || has_windows_prefix
        || path.split('/').any(invalid_memory_component)
    {
        return Err(CapabilityError::InvalidMemoryPath);
    }
    Ok(())
}

fn invalid_memory_component(component: &str) -> bool {
    if validate_single_component(component, CapabilityError::InvalidMemoryPath).is_err()
        || component.ends_with(['.', ' '])
        || component
            .chars()
            .any(|character| matches!(character, '<' | '>' | ':' | '"' | '|' | '?' | '*'))
    {
        return true;
    }

    let basename = component.split('.').next().unwrap_or(component);
    let uppercase = basename.to_ascii_uppercase();
    matches!(uppercase.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || uppercase.strip_prefix("COM").is_some_and(|number| {
            matches!(number, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
        })
        || uppercase.strip_prefix("LPT").is_some_and(|number| {
            matches!(number, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
        })
}

fn percent_encode_component(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push('%');
            encoded.push(char::from(HEX[(byte >> 4) as usize]));
            encoded.push(char::from(HEX[(byte & 0x0f) as usize]));
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    };

    struct FixedClock(u64);

    impl Clock for FixedClock {
        fn now(&self) -> u64 {
            self.0
        }
    }

    struct MutableClock(AtomicU64);

    impl MutableClock {
        fn new(now: u64) -> Self {
            Self(AtomicU64::new(now))
        }

        fn set(&self, now: u64) {
            self.0.store(now, Ordering::SeqCst);
        }
    }

    impl Clock for MutableClock {
        fn now(&self) -> u64 {
            self.0.load(Ordering::SeqCst)
        }
    }

    struct FixedNonce(Mutex<Vec<[u8; NONCE_BYTES]>>);

    impl FixedNonce {
        fn new(nonces: Vec<[u8; NONCE_BYTES]>) -> Self {
            Self(Mutex::new(nonces.into_iter().rev().collect()))
        }
    }

    impl NonceSource for FixedNonce {
        fn nonce(&self) -> Result<[u8; NONCE_BYTES], CapabilityError> {
            self.0
                .lock()
                .unwrap()
                .pop()
                .ok_or(CapabilityError::NonceUnavailable)
        }
    }

    #[test]
    fn round_trip_exact_root_memory_resource() {
        let service = CapabilityService::with_sources(
            "server-control-secret",
            Arc::new(FixedClock(1_000)),
            Arc::new(FixedNonce::new(vec![[7; NONCE_BYTES]])),
            16,
        )
        .unwrap();
        let resource = CapabilityResource::memory("photo one.png").unwrap();
        let grant = CapabilityGrant {
            instance_slug: "little-moon".to_owned(),
            resource: resource.clone(),
            method: CapabilityMethod::Get,
            audience: CapabilityAudience::ModelProvider,
            expires_at: 1_060,
            replay: ReplayPolicy::ReusableWithinExpiry,
        };

        let token = service.mint(grant).unwrap();
        let target = CapabilityTarget::new(
            "little-moon",
            resource,
            CapabilityMethod::Get,
            CapabilityAudience::ModelProvider,
        )
        .unwrap();
        let verified = service.verify(token.as_str(), &target).unwrap();

        assert_eq!(verified.version, CAPABILITY_VERSION);
        assert_eq!(verified.expires_at, 1_060);
        assert_eq!(verified.replay, ReplayPolicy::ReusableWithinExpiry);
        assert!(
            token
                .as_str()
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        );
    }

    #[test]
    fn authenticated_get_capability_rejects_non_get_request_method() {
        let service = CapabilityService::with_sources(
            "server-control-secret",
            Arc::new(FixedClock(1_000)),
            Arc::new(FixedNonce::new(vec![[8; NONCE_BYTES]])),
            16,
        )
        .unwrap();
        let resource = CapabilityResource::uploaded_file("upload_123").unwrap();
        let token = service
            .mint(CapabilityGrant {
                instance_slug: "little-moon".to_owned(),
                resource: resource.clone(),
                method: CapabilityMethod::Get,
                audience: CapabilityAudience::Browser,
                expires_at: 1_060,
                replay: ReplayPolicy::ReusableWithinExpiry,
            })
            .unwrap();
        let target = CapabilityTarget::new(
            "little-moon",
            resource,
            CapabilityMethod::Get,
            CapabilityAudience::Browser,
        )
        .unwrap();

        assert!(matches!(
            service.verify_request_method(token.as_str(), &target, "POST"),
            Err(CapabilityError::WrongMethod)
        ));
    }

    #[test]
    fn nested_memory_and_upload_resources_round_trip_with_canonical_url_encoding() {
        let service = CapabilityService::with_sources(
            "server-control-secret",
            Arc::new(FixedClock(2_000)),
            Arc::new(FixedNonce::new(vec![[9; NONCE_BYTES], [10; NONCE_BYTES]])),
            16,
        )
        .unwrap();
        let memory = CapabilityResource::memory("旅行 notes/cover #1%.png").unwrap();
        assert_eq!(
            memory.encoded_path(),
            "%E6%97%85%E8%A1%8C%20notes/cover%20%231%25.png"
        );
        let upload = CapabilityResource::uploaded_file("upload id#1").unwrap();
        assert_eq!(upload.encoded_path(), "upload%20id%231");

        for (resource, audience) in [
            (memory, CapabilityAudience::ModelProvider),
            (upload, CapabilityAudience::NativeRelay),
        ] {
            let token = service
                .mint(CapabilityGrant {
                    instance_slug: "moon_1".to_owned(),
                    resource: resource.clone(),
                    method: CapabilityMethod::Get,
                    audience,
                    expires_at: 2_100,
                    replay: ReplayPolicy::ReusableWithinExpiry,
                })
                .unwrap();
            let target =
                CapabilityTarget::new("moon_1", resource, CapabilityMethod::Get, audience).unwrap();
            assert!(service.verify(token.as_str(), &target).is_ok());
        }
    }

    #[test]
    fn canonical_identifiers_reject_path_ambiguity_and_bounds() {
        for path in [
            "",
            "/absolute",
            "trailing/",
            "double//slash",
            "./dot",
            "a/../parent",
            "back\\slash",
            "nul\0byte",
            "line\nbreak",
            "C:/drive",
        ] {
            assert!(
                matches!(
                    CapabilityResource::memory(path),
                    Err(CapabilityError::InvalidMemoryPath)
                ),
                "accepted {path:?}"
            );
        }
        assert!(matches!(
            CapabilityResource::memory("a".repeat(MAX_RESOURCE_BYTES + 1)),
            Err(CapabilityError::InvalidMemoryPath)
        ));
        for id in ["", ".", "..", "a/b", "a\\b", "a\0b"] {
            assert!(matches!(
                CapabilityResource::uploaded_file(id),
                Err(CapabilityError::InvalidUploadId)
            ));
        }
        for slug in ["", ".", "..", " moon", "moon ", "moon/path", "møøn"] {
            assert!(
                matches!(
                    CapabilityTarget::new(
                        slug,
                        CapabilityResource::memory("ok.md").unwrap(),
                        CapabilityMethod::Get,
                        CapabilityAudience::Browser,
                    ),
                    Err(CapabilityError::InvalidInstanceSlug)
                ),
                "accepted {slug:?}"
            );
        }
    }

    #[test]
    fn memory_paths_enforce_platform_independent_canonical_spelling() {
        for path in [
            "Cafe\u{301}.md",
            "CON",
            "con.txt",
            "PRN.log",
            "aux",
            "NUL.bin",
            "COM1",
            "com9.json",
            "LPT1",
            "lpt9.txt",
            "folder/trailing.",
            "folder/trailing ",
            "bad<name",
            "bad>name",
            "bad:name",
            "bad\"name",
            "bad|name",
            "bad?name",
            "bad*name",
        ] {
            assert!(
                matches!(
                    CapabilityResource::memory(path),
                    Err(CapabilityError::InvalidMemoryPath)
                ),
                "accepted {path:?}"
            );
        }

        for path in [
            "Café.md",
            "Folder/UPPER CASE.txt",
            "concierge.txt",
            "COM10.txt",
            "résumé 100%.md",
        ] {
            assert!(
                CapabilityResource::memory(path).is_ok(),
                "rejected {path:?}"
            );
        }
    }

    #[test]
    fn percent_encoding_is_canonical_and_path_case_is_signed_exactly() {
        let service = CapabilityService::with_sources(
            "server-control-secret",
            Arc::new(FixedClock(2_100)),
            Arc::new(FixedNonce::new(vec![[30; NONCE_BYTES]])),
            1,
        )
        .unwrap();
        let resource = CapabilityResource::memory("Folder/Café 100%.md").unwrap();
        assert_eq!(resource.encoded_path(), "Folder/Caf%C3%A9%20100%25.md");
        let token = service
            .mint(CapabilityGrant {
                instance_slug: "moon".to_owned(),
                resource: resource.clone(),
                method: CapabilityMethod::Get,
                audience: CapabilityAudience::Browser,
                expires_at: 2_200,
                replay: ReplayPolicy::ReusableWithinExpiry,
            })
            .unwrap();
        let exact = CapabilityTarget::new(
            "moon",
            resource,
            CapabilityMethod::Get,
            CapabilityAudience::Browser,
        )
        .unwrap();
        let case_variant = CapabilityTarget::new(
            "moon",
            CapabilityResource::memory("folder/Café 100%.md").unwrap(),
            CapabilityMethod::Get,
            CapabilityAudience::Browser,
        )
        .unwrap();

        assert!(service.verify(token.as_str(), &exact).is_ok());
        assert!(matches!(
            service.verify(token.as_str(), &case_variant),
            Err(CapabilityError::WrongResource)
        ));
    }

    #[test]
    fn tampering_any_token_segment_fails_authentication() {
        let service = CapabilityService::with_sources(
            "server-control-secret",
            Arc::new(FixedClock(3_000)),
            Arc::new(FixedNonce::new(vec![[11; NONCE_BYTES]])),
            16,
        )
        .unwrap();
        let resource = CapabilityResource::memory("nested/file.png").unwrap();
        let token = service
            .mint(CapabilityGrant {
                instance_slug: "moon".to_owned(),
                resource: resource.clone(),
                method: CapabilityMethod::Get,
                audience: CapabilityAudience::ModelProvider,
                expires_at: 3_100,
                replay: ReplayPolicy::ReusableWithinExpiry,
            })
            .unwrap();
        let target = CapabilityTarget::new(
            "moon",
            resource,
            CapabilityMethod::Get,
            CapabilityAudience::ModelProvider,
        )
        .unwrap();
        let segments: Vec<_> = token.as_str().split('.').collect();

        for index in 0..3 {
            let mut changed: Vec<String> = segments
                .iter()
                .map(|segment| (*segment).to_owned())
                .collect();
            let replacement = if changed[index].starts_with('A') {
                'B'
            } else {
                'A'
            };
            changed[index].replace_range(0..1, &replacement.to_string());
            assert!(matches!(
                service.verify(&changed.join("."), &target),
                Err(CapabilityError::InvalidMac)
            ));
        }
    }

    #[test]
    fn exact_key_audience_instance_and_typed_resource_are_required() {
        let service = CapabilityService::with_sources(
            "correct-key",
            Arc::new(FixedClock(4_000)),
            Arc::new(FixedNonce::new(vec![[12; NONCE_BYTES]])),
            16,
        )
        .unwrap();
        let resource = CapabilityResource::uploaded_file("same-name").unwrap();
        let token = service
            .mint(CapabilityGrant {
                instance_slug: "moon".to_owned(),
                resource: resource.clone(),
                method: CapabilityMethod::Get,
                audience: CapabilityAudience::Browser,
                expires_at: 4_100,
                replay: ReplayPolicy::ReusableWithinExpiry,
            })
            .unwrap();
        let target = |instance, resource, audience| {
            CapabilityTarget::new(instance, resource, CapabilityMethod::Get, audience).unwrap()
        };

        assert!(matches!(
            service.verify(
                token.as_str(),
                &target("moon", resource.clone(), CapabilityAudience::ModelProvider)
            ),
            Err(CapabilityError::WrongAudience)
        ));
        assert!(matches!(
            service.verify(
                token.as_str(),
                &target("other", resource.clone(), CapabilityAudience::Browser)
            ),
            Err(CapabilityError::WrongInstance)
        ));
        assert!(matches!(
            service.verify(
                token.as_str(),
                &target(
                    "moon",
                    CapabilityResource::uploaded_file("other").unwrap(),
                    CapabilityAudience::Browser,
                )
            ),
            Err(CapabilityError::WrongResource)
        ));
        assert!(matches!(
            service.verify(
                token.as_str(),
                &target(
                    "moon",
                    CapabilityResource::memory("same-name").unwrap(),
                    CapabilityAudience::Browser,
                )
            ),
            Err(CapabilityError::WrongResource)
        ));

        let wrong_key = CapabilityService::with_sources(
            "wrong-key",
            Arc::new(FixedClock(4_000)),
            Arc::new(FixedNonce::new(vec![])),
            16,
        )
        .unwrap();
        assert!(matches!(
            wrong_key.verify(
                token.as_str(),
                &target("moon", resource, CapabilityAudience::Browser)
            ),
            Err(CapabilityError::InvalidMac)
        ));
    }

    #[test]
    fn expiry_is_exclusive_and_lifetime_is_bounded() {
        let clock = Arc::new(MutableClock::new(5_000));
        let service = CapabilityService::with_sources(
            "server-control-secret",
            clock.clone(),
            Arc::new(FixedNonce::new(vec![[13; NONCE_BYTES]])),
            16,
        )
        .unwrap();
        let resource = CapabilityResource::memory("file.md").unwrap();
        let target = CapabilityTarget::new(
            "moon",
            resource.clone(),
            CapabilityMethod::Get,
            CapabilityAudience::Browser,
        )
        .unwrap();
        let token = service
            .mint(CapabilityGrant {
                instance_slug: "moon".to_owned(),
                resource: resource.clone(),
                method: CapabilityMethod::Get,
                audience: CapabilityAudience::Browser,
                expires_at: 5_001,
                replay: ReplayPolicy::ReusableWithinExpiry,
            })
            .unwrap();
        assert!(service.verify(token.as_str(), &target).is_ok());
        clock.set(5_001);
        assert!(matches!(
            service.verify(token.as_str(), &target),
            Err(CapabilityError::Expired)
        ));

        for (expiry, expected) in [
            (5_001, CapabilityError::ExpiryNotFuture),
            (5_001 + MAX_TTL_SECONDS + 1, CapabilityError::ExpiryTooFar),
        ] {
            assert!(matches!(
                service.mint(CapabilityGrant {
                    instance_slug: "moon".to_owned(),
                    resource: resource.clone(),
                    method: CapabilityMethod::Get,
                    audience: CapabilityAudience::Browser,
                    expires_at: expiry,
                    replay: ReplayPolicy::ReusableWithinExpiry,
                }),
                Err(error) if error == expected
            ));
        }
    }

    #[test]
    fn replay_policy_allows_provider_retry_but_consumes_single_use_once() {
        let service = CapabilityService::with_sources(
            "server-control-secret",
            Arc::new(FixedClock(6_000)),
            Arc::new(FixedNonce::new(vec![[14; NONCE_BYTES], [15; NONCE_BYTES]])),
            16,
        )
        .unwrap();
        let resource = CapabilityResource::memory("file.md").unwrap();
        let target = CapabilityTarget::new(
            "moon",
            resource.clone(),
            CapabilityMethod::Get,
            CapabilityAudience::ModelProvider,
        )
        .unwrap();

        for (policy, second_ok) in [
            (ReplayPolicy::ReusableWithinExpiry, true),
            (ReplayPolicy::SingleUse, false),
        ] {
            let token = service
                .mint(CapabilityGrant {
                    instance_slug: "moon".to_owned(),
                    resource: resource.clone(),
                    method: CapabilityMethod::Get,
                    audience: CapabilityAudience::ModelProvider,
                    expires_at: 6_100,
                    replay: policy,
                })
                .unwrap();
            assert!(service.verify(token.as_str(), &target).is_ok());
            let second = service.verify(token.as_str(), &target);
            assert_eq!(second.is_ok(), second_ok);
            if !second_ok {
                assert!(matches!(second, Err(CapabilityError::AlreadyConsumed)));
            }
        }
    }

    #[test]
    fn reusable_capability_is_stateless_across_service_recreation() {
        let first = CapabilityService::with_sources(
            "server-control-secret",
            Arc::new(FixedClock(6_100)),
            Arc::new(FixedNonce::new(vec![[31; NONCE_BYTES], [32; NONCE_BYTES]])),
            1,
        )
        .unwrap();
        let resource = CapabilityResource::memory("Folder/Résumé 1%.md").unwrap();
        let target = CapabilityTarget::new(
            "moon",
            resource.clone(),
            CapabilityMethod::Get,
            CapabilityAudience::Browser,
        )
        .unwrap();
        let reusable = first
            .mint(CapabilityGrant {
                instance_slug: "moon".to_owned(),
                resource: resource.clone(),
                method: CapabilityMethod::Get,
                audience: CapabilityAudience::Browser,
                expires_at: 6_200,
                replay: ReplayPolicy::ReusableWithinExpiry,
            })
            .unwrap();
        let single_use = first
            .mint(CapabilityGrant {
                instance_slug: "moon".to_owned(),
                resource: resource.clone(),
                method: CapabilityMethod::Get,
                audience: CapabilityAudience::Browser,
                expires_at: 6_200,
                replay: ReplayPolicy::SingleUse,
            })
            .unwrap();

        let recreated = CapabilityService::with_sources(
            "server-control-secret",
            Arc::new(FixedClock(6_100)),
            Arc::new(FixedNonce::new(vec![])),
            1,
        )
        .unwrap();
        assert!(recreated.verify(reusable.as_str(), &target).is_ok());
        assert!(matches!(
            recreated.verify(single_use.as_str(), &target),
            Err(CapabilityError::ReplayStateMissing)
        ));
    }

    #[test]
    fn full_single_use_store_does_not_block_reusable_capabilities() {
        let service = CapabilityService::with_sources(
            "server-control-secret",
            Arc::new(FixedClock(6_200)),
            Arc::new(FixedNonce::new(vec![[33; NONCE_BYTES], [34; NONCE_BYTES]])),
            1,
        )
        .unwrap();
        let resource = CapabilityResource::memory("file.md").unwrap();
        let grant = |replay| CapabilityGrant {
            instance_slug: "moon".to_owned(),
            resource: resource.clone(),
            method: CapabilityMethod::Get,
            audience: CapabilityAudience::Browser,
            expires_at: 6_300,
            replay,
        };

        service.mint(grant(ReplayPolicy::SingleUse)).unwrap();
        let reusable = service
            .mint(grant(ReplayPolicy::ReusableWithinExpiry))
            .unwrap();
        let target = CapabilityTarget::new(
            "moon",
            resource,
            CapabilityMethod::Get,
            CapabilityAudience::Browser,
        )
        .unwrap();
        assert!(service.verify(reusable.as_str(), &target).is_ok());
        assert!(service.verify(reusable.as_str(), &target).is_ok());
    }

    #[test]
    fn failed_oversized_mints_never_reserve_single_use_capacity() {
        let service = CapabilityService::with_sources(
            "server-control-secret",
            Arc::new(FixedClock(6_300)),
            Arc::new(FixedNonce::new(vec![
                [35; NONCE_BYTES],
                [36; NONCE_BYTES],
                [37; NONCE_BYTES],
            ])),
            1,
        )
        .unwrap();
        let quote_heavy_component = "\"".repeat(255);
        let almost_full_component = "\"".repeat(254);
        let invalid_resource = CapabilityResource::MemoryPath {
            path: format!(
                "{0}/{0}/{0}/{1}/x",
                quote_heavy_component, almost_full_component
            ),
        };
        assert_eq!(
            match &invalid_resource {
                CapabilityResource::MemoryPath { path } => path.len(),
                CapabilityResource::UploadedFile { .. } => unreachable!(),
            },
            MAX_RESOURCE_BYTES
        );
        let grant = |resource| CapabilityGrant {
            instance_slug: "moon".to_owned(),
            resource,
            method: CapabilityMethod::Get,
            audience: CapabilityAudience::Browser,
            expires_at: 6_400,
            replay: ReplayPolicy::SingleUse,
        };

        for _ in 0..2 {
            assert!(matches!(
                service.mint(grant(invalid_resource.clone())),
                Err(CapabilityError::InvalidMemoryPath | CapabilityError::MalformedToken)
            ));
        }
        assert!(
            service
                .mint(grant(CapabilityResource::memory("valid.md").unwrap()))
                .is_ok()
        );
    }

    #[test]
    fn nonce_collisions_and_full_store_fail_closed_then_expired_entries_cleanup() {
        let clock = Arc::new(MutableClock::new(7_000));
        let service = CapabilityService::with_sources(
            "server-control-secret",
            clock.clone(),
            Arc::new(FixedNonce::new(vec![
                [16; NONCE_BYTES],
                [16; NONCE_BYTES],
                [17; NONCE_BYTES],
                [18; NONCE_BYTES],
            ])),
            1,
        )
        .unwrap();
        let grant = |expiry| CapabilityGrant {
            instance_slug: "moon".to_owned(),
            resource: CapabilityResource::memory("file.md").unwrap(),
            method: CapabilityMethod::Get,
            audience: CapabilityAudience::Browser,
            expires_at: expiry,
            replay: ReplayPolicy::SingleUse,
        };

        service.mint(grant(7_001)).unwrap();
        assert!(matches!(
            service.mint(grant(7_001)),
            Err(CapabilityError::NonceCollision)
        ));
        assert!(matches!(
            service.mint(grant(7_001)),
            Err(CapabilityError::ReplayCapacity)
        ));
        clock.set(7_001);
        assert!(service.mint(grant(7_002)).is_ok());
    }

    #[test]
    fn malformed_oversized_tokens_empty_keys_and_secret_debug_are_safe() {
        assert!(matches!(
            CapabilityService::new("", 16),
            Err(CapabilityError::EmptyControlToken)
        ));
        assert!(matches!(
            CapabilityService::new(&"x".repeat(MAX_CONTROL_TOKEN_BYTES + 1), 16),
            Err(CapabilityError::ControlTokenTooLong)
        ));
        assert!(matches!(
            CapabilityService::new("key", MAX_REPLAY_ENTRIES + 1),
            Err(CapabilityError::ReplayCapacity)
        ));
        let service = CapabilityService::with_sources(
            "server-control-secret",
            Arc::new(FixedClock(8_000)),
            Arc::new(FixedNonce::new(vec![[19; NONCE_BYTES]])),
            16,
        )
        .unwrap();
        let resource = CapabilityResource::memory("file.md").unwrap();
        let target = CapabilityTarget::new(
            "moon",
            resource.clone(),
            CapabilityMethod::Get,
            CapabilityAudience::Browser,
        )
        .unwrap();
        for malformed in ["", "one", "a.b", "a.b.c.d", "v1.===.AA", "é.aa.bb"] {
            assert!(matches!(
                service.verify(malformed, &target),
                Err(CapabilityError::MalformedToken)
            ));
        }
        assert!(matches!(
            service.verify(&"A".repeat(MAX_TOKEN_BYTES + 1), &target),
            Err(CapabilityError::MalformedToken)
        ));

        let token = service
            .mint(CapabilityGrant {
                instance_slug: "moon".to_owned(),
                resource,
                method: CapabilityMethod::Get,
                audience: CapabilityAudience::Browser,
                expires_at: 8_100,
                replay: ReplayPolicy::ReusableWithinExpiry,
            })
            .unwrap();
        let debug = format!("{token:?}");
        assert_eq!(debug, "CapabilityToken([REDACTED])");
        assert!(!debug.contains(token.as_str()));
    }

    #[test]
    fn minted_token_never_serializes_raw_server_control_token() {
        let control_token = "raw-server-token-sentinel-DO-NOT-SERIALIZE";
        let service = CapabilityService::with_sources(
            control_token,
            Arc::new(FixedClock(9_000)),
            Arc::new(FixedNonce::new(vec![[20; NONCE_BYTES]])),
            16,
        )
        .unwrap();
        let token = service
            .mint(CapabilityGrant {
                instance_slug: "moon".to_owned(),
                resource: CapabilityResource::memory("file.md").unwrap(),
                method: CapabilityMethod::Get,
                audience: CapabilityAudience::Browser,
                expires_at: 9_100,
                replay: ReplayPolicy::ReusableWithinExpiry,
            })
            .unwrap();
        let encoded_payload = token.as_str().split('.').nth(1).unwrap();
        let payload = URL_SAFE_NO_PAD.decode(encoded_payload).unwrap();

        assert!(!token.as_str().contains(control_token));
        assert!(
            !payload
                .windows(control_token.len())
                .any(|window| window == control_token.as_bytes())
        );
    }

    #[test]
    fn authenticated_unknown_envelope_and_payload_versions_fail_closed() {
        let service = CapabilityService::with_sources(
            "server-control-secret",
            Arc::new(FixedClock(10_000)),
            Arc::new(FixedNonce::new(vec![[21; NONCE_BYTES]])),
            16,
        )
        .unwrap();
        let resource = CapabilityResource::memory("file.md").unwrap();
        let token = service
            .mint(CapabilityGrant {
                instance_slug: "moon".to_owned(),
                resource: resource.clone(),
                method: CapabilityMethod::Get,
                audience: CapabilityAudience::Browser,
                expires_at: 10_100,
                replay: ReplayPolicy::ReusableWithinExpiry,
            })
            .unwrap();
        let target = CapabilityTarget::new(
            "moon",
            resource,
            CapabilityMethod::Get,
            CapabilityAudience::Browser,
        )
        .unwrap();
        let segments: Vec<_> = token.as_str().split('.').collect();
        let sign = |header: &str, payload: &str| {
            let authenticated = format!("{header}.{payload}");
            format!(
                "{authenticated}.{}",
                URL_SAFE_NO_PAD.encode(service.mac(authenticated.as_bytes()))
            )
        };

        assert!(matches!(
            service.verify(&sign("v2", segments[1]), &target),
            Err(CapabilityError::UnsupportedVersion)
        ));

        let mut payload: serde_json::Value =
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(segments[1]).unwrap()).unwrap();
        payload["version"] = serde_json::json!(2);
        let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&payload).unwrap());
        assert!(matches!(
            service.verify(&sign(TOKEN_HEADER, &payload), &target),
            Err(CapabilityError::UnsupportedVersion)
        ));
    }
}
