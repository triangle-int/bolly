use std::{fs, io, path::Path};

use serde::Deserialize;

use crate::domain::skill::{RegistryEntry, Skill, SkillSource, parse_skill_md};

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
        return Err(anyhow::anyhow!("registry returned {}", resp.status()));
    }

    let entries: Vec<RegistryEntry> = resp.json().await?;
    Ok(entries)
}

/// Install a skill from a registry entry by downloading the full directory from GitHub.
pub async fn install_from_registry(
    workspace_dir: &Path,
    entry: &RegistryEntry,
) -> anyhow::Result<Skill> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()?;

    let skill_dir = workspace_dir.join("skills").join(&entry.id);
    fs::create_dir_all(&skill_dir)?;

    // Build the GitHub Contents API path
    let contents_path = if entry.path.is_empty() {
        String::new()
    } else {
        format!("/{}", entry.path)
    };

    // Recursively download the skill directory from GitHub
    download_github_dir(
        &client,
        &entry.repo,
        &entry.git_ref,
        &contents_path,
        &skill_dir,
    )
    .await?;

    // Write source tracking
    let source = SkillSource {
        repo: entry.repo.clone(),
        version: entry.git_ref.clone(),
    };
    let source_json = serde_json::to_string_pretty(&source)?;
    fs::write(skill_dir.join(".source.json"), &source_json)?;

    // Read back the installed skill
    let skill = read_skill_dir(&skill_dir)
        .ok_or_else(|| anyhow::anyhow!("failed to read installed skill"))?;
    Ok(skill)
}

/// GitHub Contents API response item.
#[derive(Deserialize)]
struct GitHubContent {
    name: String,
    #[serde(rename = "type")]
    content_type: String,
    download_url: Option<String>,
    path: String,
}

/// Recursively download a directory from GitHub using the Contents API.
async fn download_github_dir(
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
                Box::pin(download_github_dir(
                    client, repo, git_ref, &sub_path, &sub_dir,
                ))
                .await?;
            }
            _ => {}
        }
    }

    Ok(())
}
