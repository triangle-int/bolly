use std::{
    fs,
    io::{self, ErrorKind},
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::domain::upload::UploadMeta;

pub(crate) const MAX_FILE_SIZE: u64 = 500 * 1024 * 1024; // 500 MB

const BLOCKED_EXTENSIONS: &[&str] = &[
    "exe", "dll", "so", "dylib", "bin", "msi", "dmg", "iso", "bat", "cmd", "com", "scr", "vbs",
    "wsh",
];

pub fn validate_upload(name: &str, size: u64) -> Result<String, String> {
    if size > MAX_FILE_SIZE {
        return Err(format!(
            "file too large ({} bytes, max {})",
            size, MAX_FILE_SIZE
        ));
    }

    let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();

    if ext.is_empty() {
        return Err("file must have an extension".into());
    }

    if BLOCKED_EXTENSIONS.contains(&ext.as_str()) {
        return Err(format!("file type '.{ext}' is not allowed"));
    }

    Ok(ext)
}

pub(crate) fn mime_from_ext(ext: &str) -> &'static str {
    match ext {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "json" => "application/json",
        "csv" => "text/csv",
        "md" => "text/markdown",
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "xml" => "text/xml",
        "yaml" | "yml" => "text/yaml",
        "toml" => "text/plain",
        "txt" | "log" | "ini" | "cfg" | "conf" | "env" => "text/plain",
        "py" | "rs" | "js" | "ts" | "jsx" | "tsx" | "go" | "rb" | "java" | "c" | "cpp" | "h"
        | "hpp" | "cs" | "swift" | "kt" | "scala" | "sh" | "bash" | "zsh" | "fish" | "ps1"
        | "sql" | "graphql" | "proto" | "svelte" | "vue" | "astro" | "dockerfile" | "makefile"
        | "cmake" | "r" | "lua" | "php" | "pl" | "ex" | "exs" | "zig" | "nim" | "dart" | "elm"
        | "clj" | "hs" | "ml" | "fs" | "erl" => "text/plain",
        "mp4" | "m4v" => "video/mp4",
        "webm" => "video/webm",
        "mov" => "video/quicktime",
        "avi" => "video/x-msvideo",
        "mkv" => "video/x-matroska",
        "flv" => "video/x-flv",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" => "audio/ogg",
        "m4a" => "audio/mp4",
        "zip" => "application/zip",
        "tar" | "gz" | "tgz" | "bz2" | "xz" => "application/octet-stream",
        _ => "application/octet-stream",
    }
}

pub fn save_upload(
    workspace_dir: &Path,
    instance_slug: &str,
    original_name: &str,
    bytes: &[u8],
) -> io::Result<UploadMeta> {
    let ext = validate_upload(original_name, bytes.len() as u64)
        .map_err(|e| io::Error::new(ErrorKind::InvalidInput, e))?;

    let uploads_dir = workspace_dir
        .join("instances")
        .join(instance_slug)
        .join("uploads");
    fs::create_dir_all(&uploads_dir)?;

    let ts = unix_millis();
    let id = format!("upload_{ts}.{ext}");
    // Use _blob suffix to prevent collision with .json metadata sidecar
    // (e.g. uploading a .json file would overwrite its own metadata)
    let stored_name = format!("{id}_blob.{ext}");
    let mime_type = mime_from_ext(&ext).to_string();

    // Write the file
    fs::write(uploads_dir.join(&stored_name), bytes)?;

    let meta = UploadMeta {
        id: id.clone(),
        original_name: original_name.to_string(),
        stored_name,
        mime_type,
        size: bytes.len() as u64,
        uploaded_at: ts.to_string(),
        anthropic_file_id: None,
    };

    // Write metadata sidecar
    let json = serde_json::to_string_pretty(&meta)
        .map_err(|e| io::Error::new(ErrorKind::InvalidData, e))?;
    fs::write(uploads_dir.join(format!("{id}.json")), json)?;

    log::info!(
        "[uploads] saved {id} ({original_name}, {} bytes) for {instance_slug}",
        bytes.len()
    );
    Ok(meta)
}

pub fn list_uploads(workspace_dir: &Path, instance_slug: &str) -> io::Result<Vec<UploadMeta>> {
    let uploads_dir = workspace_dir
        .join("instances")
        .join(instance_slug)
        .join("uploads");

    if !uploads_dir.is_dir() {
        return Ok(vec![]);
    }

    let mut uploads = Vec::new();
    for entry in fs::read_dir(&uploads_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        match fs::read_to_string(&path) {
            Ok(raw) => match serde_json::from_str::<UploadMeta>(&raw) {
                Ok(meta) => uploads.push(meta),
                Err(e) => log::warn!("skipping malformed upload meta {}: {e}", path.display()),
            },
            Err(e) => log::warn!("failed to read upload meta {}: {e}", path.display()),
        }
    }

    uploads.sort_by(|a, b| b.uploaded_at.cmp(&a.uploaded_at));
    Ok(uploads)
}

pub fn get_upload(
    workspace_dir: &Path,
    instance_slug: &str,
    upload_id: &str,
) -> io::Result<Option<UploadMeta>> {
    let path = workspace_dir
        .join("instances")
        .join(instance_slug)
        .join("uploads")
        .join(format!("{upload_id}.json"));

    if !path.exists() {
        return Ok(None);
    }

    let raw = fs::read_to_string(&path)?;
    let meta: UploadMeta =
        serde_json::from_str(&raw).map_err(|e| io::Error::new(ErrorKind::InvalidData, e))?;
    Ok(Some(meta))
}

pub fn get_upload_file_path(
    workspace_dir: &Path,
    instance_slug: &str,
    upload_id: &str,
) -> Option<std::path::PathBuf> {
    let meta = get_upload(workspace_dir, instance_slug, upload_id).ok()??;
    let path = workspace_dir
        .join("instances")
        .join(instance_slug)
        .join("uploads")
        .join(&meta.stored_name);
    path.exists().then_some(path)
}

pub fn delete_upload(
    workspace_dir: &Path,
    instance_slug: &str,
    upload_id: &str,
) -> io::Result<bool> {
    let uploads_dir = workspace_dir
        .join("instances")
        .join(instance_slug)
        .join("uploads");

    let meta_path = uploads_dir.join(format!("{upload_id}.json"));
    if !meta_path.exists() {
        return Ok(false);
    }

    // Read meta to find the blob file
    if let Ok(raw) = fs::read_to_string(&meta_path) {
        if let Ok(meta) = serde_json::from_str::<UploadMeta>(&raw) {
            let blob_path = uploads_dir.join(&meta.stored_name);
            let _ = fs::remove_file(&blob_path);
        }
    }

    fs::remove_file(&meta_path)?;
    Ok(true)
}

/// Extract a ZIP file to `instances/{slug}/projects/{stem}/`.
/// Returns the extraction path and a list of extracted file paths (relative).
pub fn extract_zip(
    workspace_dir: &Path,
    instance_slug: &str,
    upload_id: &str,
) -> io::Result<(std::path::PathBuf, Vec<String>)> {
    let meta = get_upload(workspace_dir, instance_slug, upload_id)?
        .ok_or_else(|| io::Error::new(ErrorKind::NotFound, "upload not found"))?;

    let file_path = get_upload_file_path(workspace_dir, instance_slug, upload_id)
        .ok_or_else(|| io::Error::new(ErrorKind::NotFound, "upload file not found"))?;

    let file = fs::File::open(&file_path)?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| io::Error::new(ErrorKind::InvalidData, format!("invalid zip: {e}")))?;

    // Derive project name from original filename (without .zip extension)
    let stem = meta
        .original_name
        .rsplit('.')
        .skip(1)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join(".");
    let stem = if stem.is_empty() {
        upload_id.to_string()
    } else {
        stem
    };

    // Sanitize stem
    let stem: String = stem
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();

    let projects_dir = workspace_dir
        .join("instances")
        .join(instance_slug)
        .join("projects");
    let extract_dir = projects_dir.join(&stem);
    fs::create_dir_all(&extract_dir)?;

    let mut extracted_files = Vec::new();

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| io::Error::new(ErrorKind::InvalidData, format!("zip entry error: {e}")))?;

        let entry_path = match entry.enclosed_name() {
            Some(p) => p.to_path_buf(),
            None => continue, // Skip entries with unsafe paths (e.g., "../")
        };

        let target = extract_dir.join(&entry_path);

        if entry.is_dir() {
            fs::create_dir_all(&target)?;
        } else {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut outfile = fs::File::create(&target)?;
            io::copy(&mut entry, &mut outfile)?;
            extracted_files.push(entry_path.to_string_lossy().to_string());
        }
    }

    log::info!(
        "[uploads] extracted zip {} → {} ({} files)",
        meta.original_name,
        extract_dir.display(),
        extracted_files.len()
    );

    Ok((extract_dir, extracted_files))
}

fn unix_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after unix epoch")
        .as_millis()
}
