//! Shared, replaceable capability authority. Cloned producers observe rotations.
use super::resource_capability::*;
use sha2::{Digest, Sha256};
use std::sync::{Arc, RwLock};

#[derive(Clone)]
pub(crate) struct ResourceAccess(Arc<RwLock<Option<CapabilityService>>>);

impl ResourceAccess {
    pub(crate) fn new(token: &str) -> Self {
        let access = Self(Arc::new(RwLock::new(None)));
        access.replace(token);
        access
    }

    pub(crate) fn replace(&self, token: &str) {
        super::tools::register_control_secret(token);
        // A fresh generation also invalidates reusable grants when reloading the same token.
        let service = if token.is_empty() {
            None
        } else {
            let mut key = Sha256::new();
            key.update(b"nolune/resource-generation/v1");
            key.update(uuid::Uuid::new_v4().as_bytes());
            key.update(token.as_bytes());
            Some(
                CapabilityService::new(&format!("{:x}", key.finalize()), 65536)
                    .expect("fixed-size capability signing key"),
            )
        };
        *self.0.write().expect("capability lock") = service;
    }

    pub(crate) fn url(
        &self,
        base: &str,
        slug: &str,
        resource: CapabilityResource,
        audience: CapabilityAudience,
    ) -> Result<String, CapabilityError> {
        CapabilityTarget::new(slug, resource.clone(), CapabilityMethod::Get, audience)?;
        let path = resource_path(slug, &resource, audience);
        let guard = self.0.read().expect("capability lock");
        let Some(service) = guard.as_ref() else {
            return Ok(format!("{base}{path}"));
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let cap = service.mint(CapabilityGrant {
            instance_slug: slug.into(),
            resource,
            method: CapabilityMethod::Get,
            audience,
            expires_at: now + MAX_TTL_SECONDS,
            replay: ReplayPolicy::ReusableWithinExpiry,
        })?;
        super::tools::register_capability_token(cap.as_str());
        Ok(format!("{base}{path}?cap={}", cap.as_str()))
    }

    pub(crate) fn verify(
        &self,
        slug: &str,
        resource: CapabilityResource,
        audience: CapabilityAudience,
        uri: &axum::http::Uri,
        method: &str,
    ) -> Result<(), CapabilityError> {
        let target =
            CapabilityTarget::new(slug, resource.clone(), CapabilityMethod::Get, audience)?;
        if uri.path() != resource_path(slug, &resource, audience) || method != "GET" {
            return Err(CapabilityError::WrongResource);
        }
        let guard = self.0.read().expect("capability lock");
        if let Some(service) = guard.as_ref() {
            let cap = uri
                .query()
                .and_then(|q| q.strip_prefix("cap="))
                .ok_or(CapabilityError::MalformedToken)?;
            service.verify_request_method(cap, &target, method)?;
        } else if uri.query().is_some() {
            return Err(CapabilityError::MalformedToken);
        }
        Ok(())
    }
}

fn resource_path(
    slug: &str,
    resource: &CapabilityResource,
    audience: CapabilityAudience,
) -> String {
    let audience = match audience {
        CapabilityAudience::Browser => "browser",
        CapabilityAudience::ModelProvider => "model-provider",
        CapabilityAudience::NativeRelay => "native-relay",
    };
    let kind = match resource {
        CapabilityResource::UploadedFile { .. } => "files",
        CapabilityResource::MemoryPath { .. } => "memory",
    };
    format!(
        "/resources/{audience}/{kind}/{slug}/{}",
        resource.encoded_path()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn minted_capability_is_registered_for_exact_redaction_exemption() {
        let access = ResourceAccess::new("integration-control-secret");
        let url = access
            .url(
                "https://self.test",
                "moon",
                CapabilityResource::uploaded_file("id").unwrap(),
                CapabilityAudience::Browser,
            )
            .unwrap();
        let cap = url.split("cap=").nth(1).unwrap();
        let secret = &cap[cap.len() - 8..];
        super::super::tools::register_control_secret(secret);

        let text = format!("{url} forged=x{secret}x Bearer {secret}X");
        let redacted = super::super::tools::redact_secrets(&text);
        assert!(redacted.starts_with(&url));
        assert_eq!(redacted.matches(secret).count(), 1);
        assert_eq!(redacted.matches("[REDACTED]").count(), 2);
    }

    #[test]
    fn maximum_control_token_does_not_overflow_generation_key() {
        let access = ResourceAccess::new(&"x".repeat(4096));
        assert!(
            access
                .url(
                    "",
                    "moon",
                    CapabilityResource::uploaded_file("id").unwrap(),
                    CapabilityAudience::Browser
                )
                .is_ok()
        );
    }
    #[test]
    fn route_boundary_enforces_expiry_replay_and_canonical_identity() {
        use std::sync::atomic::{AtomicU64, Ordering};
        struct TestClock(AtomicU64);
        impl Clock for TestClock {
            fn now(&self) -> u64 {
                self.0.load(Ordering::SeqCst)
            }
        }
        struct Nonce;
        impl NonceSource for Nonce {
            fn nonce(&self) -> Result<[u8; NONCE_BYTES], CapabilityError> {
                Ok([4; NONCE_BYTES])
            }
        }
        let clock = Arc::new(TestClock(AtomicU64::new(100)));
        for replay in [ReplayPolicy::ReusableWithinExpiry, ReplayPolicy::SingleUse] {
            clock.0.store(100, Ordering::SeqCst);
            let service =
                CapabilityService::with_sources("secret", clock.clone(), Arc::new(Nonce), 8)
                    .unwrap();
            let resource = CapabilityResource::memory("Folder/Résumé 1%.png").unwrap();
            let cap = service
                .mint(CapabilityGrant {
                    instance_slug: "moon".into(),
                    resource: resource.clone(),
                    method: CapabilityMethod::Get,
                    audience: CapabilityAudience::ModelProvider,
                    expires_at: 110,
                    replay,
                })
                .unwrap();
            let access = ResourceAccess(Arc::new(RwLock::new(Some(service))));
            let path = resource_path("moon", &resource, CapabilityAudience::ModelProvider);
            let uri: axum::http::Uri = format!("{path}?cap={}", cap.as_str()).parse().unwrap();
            for altered in [
                uri.to_string().replace("%C3", "%c3"),
                uri.to_string().replace("Folder/", "Folder%2F"),
                format!("{}&cap=extra", uri),
            ] {
                assert!(
                    access
                        .verify(
                            "moon",
                            resource.clone(),
                            CapabilityAudience::ModelProvider,
                            &altered.parse().unwrap(),
                            "GET"
                        )
                        .is_err()
                );
            }
            assert!(
                access
                    .verify(
                        "moon",
                        resource.clone(),
                        CapabilityAudience::ModelProvider,
                        &uri,
                        "HEAD"
                    )
                    .is_err()
            );
            assert!(
                access
                    .verify(
                        "moon",
                        resource.clone(),
                        CapabilityAudience::ModelProvider,
                        &uri,
                        "GET"
                    )
                    .is_ok()
            );
            assert_eq!(
                access
                    .verify(
                        "moon",
                        resource.clone(),
                        CapabilityAudience::ModelProvider,
                        &uri,
                        "GET"
                    )
                    .is_ok(),
                replay == ReplayPolicy::ReusableWithinExpiry
            );
            clock.0.store(110, Ordering::SeqCst);
            assert!(
                access
                    .verify(
                        "moon",
                        resource,
                        CapabilityAudience::ModelProvider,
                        &uri,
                        "GET"
                    )
                    .is_err()
            );
        }
    }
}
