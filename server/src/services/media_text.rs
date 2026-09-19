//! Versioned, owner-bound text representations for media memories.
//!
//! A representation lives at the exact adjacent `<media-path>.md`, starts with a
//! machine-readable v1 header containing the SHA-256 of its owner, and is followed
//! by the user-readable representation text. Sidecars are reserved and are never
//! independent text memories.

use cap_std::{
    ambient_authority,
    fs::{Dir, OpenOptions},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::{self, BufReader, Read, Write},
    path::{Component, Path, PathBuf},
};

pub const VERSION: u32 = 1;
const HEADER_PREFIX: &str = "NOLUNE_MEDIA_TEXT ";
const HASH_BUFFER_BYTES: usize = 64 * 1024;
const MAX_UPLOAD_METADATA_BYTES: usize = 1024 * 1024;

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
    #[cfg(test)]
    fail_next_write: std::sync::atomic::AtomicBool,
}

impl MediaStore {
    pub fn open(workspace_root: &Path) -> io::Result<Self> {
        let metadata = std::fs::symlink_metadata(workspace_root)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(invalid_path("workspace root must be a real directory"));
        }
        Ok(Self {
            root: Dir::open_ambient_dir(workspace_root, ambient_authority())?,
            #[cfg(test)]
            fail_next_write: std::sync::atomic::AtomicBool::new(false),
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
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Header {
    version: u32,
    sha256: String,
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
    let mut file = root.open(path)?;
    if !file.metadata()?.is_file() {
        return Err(invalid_path("target must be a regular file"));
    }
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
        assert!(main.contains("media.instance_slugs()"));

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
}
