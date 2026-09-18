use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Whether a skill runs locally or via a remote provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SkillKind {
    Local,
}

impl Default for SkillKind {
    fn default() -> Self {
        SkillKind::Local
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub builtin: bool,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub kind: SkillKind,
    /// Legacy field — kept for backwards compatibility with existing skill data.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anthropic_skill_id: Option<String>,
    /// Legacy field — kept for backwards compatibility with existing skill data.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anthropic_version: Option<String>,
    /// System prompt fragment injected when this skill is active (SKILL.md body).
    #[serde(default)]
    pub instructions: String,
    /// Where this skill was installed from (None = user-created).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<SkillSource>,
    /// Bundled resource files (references/, scripts/, assets/).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resources: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillSource {
    /// GitHub repo in "owner/repo" format.
    pub repo: String,
    /// Git ref that was installed (branch, tag, or commit SHA).
    pub version: String,
}

/// YAML frontmatter parsed from a SKILL.md file (Agent Skills spec).
#[derive(Debug, Clone, Default, Deserialize)]
#[allow(dead_code)]
pub struct SkillFrontmatter {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub compatibility: Option<String>,
    #[serde(default)]
    pub metadata: HashMap<String, String>,
    #[serde(default, rename = "allowed-tools")]
    pub allowed_tools: Option<String>,
}

/// An entry in the remote skills registry index.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryEntry {
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub icon: String,
    /// GitHub repo "owner/repo".
    pub repo: String,
    /// Branch or tag to fetch from.
    #[serde(default = "default_git_ref")]
    pub git_ref: String,
    /// Author display name.
    #[serde(default)]
    pub author: String,
    /// Subdirectory path within the repo (e.g. "skills/slidev").
    #[serde(default)]
    pub path: String,
}

fn default_git_ref() -> String {
    "main".into()
}

/// Parse SKILL.md content into frontmatter + body.
pub fn parse_skill_md(content: &str) -> (SkillFrontmatter, String) {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return (SkillFrontmatter::default(), content.to_string());
    }

    // Find closing ---
    let after_open = &trimmed[3..];
    if let Some(close_pos) = after_open.find("\n---") {
        let yaml_str = &after_open[..close_pos];
        let body = after_open[close_pos + 4..].trim_start().to_string();

        let frontmatter: SkillFrontmatter = serde_yml::from_str(yaml_str).unwrap_or_default();

        (frontmatter, body)
    } else {
        (SkillFrontmatter::default(), content.to_string())
    }
}
