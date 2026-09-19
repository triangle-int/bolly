//! Filesystem boundary for the one canonical companion (#103).
//!
//! This is the only production module allowed to enumerate the `instances/`
//! directory. Sibling directories left over from the unpublished
//! multi-instance layout are reported as obsolete and never read, migrated,
//! scheduled, listed, or deleted. All identity reads and writes go through the
//! capability-confined [`MediaStore`].

use std::{
    io,
    path::{Path, PathBuf},
};

use crate::{
    domain::companion::{
        CANONICAL_SLUG, CompanionIdentity, IDENTITY_FILE, IdentityError, is_canonical,
    },
    services::media_text::MediaStore,
};

/// Upper bound for the identity marker; anything larger is not a marker.
const MAX_IDENTITY_BYTES: usize = 4096;

/// `workspace/instances/companion`
pub fn companion_dir(workspace_dir: &Path) -> PathBuf {
    workspace_dir.join("instances").join(CANONICAL_SLUG)
}

/// `Ok(Some(identity))` when the canonical companion exists with a valid
/// marker, `Ok(None)` when it has not been created yet, and `Err` when a
/// marker exists but is not a layout this server understands.
pub fn read_identity(workspace_dir: &Path) -> Result<Option<CompanionIdentity>, IdentityError> {
    if !companion_dir(workspace_dir).is_dir() {
        return Ok(None);
    }
    let store = open_store(workspace_dir)?;
    read_identity_from(&store)
}

/// Create-or-open the canonical companion. Idempotent; never rewrites a
/// marker it cannot validate and never touches obsolete siblings.
pub fn ensure_identity(workspace_dir: &Path) -> Result<CompanionIdentity, IdentityError> {
    let store = open_store(workspace_dir)?;
    if let Some(existing) = read_identity_from(&store)? {
        return Ok(existing);
    }
    let identity = CompanionIdentity::canonical();
    let raw = serde_json::to_string_pretty(&identity).expect("identity marker serializes");
    store
        .write_instance_text(CANONICAL_SLUG, IDENTITY_FILE, &raw)
        .map_err(io_error)?;
    Ok(identity)
}

/// Sorted names of `instances/*` directories other than the canonical one.
/// Read-only: callers may warn about them but must not act on them.
pub fn obsolete_instance_dirs(workspace_dir: &Path) -> Vec<String> {
    if !workspace_dir.join("instances").is_dir() {
        return Vec::new();
    }
    let Ok(store) = MediaStore::open(workspace_dir) else {
        return Vec::new();
    };
    match store.instance_slugs() {
        Ok(slugs) => slugs
            .into_iter()
            .filter(|slug| !is_canonical(slug))
            .collect(),
        Err(error) => {
            log::warn!("cannot inspect instances directory for obsolete companions: {error}");
            Vec::new()
        }
    }
}

fn open_store(workspace_dir: &Path) -> Result<MediaStore, IdentityError> {
    MediaStore::open(workspace_dir).map_err(io_error)
}

fn read_identity_from(store: &MediaStore) -> Result<Option<CompanionIdentity>, IdentityError> {
    match store.read_instance_text(CANONICAL_SLUG, IDENTITY_FILE, MAX_IDENTITY_BYTES) {
        Ok(raw) => {
            let identity: CompanionIdentity = serde_json::from_str(&raw).map_err(|error| {
                IdentityError::UnsupportedFormat(format!(
                    "companion identity marker is not readable: {error}"
                ))
            })?;
            identity.validate()?;
            Ok(Some(identity))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) if error.kind() == io::ErrorKind::InvalidData => {
            Err(IdentityError::UnsupportedFormat(format!(
                "companion identity marker is not readable: {error}"
            )))
        }
        Err(error) => Err(io_error(error)),
    }
}

fn io_error(error: io::Error) -> IdentityError {
    IdentityError::Io(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::companion::{
        CANONICAL_SLUG, IDENTITY_FILE, STORAGE_FORMAT_VERSION, is_canonical,
    };
    use std::fs;

    #[test]
    fn canonical_slug_passes_every_identity_validator() {
        assert!(is_canonical(CANONICAL_SLUG));

        let workspace = tempfile::tempdir().unwrap();
        let media = crate::services::media_text::MediaStore::open(workspace.path()).unwrap();
        media.ensure_memory_dir(CANONICAL_SLUG).unwrap();
        assert!(
            workspace
                .path()
                .join("instances")
                .join(CANONICAL_SLUG)
                .join("memory")
                .is_dir()
        );
        assert_eq!(
            companion_dir(workspace.path()),
            workspace.path().join("instances").join(CANONICAL_SLUG)
        );

        use crate::services::resource_capability::{
            CapabilityAudience, CapabilityMethod, CapabilityResource, CapabilityTarget,
        };
        for resource in [
            CapabilityResource::uploaded_file("upload-1").unwrap(),
            CapabilityResource::memory("photos/sky.png").unwrap(),
        ] {
            CapabilityTarget::new(
                CANONICAL_SLUG,
                resource,
                CapabilityMethod::Get,
                CapabilityAudience::Browser,
            )
            .expect("canonical slug must be a valid capability instance");
        }
    }

    #[test]
    fn identity_marker_is_created_once_and_read_back() {
        let workspace = tempfile::tempdir().unwrap();
        assert_eq!(read_identity(workspace.path()), Ok(None));
        assert!(
            !companion_dir(workspace.path()).exists(),
            "reads never create"
        );

        let created = ensure_identity(workspace.path()).unwrap();
        assert_eq!(created, CompanionIdentity::canonical());
        let marker = companion_dir(workspace.path()).join(IDENTITY_FILE);
        let raw = fs::read_to_string(&marker).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(parsed["format_version"], STORAGE_FORMAT_VERSION);
        assert_eq!(parsed["slug"], CANONICAL_SLUG);

        assert_eq!(ensure_identity(workspace.path()).unwrap(), created);
        assert_eq!(fs::read_to_string(&marker).unwrap(), raw, "idempotent");
        assert_eq!(read_identity(workspace.path()), Ok(Some(created)));
    }

    #[test]
    fn a_directory_without_a_marker_is_not_yet_a_companion_and_gets_one_on_ensure() {
        let workspace = tempfile::tempdir().unwrap();
        let dir = companion_dir(workspace.path());
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("soul.md"), "existing soul").unwrap();

        assert_eq!(read_identity(workspace.path()), Ok(None));
        ensure_identity(workspace.path()).unwrap();
        assert!(dir.join(IDENTITY_FILE).is_file());
        assert_eq!(
            fs::read_to_string(dir.join("soul.md")).unwrap(),
            "existing soul"
        );
    }

    #[test]
    fn unsupported_or_foreign_markers_fail_closed_without_being_rewritten() {
        let cases = [
            r#"{"format_version":2,"slug":"companion"}"#,
            r#"{"format_version":1,"slug":"alice"}"#,
            r#"{"format_version":1,"slug":"companion","instances":["alice"]}"#,
            "not json",
        ];
        for raw in cases {
            let workspace = tempfile::tempdir().unwrap();
            let dir = companion_dir(workspace.path());
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join(IDENTITY_FILE), raw).unwrap();

            assert!(
                matches!(
                    read_identity(workspace.path()),
                    Err(IdentityError::UnsupportedFormat(_))
                ),
                "{raw}"
            );
            assert!(
                matches!(
                    ensure_identity(workspace.path()),
                    Err(IdentityError::UnsupportedFormat(_))
                ),
                "{raw}"
            );
            assert_eq!(
                fs::read_to_string(dir.join(IDENTITY_FILE)).unwrap(),
                raw,
                "marker must not be rewritten"
            );
        }
    }

    #[test]
    fn obsolete_instance_dirs_lists_siblings_and_never_touches_them() {
        let workspace = tempfile::tempdir().unwrap();
        assert!(obsolete_instance_dirs(workspace.path()).is_empty());

        let instances = workspace.path().join("instances");
        for slug in ["bob", "alice"] {
            fs::create_dir_all(instances.join(slug).join("memory")).unwrap();
            fs::write(instances.join(slug).join("soul.md"), slug).unwrap();
        }
        fs::create_dir_all(instances.join(CANONICAL_SLUG)).unwrap();
        fs::write(instances.join("stray-file.txt"), "not a directory").unwrap();

        assert_eq!(
            obsolete_instance_dirs(workspace.path()),
            vec!["alice", "bob"]
        );
        for slug in ["alice", "bob"] {
            assert_eq!(
                fs::read_to_string(instances.join(slug).join("soul.md")).unwrap(),
                slug
            );
            assert!(instances.join(slug).join("memory").is_dir());
        }
    }
}
