//! The one persistent companion identity a Nolune server owns.
//!
//! Product invariant (#100, #103): a server owns exactly one companion identity
//! and memory. Connected computers, chats, and relationship scopes are contexts
//! of that identity, never separate companions. Every persisted owner
//! reference, URL, event, and capability derives from [`CANONICAL_SLUG`].
//!
//! The on-disk layout and wire shape are described in
//! `docs/companion-storage.md`.

use serde::{Deserialize, Serialize};

/// Stable internal companion slug. Storage lives under `instances/companion/`.
pub const CANONICAL_SLUG: &str = "companion";

/// Version of the on-disk companion layout this server reads and writes.
pub const STORAGE_FORMAT_VERSION: u32 = 1;

/// Identity marker stored inside the companion directory.
pub const IDENTITY_FILE: &str = "companion.json";

/// Persisted identity marker. Any other shape fails closed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompanionIdentity {
    pub format_version: u32,
    pub slug: String,
}

impl CompanionIdentity {
    pub fn canonical() -> Self {
        Self {
            format_version: STORAGE_FORMAT_VERSION,
            slug: CANONICAL_SLUG.to_owned(),
        }
    }

    pub fn validate(&self) -> Result<(), IdentityError> {
        if self.format_version != STORAGE_FORMAT_VERSION {
            return Err(IdentityError::UnsupportedFormat(format!(
                "companion storage format {} is not supported (expected {STORAGE_FORMAT_VERSION})",
                self.format_version
            )));
        }
        if !is_canonical(&self.slug) {
            return Err(IdentityError::UnsupportedFormat(format!(
                "companion identity {:?} is not the canonical companion",
                self.slug
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdentityError {
    /// A marker exists but is not a layout this server understands.
    UnsupportedFormat(String),
    Io(String),
}

impl std::fmt::Display for IdentityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedFormat(message) | Self::Io(message) => f.write_str(message),
        }
    }
}

/// Exact match only. Case, whitespace, and path variants are foreign slugs.
pub fn is_canonical(slug: &str) -> bool {
    slug == CANONICAL_SLUG
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_exact_canonical_slug_is_accepted() {
        assert!(is_canonical(CANONICAL_SLUG));
        for foreign in [
            "",
            "Companion",
            "companion ",
            " companion",
            "companion/",
            "../companion",
            "alice",
            "instances/companion",
        ] {
            assert!(!is_canonical(foreign), "{foreign:?} must not be canonical");
        }
    }

    #[test]
    fn identity_marker_round_trips_and_rejects_other_shapes() {
        let canonical = CompanionIdentity::canonical();
        assert_eq!(canonical.validate(), Ok(()));
        let json = serde_json::to_string(&canonical).unwrap();
        assert_eq!(
            serde_json::from_str::<CompanionIdentity>(&json).unwrap(),
            canonical
        );

        let future = CompanionIdentity {
            format_version: STORAGE_FORMAT_VERSION + 1,
            ..CompanionIdentity::canonical()
        };
        assert!(matches!(
            future.validate(),
            Err(IdentityError::UnsupportedFormat(_))
        ));

        let foreign = CompanionIdentity {
            slug: "alice".into(),
            ..CompanionIdentity::canonical()
        };
        assert!(matches!(
            foreign.validate(),
            Err(IdentityError::UnsupportedFormat(_))
        ));

        assert!(
            serde_json::from_str::<CompanionIdentity>(
                r#"{"format_version":1,"slug":"companion","instances":["alice"]}"#
            )
            .is_err(),
            "unknown multi-instance fields must not deserialize"
        );
    }
}
