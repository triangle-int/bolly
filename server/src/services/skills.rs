use std::{fs, io, path::Path};

use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt, OpenOptionsMaybeDirExt};
use cap_std::{
    ambient_authority,
    fs::{Dir, OpenOptions},
};

use futures::StreamExt;
use serde::Deserialize;

use crate::domain::skill::{RegistryEntry, Skill, SkillSource, parse_skill_md};

const BUNDLED_GOG_SKILL: &str = include_str!("../../official-skills/gog/SKILL.md");
// SKILL.md is vendored verbatim from openclaw/gogcli v0.15.0:
// https://github.com/openclaw/gogcli/blob/v0.15.0/.agents/skills/gog/SKILL.md
// SHA-256: 3c3310b3df04a1c0a2a39972a983034fc8127992d8967c0cd74e3a876837a2ba
const MAX_REGISTRY_BYTES: usize = 512 * 1024;

const MAX_SKILL_FILES: usize = 128;
const MAX_SKILL_DEPTH: usize = 8;
const MAX_SKILL_FILE_BYTES: usize = 1024 * 1024;
const MAX_SKILL_TOTAL_BYTES: usize = 5 * 1024 * 1024;

fn bundled_gog_entry() -> RegistryEntry {
    RegistryEntry {
        id: "gog".into(),
        name: "gog".into(),
        description: "Google Workspace automation through the gog CLI.".into(),
        icon: String::new(),
        repo: "openclaw/gogcli".into(),
        git_ref: "v0.15.0".into(),
        author: "OpenClaw".into(),
        path: ".agents/skills/gog".into(),
    }
}

fn is_bundled_gog(entry: &RegistryEntry) -> bool {
    entry.id == "gog"
        && entry.repo == "openclaw/gogcli"
        && entry.git_ref == "v0.15.0"
        && entry.path == ".agents/skills/gog"
}

/// Merge official bundled entries into a remote registry. Bundled definitions
/// win ID collisions so an unavailable or compromised registry cannot replace
/// the reviewed local copy.
pub fn merge_registry_entries(mut remote: Vec<RegistryEntry>) -> Vec<RegistryEntry> {
    remote.retain(|entry| entry.id != "gog");
    remote.push(bundled_gog_entry());
    remote.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
    remote
}

fn validate_skill_id(id: &str) -> anyhow::Result<()> {
    if id.is_empty()
        || id.len() > 64
        || !id.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-' || byte == b'_'
        })
    {
        anyhow::bail!("invalid skill id '{id}'");
    }
    Ok(())
}

/// The built-in "skill_creator" skill that is always present.
fn builtin_skill_creator() -> Skill {
    Skill {
        id: "skill_creator".into(),
        name: "Skill Creator".into(),
        description: "Create and manage new skills for your companion. Teach it new abilities by defining instructions, triggers, and behaviors.".into(),
        icon: "+".into(),
        builtin: true,
        enabled: true,
        kind: Default::default(),
        anthropic_skill_id: None,
        anthropic_version: None,
        instructions: String::new(),
        source: None,
        resources: Vec::new(),
    }
}

/// Read all skills: builtins + user-created ones from the skills directory.
pub fn list_skills(workspace_dir: &Path) -> Vec<Skill> {
    let mut skills = vec![builtin_skill_creator()];

    let skills_dir = workspace_dir.join("skills");
    if skills_dir.is_dir() {
        if let Ok(entries) = fs::read_dir(&skills_dir) {
            for entry in entries.filter_map(Result::ok) {
                let path = entry.path();
                if path.is_dir() {
                    if let Some(skill) = read_skill_dir(&path) {
                        skills.push(skill);
                    }
                }
            }
        }
    }

    skills.sort_by(|a, b| {
        // Builtins first, then alphabetical
        b.builtin.cmp(&a.builtin).then(a.name.cmp(&b.name))
    });

    skills
}

/// Read a single skill from its directory.
///
/// Supports two formats:
/// 1. SKILL.md with YAML frontmatter (Agent Skills spec) — preferred
/// 2. skill.json + instructions.md/SKILL.md — legacy
fn read_skill_dir(path: &Path) -> Option<Skill> {
    let dir_name = path.file_name()?.to_string_lossy().to_string();
    let skill_md_path = path.join("SKILL.md");

    // Collect bundled resources
    let resources = list_resources(path);

    // Try SKILL.md with frontmatter first
    if let Ok(content) = fs::read_to_string(&skill_md_path) {
        let (fm, body) = parse_skill_md(&content);

        // If frontmatter has name+description, use the Agent Skills format
        if !fm.name.is_empty() && !fm.description.is_empty() {
            return Some(Skill {
                id: dir_name,
                name: fm.name,
                description: fm.description,
                icon: String::new(),
                builtin: false,
                enabled: true,
                kind: Default::default(),
                anthropic_skill_id: None,
                anthropic_version: None,
                instructions: body,
                source: read_source(path),
                resources,
            });
        }
    }

    // Fall back to skill.json manifest
    let manifest = path.join("skill.json");
    if let Ok(raw) = fs::read_to_string(&manifest) {
        if let Ok(mut skill) = serde_json::from_str::<Skill>(&raw) {
            if skill.id.is_empty() {
                skill.id = dir_name;
            }
            // Read instructions from SKILL.md body or instructions.md
            if skill.instructions.is_empty() {
                if let Ok(content) = fs::read_to_string(&skill_md_path) {
                    let (_, body) = parse_skill_md(&content);
                    skill.instructions = body;
                } else if let Ok(content) = fs::read_to_string(path.join("instructions.md")) {
                    skill.instructions = content;
                }
            }
            skill.resources = resources;
            if skill.source.is_none() {
                skill.source = read_source(path);
            }
            return Some(skill);
        }
    }

    // Last resort: SKILL.md without proper frontmatter, use raw content
    if let Ok(content) = fs::read_to_string(&skill_md_path) {
        return Some(Skill {
            id: dir_name.clone(),
            name: dir_name,
            description: String::new(),
            icon: String::new(),
            builtin: false,
            enabled: true,
            kind: Default::default(),
            anthropic_skill_id: None,
            anthropic_version: None,
            instructions: content,
            source: read_source(path),
            resources,
        });
    }

    None
}

/// Read .source.json if present (tracks install origin).
fn read_source(path: &Path) -> Option<SkillSource> {
    let source_path = path.join(".source.json");
    let raw = fs::read_to_string(&source_path).ok()?;
    serde_json::from_str(&raw).ok()
}

/// List bundled resource files (references/, scripts/, assets/).
fn list_resources(skill_dir: &Path) -> Vec<String> {
    let mut resources = Vec::new();
    for subdir in &["references", "scripts", "assets"] {
        let dir = skill_dir.join(subdir);
        if dir.is_dir() {
            if let Ok(entries) = fs::read_dir(&dir) {
                for entry in entries.filter_map(Result::ok) {
                    let p = entry.path();
                    if p.is_file() {
                        if let Some(name) = p.file_name() {
                            resources.push(format!("{}/{}", subdir, name.to_string_lossy()));
                        }
                    }
                }
            }
        }
    }
    resources.sort();
    resources
}

/// Get a single skill by ID.
pub fn get_skill(workspace_dir: &Path, skill_id: &str) -> Option<Skill> {
    list_skills(workspace_dir)
        .into_iter()
        .find(|s| s.id == skill_id)
}

/// Create a new skill directory with SKILL.md.
pub fn create_skill(workspace_dir: &Path, skill: &Skill) -> io::Result<()> {
    let skill_dir = workspace_dir.join("skills").join(&skill.id);
    fs::create_dir_all(&skill_dir)?;

    // Write SKILL.md with frontmatter
    let skill_md = format!(
        "---\nname: {}\ndescription: {}\n---\n\n{}",
        skill.id,
        skill.description.replace('\n', " "),
        skill.instructions
    );
    fs::write(skill_dir.join("SKILL.md"), skill_md)?;

    // Write source tracking
    if let Some(source) = &skill.source {
        let source_json = serde_json::to_string_pretty(source)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        fs::write(skill_dir.join(".source.json"), source_json)?;
    }

    Ok(())
}

/// Delete a user-created skill (cannot delete builtins).
pub fn delete_skill(workspace_dir: &Path, skill_id: &str) -> io::Result<bool> {
    if skill_id == "skill_creator" {
        return Ok(false);
    }

    let skill_dir = workspace_dir.join("skills").join(skill_id);
    if skill_dir.is_dir() {
        fs::remove_dir_all(&skill_dir)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

/// Check whether a skill is already installed locally.
pub fn is_installed(workspace_dir: &Path, skill_id: &str) -> bool {
    workspace_dir.join("skills").join(skill_id).is_dir()
}

/// Fetch the remote skills registry index.
pub async fn fetch_registry(registry_url: &str) -> anyhow::Result<Vec<RegistryEntry>> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;

    let resp = client.get(registry_url).send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("registry returned {}", resp.status());
    }
    let bytes = response_bytes_bounded(resp, MAX_REGISTRY_BYTES).await?;
    Ok(serde_json::from_slice(&bytes)?)
}

struct CapabilityFileGuard {
    parent: Dir,
    name: String,
}

impl Drop for CapabilityFileGuard {
    fn drop(&mut self) {
        let _ = self.parent.remove_file(&self.name);
    }
}

fn open_real_child_dir(parent: &Dir, name: &Path) -> io::Result<Option<Dir>> {
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

fn open_workspace_dir(workspace_dir: &Path) -> io::Result<Dir> {
    let parent_path = workspace_dir
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "workspace has no parent"))?;
    let name = workspace_dir
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "workspace has no name"))?;
    let parent = Dir::open_ambient_dir(parent_path, ambient_authority())?;
    open_real_child_dir(&parent, Path::new(name))?.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "workspace root must be a real directory",
        )
    })
}

fn open_or_create_skills_dir(workspace: &Dir) -> io::Result<Dir> {
    if let Some(skills) = open_real_child_dir(workspace, Path::new("skills"))? {
        return Ok(skills);
    }
    workspace.create_dir("skills")?;
    open_real_child_dir(workspace, Path::new("skills"))?.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "skills must be a real directory",
        )
    })
}

fn create_new_file(parent: &Dir, name: &str, bytes: &[u8]) -> io::Result<()> {
    let mut options = OpenOptions::new();
    options
        .write(true)
        .create_new(true)
        .follow(FollowSymlinks::No);
    let mut file = parent.open_with(name, &options)?;
    io::Write::write_all(&mut file, bytes)?;
    io::Write::flush(&mut file)?;
    file.sync_all()
}

#[cfg(unix)]
fn sync_capability_dir(dir: &Dir) -> io::Result<()> {
    // A cloned capability is already an open directory descriptor on Unix.
    dir.try_clone()?.into_std_file().sync_all()
}

#[cfg(windows)]
fn sync_capability_dir(dir: &Dir) -> io::Result<()> {
    // The create-new files themselves are sync_all'd. Rust's portable file API
    // cannot open a Windows directory handle suitable for FlushFileBuffers.
    let _ = dir;
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn sync_capability_dir(dir: &Dir) -> io::Result<()> {
    let _ = dir;
    Ok(())
}

fn directory_identity(dir: &Dir) -> io::Result<same_file::Handle> {
    same_file::Handle::from_file(dir.try_clone()?.into_std_file())
}

/// Publish a fully flushed file under a new name without ever replacing an
/// existing path. The hard link is an atomic, descriptor-relative create.
fn publish_new_file(parent: &Dir, name: &str, bytes: &[u8]) -> io::Result<()> {
    let temporary = format!(".{name}.tmp-{}", uuid::Uuid::new_v4());
    create_new_file(parent, &temporary, bytes)?;
    let publication = parent.hard_link(&temporary, parent, name);
    let cleanup = parent.remove_file(&temporary);
    publication?;
    cleanup?;
    sync_capability_dir(parent)
}

fn acquire_install_lock(skills: &Dir, id: &str) -> anyhow::Result<CapabilityFileGuard> {
    let name = format!(".{id}.install.lock");
    create_new_file(skills, &name, &[]).map_err(|error| {
        if error.kind() == io::ErrorKind::AlreadyExists {
            anyhow::anyhow!("skill '{id}' installation is already in progress")
        } else {
            error.into()
        }
    })?;
    Ok(CapabilityFileGuard {
        parent: skills.try_clone()?,
        name,
    })
}

fn validate_capability_install_tree(root: &Dir) -> anyhow::Result<()> {
    fn visit(dir: &Dir, depth: usize, files: &mut usize, bytes: &mut usize) -> anyhow::Result<()> {
        if depth > MAX_SKILL_DEPTH {
            anyhow::bail!("installed skill exceeds directory depth limit");
        }
        for entry in dir.entries()? {
            let entry = entry?;
            let name = entry.file_name();
            let metadata = dir.symlink_metadata(&name)?;
            if metadata.file_type().is_symlink() {
                anyhow::bail!("installed skill contains a symlink");
            }
            if metadata.is_dir() {
                let child = open_real_child_dir(dir, Path::new(&name))?.ok_or_else(|| {
                    anyhow::anyhow!("installed skill directory could not be opened safely")
                })?;
                visit(&child, depth + 1, files, bytes)?;
            } else if metadata.is_file() {
                *files += 1;
                *bytes = bytes.saturating_add(metadata.len() as usize);
                if *files > MAX_SKILL_FILES
                    || metadata.len() > MAX_SKILL_FILE_BYTES as u64
                    || *bytes > MAX_SKILL_TOTAL_BYTES
                {
                    anyhow::bail!("installed skill exceeds resource limits");
                }
            } else {
                anyhow::bail!("installed skill contains an unsupported resource");
            }
        }
        Ok(())
    }

    let skill_md = root.symlink_metadata("SKILL.md")?;
    if skill_md.file_type().is_symlink() || !skill_md.is_file() {
        anyhow::bail!("installed SKILL.md is not a regular file");
    }
    if skill_md.len() > MAX_SKILL_FILE_BYTES as u64 {
        anyhow::bail!("installed SKILL.md exceeds file-size limit");
    }
    let content = root.read_to_string("SKILL.md")?;
    let (frontmatter, body) = parse_skill_md(&content);
    if frontmatter.name.trim().is_empty()
        || frontmatter.description.trim().is_empty()
        || body.trim().is_empty()
    {
        anyhow::bail!("installed SKILL.md has invalid frontmatter or instructions");
    }
    let mut files = 0;
    let mut bytes = 0;
    visit(root, 0, &mut files, &mut bytes)
}

fn bundled_skill_from_dir(installed: &Dir, entry: &RegistryEntry) -> anyhow::Result<Skill> {
    let content = installed.read_to_string("SKILL.md")?;
    let (frontmatter, instructions) = parse_skill_md(&content);
    Ok(Skill {
        id: entry.id.clone(),
        name: frontmatter.name,
        description: frontmatter.description,
        icon: String::new(),
        builtin: false,
        enabled: true,
        kind: Default::default(),
        anthropic_skill_id: None,
        anthropic_version: None,
        instructions,
        source: Some(SkillSource {
            repo: entry.repo.clone(),
            version: entry.git_ref.clone(),
            path: Some(entry.path.clone()),
        }),
        resources: Vec::new(),
    })
}

fn install_bundled_gog_with_hook<F>(
    workspace_dir: &Path,
    entry: &RegistryEntry,
    before_identity_check: F,
) -> anyhow::Result<Skill>
where
    F: FnOnce(&Dir) -> io::Result<()>,
{
    install_bundled_gog_with_hooks(workspace_dir, entry, |_| Ok(()), before_identity_check)
}

fn install_bundled_gog_with_hooks<F, G>(
    workspace_dir: &Path,
    entry: &RegistryEntry,
    before_skill_publish: F,
    before_identity_check: G,
) -> anyhow::Result<Skill>
where
    F: FnOnce(&Dir) -> io::Result<()>,
    G: FnOnce(&Dir) -> io::Result<()>,
{
    validate_skill_id(&entry.id)?;
    let workspace = open_workspace_dir(workspace_dir)?;
    let skills = open_or_create_skills_dir(&workspace)?;
    let _lock = acquire_install_lock(&skills, &entry.id)?;

    skills.create_dir(&entry.id).map_err(|error| {
        if error.kind() == io::ErrorKind::AlreadyExists {
            anyhow::anyhow!(
                "skill '{}' destination already exists; it may be an incomplete installation and must be inspected or removed manually before retrying",
                entry.id
            )
        } else {
            error.into()
        }
    })?;
    // Never automatically remove this directory after creation. A failed
    // install remains available for safe manual inspection/removal, and
    // SKILL.md is published last so pre-publication failures are not loadable.
    let installed = open_real_child_dir(&skills, Path::new(&entry.id))?
        .ok_or_else(|| anyhow::anyhow!("failed to open created skill directory"))?;
    let installed_identity = directory_identity(&installed)?;
    sync_capability_dir(&skills)?;

    let source = SkillSource {
        repo: entry.repo.clone(),
        version: entry.git_ref.clone(),
        path: Some(entry.path.clone()),
    };
    publish_new_file(
        &installed,
        ".source.json",
        &serde_json::to_vec_pretty(&source)?,
    )?;
    before_skill_publish(&installed)?;
    // SKILL.md is the loader's recognition marker, so publish it only after
    // every prerequisite is durable.
    publish_new_file(&installed, "SKILL.md", BUNDLED_GOG_SKILL.as_bytes())?;
    validate_capability_install_tree(&installed)?;
    let skill = bundled_skill_from_dir(&installed, entry)?;

    before_identity_check(&skills)?;
    let current = open_real_child_dir(&skills, Path::new(&entry.id))?
        .ok_or_else(|| anyhow::anyhow!("skill destination changed during installation"))?;
    if directory_identity(&current)? != installed_identity {
        anyhow::bail!("skill destination changed during installation");
    }
    Ok(skill)
}

/// Install a reviewed bundled skill or a registry skill.
pub async fn install_from_registry(
    workspace_dir: &Path,
    entry: &RegistryEntry,
) -> anyhow::Result<Skill> {
    if is_bundled_gog(entry) {
        return install_bundled_gog_with_hook(workspace_dir, entry, |_| Ok(()));
    }

    // Preserve the existing remote-registry behavior; its separate trust model
    // is outside the bundled-offline installation path.
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()?;
    let skill_dir = workspace_dir.join("skills").join(&entry.id);
    fs::create_dir_all(&skill_dir)?;
    let contents_path = if entry.path.is_empty() {
        String::new()
    } else {
        format!("/{}", entry.path)
    };
    download_github_dir_legacy(
        &client,
        &entry.repo,
        &entry.git_ref,
        &contents_path,
        &skill_dir,
    )
    .await?;
    let source = SkillSource {
        repo: entry.repo.clone(),
        version: entry.git_ref.clone(),
        path: (!entry.path.is_empty()).then(|| entry.path.clone()),
    };
    fs::write(
        skill_dir.join(".source.json"),
        serde_json::to_vec_pretty(&source)?,
    )?;
    read_skill_dir(&skill_dir).ok_or_else(|| anyhow::anyhow!("failed to read installed skill"))
}

/// GitHub Contents API response item.
#[derive(Debug, Deserialize)]
struct GitHubContent {
    name: String,
    #[serde(rename = "type")]
    content_type: String,
    download_url: Option<String>,
    path: String,
}

/// Legacy remote-registry downloader, intentionally unchanged from the
/// pre-existing path while bundled official skills use the confined installer.
async fn download_github_dir_legacy(
    client: &reqwest::Client,
    repo: &str,
    git_ref: &str,
    api_path: &str,
    local_dir: &Path,
) -> anyhow::Result<()> {
    let url = format!(
        "https://api.github.com/repos/{}/contents{}?ref={}",
        repo, api_path, git_ref
    );
    let resp = client
        .get(&url)
        .header("User-Agent", "nolune-skills-installer")
        .header("Accept", "application/vnd.github.v3+json")
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(anyhow::anyhow!(
            "GitHub API returned {} for {}",
            resp.status(),
            url
        ));
    }
    let items: Vec<GitHubContent> = resp.json().await?;
    for item in items {
        match item.content_type.as_str() {
            "file" => {
                if let Some(download_url) = &item.download_url {
                    let file_resp = client
                        .get(download_url)
                        .header("User-Agent", "nolune-skills-installer")
                        .send()
                        .await?;
                    if file_resp.status().is_success() {
                        let bytes = file_resp.bytes().await?;
                        fs::write(local_dir.join(&item.name), &bytes)?;
                    }
                }
            }
            "dir" => {
                let sub_dir = local_dir.join(&item.name);
                fs::create_dir_all(&sub_dir)?;
                let sub_path = format!("/{}", item.path);
                Box::pin(download_github_dir_legacy(
                    client, repo, git_ref, &sub_path, &sub_dir,
                ))
                .await?;
            }
            _ => {}
        }
    }
    Ok(())
}

async fn response_bytes_bounded(
    response: reqwest::Response,
    maximum: usize,
) -> anyhow::Result<Vec<u8>> {
    if response
        .content_length()
        .is_some_and(|length| length > maximum as u64)
    {
        anyhow::bail!("registry response is too large");
    }
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        if bytes.len().saturating_add(chunk.len()) > maximum {
            anyhow::bail!("registry response is too large");
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn remote_gog() -> RegistryEntry {
        RegistryEntry {
            id: "gog".into(),
            name: "Remote gog".into(),
            description: "remote duplicate".into(),
            icon: String::new(),
            repo: "someone/else".into(),
            git_ref: "main".into(),
            author: "Someone".into(),
            path: "skills/gog".into(),
        }
    }

    #[test]
    fn bundled_gog_is_listed_once_and_wins_remote_deduplication() {
        let entries = merge_registry_entries(vec![remote_gog()]);
        let gog: Vec<_> = entries.iter().filter(|entry| entry.id == "gog").collect();
        assert_eq!(gog.len(), 1);
        assert_eq!(gog[0].repo, "openclaw/gogcli");
        assert_eq!(gog[0].git_ref, "v0.15.0");
        assert_eq!(gog[0].path, ".agents/skills/gog");
    }

    #[tokio::test]
    async fn bundled_gog_installs_verbatim_without_network_and_records_upstream_provenance() {
        use sha2::{Digest, Sha256};

        let workspace = tempfile::tempdir().unwrap();
        let entry = merge_registry_entries(Vec::new())
            .into_iter()
            .find(|entry| entry.id == "gog")
            .unwrap();

        let installed = install_from_registry(workspace.path(), &entry)
            .await
            .unwrap();
        assert_eq!(installed.id, "gog");
        let source = installed.source.unwrap();
        assert_eq!(source.repo, "openclaw/gogcli");
        assert_eq!(source.version, "v0.15.0");
        assert_eq!(source.path.as_deref(), Some(".agents/skills/gog"));
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(
                &fs::read(workspace.path().join("skills/gog/.source.json")).unwrap()
            )
            .unwrap(),
            serde_json::json!({
                "repo": "openclaw/gogcli",
                "version": "v0.15.0",
                "path": ".agents/skills/gog"
            })
        );
        assert_eq!(get_skill(workspace.path(), "gog").unwrap().name, "gog");

        let bytes = fs::read(workspace.path().join("skills/gog/SKILL.md")).unwrap();
        assert_eq!(bytes, BUNDLED_GOG_SKILL.as_bytes());
        assert_eq!(
            format!("{:x}", Sha256::digest(&bytes)),
            "3c3310b3df04a1c0a2a39972a983034fc8127992d8967c0cd74e3a876837a2ba"
        );
        assert!(
            !workspace
                .path()
                .join("skills/gog/references/setup.md")
                .exists()
        );
    }

    #[tokio::test]
    async fn bundled_install_refuses_to_overwrite_existing_skill() {
        let workspace = tempfile::tempdir().unwrap();
        let existing = workspace.path().join("skills/gog");
        fs::create_dir_all(&existing).unwrap();
        let entry = merge_registry_entries(Vec::new())
            .into_iter()
            .find(|entry| entry.id == "gog")
            .unwrap();

        let error = install_from_registry(workspace.path(), &entry)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("already exists"));
        assert!(error.to_string().contains("incomplete installation"));
        assert!(fs::read_dir(existing).unwrap().next().is_none());
    }

    #[tokio::test]
    async fn concurrent_bundled_installs_publish_once_without_overwrite() {
        let workspace = tempfile::tempdir().unwrap();
        let path = workspace.path().to_path_buf();
        let entry = bundled_gog_entry();
        let (left, right) = tokio::join!(
            install_from_registry(&path, &entry),
            install_from_registry(&path, &entry)
        );
        assert_eq!(usize::from(left.is_ok()) + usize::from(right.is_ok()), 1);
        let installed = fs::read(path.join("skills/gog/SKILL.md")).unwrap();
        assert_eq!(installed, BUNDLED_GOG_SKILL.as_bytes());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn bundled_install_refuses_symlink_target_without_touching_destination() {
        use std::os::unix::fs::symlink;

        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::create_dir_all(workspace.path().join("skills")).unwrap();
        fs::write(outside.path().join("sentinel"), "keep me").unwrap();
        symlink(outside.path(), workspace.path().join("skills/gog")).unwrap();

        let error = install_from_registry(workspace.path(), &bundled_gog_entry())
            .await
            .unwrap_err();
        assert!(error.to_string().contains("already exists"));
        assert!(error.to_string().contains("incomplete installation"));
        assert_eq!(
            fs::read_to_string(outside.path().join("sentinel")).unwrap(),
            "keep me"
        );
        assert!(!outside.path().join("SKILL.md").exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn bundled_install_refuses_symlinked_skills_directory() {
        use std::os::unix::fs::symlink;

        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("sentinel"), "keep me").unwrap();
        symlink(outside.path(), workspace.path().join("skills")).unwrap();

        assert!(
            install_from_registry(workspace.path(), &bundled_gog_entry())
                .await
                .is_err()
        );
        assert_eq!(
            fs::read_to_string(outside.path().join("sentinel")).unwrap(),
            "keep me"
        );
        assert!(!outside.path().join("gog").exists());
    }

    #[cfg(unix)]
    #[test]
    fn bundled_install_stays_bound_to_open_workspace_after_path_swap() {
        use std::os::unix::fs::symlink;

        let container = tempfile::tempdir().unwrap();
        let workspace = container.path().join("workspace");
        let held = container.path().join("held-workspace");
        let outside = tempfile::tempdir().unwrap();
        fs::create_dir(&workspace).unwrap();
        fs::write(outside.path().join("sentinel"), "keep me").unwrap();

        install_bundled_gog_with_hook(&workspace, &bundled_gog_entry(), |_| {
            fs::rename(&workspace, &held)?;
            symlink(outside.path(), &workspace)?;
            Ok(())
        })
        .unwrap();

        assert_eq!(
            fs::read_to_string(outside.path().join("sentinel")).unwrap(),
            "keep me"
        );
        assert!(!outside.path().join("gog").exists());
        assert_eq!(
            fs::read(held.join("skills/gog/SKILL.md")).unwrap(),
            BUNDLED_GOG_SKILL.as_bytes()
        );
    }

    #[test]
    fn bundled_install_failure_before_skill_publication_preserves_incomplete_directory() {
        let workspace = tempfile::tempdir().unwrap();
        let expected_error = "injected failure before SKILL.md publication";

        let error = install_bundled_gog_with_hooks(
            workspace.path(),
            &bundled_gog_entry(),
            |_| Err(io::Error::other(expected_error)),
            |_| Ok(()),
        )
        .unwrap_err();

        assert!(error.to_string().contains(expected_error));
        assert!(get_skill(workspace.path(), "gog").is_none());
        let source_path = workspace.path().join("skills/gog/.source.json");
        let source_before_retry = fs::read(&source_path).unwrap();
        assert!(
            serde_json::from_slice::<serde_json::Value>(&source_before_retry)
                .unwrap()
                .get("repo")
                .is_some_and(|repo| repo == "openclaw/gogcli")
        );
        assert!(!workspace.path().join("skills/gog/SKILL.md").exists());

        let retry_error = install_bundled_gog_with_hooks(
            workspace.path(),
            &bundled_gog_entry(),
            |_| Ok(()),
            |_| Ok(()),
        )
        .unwrap_err();
        assert!(retry_error.to_string().contains("already exists"));
        assert!(retry_error.to_string().contains("incomplete installation"));
        assert_eq!(fs::read(source_path).unwrap(), source_before_retry);
        assert!(!workspace.path().join("skills/gog/SKILL.md").exists());
    }

    #[test]
    fn bundled_install_rejects_destination_swap_after_writes_without_deleting_either_directory() {
        let workspace = tempfile::tempdir().unwrap();
        let moved = workspace.path().join("skills/validated-gog");
        let error =
            install_bundled_gog_with_hook(workspace.path(), &bundled_gog_entry(), |skills| {
                assert_eq!(skills.read("gog/SKILL.md")?, BUNDLED_GOG_SKILL.as_bytes());
                assert!(
                    skills
                        .read_to_string("gog/.source.json")?
                        .contains("openclaw/gogcli")
                );
                skills.rename("gog", skills, "validated-gog")?;
                create_new_file(skills, "validated-gog/sentinel", b"validated stays")?;
                skills.create_dir("gog")?;
                create_new_file(
                    skills,
                    "gog/SKILL.md",
                    b"---\nname: malicious\ndescription: malicious\n---\n\nmalicious",
                )?;
                create_new_file(skills, "gog/sentinel", b"attacker stays")
            })
            .unwrap_err();

        assert!(error.to_string().contains("changed during installation"));
        assert_eq!(
            fs::read_to_string(workspace.path().join("skills/gog/sentinel")).unwrap(),
            "attacker stays"
        );
        assert_eq!(
            fs::read(workspace.path().join("skills/gog/SKILL.md")).unwrap(),
            b"---\nname: malicious\ndescription: malicious\n---\n\nmalicious"
        );
        assert_eq!(
            fs::read_to_string(moved.join("sentinel")).unwrap(),
            "validated stays"
        );
        assert_eq!(
            fs::read(moved.join("SKILL.md")).unwrap(),
            BUNDLED_GOG_SKILL.as_bytes()
        );
    }
}
