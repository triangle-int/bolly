//! Versioned, owner-bound text representations for media memories.
//!
//! A representation lives at the exact adjacent `<media-path>.md`, starts with a
//! machine-readable v1 header containing the SHA-256 of its owner, and is followed
//! by the user-readable representation text. Sidecars are reserved and are never
//! independent text memories.

use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt, OpenOptionsMaybeDirExt};
use cap_std::{
    ambient_authority,
    fs::{Dir, OpenOptions},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    io::{self, BufReader, Read, Write},
    path::{Component, Path, PathBuf},
};

pub const VERSION: u32 = 1;
const HEADER_PREFIX: &str = "NOLUNE_MEDIA_TEXT ";
const HASH_BUFFER_BYTES: usize = 64 * 1024;
const MAX_UPLOAD_METADATA_BYTES: usize = 1024 * 1024;
const MAX_LEGACY_OBSERVATION_BYTES: usize = 1024 * 1024;
const MAX_LEGACY_CLEANUP_ENTRIES: usize = 10_000;
const LEGACY_CLEANUP_TOMBSTONE_PREFIX: &str = ".legacy-screen-cleanup-";
const LEGACY_CLEANUP_TOMBSTONE_SUFFIX: &str = ".json";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryEntry {
    pub name: String,
    pub is_dir: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryMetadata {
    pub is_file: bool,
    pub is_dir: bool,
    pub len: u64,
}

/// Filesystem authority anchored to the configured workspace at startup.
pub struct MediaStore {
    root: Dir,
    upload_dirs: std::sync::Mutex<HashMap<String, std::sync::Arc<Dir>>>,
    #[cfg(test)]
    fail_next_write: std::sync::atomic::AtomicBool,
    #[cfg(test)]
    legacy_cleanup_failures: std::sync::Mutex<std::collections::HashSet<String>>,
}

impl MediaStore {
    pub fn open(workspace_root: &Path) -> io::Result<Self> {
        let metadata = std::fs::symlink_metadata(workspace_root)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(invalid_path("workspace root must be a real directory"));
        }
        Ok(Self {
            root: Dir::open_ambient_dir(workspace_root, ambient_authority())?,
            upload_dirs: std::sync::Mutex::new(HashMap::new()),
            #[cfg(test)]
            fail_next_write: std::sync::atomic::AtomicBool::new(false),
            #[cfg(test)]
            legacy_cleanup_failures: std::sync::Mutex::new(std::collections::HashSet::new()),
        })
    }

    fn memory_path(&self, slug: &str, path: &str) -> io::Result<PathBuf> {
        Ok(self.instance_path(slug, &format!("memory/{path}"))?)
    }

    fn instance_path(&self, slug: &str, path: &str) -> io::Result<PathBuf> {
        validate_slug(slug)?;
        let relative = validate_relative(path)?;
        Ok(Path::new("instances").join(slug).join(relative))
    }

    pub fn read_instance_text(
        &self,
        slug: &str,
        path: &str,
        max_bytes: usize,
    ) -> io::Result<String> {
        let relative = self.instance_path(slug, path)?;
        reject_symlinks(&self.root, &relative, false)?;
        let bytes = read_bounded(&self.root, &relative, max_bytes)?;
        String::from_utf8(bytes).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("instance text is not valid UTF-8: {error}"),
            )
        })
    }

    pub fn write_instance_text(&self, slug: &str, path: &str, content: &str) -> io::Result<()> {
        let relative = self.instance_path(slug, path)?;
        atomic_publish(&self.root, &relative, |file| {
            file.write_all(content.as_bytes())
        })
    }

    fn owner_and_sidecar(&self, slug: &str, path: &str) -> io::Result<(PathBuf, PathBuf)> {
        let (owner, sidecar) = owner_and_sidecar(path)?;
        validate_slug(slug)?;
        let memory = Path::new("instances").join(slug).join("memory");
        Ok((memory.join(owner), memory.join(sidecar)))
    }

    pub fn ensure_memory_dir(&self, slug: &str) -> io::Result<()> {
        validate_slug(slug)?;
        let memory = Path::new("instances").join(slug).join("memory");
        reject_symlinks(&self.root, &memory, true)?;
        self.root.create_dir_all(&memory)?;
        reject_symlinks(&self.root, &memory, false)
    }

    pub fn instance_slugs(&self) -> io::Result<Vec<String>> {
        let instances = Path::new("instances");
        reject_symlinks(&self.root, instances, false)?;
        let mut slugs = Vec::new();
        for entry in self.root.read_dir(instances)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            if file_type.is_symlink() {
                return Err(invalid_path("symlink instance paths are not allowed"));
            }
            if file_type.is_dir() {
                slugs.push(
                    entry
                        .file_name()
                        .into_string()
                        .map_err(|_| invalid_path("instance slug is not UTF-8"))?,
                );
            }
        }
        slugs.sort();
        Ok(slugs)
    }

    /// Validate an upload through held capability directories without reading its blob.
    /// Metadata and blob are opened no-follow and bound by id, name, MIME, and size.
    pub fn upload_descriptor(
        &self,
        slug: &str,
        upload_id: &str,
    ) -> io::Result<crate::domain::upload::UploadMeta> {
        self.validated_upload(slug, upload_id).map(|(meta, _)| meta)
    }

    pub(crate) fn open_upload_blob(
        &self,
        slug: &str,
        upload_id: &str,
    ) -> io::Result<(crate::domain::upload::UploadMeta, cap_std::fs::File)> {
        self.validated_upload(slug, upload_id)
    }

    fn validated_upload(
        &self,
        slug: &str,
        upload_id: &str,
    ) -> io::Result<(crate::domain::upload::UploadMeta, cap_std::fs::File)> {
        validate_slug(slug)?;
        let id_path = validate_relative(upload_id)?;
        if id_path.components().count() != 1 || upload_id.len() > 255 {
            return Err(invalid_path("invalid upload id"));
        }
        let uploads = self.upload_dir(slug)?;

        let metadata_name = format!("{upload_id}.json");
        let metadata = read_named_regular_file(&uploads, &metadata_name, 64 * 1024)?
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "upload not found"))?;
        let meta: crate::domain::upload::UploadMeta = serde_json::from_slice(&metadata)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        if meta.id != upload_id {
            return Err(invalid_path("upload metadata id mismatch"));
        }
        let extension = upload_id
            .rsplit_once('.')
            .map(|(_, extension)| extension)
            .filter(|extension| !extension.is_empty())
            .ok_or_else(|| invalid_path("upload id has no extension"))?;
        let expected_name = format!("{upload_id}_blob.{extension}");
        if meta.stored_name != expected_name
            || validate_relative(&meta.stored_name)?.components().count() != 1
        {
            return Err(invalid_path("upload stored name mismatch"));
        }
        if meta.mime_type != crate::services::uploads::mime_from_ext(extension) {
            return Err(invalid_path("upload MIME mismatch"));
        }
        if meta.size > crate::services::uploads::MAX_FILE_SIZE {
            return Err(invalid_path("upload size exceeds limit"));
        }
        let blob = open_named_regular_file(&uploads, &meta.stored_name)?
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "upload blob not found"))?;
        if blob.metadata()?.len() != meta.size {
            return Err(invalid_path("upload size mismatch"));
        }
        Ok((meta, blob))
    }

    /// Read an inline attachment through a held no-follow handle with a fixed small cap.
    pub fn read_upload_inline(
        &self,
        slug: &str,
        upload_id: &str,
    ) -> io::Result<(crate::domain::upload::UploadMeta, Vec<u8>)> {
        const MAX_INLINE_UPLOAD_BYTES: usize = 4 * 1024 * 1024;
        let (meta, blob) = self.validated_upload(slug, upload_id)?;
        if meta.size > MAX_INLINE_UPLOAD_BYTES as u64 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "inline attachment exceeds 4 MiB limit",
            ));
        }
        let bytes = read_bounded_file(blob, MAX_INLINE_UPLOAD_BYTES)?;
        if bytes.len() as u64 != meta.size {
            return Err(invalid_path("upload size mismatch"));
        }
        Ok((meta, bytes))
    }

    fn upload_dir(&self, slug: &str) -> io::Result<std::sync::Arc<Dir>> {
        let mut cache = self
            .upload_dirs
            .lock()
            .map_err(|_| io::Error::other("upload directory cache poisoned"))?;
        if let Some(directory) = cache.get(slug) {
            return Ok(directory.clone());
        }
        let Some(instances) = open_real_child_dir(&self.root, "instances")? else {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "instance not found",
            ));
        };
        let Some(instance) = open_real_child_dir(&instances, slug)? else {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "instance not found",
            ));
        };
        let Some(uploads) = open_real_child_dir(&instance, "uploads")? else {
            return Err(io::Error::new(io::ErrorKind::NotFound, "uploads not found"));
        };
        let uploads = std::sync::Arc::new(uploads);
        cache.insert(slug.to_owned(), uploads.clone());
        Ok(uploads)
    }

    /// Remove unpublished passive-screen-capture artifacts through persistently
    /// opened capability directories. Only strict observation/upload pairs are
    /// reconciled; malformed, ambiguous, and unreferenced data is preserved.
    pub fn cleanup_legacy_screen_capture(&self) -> io::Result<()> {
        let Some(instances) = open_real_child_dir(&self.root, "instances")? else {
            return Ok(());
        };
        let mut errors = Vec::new();
        let entries = instances.read_dir(".")?;
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    errors.push(format!("instance entry: {error}"));
                    continue;
                }
            };
            let slug = match entry.file_name().into_string() {
                Ok(slug) => slug,
                Err(_) => {
                    errors.push("non-UTF-8 instance name".to_owned());
                    continue;
                }
            };
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(error) => {
                    errors.push(format!("{slug}: {error}"));
                    continue;
                }
            };
            if file_type.is_symlink() || !file_type.is_dir() {
                continue;
            }
            let instance = match open_real_child_dir(&instances, &slug) {
                Ok(Some(instance)) => instance,
                Ok(None) => continue,
                Err(error) => {
                    errors.push(format!("{slug}: {error}"));
                    continue;
                }
            };
            if let Err(error) = self.cleanup_legacy_screen_capture_instance(&slug, &instance) {
                errors.push(format!("{slug}: {error}"));
            }
        }
        finish_aggregated(errors)
    }

    fn cleanup_legacy_screen_capture_instance(&self, slug: &str, instance: &Dir) -> io::Result<()> {
        let Some(observations) = open_real_child_dir(instance, "observations")? else {
            return Ok(());
        };
        let mut scan_errors = Vec::new();
        let mut candidates = Vec::new();
        let mut reference_counts = HashMap::<String, usize>::new();
        let mut entry_names = HashSet::new();
        let mut inspected = 0_usize;

        for entry in observations.read_dir(".")? {
            inspected += 1;
            if inspected > MAX_LEGACY_CLEANUP_ENTRIES {
                scan_errors.push("legacy observation cleanup entry limit exceeded".to_owned());
                break;
            }
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    scan_errors.push(format!("observation entry: {error}"));
                    continue;
                }
            };
            let raw_name = entry.file_name();
            let display_name = raw_name.to_string_lossy();
            let utf8_name = raw_name.to_str().map(str::to_owned);
            if let Some(name) = &utf8_name {
                // Include every name, including symlinks and non-files, so a
                // later rename can never replace a pre-existing tombstone path.
                entry_names.insert(name.clone());
            }
            if self.take_legacy_cleanup_failure(slug, &format!("scan-entry:{display_name}")) {
                scan_errors.push(format!(
                    "observations/{display_name}: injected directory entry failure"
                ));
                continue;
            }
            if self.take_legacy_cleanup_failure(slug, &format!("scan-file-type:{display_name}")) {
                scan_errors.push(format!(
                    "observations/{display_name}: injected file type failure"
                ));
                continue;
            }
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(error) => {
                    scan_errors.push(format!("observations/{display_name}: {error}"));
                    continue;
                }
            };
            if file_type.is_symlink() || !file_type.is_file() {
                continue;
            }
            let Some(name) = utf8_name else {
                continue;
            };
            let Some(source) = legacy_observation_source(&name) else {
                continue;
            };
            if self.take_legacy_cleanup_failure(slug, &format!("scan-read:{name}")) {
                scan_errors.push(format!("observations/{name}: injected read failure"));
                continue;
            }
            let bytes =
                match read_named_regular_file(&observations, &name, MAX_LEGACY_OBSERVATION_BYTES) {
                    Ok(Some(bytes)) => bytes,
                    Ok(None) => {
                        scan_errors.push(format!(
                            "observations/{name}: regular file changed during scan"
                        ));
                        continue;
                    }
                    Err(error) => {
                        scan_errors.push(format!("observations/{name}: {error}"));
                        continue;
                    }
                };
            let observation: LegacyScreenObservation = match serde_json::from_slice(&bytes) {
                Ok(observation) => observation,
                Err(_) => continue,
            };
            // Every schema-valid preserved observation participates in
            // ambiguity. Producer validation decides deletion eligibility,
            // not whether its reference can protect an upload.
            *reference_counts
                .entry(observation.upload_id.clone())
                .or_default() += 1;
            if parse_prefixed_millis(&observation.id, "obs_", "").is_none() {
                continue;
            }
            if parse_ascii_millis(&observation.created_at).is_none()
                || parse_prefixed_millis(&observation.upload_id, "upload_", ".mp4").is_none()
                || !source.matches(&name, &observation.id)
            {
                continue;
            }
            candidates.push(LegacyObservationCandidate {
                name,
                source,
                observation,
            });
        }

        // No mutation in this instance is permitted until every observation
        // directory entry and every relevant bounded file read completed.
        if !scan_errors.is_empty() {
            return finish_aggregated(scan_errors);
        }

        let uploads = open_real_child_dir(instance, "uploads")?;
        let mut errors = Vec::new();
        if let Some(uploads) = uploads {
            for candidate in candidates {
                if reference_counts.get(&candidate.observation.upload_id) != Some(&1) {
                    continue;
                }
                if let Err(error) = self.reconcile_legacy_observation(
                    slug,
                    &observations,
                    &uploads,
                    &entry_names,
                    &candidate,
                ) {
                    errors.push(format!("observations/{}: {error}", candidate.name));
                }
            }
        }

        match observations.read_dir(".") {
            Ok(mut entries) => {
                if entries.next().is_none()
                    && let Err(error) = instance.remove_dir("observations")
                    && error.kind() != io::ErrorKind::NotFound
                {
                    errors.push(format!("observations: {error}"));
                }
            }
            Err(error) => errors.push(format!("observations: {error}")),
        }
        finish_aggregated(errors)
    }

    fn reconcile_legacy_observation(
        &self,
        slug: &str,
        observations: &Dir,
        uploads: &Dir,
        entry_names: &HashSet<String>,
        candidate: &LegacyObservationCandidate,
    ) -> io::Result<()> {
        let observation = &candidate.observation;
        let metadata_name = format!("{}.json", observation.upload_id);
        let expected_stored_name = format!("{}_blob.mp4", observation.upload_id);
        let upload =
            match read_named_regular_file(uploads, &metadata_name, MAX_UPLOAD_METADATA_BYTES)? {
                Some(metadata_bytes) => {
                    let upload: LegacyUploadMetadata = match serde_json::from_slice(&metadata_bytes)
                    {
                        Ok(upload) => upload,
                        Err(_) => return Ok(()),
                    };
                    if !upload.matches_observation(
                        observation,
                        &metadata_name,
                        &expected_stored_name,
                    ) {
                        return Ok(());
                    }
                    Some(upload)
                }
                None if candidate.source == LegacyObservationSource::Tombstone => {
                    match uploads.symlink_metadata(&metadata_name) {
                        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                        Ok(_) => return Ok(()),
                        Err(error) => return Err(error),
                    }
                }
                None => return Ok(()),
            };

        match uploads.symlink_metadata(&expected_stored_name) {
            Ok(metadata)
                if metadata.file_type().is_symlink()
                    || !metadata.is_file()
                    || upload
                        .as_ref()
                        .is_none_or(|upload| metadata.len() != upload.size) =>
            {
                return Ok(());
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }

        let tombstone_name = legacy_cleanup_tombstone_name(&observation.id);
        if candidate.source == LegacyObservationSource::Normal {
            // Never replace an arbitrary pre-existing hidden file.
            if entry_names.contains(&tombstone_name) {
                return Ok(());
            }
            observations.rename(&candidate.name, observations, &tombstone_name)?;
            self.fail_legacy_cleanup_after(slug, "rename")?;
        }

        // A tombstone observed after a failed/unclean rename is not durable
        // proof until this directory sync succeeds. Never delete upload state
        // first, even on a retry.
        sync_capability_dir(observations)?;

        self.remove_legacy_file(slug, "uploads", uploads, &expected_stored_name)?;
        sync_capability_dir(uploads)?;
        self.fail_legacy_cleanup_after(slug, "blob")?;

        self.remove_legacy_file(slug, "uploads", uploads, &metadata_name)?;
        sync_capability_dir(uploads)?;
        self.fail_legacy_cleanup_after(slug, "metadata")?;

        self.remove_legacy_file(slug, "observations", observations, &tombstone_name)?;
        sync_capability_dir(observations)?;
        self.fail_legacy_cleanup_after(slug, "tombstone")?;
        log::info!("removed legacy passive screen recording observation for {slug}");
        Ok(())
    }

    fn fail_legacy_cleanup_after(&self, _slug: &str, _step: &str) -> io::Result<()> {
        if self.take_legacy_cleanup_failure(_slug, &format!("after-{_step}")) {
            return Err(io::Error::other(format!(
                "injected legacy cleanup failure after {_step}"
            )));
        }
        Ok(())
    }

    fn take_legacy_cleanup_failure(&self, _slug: &str, _point: &str) -> bool {
        #[cfg(test)]
        {
            return self
                .legacy_cleanup_failures
                .lock()
                .unwrap()
                .remove(&format!("{_slug}/{_point}"));
        }
        #[cfg(not(test))]
        false
    }

    fn remove_legacy_file(
        &self,
        _slug: &str,
        _dir: &str,
        handle: &Dir,
        name: &str,
    ) -> io::Result<()> {
        remove_one_if_present(handle, Path::new(name))
    }

    /// Remove the retired child-agent framework state (`agents/` configs,
    /// markers, and histories; `agent_runs/` traces) from the canonical
    /// companion through the persistent workspace capability (#93). Bounded,
    /// idempotent, and never touches obsolete siblings.
    pub fn cleanup_legacy_child_agents(&self) -> io::Result<()> {
        let Some(instances) = open_real_child_dir(&self.root, "instances")? else {
            return Ok(());
        };
        let Some(companion) =
            open_real_child_dir(&instances, crate::domain::companion::CANONICAL_SLUG)?
        else {
            return Ok(());
        };
        let mut errors = Vec::new();
        for name in ["agents", "agent_runs"] {
            if let Err(error) = remove_legacy_dir(&companion, name) {
                errors.push(format!("{name}: {error}"));
            }
        }
        finish_aggregated(errors)
    }

    /// Remove the retired per-day `stats/` aggregate store from the canonical
    /// companion. Bounded, idempotent, and never touches obsolete siblings.
    pub fn cleanup_legacy_stats(&self) -> io::Result<()> {
        let Some(instances) = open_real_child_dir(&self.root, "instances")? else {
            return Ok(());
        };
        let Some(companion) =
            open_real_child_dir(&instances, crate::domain::companion::CANONICAL_SLUG)?
        else {
            return Ok(());
        };
        let Some(stats) = open_real_child_dir(&companion, "stats")? else {
            return Ok(());
        };
        let mut errors = Vec::new();
        let mut names = Vec::new();
        for entry in stats.read_dir(".")? {
            match entry {
                Ok(entry) => names.push(entry.file_name()),
                Err(error) => errors.push(format!("stats entry: {error}")),
            }
        }
        for name in names {
            let label = name.to_string_lossy().into_owned();
            let is_daily_file = label.ends_with(".json")
                && stats
                    .symlink_metadata(&name)
                    .map(|metadata| metadata.is_file())
                    .unwrap_or(false);
            if !is_daily_file {
                errors.push(format!("stats/{label}: unexpected entry left in place"));
                continue;
            }
            if let Err(error) = stats.remove_file(&name) {
                errors.push(format!("stats/{label}: {error}"));
            }
        }
        drop(stats);
        if errors.is_empty() {
            companion.remove_dir("stats")?;
        }
        finish_aggregated(errors)
    }

    pub fn memory_exists(&self, slug: &str, path: &str) -> io::Result<bool> {
        let relative = self.memory_path(slug, path)?;
        reject_symlinks(&self.root, &relative, true)?;
        match self.root.metadata(&relative) {
            Ok(metadata) => Ok(metadata.is_file()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error),
        }
    }

    pub fn rename_memory(&self, slug: &str, from: &str, to: &str) -> io::Result<()> {
        let from = self.memory_path(slug, from)?;
        let to = self.memory_path(slug, to)?;
        reject_symlinks(&self.root, &from, false)?;
        reject_symlinks(&self.root, &to, true)?;
        ensure_parent(&self.root, &to)?;
        self.root.rename(&from, &self.root, &to)
    }

    pub fn write_memory_text(&self, slug: &str, path: &str, content: &str) -> io::Result<()> {
        let relative = self.memory_path(slug, path)?;
        atomic_publish(&self.root, &relative, |file| {
            file.write_all(content.as_bytes())
        })
    }

    pub fn memory_metadata(&self, slug: &str, path: &str) -> io::Result<MemoryMetadata> {
        let relative = self.memory_path(slug, path)?;
        reject_symlinks(&self.root, &relative, false)?;
        let metadata = self.root.metadata(&relative)?;
        Ok(MemoryMetadata {
            is_file: metadata.is_file(),
            is_dir: metadata.is_dir(),
            len: metadata.len(),
        })
    }

    pub fn list_memory_dir(
        &self,
        slug: &str,
        path: Option<&str>,
    ) -> io::Result<Vec<DirectoryEntry>> {
        validate_slug(slug)?;
        let mut relative = Path::new("instances").join(slug).join("memory");
        if let Some(path) = path.filter(|path| !path.is_empty()) {
            relative.push(validate_relative(path)?);
        }
        reject_symlinks(&self.root, &relative, false)?;
        let mut entries = Vec::new();
        for entry in self.root.read_dir(&relative)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            if file_type.is_symlink() {
                return Err(invalid_path("symlink memory paths are not allowed"));
            }
            entries.push(DirectoryEntry {
                name: entry
                    .file_name()
                    .into_string()
                    .map_err(|_| invalid_path("memory filename is not UTF-8"))?,
                is_dir: file_type.is_dir(),
            });
        }
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(entries)
    }

    /// Publish one uploaded blob without reopening the configured workspace path.
    pub fn publish_upload_owner(&self, slug: &str, path: &str, upload_id: &str) -> io::Result<()> {
        let metadata = self.upload_metadata(slug, upload_id)?;
        let stored_name = metadata
            .get("stored_name")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| invalid_path("missing stored_name"))?;
        let stored_name = validate_single_component(stored_name, "invalid stored upload name")?;
        let source_path = Path::new("instances")
            .join(slug)
            .join("uploads")
            .join(stored_name);
        reject_symlinks(&self.root, &source_path, false)?;
        let mut source = self.root.open(&source_path)?;
        if !source.metadata()?.is_file() {
            return Err(invalid_path("upload source must be a regular file"));
        }
        self.publish_owner(slug, path, &mut source)
    }

    pub fn upload_mime_type(&self, slug: &str, upload_id: &str) -> io::Result<Option<String>> {
        Ok(self
            .upload_metadata(slug, upload_id)?
            .get("mime_type")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned))
    }

    fn upload_metadata(&self, slug: &str, upload_id: &str) -> io::Result<serde_json::Value> {
        validate_slug(slug)?;
        let upload_id = validate_single_component(upload_id, "invalid upload id")?;
        let metadata_path = Path::new("instances")
            .join(slug)
            .join("uploads")
            .join(format!("{}.json", upload_id.to_string_lossy()));
        reject_symlinks(&self.root, &metadata_path, false)?;
        let metadata = read_bounded(&self.root, &metadata_path, MAX_UPLOAD_METADATA_BYTES)?;
        serde_json::from_slice(&metadata).map_err(|_| invalid_path("invalid upload metadata"))
    }

    /// Copy owner bytes into a create-new same-directory temporary file, fsync it,
    /// and atomically replace the destination through the workspace handle.
    pub fn publish_owner(&self, slug: &str, path: &str, source: &mut impl Read) -> io::Result<()> {
        let (owner, _) = self.owner_and_sidecar(slug, path)?;
        atomic_publish(&self.root, &owner, |file| {
            io::copy(source, file)?;
            Ok(())
        })
    }

    /// Atomically persist agent-authored text bound to the current owner bytes.
    pub fn write(&self, slug: &str, path: &str, content: &str) -> io::Result<()> {
        let (owner, sidecar) = self.owner_and_sidecar(slug, path)?;
        reject_symlinks(&self.root, &owner, false)?;
        if content.trim().is_empty() {
            reject_symlinks(&self.root, &sidecar, true)?;
            return remove_one_if_present(&self.root, &sidecar);
        }
        let owner_file = self.root.open(&owner)?;
        if !owner_file.metadata()?.is_file() {
            return Err(invalid_path("media owner must be a regular file"));
        }
        let header = Header {
            version: VERSION,
            sha256: sha256_reader(owner_file)?,
        };
        #[cfg(test)]
        if self
            .fail_next_write
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            return Err(io::Error::other("injected sidecar persistence failure"));
        }
        let encoded_header = serde_json::to_string(&header).map_err(io::Error::other)?;
        atomic_publish(&self.root, &sidecar, |temporary| {
            write!(temporary, "{HEADER_PREFIX}{encoded_header}\n{content}")
        })
    }

    /// Read text only when the sidecar is valid and still matches the owner.
    pub fn read(&self, slug: &str, path: &str) -> Result<String, String> {
        let (owner, sidecar) = self
            .owner_and_sidecar(slug, path)
            .map_err(|error| error.to_string())?;
        reject_symlinks(&self.root, &owner, false).map_err(|error| error.to_string())?;
        reject_symlinks(&self.root, &sidecar, false).map_err(|error| error.to_string())?;
        let persisted = self
            .root
            .read_to_string(&sidecar)
            .map_err(|_| "media text representation is missing or unreadable".to_owned())?;
        let (header_line, content) = persisted
            .split_once('\n')
            .ok_or_else(|| "media text representation header is malformed".to_owned())?;
        let encoded_header = header_line
            .strip_prefix(HEADER_PREFIX)
            .ok_or_else(|| "media text representation header is malformed".to_owned())?;
        let header: Header = serde_json::from_str(encoded_header)
            .map_err(|_| "media text representation header is malformed".to_owned())?;
        if header.version != VERSION {
            return Err("media text representation version is unsupported".into());
        }
        if header.sha256.len() != 64 || !header.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err("media text representation digest is malformed".into());
        }
        let owner_file = self.root.open(&owner).map_err(|error| error.to_string())?;
        if !owner_file
            .metadata()
            .map_err(|error| error.to_string())?
            .is_file()
        {
            return Err("media owner must be a regular file".into());
        }
        let current_digest = sha256_reader(owner_file).map_err(|error| error.to_string())?;
        if !header.sha256.eq_ignore_ascii_case(&current_digest) {
            return Err("media text representation digest does not match its owner".into());
        }
        if content.trim().is_empty() {
            return Err("media text representation is empty".into());
        }
        Ok(content.to_owned())
    }

    /// Read a regular memory file without following an attacker-controlled path.
    pub fn read_memory_file(
        &self,
        slug: &str,
        path: &str,
        max_bytes: usize,
    ) -> io::Result<Vec<u8>> {
        let relative = self.memory_path(slug, path)?;
        reject_symlinks(&self.root, &relative, false)?;
        let mut file = self.root.open(&relative)?;
        if !file.metadata()?.is_file() {
            return Err(invalid_path("memory target must be a regular file"));
        }
        let limit = u64::try_from(max_bytes).unwrap_or(u64::MAX);
        let mut bytes = Vec::new();
        std::io::Read::by_ref(&mut file)
            .take(limit.saturating_add(1))
            .read_to_end(&mut bytes)?;
        if bytes.len() > max_bytes {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "memory file is too large",
            ));
        }
        Ok(bytes)
    }

    pub fn read_memory_text(&self, slug: &str, path: &str) -> io::Result<String> {
        let bytes = self.read_memory_file(slug, path, 16 * 1024 * 1024)?;
        String::from_utf8(bytes).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("memory text is not valid UTF-8: {error}"),
            )
        })
    }

    /// Enumerate indexable memory owners without leaving the workspace capability.
    pub fn memory_files(&self, slug: &str) -> Result<Vec<String>, String> {
        validate_slug(slug).map_err(|error| error.to_string())?;
        let base = Path::new("instances").join(slug).join("memory");
        let mut files = Vec::new();
        match self.collect_memory_files(&base, Path::new(""), &mut files) {
            Ok(()) => {
                files.sort();
                Ok(files)
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(error) => Err(error.to_string()),
        }
    }

    fn collect_memory_files(
        &self,
        base: &Path,
        relative_dir: &Path,
        files: &mut Vec<String>,
    ) -> io::Result<()> {
        let current = base.join(relative_dir);
        reject_symlinks(&self.root, &current, false)?;
        for entry in self.root.read_dir(&current)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name
                .to_str()
                .ok_or_else(|| invalid_path("memory filename is not UTF-8"))?;
            if name.starts_with('.') || name.starts_with('_') {
                continue;
            }
            let relative = relative_dir.join(name);
            let full = base.join(&relative);
            let file_type = entry.file_type()?;
            if file_type.is_symlink() {
                return Err(invalid_path("symlink memory paths are not allowed"));
            }
            if file_type.is_dir() {
                self.collect_memory_files(base, &relative, files)?;
                continue;
            }
            if !file_type.is_file() {
                continue;
            }
            let path = relative
                .to_str()
                .ok_or_else(|| invalid_path("memory path is not UTF-8"))?;
            if media_path(path).is_some() {
                continue;
            }
            let extension = relative
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if extension == "md" || source_type(path).is_some() {
                reject_symlinks(&self.root, &full, false)?;
                files.push(path.to_owned());
            }
        }
        Ok(())
    }

    /// Reconcile filesystem deletion relative to the persistent workspace handle.
    pub fn remove(&self, slug: &str, path: &str) -> io::Result<()> {
        validate_slug(slug)?;
        validate_relative(path)?;
        let targets = if source_type(path).is_some() {
            vec![sidecar_path(path), path.to_owned()]
        } else {
            vec![path.to_owned()]
        };
        let mut errors = Vec::new();
        for target in targets {
            let result = self.memory_path(slug, &target).and_then(|relative| {
                reject_symlinks(&self.root, &relative, true)?;
                remove_one_if_present(&self.root, &relative)
            });
            if let Err(error) = result {
                errors.push(format!("{target}: {error}"));
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(io::Error::other(errors.join("; ")))
        }
    }
}

#[cfg(test)]
impl MediaStore {
    pub(crate) fn inject_next_write_failure(&self) {
        self.fail_next_write
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    fn inject_legacy_cleanup_failure(&self, slug: &str, point: &str) {
        self.legacy_cleanup_failures
            .lock()
            .unwrap()
            .insert(format!("{slug}/{point}"));
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Header {
    version: u32,
    sha256: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyScreenObservation {
    id: String,
    upload_id: String,
    #[serde(rename = "machine_id")]
    _machine_id: String,
    #[serde(rename = "analysis")]
    _analysis: String,
    created_at: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyUploadMetadata {
    id: String,
    original_name: String,
    stored_name: String,
    mime_type: String,
    size: u64,
    uploaded_at: String,
    #[serde(default, rename = "anthropic_file_id")]
    _anthropic_file_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LegacyObservationSource {
    Normal,
    Tombstone,
}

impl LegacyObservationSource {
    fn matches(self, name: &str, observation_id: &str) -> bool {
        match self {
            Self::Normal => name == format!("{observation_id}.json"),
            Self::Tombstone => name == legacy_cleanup_tombstone_name(observation_id),
        }
    }
}

struct LegacyObservationCandidate {
    name: String,
    source: LegacyObservationSource,
    observation: LegacyScreenObservation,
}

impl LegacyUploadMetadata {
    fn matches_observation(
        &self,
        observation: &LegacyScreenObservation,
        metadata_name: &str,
        expected_stored_name: &str,
    ) -> bool {
        let Some(upload_millis) = parse_prefixed_millis(&self.id, "upload_", ".mp4") else {
            return false;
        };
        self.id == observation.upload_id
            && metadata_name == format!("{}.json", self.id)
            && self.original_name == "screen_recording.mp4"
            && self.mime_type == "video/mp4"
            && self.stored_name == expected_stored_name
            && parse_ascii_millis(&self.uploaded_at) == Some(upload_millis)
    }
}

fn parse_ascii_millis(value: &str) -> Option<u128> {
    // The retired producers formatted a u128 millisecond value. Bound the
    // decimal width and use checked arithmetic so hostile values never wrap or
    // trigger an oversized allocation.
    if value.is_empty()
        || value.len() > 39
        || (value.len() > 1 && value.starts_with('0'))
        || !value.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    value.bytes().try_fold(0_u128, |parsed, byte| {
        parsed.checked_mul(10)?.checked_add(u128::from(byte - b'0'))
    })
}

fn parse_prefixed_millis(value: &str, prefix: &str, suffix: &str) -> Option<u128> {
    let digits = value.strip_prefix(prefix)?.strip_suffix(suffix)?;
    parse_ascii_millis(digits)
}

fn legacy_cleanup_tombstone_name(observation_id: &str) -> String {
    format!("{LEGACY_CLEANUP_TOMBSTONE_PREFIX}{observation_id}{LEGACY_CLEANUP_TOMBSTONE_SUFFIX}")
}

fn legacy_observation_source(name: &str) -> Option<LegacyObservationSource> {
    if name.starts_with(LEGACY_CLEANUP_TOMBSTONE_PREFIX)
        && name.ends_with(LEGACY_CLEANUP_TOMBSTONE_SUFFIX)
    {
        Some(LegacyObservationSource::Tombstone)
    } else if name.ends_with(".json") {
        Some(LegacyObservationSource::Normal)
    } else {
        None
    }
}

pub fn source_type(path: &str) -> Option<&'static str> {
    match Path::new(path)
        .extension()?
        .to_str()?
        .to_ascii_lowercase()
        .as_str()
    {
        "jpg" | "jpeg" | "png" | "webp" | "gif" | "svg" => Some("media_image"),
        "pdf" => Some("media_document"),
        "mp4" | "mov" => Some("media_video"),
        "mp3" | "wav" => Some("media_audio"),
        _ => None,
    }
}

pub fn sidecar_path(media_path: &str) -> String {
    format!("{media_path}.md")
}

pub fn media_path(sidecar: &str) -> Option<&str> {
    let path = sidecar.strip_suffix(".md")?;
    source_type(path).map(|_| path)
}

fn invalid_path(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

fn validate_relative(path: &str) -> io::Result<PathBuf> {
    let bytes = path.as_bytes();
    let has_windows_prefix = bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
    if path.is_empty()
        || path.contains('\\')
        || path.split('/').any(str::is_empty)
        || has_windows_prefix
    {
        return Err(invalid_path("invalid memory path"));
    }
    let parsed = Path::new(path);
    if parsed.is_absolute()
        || parsed
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(invalid_path("invalid memory path"));
    }
    Ok(parsed.to_owned())
}

fn validate_slug(slug: &str) -> io::Result<()> {
    let parsed = validate_relative(slug)?;
    if parsed.components().count() != 1 {
        return Err(invalid_path("invalid instance slug"));
    }
    Ok(())
}

fn validate_single_component(value: &str, message: &str) -> io::Result<PathBuf> {
    let parsed = validate_relative(value)?;
    if parsed.components().count() != 1 {
        return Err(invalid_path(message));
    }
    Ok(parsed)
}

#[cfg(test)]
fn open_root(memory_dir: &Path) -> io::Result<Dir> {
    let metadata = std::fs::symlink_metadata(memory_dir)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(invalid_path("memory root must be a real directory"));
    }
    Dir::open_ambient_dir(memory_dir, ambient_authority())
}

/// Check the currently observable path components for symlinks. Capability-
/// relative operations remain the confinement boundary if a component races.
fn reject_symlinks(root: &Dir, relative: &Path, target_may_be_missing: bool) -> io::Result<()> {
    let components: Vec<_> = relative.components().collect();
    let mut current = PathBuf::new();
    for (index, component) in components.iter().enumerate() {
        let Component::Normal(component) = component else {
            return Err(invalid_path("invalid memory path"));
        };
        current.push(component);
        match root.symlink_metadata(&current) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    return Err(invalid_path("symlink memory paths are not allowed"));
                }
                let is_target = index + 1 == components.len();
                if !is_target && !metadata.is_dir() {
                    return Err(invalid_path("media parent must be a real directory"));
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound && target_may_be_missing => {
                return Ok(());
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn ensure_parent(root: &Dir, relative: &Path) -> io::Result<()> {
    let parent = relative
        .parent()
        .ok_or_else(|| invalid_path("media path has no parent"))?;
    if parent.as_os_str().is_empty() {
        return Ok(());
    }
    reject_symlinks(root, parent, true)?;
    root.create_dir_all(parent)?;
    reject_symlinks(root, parent, false)
}

fn sha256_reader(reader: impl Read) -> io::Result<String> {
    let mut reader = BufReader::with_capacity(HASH_BUFFER_BYTES, reader);
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; HASH_BUFFER_BYTES];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    let digest = digest.finalize();
    Ok(format!("{digest:x}"))
}

fn owner_and_sidecar(path: &str) -> io::Result<(PathBuf, PathBuf)> {
    if source_type(path).is_none() {
        return Err(invalid_path("unsupported media path"));
    }
    let owner = validate_relative(path)?;
    let sidecar = validate_relative(&sidecar_path(path))?;
    Ok((owner, sidecar))
}

fn temporary_path(target: &Path) -> io::Result<PathBuf> {
    let parent = target
        .parent()
        .ok_or_else(|| invalid_path("media target has no parent"))?;
    Ok(parent.join(format!(".nolune-memory-{}.tmp", uuid::Uuid::new_v4())))
}

fn read_bounded(root: &Dir, path: &Path, max_bytes: usize) -> io::Result<Vec<u8>> {
    let file = root.open(path)?;
    if !file.metadata()?.is_file() {
        return Err(invalid_path("target must be a regular file"));
    }
    read_bounded_file(file, max_bytes)
}

fn read_bounded_file(mut file: cap_std::fs::File, max_bytes: usize) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    std::io::Read::by_ref(&mut file)
        .take(
            u64::try_from(max_bytes)
                .unwrap_or(u64::MAX)
                .saturating_add(1),
        )
        .read_to_end(&mut bytes)?;
    if bytes.len() > max_bytes {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "file is too large",
        ));
    }
    Ok(bytes)
}

fn open_real_child_dir(parent: &Dir, name: &str) -> io::Result<Option<Dir>> {
    let mut options = OpenOptions::new();
    options
        .read(true)
        .maybe_dir(true)
        .follow(FollowSymlinks::No);
    match parent.open_with(name, &options) {
        Ok(file) if file.metadata()?.is_dir() => Ok(Some(Dir::from_std_file(file.into_std()))),
        Ok(_) => Ok(None),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => match parent.symlink_metadata(name) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => Ok(None),
            _ => Err(error),
        },
    }
}

fn read_named_regular_file(dir: &Dir, name: &str, max_bytes: usize) -> io::Result<Option<Vec<u8>>> {
    let Some(file) = open_named_regular_file(dir, name)? else {
        return Ok(None);
    };
    read_bounded_file(file, max_bytes).map(Some)
}

fn open_named_regular_file(dir: &Dir, name: &str) -> io::Result<Option<cap_std::fs::File>> {
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    match dir.open_with(name, &options) {
        Ok(file) if file.metadata()?.is_file() => Ok(Some(file)),
        Ok(_) => Ok(None),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => match dir.symlink_metadata(name) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => Ok(None),
            _ => Err(error),
        },
    }
}

fn finish_aggregated(errors: Vec<String>) -> io::Result<()> {
    if errors.is_empty() {
        Ok(())
    } else {
        Err(io::Error::other(errors.join("; ")))
    }
}

fn sync_capability_dir(dir: &Dir) -> io::Result<()> {
    dir.open(".")?.sync_all()
}

fn atomic_publish(
    root: &Dir,
    target: &Path,
    write: impl FnOnce(&mut cap_std::fs::File) -> io::Result<()>,
) -> io::Result<()> {
    ensure_parent(root, target)?;
    reject_symlinks(root, target, true)?;
    let temporary = temporary_path(target)?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    let mut file = root.open_with(&temporary, &options)?;
    let write_result = (|| {
        write(&mut file)?;
        file.flush()?;
        file.sync_all()
    })();
    drop(file);
    if let Err(error) = write_result {
        let _ = root.remove_file(&temporary);
        return Err(error);
    }
    let result = root.rename(&temporary, root, target);
    if result.is_err() {
        let _ = root.remove_file(&temporary);
    }
    result
}

/// Copy owner bytes into a create-new same-directory temporary file, fsync it,
/// and atomically replace the destination through the memory directory handle.
#[cfg(test)]
pub fn publish_owner(memory_dir: &Path, path: &str, source: &mut impl Read) -> io::Result<()> {
    let (owner, _) = owner_and_sidecar(path)?;
    let root = open_root(memory_dir)?;
    atomic_publish(&root, &owner, |file| {
        io::copy(source, file)?;
        Ok(())
    })
}

/// Atomically persist agent-authored text bound to the current owner bytes.
#[cfg(test)]
pub fn write(memory_dir: &Path, path: &str, content: &str) -> io::Result<()> {
    let (owner, sidecar) = owner_and_sidecar(path)?;
    let root = open_root(memory_dir)?;
    reject_symlinks(&root, &owner, false)?;
    if content.trim().is_empty() {
        reject_symlinks(&root, &sidecar, true)?;
        return remove_one_if_present(&root, &sidecar);
    }
    let owner_file = root.open(&owner)?;
    if !owner_file.metadata()?.is_file() {
        return Err(invalid_path("media owner must be a regular file"));
    }
    let header = Header {
        version: VERSION,
        sha256: sha256_reader(owner_file)?,
    };

    let encoded_header = serde_json::to_string(&header).map_err(io::Error::other)?;
    atomic_publish(&root, &sidecar, |temporary| {
        write!(temporary, "{HEADER_PREFIX}{encoded_header}\n{content}")
    })
}

/// Read text only when the sidecar is valid and still matches the current owner.
#[cfg(test)]
pub fn read(memory_dir: &Path, path: &str) -> Result<String, String> {
    let (owner, sidecar) = owner_and_sidecar(path).map_err(|error| error.to_string())?;
    let root = open_root(memory_dir).map_err(|error| error.to_string())?;
    reject_symlinks(&root, &owner, false).map_err(|error| error.to_string())?;
    reject_symlinks(&root, &sidecar, false).map_err(|error| error.to_string())?;
    let persisted = root
        .read_to_string(&sidecar)
        .map_err(|_| "media text representation is missing or unreadable".to_owned())?;
    let (header_line, content) = persisted
        .split_once('\n')
        .ok_or_else(|| "media text representation header is malformed".to_owned())?;
    let encoded_header = header_line
        .strip_prefix(HEADER_PREFIX)
        .ok_or_else(|| "media text representation header is malformed".to_owned())?;
    let header: Header = serde_json::from_str(encoded_header)
        .map_err(|_| "media text representation header is malformed".to_owned())?;
    if header.version != VERSION {
        return Err("media text representation version is unsupported".into());
    }
    if header.sha256.len() != 64 || !header.sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("media text representation digest is malformed".into());
    }
    let owner_file = root.open(&owner).map_err(|error| error.to_string())?;
    if !owner_file
        .metadata()
        .map_err(|error| error.to_string())?
        .is_file()
    {
        return Err("media owner must be a regular file".into());
    }
    let current_digest = sha256_reader(owner_file).map_err(|error| error.to_string())?;
    if !header.sha256.eq_ignore_ascii_case(&current_digest) {
        return Err("media text representation digest does not match its owner".into());
    }
    if content.trim().is_empty() {
        return Err("media text representation is empty".into());
    }
    Ok(content.to_owned())
}

/// Remove a retired directory of regular files. Symlinks and nested
/// directories are left in place and reported so nothing outside the
/// workspace can be affected.
fn remove_legacy_dir(parent: &Dir, name: &str) -> io::Result<()> {
    let Some(dir) = open_real_child_dir(parent, name)? else {
        return Ok(());
    };
    let mut errors = Vec::new();
    let mut names = Vec::new();
    for (index, entry) in dir.read_dir(".")?.enumerate() {
        if index >= MAX_LEGACY_CLEANUP_ENTRIES {
            errors.push("entry limit exceeded".to_owned());
            break;
        }
        match entry {
            Ok(entry) => names.push(entry.file_name()),
            Err(error) => errors.push(format!("entry: {error}")),
        }
    }
    for entry_name in names {
        let label = entry_name.to_string_lossy().into_owned();
        let is_regular = dir
            .symlink_metadata(&entry_name)
            .map(|metadata| metadata.is_file())
            .unwrap_or(false);
        if !is_regular {
            errors.push(format!("{label}: unexpected entry left in place"));
            continue;
        }
        if let Err(error) = dir.remove_file(&entry_name) {
            errors.push(format!("{label}: {error}"));
        }
    }
    drop(dir);
    if errors.is_empty() {
        parent.remove_dir(name)?;
    }
    finish_aggregated(errors)
}

fn remove_one_if_present(root: &Dir, path: &Path) -> io::Result<()> {
    match root.remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// Reconcile filesystem deletion. Owner and sidecar cleanup are independent and
/// missing files are already-clean success. A sidecar path preserves its owner.
#[cfg(test)]
pub fn remove(memory_dir: &Path, path: &str) -> io::Result<()> {
    validate_relative(path)?;
    let root = match open_root(memory_dir) {
        Ok(root) => root,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    let mut targets = Vec::new();
    if source_type(path).is_some() {
        targets.push(sidecar_path(path));
        targets.push(path.to_owned());
    } else {
        targets.push(path.to_owned());
    }

    let mut errors = Vec::new();
    for target in targets {
        let relative = validate_relative(&target);
        let result = relative.and_then(|relative| {
            reject_symlinks(&root, &relative, true)?;
            remove_one_if_present(&root, &relative)
        });
        match result {
            Ok(()) => {}
            Err(error) => errors.push(format!("{target}: {error}")),
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(io::Error::other(errors.join("; ")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("photo.png"), b"owner bytes").unwrap();
        root
    }

    #[test]
    fn v1_association_and_source_types() {
        assert_eq!(VERSION, 1);
        for (path, kind) in [
            ("moments/photo.JPG", "media_image"),
            ("photo.jpeg", "media_image"),
            ("photo.png", "media_image"),
            ("photo.webp", "media_image"),
            ("photo.gif", "media_image"),
            ("drawing.svg", "media_image"),
            ("documents/report.PDF", "media_document"),
            ("clip.mp4", "media_video"),
            ("clip.MOV", "media_video"),
            ("voice.mp3", "media_audio"),
            ("voice.WAV", "media_audio"),
        ] {
            assert_eq!(source_type(path), Some(kind));
            assert_eq!(media_path(&sidecar_path(path)), Some(path));
        }
        for path in ["note.md", "notes.pdf.md", "archive.zip", "photo"] {
            assert_eq!(source_type(path), None);
        }
        assert_eq!(media_path("report.pdf.notes.md"), None);
    }

    #[test]
    fn representation_roundtrip_is_versioned_and_bound_to_owner_bytes() {
        let root = fixture();
        write(
            root.path(),
            "photo.png",
            "user text\nversion: 99\nsha256: fake",
        )
        .unwrap();
        let persisted = std::fs::read_to_string(root.path().join("photo.png.md")).unwrap();
        assert!(persisted.starts_with("NOLUNE_MEDIA_TEXT "));
        assert_eq!(
            read(root.path(), "photo.png").unwrap(),
            "user text\nversion: 99\nsha256: fake"
        );
    }

    #[test]
    fn representation_refuses_digest_mismatch_and_malformed_header() {
        let root = fixture();
        write(root.path(), "photo.png", "old description").unwrap();
        std::fs::write(root.path().join("photo.png"), b"replacement bytes").unwrap();
        assert!(
            read(root.path(), "photo.png")
                .unwrap_err()
                .contains("digest")
        );

        std::fs::write(root.path().join("photo.png.md"), "plain legacy text").unwrap();
        assert!(
            read(root.path(), "photo.png")
                .unwrap_err()
                .contains("header")
        );
    }

    #[test]
    fn all_entrypoints_reject_unsafe_relative_paths() {
        let root = fixture();
        for path in [
            "/tmp/photo.png",
            "../photo.png",
            "nested\\photo.png",
            "C:/photo.png",
            "./photo.png",
            "nested//photo.png",
            "",
        ] {
            let mut source = io::Cursor::new(b"owner bytes");
            assert!(
                write(root.path(), path, "text").is_err(),
                "write accepted {path:?}"
            );
            assert!(read(root.path(), path).is_err(), "read accepted {path:?}");
            assert!(
                publish_owner(root.path(), path, &mut source).is_err(),
                "owner write accepted {path:?}"
            );
            assert!(
                remove(root.path(), path).is_err(),
                "remove accepted {path:?}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn all_entrypoints_reject_symlink_files_and_directories() {
        use std::os::unix::fs::symlink;

        let root = fixture();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("escape.png"), b"outside").unwrap();
        symlink(outside.path(), root.path().join("linked")).unwrap();
        symlink(
            outside.path().join("escape.png"),
            root.path().join("linked-file.png"),
        )
        .unwrap();

        for path in ["linked/escape.png", "linked-file.png"] {
            let mut source = io::Cursor::new(b"owner bytes");
            assert!(
                write(root.path(), path, "text").is_err(),
                "write accepted {path}"
            );
            assert!(read(root.path(), path).is_err(), "read accepted {path}");
            assert!(
                publish_owner(root.path(), path, &mut source).is_err(),
                "owner write accepted {path}"
            );
            assert!(remove(root.path(), path).is_err(), "remove accepted {path}");
        }
        assert_eq!(
            std::fs::read(outside.path().join("escape.png")).unwrap(),
            b"outside"
        );
    }

    #[cfg(unix)]
    #[test]
    fn empty_representation_write_rejects_a_symlink_sidecar() {
        use std::os::unix::fs::symlink;

        let root = fixture();
        let outside = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(outside.path(), b"outside sentinel").unwrap();
        symlink(outside.path(), root.path().join("photo.png.md")).unwrap();

        assert!(write(root.path(), "photo.png", "").is_err());
        assert!(root.path().join("photo.png.md").is_symlink());
        assert_eq!(std::fs::read(outside.path()).unwrap(), b"outside sentinel");
    }

    #[test]
    fn production_media_io_is_capability_relative() {
        let production = include_str!("media_text.rs")
            .split("#[cfg(test)]\nmod tests")
            .next()
            .unwrap();
        for forbidden in [
            "owner_path_for_write",
            "fs::read_to_string",
            "fs::remove_file",
            "NamedTempFile",
            "File::open",
        ] {
            assert!(
                !production.contains(forbidden),
                "production media I/O still contains pathname operation {forbidden}"
            );
        }
        for required in [".open_with(", ".rename(", ".remove_file("] {
            assert!(
                production.contains(required),
                "production media I/O does not use capability operation {required}"
            );
        }
        assert!(production.contains("#[cfg(test)]\nfn open_root"));
        assert_eq!(production.matches("Dir::open_ambient_dir").count(), 2);

        let callsites = [
            (include_str!("vector.rs"), "#[cfg(test)]\nmod tests"),
            (include_str!("keyword_search.rs"), "#[cfg(test)]\nmod tests"),
            (
                include_str!("memory.rs"),
                "#[cfg(test)]\nmod strict_scan_tests",
            ),
            (
                include_str!("tools/memory_tools.rs"),
                "#[cfg(test)]\nmod embedding_fallback_tests",
            ),
            (
                include_str!("../routes/instances.rs"),
                "#[cfg(test)]\nmod media_tests",
            ),
        ];
        for (source, test_marker) in callsites {
            let source = source.split(test_marker).next().unwrap();
            for forbidden in [
                "media_text::read(",
                "media_text::write(",
                "media_text::remove(",
                "media_text::publish_owner(",
                "cleanup_empty_dirs(",
                "fs::copy(",
                "Dir::open_ambient_dir(",
            ] {
                assert!(
                    !source.contains(forbidden),
                    "production media call site contains {forbidden}"
                );
            }
        }
        let routes = include_str!("../routes/instances.rs")
            .split("#[cfg(test)]\nmod media_tests")
            .next()
            .unwrap();
        assert!(!routes.contains("tokio::fs::read("));

        let memory = include_str!("memory.rs")
            .split("#[cfg(test)]\nmod strict_scan_tests")
            .next()
            .unwrap();
        for forbidden in [
            "std::fs::",
            "fs::read",
            "fs::write",
            "fs::create_dir",
            "workspace_dir.join(\"instances\")",
        ] {
            assert!(
                !memory.contains(forbidden),
                "production memory code contains ambient flow {forbidden}"
            );
        }
        for required in ["read_instance_text", "write_instance_text"] {
            assert!(
                memory.contains(required),
                "derived memory persistence must use {required}"
            );
        }

        let graph_callsite_sources = [
            include_str!("chat.rs")
                .split("#[cfg(test)]")
                .next()
                .unwrap(),
            include_str!("tools/memory_tools.rs")
                .split("#[cfg(test)]\nmod embedding_fallback_tests")
                .next()
                .unwrap(),
            routes,
        ];
        for source in graph_callsite_sources {
            for forbidden in [
                "load_graph(&state.workspace_dir",
                "load_graph(workspace_dir",
                "save_graph(&self.workspace_dir",
                "add_edge(&self.workspace_dir",
                "MemoryConnectTool::new(workspace_dir",
            ] {
                assert!(
                    !source.contains(forbidden),
                    "production graph call site contains ambient flow {forbidden}"
                );
            }
        }

        let main = include_str!("../main.rs");
        assert!(!main.contains("std::fs::read_dir"));
        assert!(main.contains("services::companion::obsolete_instance_dirs("));
        let companion = include_str!("companion.rs");
        assert!(companion.contains("store.instance_slugs()"));
        assert!(!companion.contains("fs::read_dir"));

        let import_route = routes
            .split("async fn import_instance")
            .nth(1)
            .expect("import route")
            .split("#[cfg(test)]")
            .next()
            .unwrap();
        for forbidden in ["create_dir", "Command::new", "tar", "multipart", "read("] {
            assert!(
                !import_route.contains(forbidden),
                "disabled import route contains unsafe operation {forbidden}"
            );
        }

        let restore = include_str!("tools/system.rs")
            .split("impl Tool for ImportProfileTool")
            .nth(1)
            .expect("restore tool")
            .split("#[cfg(test)]")
            .next()
            .unwrap();
        for forbidden in ["fs::read", "Command::new", "archive_path", "instance_dir"] {
            assert!(
                !restore.contains(forbidden),
                "disabled restore tool contains unsafe operation {forbidden}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn derived_instance_text_stays_bound_to_open_workspace_after_path_swap() {
        use std::os::unix::fs::symlink;

        let parent = tempfile::tempdir().unwrap();
        let workspace = parent.path().join("workspace");
        let parked = parent.path().join("parked");
        let outside = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(workspace.join("instances/one")).unwrap();
        std::fs::create_dir_all(outside.path().join("instances/one")).unwrap();
        std::fs::write(
            outside.path().join("instances/one/memory_graph.json"),
            b"outside sentinel",
        )
        .unwrap();
        let store = MediaStore::open(&workspace).unwrap();
        std::fs::rename(&workspace, &parked).unwrap();
        symlink(outside.path(), &workspace).unwrap();

        store
            .write_instance_text("one", "memory_graph.json", "inside graph")
            .unwrap();
        assert_eq!(
            store
                .read_instance_text("one", "memory_graph.json", 1024)
                .unwrap(),
            "inside graph"
        );
        assert_eq!(
            std::fs::read(outside.path().join("instances/one/memory_graph.json")).unwrap(),
            b"outside sentinel"
        );
    }

    #[cfg(unix)]
    #[test]
    fn derived_instance_text_rejects_symlink_target() {
        use std::os::unix::fs::symlink;

        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::NamedTempFile::new().unwrap();
        std::fs::create_dir_all(workspace.path().join("instances/one")).unwrap();
        std::fs::write(outside.path(), b"outside sentinel").unwrap();
        symlink(
            outside.path(),
            workspace.path().join("instances/one/memory_catalog.txt"),
        )
        .unwrap();
        let store = MediaStore::open(workspace.path()).unwrap();

        assert!(
            store
                .read_instance_text("one", "memory_catalog.txt", 1024)
                .is_err()
        );
        assert!(
            store
                .write_instance_text("one", "memory_catalog.txt", "replacement")
                .is_err()
        );
        assert_eq!(std::fs::read(outside.path()).unwrap(), b"outside sentinel");
    }

    #[cfg(unix)]
    #[test]
    fn derived_instance_text_rejects_instance_directory_swap() {
        use std::os::unix::fs::symlink;

        let workspace = tempfile::tempdir().unwrap();
        let instance = workspace.path().join("instances/one");
        let parked = workspace.path().join("instances/parked");
        let outside = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(&instance).unwrap();
        std::fs::write(instance.join("memory_graph.json"), b"inside graph").unwrap();
        std::fs::create_dir_all(outside.path().join("one")).unwrap();
        for name in ["memory_graph.json", "memory_catalog.txt"] {
            std::fs::write(outside.path().join("one").join(name), b"outside sentinel").unwrap();
        }
        let store = MediaStore::open(workspace.path()).unwrap();
        std::fs::rename(&instance, &parked).unwrap();
        symlink(outside.path().join("one"), &instance).unwrap();

        for name in ["memory_graph.json", "memory_catalog.txt"] {
            assert!(store.read_instance_text("one", name, 1024).is_err());
            assert!(
                store
                    .write_instance_text("one", name, "replacement")
                    .is_err()
            );
            assert_eq!(
                std::fs::read(outside.path().join("one").join(name)).unwrap(),
                b"outside sentinel"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn persistent_workspace_anchor_survives_path_replacement() {
        use std::os::unix::fs::symlink;

        let parent = tempfile::tempdir().unwrap();
        let workspace = parent.path().join("workspace");
        let parked = parent.path().join("parked");
        let outside = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(workspace.join("instances/one/memory")).unwrap();
        std::fs::create_dir_all(outside.path().join("instances/one/memory")).unwrap();
        std::fs::write(
            outside.path().join("instances/one/memory/photo.png"),
            b"outside sentinel",
        )
        .unwrap();

        let store = MediaStore::open(&workspace).unwrap();
        std::fs::rename(&workspace, &parked).unwrap();
        symlink(outside.path(), &workspace).unwrap();

        store
            .publish_owner("one", "photo.png", &mut io::Cursor::new(b"inside owner"))
            .unwrap();
        store.write("one", "photo.png", "inside text").unwrap();
        assert_eq!(store.read("one", "photo.png").unwrap(), "inside text");
        assert_eq!(
            store.read_memory_file("one", "photo.png", 1024).unwrap(),
            b"inside owner"
        );
        store.remove("one", "photo.png").unwrap();

        assert!(!parked.join("instances/one/memory/photo.png").exists());
        assert_eq!(
            std::fs::read(outside.path().join("instances/one/memory/photo.png")).unwrap(),
            b"outside sentinel"
        );
    }

    #[cfg(unix)]
    #[test]
    fn persistent_store_rejects_preexisting_intermediate_symlink() {
        use std::os::unix::fs::symlink;

        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(workspace.path().join("instances/one/memory")).unwrap();
        std::fs::write(outside.path().join("photo.png"), b"outside sentinel").unwrap();
        symlink(
            outside.path(),
            workspace.path().join("instances/one/memory/linked"),
        )
        .unwrap();
        let store = MediaStore::open(workspace.path()).unwrap();

        assert!(
            store
                .read_memory_file("one", "linked/photo.png", 1024)
                .is_err()
        );
        assert!(
            store
                .publish_owner(
                    "one",
                    "linked/photo.png",
                    &mut io::Cursor::new(b"replacement"),
                )
                .is_err()
        );
        assert_eq!(
            std::fs::read(outside.path().join("photo.png")).unwrap(),
            b"outside sentinel"
        );
    }

    #[test]
    fn persistent_store_validates_slug_and_bounded_reads() {
        let workspace = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(workspace.path().join("instances/one/memory")).unwrap();
        std::fs::write(
            workspace.path().join("instances/one/memory/note.md"),
            b"12345",
        )
        .unwrap();
        let store = MediaStore::open(workspace.path()).unwrap();

        for slug in ["", ".", "..", "a/b", "a\\b", "C:"] {
            assert!(store.read_memory_file(slug, "note.md", 1024).is_err());
        }
        assert!(store.read_memory_file("one", "note.md", 4).is_err());
        assert_eq!(
            store.read_memory_file("one", "note.md", 5).unwrap(),
            b"12345"
        );
    }

    #[test]
    fn owner_publication_copies_through_the_memory_capability() {
        let root = tempfile::tempdir().unwrap();
        let source = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(source.path(), b"new owner bytes").unwrap();

        let mut source_file = std::fs::File::open(source.path()).unwrap();
        publish_owner(root.path(), "nested/photo.png", &mut source_file).unwrap();

        assert_eq!(
            std::fs::read(root.path().join("nested/photo.png")).unwrap(),
            b"new owner bytes"
        );
    }

    #[cfg(unix)]
    #[test]
    fn directory_symlink_swap_cannot_escape_memory_capability() {
        use std::{
            os::unix::fs::symlink,
            sync::{Arc, Barrier},
            thread,
        };

        const ITERATIONS: usize = 2_000;
        let root = tempfile::tempdir().unwrap();
        let memory = root.path().join("instances/one/memory");
        let outside = tempfile::tempdir().unwrap();
        let source = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(source.path(), b"inside replacement").unwrap();
        std::fs::create_dir_all(memory.join("nested")).unwrap();
        std::fs::write(memory.join("nested/photo.png"), b"inside owner").unwrap();
        let store = MediaStore::open(root.path()).unwrap();
        store
            .write("one", "nested/photo.png", "inside text")
            .unwrap();

        let outside_owner = outside.path().join("photo.png");
        let outside_sidecar = outside.path().join("photo.png.md");
        std::fs::write(&outside_owner, b"outside sentinel owner").unwrap();
        let digest = format!("{:x}", Sha256::digest(b"outside sentinel owner"));
        std::fs::write(
            &outside_sidecar,
            format!(
                "{HEADER_PREFIX}{{\"version\":{VERSION},\"sha256\":\"{digest}\"}}\noutside sentinel text"
            ),
        )
        .unwrap();
        let expected_owner = std::fs::read(&outside_owner).unwrap();
        let expected_sidecar = std::fs::read(&outside_sidecar).unwrap();

        let start = Arc::new(Barrier::new(2));
        let finish = Arc::new(Barrier::new(2));
        let swap_root = memory;
        let swap_outside = outside.path().to_owned();
        let swap_start = start.clone();
        let swap_finish = finish.clone();
        let swapper = thread::spawn(move || {
            let nested = swap_root.join("nested");
            let parked = swap_root.join("parked");
            swap_start.wait();
            for _ in 0..ITERATIONS {
                if std::fs::rename(&nested, &parked).is_ok() {
                    if symlink(&swap_outside, &nested).is_ok() {
                        thread::yield_now();
                        let _ = std::fs::remove_file(&nested);
                    }
                    let _ = std::fs::rename(&parked, &nested);
                }
            }
            swap_finish.wait();
        });

        start.wait();
        for iteration in 0..ITERATIONS {
            let mut source_file = std::fs::File::open(source.path()).unwrap();
            let _ = store.publish_owner("one", "nested/photo.png", &mut source_file);
            let _ = store.write("one", "nested/photo.png", "inside text");
            if let Ok(text) = store.read("one", "nested/photo.png") {
                assert_ne!(text, "outside sentinel text", "escaped on read {iteration}");
            }
            let _ = store.remove("one", "nested/photo.png");
        }
        finish.wait();
        swapper.join().unwrap();

        assert_eq!(std::fs::read(&outside_owner).unwrap(), expected_owner);
        assert_eq!(std::fs::read(&outside_sidecar).unwrap(), expected_sidecar);
    }

    fn upload_metadata(id: &str, original_name: &str, stored_name: &str, size: u64) -> String {
        let uploaded_at = parse_prefixed_millis(id, "upload_", ".mp4")
            .map(|millis| millis.to_string())
            .unwrap_or_else(|| "1".to_owned());
        serde_json::json!({
            "id": id,
            "original_name": original_name,
            "stored_name": stored_name,
            "mime_type": "video/mp4",
            "size": size,
            "uploaded_at": uploaded_at
        })
        .to_string()
    }

    fn observation(id: &str, upload_id: &str) -> String {
        let created_at = parse_prefixed_millis(id, "obs_", "")
            .map(|millis| millis.to_string())
            .unwrap_or_else(|| "1".to_owned());
        serde_json::json!({
            "id": id,
            "upload_id": upload_id,
            "machine_id": "machine-one",
            "analysis": "legacy analysis",
            "created_at": created_at
        })
        .to_string()
    }

    fn seed_strict_legacy_pair(
        workspace: &Path,
        slug: &str,
        observation_millis: u64,
        upload_millis: u64,
    ) -> (PathBuf, PathBuf, String, String, String) {
        let instance = workspace.join("instances").join(slug);
        let observations = instance.join("observations");
        let uploads = instance.join("uploads");
        std::fs::create_dir_all(&observations).unwrap();
        std::fs::create_dir_all(&uploads).unwrap();
        let observation_id = format!("obs_{observation_millis}");
        let upload_id = format!("upload_{upload_millis}.mp4");
        let stored_name = format!("{upload_id}_blob.mp4");
        std::fs::write(uploads.join(&stored_name), b"data").unwrap();
        std::fs::write(
            uploads.join(format!("{upload_id}.json")),
            upload_metadata(&upload_id, "screen_recording.mp4", &stored_name, 4),
        )
        .unwrap();
        std::fs::write(
            observations.join(format!("{observation_id}.json")),
            observation(&observation_id, &upload_id),
        )
        .unwrap();
        (
            observations,
            uploads,
            observation_id,
            upload_id,
            stored_name,
        )
    }

    #[test]
    fn cleanup_reconciles_only_strictly_referenced_recording_pairs_and_is_idempotent() {
        let workspace = tempfile::tempdir().unwrap();
        let instance = workspace.path().join("instances/one");
        let observations = instance.join("observations");
        let uploads = instance.join("uploads");
        std::fs::create_dir_all(&observations).unwrap();
        std::fs::create_dir_all(&uploads).unwrap();

        let recording_id = "upload_1000.mp4";
        let recording_blob = "upload_1000.mp4_blob.mp4";
        std::fs::write(uploads.join(recording_blob), b"legacy").unwrap();
        std::fs::write(
            uploads.join(format!("{recording_id}.json")),
            upload_metadata(recording_id, "screen_recording.mp4", recording_blob, 6),
        )
        .unwrap();
        std::fs::write(
            observations.join("obs_1001.json"),
            observation("obs_1001", recording_id),
        )
        .unwrap();

        // A confirmed observation whose blob is already absent is retry-clean.
        let missing_id = "upload_2000.mp4";
        std::fs::write(
            uploads.join(format!("{missing_id}.json")),
            upload_metadata(
                missing_id,
                "screen_recording.mp4",
                "upload_2000.mp4_blob.mp4",
                4,
            ),
        )
        .unwrap();
        std::fs::write(
            observations.join("obs_2001.json"),
            observation("obs_2001", missing_id),
        )
        .unwrap();

        // Filename alone is never enough: this exact-looking pair is unreferenced.
        let unreferenced_id = "upload_3000.mp4";
        let unreferenced_blob = "upload_3000.mp4_blob.mp4";
        std::fs::write(uploads.join(unreferenced_blob), b"keep").unwrap();
        std::fs::write(
            uploads.join(format!("{unreferenced_id}.json")),
            upload_metadata(
                unreferenced_id,
                "screen_recording.mp4",
                unreferenced_blob,
                4,
            ),
        )
        .unwrap();

        // Malformed, extra-field, and filename/id-mismatched observations are preserved.
        std::fs::write(observations.join("malformed.json"), "{").unwrap();
        let mut extra: serde_json::Value =
            serde_json::from_str(&observation("obs_3001", unreferenced_id)).unwrap();
        extra["unexpected"] = serde_json::json!(true);
        std::fs::write(observations.join("extra.json"), extra.to_string()).unwrap();
        std::fs::write(
            observations.join("wrong-filename.json"),
            observation("obs_3002", unreferenced_id),
        )
        .unwrap();

        let store = MediaStore::open(workspace.path()).unwrap();
        store.cleanup_legacy_screen_capture().unwrap();
        assert!(!uploads.join(recording_blob).exists());
        assert!(!uploads.join(format!("{recording_id}.json")).exists());
        assert!(!observations.join("obs_1001.json").exists());
        assert!(!uploads.join(format!("{missing_id}.json")).exists());
        assert!(!observations.join("obs_2001.json").exists());
        for path in [
            uploads.join(unreferenced_blob),
            uploads.join(format!("{unreferenced_id}.json")),
            observations.join("malformed.json"),
            observations.join("extra.json"),
            observations.join("wrong-filename.json"),
        ] {
            assert!(path.exists(), "{} must be preserved", path.display());
        }
        assert!(instance.join("observations").is_dir());

        store.cleanup_legacy_screen_capture().unwrap();
    }

    #[test]
    fn cleanup_preserves_ambiguous_duplicate_references_and_mismatched_pairs() {
        let workspace = tempfile::tempdir().unwrap();
        let instance = workspace.path().join("instances/one");
        let uploads = instance.join("uploads");
        let observations = instance.join("observations");
        std::fs::create_dir_all(&uploads).unwrap();
        std::fs::create_dir_all(&observations).unwrap();
        let duplicate_id = "upload_4000.mp4";
        let duplicate_blob = "upload_4000.mp4_blob.mp4";
        std::fs::write(uploads.join(duplicate_blob), b"keep").unwrap();
        std::fs::write(
            uploads.join(format!("{duplicate_id}.json")),
            upload_metadata(duplicate_id, "screen_recording.mp4", duplicate_blob, 4),
        )
        .unwrap();
        std::fs::write(
            observations.join("obs_4001.json"),
            observation("obs_4001", duplicate_id),
        )
        .unwrap();
        std::fs::write(
            observations.join("obs_4002.json"),
            observation("obs_4002", duplicate_id),
        )
        .unwrap();
        let wrong_size_id = "upload_5000.mp4";
        let wrong_size_blob = "upload_5000.mp4_blob.mp4";
        std::fs::write(uploads.join(wrong_size_blob), b"two").unwrap();
        std::fs::write(
            uploads.join(format!("{wrong_size_id}.json")),
            upload_metadata(wrong_size_id, "screen_recording.mp4", wrong_size_blob, 99),
        )
        .unwrap();
        std::fs::write(
            observations.join("obs_5001.json"),
            observation("obs_5001", wrong_size_id),
        )
        .unwrap();

        let store = MediaStore::open(workspace.path()).unwrap();
        store.cleanup_legacy_screen_capture().unwrap();
        for path in [
            uploads.join(duplicate_blob),
            uploads.join(format!("{duplicate_id}.json")),
            observations.join("obs_4001.json"),
            observations.join("obs_4002.json"),
            uploads.join(wrong_size_blob),
            uploads.join(format!("{wrong_size_id}.json")),
            observations.join("obs_5001.json"),
        ] {
            assert!(path.exists(), "{} must be preserved", path.display());
        }
    }

    #[test]
    fn cleanup_preserves_ambiguity_across_normal_observations_and_tombstones() {
        let workspace = tempfile::tempdir().unwrap();
        let (observations, uploads, _, upload_id, stored_name) =
            seed_strict_legacy_pair(workspace.path(), "one", 5501, 5500);
        std::fs::write(
            observations.join(legacy_cleanup_tombstone_name("obs_5502")),
            observation("obs_5502", &upload_id),
        )
        .unwrap();

        let store = MediaStore::open(workspace.path()).unwrap();
        store.cleanup_legacy_screen_capture().unwrap();
        assert!(observations.join("obs_5501.json").exists());
        assert!(
            observations
                .join(legacy_cleanup_tombstone_name("obs_5502"))
                .exists()
        );
        assert!(uploads.join(format!("{upload_id}.json")).exists());
        assert!(uploads.join(stored_name).exists());
    }

    #[test]
    fn cleanup_counts_schema_valid_preserved_references_as_ambiguous() {
        let workspace = tempfile::tempdir().unwrap();
        let (observations, uploads, _, upload_id, stored_name) =
            seed_strict_legacy_pair(workspace.path(), "one", 5551, 5550);
        std::fs::write(
            observations.join("custom.json"),
            observation("custom", &upload_id),
        )
        .unwrap();

        let store = MediaStore::open(workspace.path()).unwrap();
        store.cleanup_legacy_screen_capture().unwrap();
        assert!(observations.join("obs_5551.json").exists());
        assert!(observations.join("custom.json").exists());
        assert!(uploads.join(format!("{upload_id}.json")).exists());
        assert!(uploads.join(stored_name).exists());
    }

    #[cfg(unix)]
    #[test]
    fn cleanup_never_replaces_a_preexisting_tombstone_symlink() {
        use std::os::unix::fs::symlink;

        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(outside.path(), b"outside sentinel").unwrap();
        let (observations, uploads, observation_id, upload_id, stored_name) =
            seed_strict_legacy_pair(workspace.path(), "one", 5561, 5560);
        let tombstone = observations.join(legacy_cleanup_tombstone_name(&observation_id));
        symlink(outside.path(), &tombstone).unwrap();

        let store = MediaStore::open(workspace.path()).unwrap();
        store.cleanup_legacy_screen_capture().unwrap();
        assert!(observations.join(format!("{observation_id}.json")).exists());
        assert!(
            tombstone
                .symlink_metadata()
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(std::fs::read(outside.path()).unwrap(), b"outside sentinel");
        assert!(uploads.join(format!("{upload_id}.json")).exists());
        assert!(uploads.join(stored_name).exists());
    }

    #[test]
    fn cleanup_requires_exact_bounded_ascii_producer_identifiers() {
        assert_eq!(parse_ascii_millis("0"), Some(0));
        assert_eq!(
            parse_ascii_millis("340282366920938463463374607431768211455"),
            Some(u128::MAX)
        );
        for invalid in [
            "",
            "01",
            "١",
            "１２",
            "+1",
            "-1",
            "1.0",
            "340282366920938463463374607431768211456",
            "1000000000000000000000000000000000000000",
        ] {
            assert_eq!(parse_ascii_millis(invalid), None, "accepted {invalid:?}");
        }
        assert_eq!(parse_prefixed_millis("obs_42", "obs_", ""), Some(42));
        assert_eq!(
            parse_prefixed_millis("upload_42.mp4", "upload_", ".mp4"),
            Some(42)
        );
        for invalid in ["obs-anything", "obs_01", "obs_١", "obs_1.json"] {
            assert_eq!(parse_prefixed_millis(invalid, "obs_", ""), None);
        }
        for invalid in [
            "upload_1",
            "upload_1.mov",
            "upload_01.mp4",
            "upload_١.mp4",
            "anything",
        ] {
            assert_eq!(parse_prefixed_millis(invalid, "upload_", ".mp4"), None);
        }

        let workspace = tempfile::tempdir().unwrap();
        let instance = workspace.path().join("instances/one");
        let observations = instance.join("observations");
        let uploads = instance.join("uploads");
        std::fs::create_dir_all(&observations).unwrap();
        std::fs::create_dir_all(&uploads).unwrap();
        std::fs::write(uploads.join("custom_blob.mp4"), b"data").unwrap();
        std::fs::write(
            uploads.join("custom.json"),
            upload_metadata("custom", "screen_recording.mp4", "custom_blob.mp4", 4),
        )
        .unwrap();
        std::fs::write(
            observations.join("custom.json"),
            observation("custom", "custom"),
        )
        .unwrap();

        let store = MediaStore::open(workspace.path()).unwrap();
        store.cleanup_legacy_screen_capture().unwrap();
        assert!(observations.join("custom.json").exists());
        assert!(uploads.join("custom.json").exists());
        assert!(uploads.join("custom_blob.mp4").exists());
    }

    #[test]
    fn cleanup_preserves_pairs_with_any_non_producer_metadata_field() {
        for (index, field, invalid_value) in [
            (0_u64, "id", serde_json::json!("upload_9999.mp4")),
            (1, "original_name", serde_json::json!("recording.mp4")),
            (2, "stored_name", serde_json::json!("other.mp4")),
            (
                3,
                "mime_type",
                serde_json::json!("application/octet-stream"),
            ),
            (4, "size", serde_json::json!(5)),
            (5, "uploaded_at", serde_json::json!("9999")),
        ] {
            let workspace = tempfile::tempdir().unwrap();
            let (observations, uploads, observation_id, upload_id, stored_name) =
                seed_strict_legacy_pair(workspace.path(), "one", 5601 + index, 5600 + index);
            let metadata_path = uploads.join(format!("{upload_id}.json"));
            let mut metadata: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&metadata_path).unwrap()).unwrap();
            metadata[field] = invalid_value;
            std::fs::write(&metadata_path, metadata.to_string()).unwrap();

            let store = MediaStore::open(workspace.path()).unwrap();
            store.cleanup_legacy_screen_capture().unwrap();
            assert!(observations.join(format!("{observation_id}.json")).exists());
            assert!(metadata_path.exists());
            assert!(uploads.join(stored_name).exists());
        }
    }

    #[test]
    fn cleanup_preserves_observation_with_non_ascii_or_noncanonical_created_at() {
        for (index, created_at) in ["١", "01", "+1", "340282366920938463463374607431768211456"]
            .into_iter()
            .enumerate()
        {
            let workspace = tempfile::tempdir().unwrap();
            let (observations, uploads, observation_id, upload_id, stored_name) =
                seed_strict_legacy_pair(
                    workspace.path(),
                    "one",
                    5701 + index as u64,
                    5700 + index as u64,
                );
            let observation_path = observations.join(format!("{observation_id}.json"));
            let mut value: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&observation_path).unwrap()).unwrap();
            value["created_at"] = serde_json::json!(created_at);
            std::fs::write(&observation_path, value.to_string()).unwrap();

            let store = MediaStore::open(workspace.path()).unwrap();
            store.cleanup_legacy_screen_capture().unwrap();
            assert!(observation_path.exists());
            assert!(uploads.join(format!("{upload_id}.json")).exists());
            assert!(uploads.join(stored_name).exists());
        }
    }

    #[test]
    fn cleanup_retry_converges_after_every_durable_transition() {
        for (index, step) in ["rename", "blob", "metadata", "tombstone"]
            .into_iter()
            .enumerate()
        {
            let workspace = tempfile::tempdir().unwrap();
            let (observations, uploads, observation_id, upload_id, stored_name) =
                seed_strict_legacy_pair(
                    workspace.path(),
                    "one",
                    6001 + index as u64,
                    6000 + index as u64,
                );
            let tombstone = observations.join(legacy_cleanup_tombstone_name(&observation_id));
            let store = MediaStore::open(workspace.path()).unwrap();
            store.inject_legacy_cleanup_failure("one", &format!("after-{step}"));

            let error = store.cleanup_legacy_screen_capture().unwrap_err();
            assert!(error.to_string().contains(&format!("after {step}")));
            assert!(!observations.join(format!("{observation_id}.json")).exists());
            match step {
                "rename" => {
                    assert!(uploads.join(&stored_name).exists());
                    assert!(uploads.join(format!("{upload_id}.json")).exists());
                    assert!(tombstone.exists());
                }
                "blob" => {
                    assert!(!uploads.join(&stored_name).exists());
                    assert!(uploads.join(format!("{upload_id}.json")).exists());
                    assert!(tombstone.exists());
                }
                "metadata" => {
                    assert!(!uploads.join(&stored_name).exists());
                    assert!(!uploads.join(format!("{upload_id}.json")).exists());
                    assert!(tombstone.exists());
                }
                "tombstone" => assert!(!tombstone.exists()),
                _ => unreachable!(),
            }

            store.cleanup_legacy_screen_capture().unwrap();
            assert!(!uploads.join(&stored_name).exists());
            assert!(!uploads.join(format!("{upload_id}.json")).exists());
            assert!(!tombstone.exists());
        }
    }

    #[test]
    fn cleanup_scan_failures_are_fail_closed_per_instance_and_other_instances_continue() {
        for point in ["scan-entry", "scan-file-type", "scan-read"] {
            let workspace = tempfile::tempdir().unwrap();
            let (one_observations, one_uploads, one_observation_id, one_upload_id, one_blob) =
                seed_strict_legacy_pair(workspace.path(), "one", 7001, 7000);
            let (_, two_uploads, _, two_upload_id, two_blob) =
                seed_strict_legacy_pair(workspace.path(), "two", 8001, 8000);
            let store = MediaStore::open(workspace.path()).unwrap();
            store.inject_legacy_cleanup_failure(
                "one",
                &format!("{point}:{one_observation_id}.json"),
            );

            let error = store.cleanup_legacy_screen_capture().unwrap_err();
            assert!(error.to_string().contains("injected"));
            assert!(
                one_observations
                    .join(format!("{one_observation_id}.json"))
                    .exists()
            );
            assert!(one_uploads.join(format!("{one_upload_id}.json")).exists());
            assert!(one_uploads.join(&one_blob).exists());
            assert!(!two_uploads.join(format!("{two_upload_id}.json")).exists());
            assert!(!two_uploads.join(&two_blob).exists());
        }
    }

    #[test]
    fn cleanup_aborts_instance_before_deleting_on_over_limit_duplicate_scan() {
        let workspace = tempfile::tempdir().unwrap();
        let instance = workspace.path().join("instances/one");
        let observations = instance.join("observations");
        let uploads = instance.join("uploads");
        std::fs::create_dir_all(&observations).unwrap();
        std::fs::create_dir_all(&uploads).unwrap();

        let upload_id = "upload_1000.mp4";
        let stored_name = "upload_1000.mp4_blob.mp4";
        std::fs::write(uploads.join(stored_name), b"data").unwrap();
        std::fs::write(
            uploads.join(format!("{upload_id}.json")),
            serde_json::json!({
                "id": upload_id,
                "original_name": "screen_recording.mp4",
                "stored_name": stored_name,
                "mime_type": "video/mp4",
                "size": 4,
                "uploaded_at": "1000"
            })
            .to_string(),
        )
        .unwrap();
        std::fs::write(
            observations.join("obs_1001.json"),
            observation("obs_1001", upload_id),
        )
        .unwrap();
        for index in 0..MAX_LEGACY_CLEANUP_ENTRIES {
            let duplicate_id = format!("obs_{}", 10_000 + index);
            std::fs::write(
                observations.join(format!("{duplicate_id}.json")),
                observation(&duplicate_id, upload_id),
            )
            .unwrap();
        }

        let store = MediaStore::open(workspace.path()).unwrap();
        let error = store.cleanup_legacy_screen_capture().unwrap_err();
        assert!(error.to_string().contains("entry limit exceeded"));
        assert!(observations.join("obs_1001.json").exists());
        assert!(uploads.join(format!("{upload_id}.json")).exists());
        assert!(uploads.join(stored_name).exists());
    }

    #[test]
    fn cleanup_aborts_instance_before_deleting_when_regular_file_read_is_incomplete() {
        let workspace = tempfile::tempdir().unwrap();
        let instance = workspace.path().join("instances/one");
        let observations = instance.join("observations");
        let uploads = instance.join("uploads");
        std::fs::create_dir_all(&observations).unwrap();
        std::fs::create_dir_all(&uploads).unwrap();

        let upload_id = "upload_2000.mp4";
        let stored_name = "upload_2000.mp4_blob.mp4";
        std::fs::write(uploads.join(stored_name), b"data").unwrap();
        std::fs::write(
            uploads.join(format!("{upload_id}.json")),
            serde_json::json!({
                "id": upload_id,
                "original_name": "screen_recording.mp4",
                "stored_name": stored_name,
                "mime_type": "video/mp4",
                "size": 4,
                "uploaded_at": "2000"
            })
            .to_string(),
        )
        .unwrap();
        std::fs::write(
            observations.join("obs_2001.json"),
            observation("obs_2001", upload_id),
        )
        .unwrap();
        std::fs::write(
            observations.join("unreadable.json"),
            vec![b'x'; MAX_LEGACY_OBSERVATION_BYTES + 1],
        )
        .unwrap();

        let store = MediaStore::open(workspace.path()).unwrap();
        let error = store.cleanup_legacy_screen_capture().unwrap_err();
        assert!(error.to_string().contains("file is too large"));
        assert!(observations.join("obs_2001.json").exists());
        assert!(uploads.join(format!("{upload_id}.json")).exists());
        assert!(uploads.join(stored_name).exists());
    }

    #[test]
    fn child_agent_cleanup_removes_retired_state_from_the_companion_only() {
        let workspace = tempfile::tempdir().unwrap();
        let companion = workspace
            .path()
            .join("instances")
            .join(crate::domain::companion::CANONICAL_SLUG);
        let agents = companion.join("agents");
        let runs = companion.join("agent_runs");
        std::fs::create_dir_all(&agents).unwrap();
        std::fs::create_dir_all(&runs).unwrap();
        std::fs::write(agents.join("companion.toml"), b"legacy").unwrap();
        std::fs::write(agents.join(".last_run_companion"), b"1").unwrap();
        std::fs::write(agents.join("companion_history.json"), b"[]").unwrap();
        std::fs::write(runs.join("run_1.json"), b"{}").unwrap();
        std::fs::write(companion.join("soul.md"), b"keep").unwrap();
        let obsolete = workspace.path().join("instances/alice/agents");
        std::fs::create_dir_all(&obsolete).unwrap();
        std::fs::write(obsolete.join("custom.toml"), b"keep").unwrap();

        let store = MediaStore::open(workspace.path()).unwrap();
        store.cleanup_legacy_child_agents().unwrap();
        assert!(!agents.exists());
        assert!(!runs.exists());
        assert_eq!(std::fs::read(companion.join("soul.md")).unwrap(), b"keep");
        assert_eq!(
            std::fs::read(obsolete.join("custom.toml")).unwrap(),
            b"keep"
        );
        store.cleanup_legacy_child_agents().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn child_agent_cleanup_preserves_preexisting_agents_symlink_target() {
        use std::os::unix::fs::symlink;

        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let companion = workspace
            .path()
            .join("instances")
            .join(crate::domain::companion::CANONICAL_SLUG);
        std::fs::create_dir_all(&companion).unwrap();
        std::fs::write(outside.path().join("companion.toml"), b"outside sentinel").unwrap();
        symlink(outside.path(), companion.join("agents")).unwrap();

        let store = MediaStore::open(workspace.path()).unwrap();
        store.cleanup_legacy_child_agents().unwrap();
        assert_eq!(
            std::fs::read(outside.path().join("companion.toml")).unwrap(),
            b"outside sentinel"
        );
    }

    #[cfg(unix)]
    #[test]
    fn child_agent_cleanup_does_not_follow_swapped_instances_parent() {
        use std::os::unix::fs::symlink;

        let parent = tempfile::tempdir().unwrap();
        let workspace = parent.path().join("workspace");
        let parked_instances = parent.path().join("parked-instances");
        let outside = tempfile::tempdir().unwrap();
        let slug = crate::domain::companion::CANONICAL_SLUG;
        std::fs::create_dir_all(workspace.join("instances").join(slug).join("agents")).unwrap();
        std::fs::create_dir_all(outside.path().join(slug).join("agents")).unwrap();
        std::fs::write(
            outside.path().join(slug).join("agents/companion.toml"),
            b"outside sentinel",
        )
        .unwrap();
        let store = MediaStore::open(&workspace).unwrap();
        std::fs::rename(workspace.join("instances"), &parked_instances).unwrap();
        symlink(outside.path(), workspace.join("instances")).unwrap();

        store.cleanup_legacy_child_agents().unwrap();
        assert_eq!(
            std::fs::read(outside.path().join(slug).join("agents/companion.toml")).unwrap(),
            b"outside sentinel"
        );
    }

    #[cfg(unix)]
    #[test]
    fn cleanup_does_not_follow_observation_or_upload_symlinks() {
        use std::os::unix::fs::symlink;

        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let instance = workspace.path().join("instances/one");
        let uploads = instance.join("uploads");
        std::fs::create_dir_all(&uploads).unwrap();
        std::fs::write(outside.path().join("sentinel"), b"keep").unwrap();
        symlink(outside.path(), instance.join("observations")).unwrap();
        symlink(outside.path().join("sentinel"), uploads.join("linked.mp4")).unwrap();
        std::fs::write(
            uploads.join("linked.json"),
            upload_metadata("linked", "screen_recording.mp4", "linked.mp4", 4),
        )
        .unwrap();

        let store = MediaStore::open(workspace.path()).unwrap();
        store.cleanup_legacy_screen_capture().unwrap();
        assert_eq!(
            std::fs::read(outside.path().join("sentinel")).unwrap(),
            b"keep"
        );
        assert!(instance.join("observations").symlink_metadata().is_ok());
        assert!(uploads.join("linked.json").exists());
        assert!(uploads.join("linked.mp4").symlink_metadata().is_ok());
    }

    #[test]
    fn upload_reader_binds_metadata_identity_name_mime_and_size() {
        let workspace = tempfile::tempdir().unwrap();
        let upload = crate::services::uploads::save_upload(
            workspace.path(),
            "one",
            "photo.png",
            b"safe bytes",
        )
        .unwrap();
        let uploads = workspace.path().join("instances/one/uploads");
        let metadata_path = uploads.join(format!("{}.json", upload.id));
        let original = std::fs::read(&metadata_path).unwrap();
        let store = MediaStore::open(workspace.path()).unwrap();
        assert_eq!(
            store.read_upload_inline("one", &upload.id).unwrap().1,
            b"safe bytes"
        );
        for (field, value) in [
            ("id", serde_json::json!("other.png")),
            ("stored_name", serde_json::json!("../outside.png")),
            ("stored_name", serde_json::json!("/tmp/outside.png")),
            ("mime_type", serde_json::json!("text/plain")),
            ("size", serde_json::json!(999)),
        ] {
            let mut metadata: serde_json::Value = serde_json::from_slice(&original).unwrap();
            metadata[field] = value;
            std::fs::write(&metadata_path, serde_json::to_vec(&metadata).unwrap()).unwrap();
            assert!(
                store.read_upload_inline("one", &upload.id).is_err(),
                "accepted {field}"
            );
        }
        std::fs::write(metadata_path, original).unwrap();
        for id in ["../photo.png", "/tmp/photo.png", "nested/photo.png"] {
            assert!(store.read_upload_inline("one", id).is_err());
        }
    }

    #[cfg(unix)]
    #[test]
    fn upload_reader_rejects_links_and_keeps_held_upload_directory_on_swap() {
        use std::os::unix::fs::symlink;
        let parent = tempfile::tempdir().unwrap();
        let workspace = parent.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        let upload =
            crate::services::uploads::save_upload(&workspace, "one", "photo.png", b"safe bytes")
                .unwrap();
        let uploads = workspace.join("instances/one/uploads");
        let store = MediaStore::open(&workspace).unwrap();
        assert_eq!(
            store.read_upload_inline("one", &upload.id).unwrap().1,
            b"safe bytes"
        );

        let parked = parent.path().join("parked-uploads");
        std::fs::rename(&uploads, &parked).unwrap();
        let outside = parent.path().join("outside");
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join(format!("{}.json", upload.id)), b"outside").unwrap();
        symlink(&outside, &uploads).unwrap();
        assert_eq!(
            store.read_upload_inline("one", &upload.id).unwrap().1,
            b"safe bytes"
        );

        let blob = parked.join(&upload.stored_name);
        std::fs::remove_file(&blob).unwrap();
        symlink(outside.join("missing"), &blob).unwrap();
        assert!(store.read_upload_inline("one", &upload.id).is_err());
    }

    #[test]
    fn upload_descriptor_does_not_allocate_large_blobs_and_inline_reads_are_capped() {
        let workspace = tempfile::tempdir().unwrap();
        let image =
            crate::services::uploads::save_upload(workspace.path(), "one", "large.png", b"x")
                .unwrap();
        let uploads = workspace.path().join("instances/one/uploads");
        let image_blob = uploads.join(&image.stored_name);
        let large_size = 128_u64 * 1024 * 1024;
        std::fs::OpenOptions::new()
            .write(true)
            .open(&image_blob)
            .unwrap()
            .set_len(large_size)
            .unwrap();
        let image_meta = uploads.join(format!("{}.json", image.id));
        let mut json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&image_meta).unwrap()).unwrap();
        json["size"] = serde_json::json!(large_size);
        std::fs::write(&image_meta, serde_json::to_vec(&json).unwrap()).unwrap();

        let text =
            crate::services::uploads::save_upload(workspace.path(), "one", "large.txt", b"x")
                .unwrap();
        let text_blob = uploads.join(&text.stored_name);
        let text_size = 4_u64 * 1024 * 1024 + 1;
        std::fs::OpenOptions::new()
            .write(true)
            .open(&text_blob)
            .unwrap()
            .set_len(text_size)
            .unwrap();
        let text_meta = uploads.join(format!("{}.json", text.id));
        let mut json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&text_meta).unwrap()).unwrap();
        json["size"] = serde_json::json!(text_size);
        std::fs::write(&text_meta, serde_json::to_vec(&json).unwrap()).unwrap();

        let store = MediaStore::open(workspace.path()).unwrap();
        assert_eq!(
            store.upload_descriptor("one", &image.id).unwrap().size,
            large_size
        );
        assert!(store.read_upload_inline("one", &text.id).is_err());
    }
}

#[cfg(test)]
mod legacy_stats_cleanup_tests {
    use super::*;
    use crate::domain::companion::CANONICAL_SLUG;

    #[test]
    fn legacy_stats_directory_is_removed_from_the_companion_only_and_idempotently() {
        let ws = tempfile::tempdir().unwrap();
        let companion = ws.path().join("instances").join(CANONICAL_SLUG);
        std::fs::create_dir_all(companion.join("stats")).unwrap();
        std::fs::write(companion.join("stats/2026-01-01.json"), "{}").unwrap();
        std::fs::write(companion.join("stats/2026-01-02.json"), "{}").unwrap();
        std::fs::write(companion.join("rhythm.json"), "{}").unwrap();
        std::fs::write(companion.join("soul.md"), "soul").unwrap();
        let obsolete = ws.path().join("instances/alice/stats");
        std::fs::create_dir_all(&obsolete).unwrap();
        std::fs::write(obsolete.join("2026-01-01.json"), "{}").unwrap();

        let store = MediaStore::open(ws.path()).unwrap();
        store.cleanup_legacy_stats().unwrap();
        assert!(!companion.join("stats").exists());
        assert!(companion.join("rhythm.json").is_file());
        assert!(companion.join("soul.md").is_file());
        assert!(
            obsolete.join("2026-01-01.json").is_file(),
            "obsolete sibling directories are never touched"
        );

        store.cleanup_legacy_stats().unwrap();
        assert!(!companion.join("stats").exists());
    }

    #[test]
    fn legacy_stats_cleanup_without_a_companion_is_a_no_op() {
        let ws = tempfile::tempdir().unwrap();
        let store = MediaStore::open(ws.path()).unwrap();
        store.cleanup_legacy_stats().unwrap();
        assert!(!ws.path().join("instances").exists());
    }
}
