use serde::{Deserialize, Serialize};

/// Configuration for a child agent — a specialized sub-agent with its own
/// schedule, prompt, and model. Stored as TOML in instances/{slug}/agents/{name}.toml.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ChildAgentConfig {
    /// Human-readable name (e.g. "reflection", "email-monitor").
    pub name: String,
    /// What this agent does — shown in thoughts and logs.
    pub description: String,
    /// System prompt: defines the agent's personality and task.
    pub prompt: String,
    /// How often to run, in hours (e.g. 72 = every 3 days, 1 = hourly).
    pub interval_hours: f64,
    /// Model tier: "heavy" (Opus), "default" (configured model), "fast" (Sonnet), "cheap" (Haiku).
    #[serde(default = "default_model")]
    pub model: String,
    /// Whether to run triage before waking (cheap model decides if agent should run).
    /// If false, runs every time the interval elapses.
    #[serde(default)]
    pub triage: bool,
    /// Whether the agent has access to tools. If false, just generates text.
    #[serde(default = "default_true")]
    pub tools: bool,
    /// Whether this agent is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Tool groups this agent has access to.
    /// Available: memory, creative, communication, files, commands, email, computer, media
    /// Empty = all basic groups (memory, creative, communication, files, commands).
    #[serde(default)]
    pub tool_groups: Vec<String>,
}

fn default_model() -> String {
    "default".to_string()
}
fn default_true() -> bool {
    true
}
