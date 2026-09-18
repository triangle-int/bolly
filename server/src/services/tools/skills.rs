use std::path::{Path, PathBuf};

use crate::services::tool::{Tool, ToolDefinition};
use schemars::JsonSchema;
use serde::Deserialize;

use super::{ToolExecError, openai_schema};

// ---------------------------------------------------------------------------
// list_skills
// ---------------------------------------------------------------------------

pub struct ListSkillsTool {
    workspace_dir: PathBuf,
}

impl ListSkillsTool {
    pub fn new(workspace_dir: &Path, _api_key: &str) -> Self {
        Self {
            workspace_dir: workspace_dir.to_path_buf(),
        }
    }
}

#[derive(Deserialize, JsonSchema)]
pub struct ListSkillsArgs {
    /// Optional filter: "enabled", "disabled", or "all" (default: "all").
    pub filter: Option<String>,
}

impl Tool for ListSkillsTool {
    const NAME: &'static str = "list_skills";
    type Error = ToolExecError;
    type Args = ListSkillsArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "list_skills".into(),
            description: "List installed skills with name, description, and status.".into(),
            parameters: openai_schema::<ListSkillsArgs>(),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let skills = crate::services::skills::list_skills(&self.workspace_dir);

        if skills.is_empty() {
            return Ok("no skills installed".into());
        }

        let filter = args.filter.as_deref().unwrap_or("all").to_lowercase();
        let filtered: Vec<_> = skills
            .iter()
            .filter(|s| match filter.as_str() {
                "enabled" => s.enabled,
                "disabled" => !s.enabled,
                _ => true,
            })
            .collect();

        if filtered.is_empty() {
            return Ok(format!("no {filter} skills"));
        }

        let mut out = String::new();
        for s in &filtered {
            let source = s
                .source
                .as_ref()
                .map(|src| format!(" (from {})", src.repo))
                .unwrap_or_default();
            out.push_str(&format!(
                "- {} [local]{}: {}\n",
                s.name, source, s.description
            ));
        }
        Ok(out)
    }
}

// ---------------------------------------------------------------------------
// activate_skill
// ---------------------------------------------------------------------------

pub struct ActivateSkillTool {
    workspace_dir: PathBuf,
}

impl ActivateSkillTool {
    pub fn new(workspace_dir: &Path, _api_key: &str) -> Self {
        Self {
            workspace_dir: workspace_dir.to_path_buf(),
        }
    }
}

#[derive(Deserialize, JsonSchema)]
pub struct ActivateSkillArgs {
    /// The name of the skill to activate (must match an installed, enabled skill).
    pub skill_name: String,
}

impl Tool for ActivateSkillTool {
    const NAME: &'static str = "activate_skill";
    type Error = ToolExecError;
    type Args = ActivateSkillArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "activate_skill".into(),
            description: "Activate a skill before using it. Returns instructions for the skill."
                .into(),
            parameters: openai_schema::<ActivateSkillArgs>(),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let skills = crate::services::skills::list_skills(&self.workspace_dir);
        let needle = args.skill_name.to_lowercase();
        let found = skills
            .iter()
            .find(|s| s.name.to_lowercase() == needle || s.id == needle);
        match found {
            Some(s) if s.enabled => {
                // Local skill: return instructions (prompt injection)
                let mut result = format!("# {} skill activated\n\n{}", s.name, s.instructions);
                let refs: Vec<_> = s
                    .resources
                    .iter()
                    .filter(|r| r.starts_with("references/"))
                    .collect();
                if !refs.is_empty() {
                    result.push_str("\n\n## reference files\n\
                        **MANDATORY**: read these BEFORE running any commands — use `read_skill_reference` with skill_id=\"");
                    result.push_str(&s.id);
                    result.push_str("\":\n");
                    for r in refs {
                        result.push_str(&format!("- {}\n", r));
                    }
                }
                Ok(result)
            }
            Some(s) => Err(ToolExecError(format!("skill '{}' is disabled", s.name))),
            None => Err(ToolExecError(format!(
                "skill '{}' not found",
                args.skill_name
            ))),
        }
    }
}

// ---------------------------------------------------------------------------
// read_skill_reference
// ---------------------------------------------------------------------------

pub struct ReadSkillReferenceTool {
    workspace_dir: PathBuf,
}

impl ReadSkillReferenceTool {
    pub fn new(workspace_dir: &Path) -> Self {
        Self {
            workspace_dir: workspace_dir.to_path_buf(),
        }
    }
}

#[derive(Deserialize, JsonSchema)]
pub struct ReadSkillReferenceArgs {
    /// The skill ID (directory name) of the installed skill.
    pub skill_id: String,
    /// The resource path to read, e.g. "references/core-exporting.md".
    pub filename: String,
}

impl Tool for ReadSkillReferenceTool {
    const NAME: &'static str = "read_skill_reference";
    type Error = ToolExecError;
    type Args = ReadSkillReferenceArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "read_skill_reference".into(),
            description: "Read a reference file bundled with a skill.".into(),
            parameters: openai_schema::<ReadSkillReferenceArgs>(),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let skill = crate::services::skills::get_skill(&self.workspace_dir, &args.skill_id)
            .ok_or_else(|| ToolExecError(format!("skill '{}' not found", args.skill_id)))?;

        if !skill.resources.contains(&args.filename) {
            let available = if skill.resources.is_empty() {
                "this skill has no reference files".to_string()
            } else {
                format!("available: {}", skill.resources.join(", "))
            };
            return Err(ToolExecError(format!(
                "'{}' not found in skill '{}'. {}",
                args.filename, args.skill_id, available
            )));
        }

        let path = self
            .workspace_dir
            .join("skills")
            .join(&args.skill_id)
            .join(&args.filename);

        std::fs::read_to_string(&path)
            .map_err(|e| ToolExecError(format!("failed to read '{}': {}", args.filename, e)))
    }
}
