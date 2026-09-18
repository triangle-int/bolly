use std::{
    env, fs, io,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default)]
    pub auth_token: String,
    #[serde(default)]
    pub static_dir: String,
    #[serde(default)]
    pub landing_url: String,
    #[serde(default)]
    pub llm: LlmConfig,
    #[serde(default = "default_registry_url")]
    pub registry_url: String,
    #[serde(default)]
    pub public_url: String,
    #[serde(default)]
    pub plan: String,
    #[serde(default)]
    pub mcp_servers: Vec<McpServerConfig>,
    #[serde(default)]
    pub github: GithubConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct GithubConfig {
    #[serde(default)]
    pub token: String,
}

/// A single SMTP/IMAP email account.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct EmailConfig {
    #[serde(default)]
    pub smtp_host: String,
    #[serde(default = "default_smtp_port")]
    pub smtp_port: u16,
    #[serde(default)]
    pub smtp_user: String,
    #[serde(default)]
    pub smtp_password: String,
    #[serde(default)]
    pub smtp_from: String,
    #[serde(default)]
    pub imap_host: String,
    #[serde(default = "default_imap_port")]
    pub imap_port: u16,
    #[serde(default)]
    pub imap_user: String,
    #[serde(default)]
    pub imap_password: String,
}

fn default_smtp_port() -> u16 { 587 }
fn default_imap_port() -> u16 { 993 }

/// Per-instance configuration stored at `instances/{slug}/instance.toml`.
/// Holds settings that are specific to one user/instance, such as GitHub token.
/// Takes precedence over global `config.toml` for the same fields.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct InstanceConfig {
    #[serde(default)]
    pub github: GithubConfig,
    /// ElevenLabs voice ID override for this instance.
    #[serde(default)]
    pub elevenlabs_voice_id: String,
    /// Whether voice mode (TTS) is enabled. Default: false.
    #[serde(default)]
    pub voice_enabled: bool,
    /// Visual skin for this instance (e.g. "orb", "mint"). Default: "orb".
    #[serde(default = "default_skin")]
    pub skin: String,
    /// Whether to record the user's screen between heartbeats and analyze it.
    /// Disabled by default — the agent can suggest enabling it.
    #[serde(default)]
    pub screen_recording: bool,
}

fn default_skin() -> String { "orb".to_string() }

impl InstanceConfig {
    /// Load per-instance config from `instances/{slug}/instance.toml`.
    /// Returns default (empty) config if the file doesn't exist.
    pub fn load(workspace_dir: &Path, instance_slug: &str) -> Self {
        let path = workspace_dir
            .join("instances")
            .join(instance_slug)
            .join("instance.toml");
        let raw = match fs::read_to_string(&path) {
            Ok(r) => r,
            Err(_) => return Self::default(),
        };
        toml::from_str::<InstanceConfig>(&raw).unwrap_or_default()
    }

    /// Save per-instance config to `instances/{slug}/instance.toml`.
    pub fn save(&self, workspace_dir: &Path, instance_slug: &str) -> anyhow::Result<()> {
        let dir = workspace_dir.join("instances").join(instance_slug);
        fs::create_dir_all(&dir)?;
        let raw = toml::to_string_pretty(self)?;
        fs::write(dir.join("instance.toml"), raw)?;
        Ok(())
    }

    /// Return the effective GitHub token: instance-level if set, otherwise fall back to global.
    #[allow(dead_code)]
    pub fn effective_github_token<'a>(&'a self, global: &'a Config) -> Option<&'a str> {
        if !self.github.token.is_empty() {
            return Some(&self.github.token);
        }
        if !global.github.token.is_empty() {
            return Some(&global.github.token);
        }
        None
    }
}

/// Wrapper for `instances/{slug}/email.toml` — supports multiple accounts.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct EmailAccounts {
    #[serde(default)]
    pub accounts: Vec<EmailConfig>,
}

impl EmailAccounts {
    /// Load email accounts from `instances/{slug}/email.toml`.
    /// Supports both legacy flat format (single account) and `[[accounts]]` array.
    pub fn load(workspace_dir: &Path, instance_slug: &str) -> Vec<EmailConfig> {
        let path = workspace_dir
            .join("instances")
            .join(instance_slug)
            .join("email.toml");
        let raw = match fs::read_to_string(&path) {
            Ok(r) => r,
            Err(_) => return vec![],
        };

        // Try new format first: [[accounts]]
        if let Ok(wrapper) = toml::from_str::<EmailAccounts>(&raw) {
            if !wrapper.accounts.is_empty() {
                return wrapper.accounts.into_iter()
                    .filter(|c| !c.smtp_host.is_empty() || !c.imap_host.is_empty())
                    .collect();
            }
        }

        // Fall back to legacy flat format (single account)
        if let Ok(cfg) = toml::from_str::<EmailConfig>(&raw) {
            if !cfg.smtp_host.is_empty() || !cfg.imap_host.is_empty() {
                return vec![cfg];
            }
        }

        vec![]
    }

    /// Save email accounts to `instances/{slug}/email.toml`.
    pub fn save(accounts: &[EmailConfig], workspace_dir: &Path, instance_slug: &str) -> anyhow::Result<()> {
        let dir = workspace_dir.join("instances").join(instance_slug);
        fs::create_dir_all(&dir)?;
        let wrapper = EmailAccounts { accounts: accounts.to_vec() };
        let raw = toml::to_string_pretty(&wrapper)?;
        fs::write(dir.join("email.toml"), raw)?;
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct McpServerConfig {
    /// Human-readable name for this MCP server.
    pub name: String,
    /// URL for HTTP/SSE transport (e.g. "https://mcp.excalidraw.com/mcp").
    pub url: Option<String>,
    /// Command for stdio transport (e.g. "npx" or "node").
    pub command: Option<String>,
    /// Arguments for the stdio command.
    #[serde(default)]
    pub args: Vec<String>,
    /// HTTP headers to send with requests / env vars for stdio servers.
    #[serde(default)]
    pub headers: std::collections::HashMap<String, String>,
}

/// Default heavy model (Opus).
impl LlmProvider {
    /// Heavy model for complex tasks.
    pub fn heavy_model(&self) -> &'static str {
        match self {
            Self::Api => "claude-opus-4-6",
            Self::Openai => "gpt-5.4",
        }
    }

    /// Fast model for casual tasks.
    pub fn fast_model(&self) -> &'static str {
        match self {
            Self::Api => "claude-sonnet-4-6",
            Self::Openai => "gpt-5.4",
        }
    }

    /// Cheapest model for background tasks (classification, extraction).
    pub fn cheap_model(&self) -> &'static str {
        match self {
            Self::Api => "claude-haiku-4-5-20251001",
            Self::Openai => "gpt-5.4-mini",
        }
    }
}


#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LlmProvider {
    /// Direct Anthropic API (requires API key). Format: Anthropic Messages.
    Api,
    /// OpenAI API (requires API key). Format: OpenAI Responses.
    Openai,
}

// Custom deserializer: migrate removed providers (claude_cli → api, codex → openai)
impl<'de> serde::Deserialize<'de> for LlmProvider {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        match s.as_str() {
            "api" => Ok(LlmProvider::Api),
            "openai" => Ok(LlmProvider::Openai),
            // Migration: removed providers fall back to their closest replacement
            "claude_cli" | "cli" => {
                log::warn!("migrating provider 'claude_cli' → 'api' (Claude CLI removed)");
                Ok(LlmProvider::Api)
            }
            "codex" => {
                log::warn!("migrating provider 'codex' → 'openai' (Codex removed)");
                Ok(LlmProvider::Openai)
            }
            other => Err(serde::de::Error::unknown_variant(other, &["api", "openai"])),
        }
    }
}

impl LlmProvider {
    /// Whether this provider uses OpenAI chat completions format (vs Anthropic messages).
    pub fn is_openai_format(&self) -> bool {
        matches!(self, LlmProvider::Openai)
    }
}

impl Default for LlmProvider {
    fn default() -> Self { LlmProvider::Api }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelMode {
    Auto,
    Fast,
    Heavy,
}

impl Default for ModelMode {
    fn default() -> Self { ModelMode::Auto }
}

fn default_heavy_multiplier() -> f32 { 1.7 }

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LlmConfig {
    #[serde(default)]
    pub provider: LlmProvider,
    #[serde(default)]
    pub tokens: LlmTokens,
    #[serde(default)]
    pub model_mode: ModelMode,
    #[serde(default = "default_heavy_multiplier")]
    pub heavy_multiplier: f32,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct LlmTokens {
    #[serde(default, rename = "OPEN_AI", alias = "open_ai", alias = "openai")]
    pub open_ai: String,
    #[serde(default, rename = "ANTHROPIC", alias = "anthropic")]
    pub anthropic: String,
    #[serde(default, rename = "BRAVE_SEARCH", alias = "brave_search", alias = "brave")]
    pub brave_search: String,
    #[serde(default, rename = "OPENROUTER", alias = "open_router", alias = "openrouter")]
    pub open_router: String,
    #[serde(default, rename = "ELEVENLABS", alias = "elevenlabs")]
    pub elevenlabs: String,
    #[serde(default, rename = "GOOGLE_AI", alias = "google_ai", alias = "gemini")]
    pub google_ai: String,
}


fn default_host() -> String {
    "0.0.0.0".into()
}

fn default_port() -> u16 {
    26559
}

fn default_registry_url() -> String {
    "https://raw.githubusercontent.com/triangle-int/bolly-skills/main/registry.json".into()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_port(),
            auth_token: String::new(),
            static_dir: String::new(),
            landing_url: String::new(),
            public_url: String::new(),
            llm: LlmConfig::default(),
            registry_url: default_registry_url(),
            plan: String::new(),
            mcp_servers: Vec::new(),
            github: GithubConfig::default(),
        }
    }
}

impl LlmConfig {
    /// The heavy model for the current provider.
    pub fn model_name(&self) -> &'static str {
        self.provider.heavy_model()
    }

    /// The fast model for the current provider.
    pub fn fast_model_name(&self) -> &'static str {
        self.provider.fast_model()
    }

    /// The Anthropic API key, or None if not configured.
    pub fn api_key(&self) -> Option<&str> {
        if self.tokens.anthropic.is_empty() { None } else { Some(&self.tokens.anthropic) }
    }

    /// Whether the LLM is fully configured.
    pub fn is_configured(&self) -> bool {
        match self.provider {
            LlmProvider::Api => self.api_key().is_some(),
            LlmProvider::Openai => !self.tokens.open_ai.is_empty(),
        }
    }

    /// List of service names that have API keys set.
    pub fn configured_providers(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if !self.tokens.anthropic.is_empty() { out.push("anthropic"); }
        if !self.tokens.open_ai.is_empty() { out.push("openai"); }
        if !self.tokens.open_router.is_empty() { out.push("openrouter"); }
        if !self.tokens.brave_search.is_empty() { out.push("brave_search"); }
        out
    }

    /// Get Anthropic API key + model (for count_tokens API etc.).
    pub fn anthropic_credentials(&self) -> Option<(&str, &str)> {
        let key = if self.tokens.anthropic.is_empty() { return None } else { &self.tokens.anthropic };
        Some((key, self.model_name()))
    }
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            provider: LlmProvider::default(),
            tokens: LlmTokens::default(),
            model_mode: ModelMode::default(),
            heavy_multiplier: default_heavy_multiplier(),
        }
    }
}

impl Default for LlmTokens {
    fn default() -> Self {
        Self {
            open_ai: String::new(),
            anthropic: String::new(),
            brave_search: String::new(),
            open_router: String::new(),
            elevenlabs: String::new(),
            google_ai: String::new(),
        }
    }
}

pub fn workspace_root() -> PathBuf {
    if let Some(path) = env::var_os("BOLLY_HOME") {
        return PathBuf::from(path);
    }

    dirs::home_dir()
        .expect("failed to resolve home directory")
        .join(".bolly")
}

pub fn config_path() -> PathBuf {
    workspace_root().join("config.toml")
}

fn ensure_config_exists(path: &Path) -> io::Result<()> {
    if path.exists() {
        return Ok(());
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let default_config =
        toml::to_string_pretty(&Config::default()).expect("default config should serialize");
    fs::write(path, default_config)
}

fn ensure_workspace_layout(path: &Path) -> io::Result<()> {
    fs::create_dir_all(path)?;
    fs::create_dir_all(path.join("instances"))?;
    fs::create_dir_all(path.join("skills"))?;
    Ok(())
}

pub fn load_config() -> anyhow::Result<Config> {
    let path = config_path();
    ensure_config_exists(&path)?;
    let raw = fs::read_to_string(&path)?;
    let mut config: Config = toml::from_str(&raw)?;
    ensure_workspace_layout(&workspace_root())?;

    // Allow env var overrides for managed hosting
    if let Ok(token) = env::var("BOLLY_AUTH_TOKEN") {
        if !token.is_empty() {
            config.auth_token = token;
        }
    }

    if let Ok(url) = env::var("LANDING_URL") {
        if !url.is_empty() {
            config.landing_url = url;
        }
    }
    if let Ok(url) = env::var("BOLLY_PUBLIC_URL") {
        if !url.is_empty() {
            config.public_url = url;
        }
    }

    // PORT env var override (set by Fly.io machine config)
    if let Ok(port) = env::var("PORT") {
        if let Ok(p) = port.parse::<u16>() {
            config.port = p;
        }
    }

    // API key overrides from env (for managed hosting)
    if let Ok(key) = env::var("ANTHROPIC_API_KEY") {
        if !key.is_empty() {
            config.llm.tokens.anthropic = key;
        }
    }
    if let Ok(key) = env::var("OPENAI_API_KEY") {
        if !key.is_empty() {
            config.llm.tokens.open_ai = key;
        }
    }
    if let Ok(key) = env::var("OPENROUTER_API_KEY") {
        if !key.is_empty() {
            config.llm.tokens.open_router = key;
        }
    }
    if let Ok(key) = env::var("BRAVE_SEARCH_API_KEY") {
        if !key.is_empty() {
            config.llm.tokens.brave_search = key;
        }
    }
    if let Ok(key) = env::var("ELEVENLABS_API_KEY") {
        if !key.is_empty() {
            config.llm.tokens.elevenlabs = key;
        }
    }
    if let Ok(key) = env::var("GOOGLE_AI_API_KEY") {
        if !key.is_empty() {
            config.llm.tokens.google_ai = key;
        }
    }
    // GitHub token override
    if let Ok(token) = env::var("GITHUB_TOKEN") {
        if !token.is_empty() {
            config.github.token = token;
        }
    }

    if let Ok(mode) = env::var("BOLLY_MODEL_MODE") {
        match mode.to_lowercase().as_str() {
            "auto" => config.llm.model_mode = ModelMode::Auto,
            "fast" => config.llm.model_mode = ModelMode::Fast,
            "heavy" => config.llm.model_mode = ModelMode::Heavy,
            _ => {}
        }
    }

    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::InstanceConfig;

    #[test]
    fn legacy_instance_config_preserves_voice_settings() {
        for enabled in [true, false] {
            let config: InstanceConfig = toml::from_str(&format!(
                "music_enabled = {enabled}\nvoice_enabled = true\nelevenlabs_voice_id = 'test-voice'\n"
            )).unwrap();
            assert!(config.voice_enabled);
            assert_eq!(config.elevenlabs_voice_id, "test-voice");
            let saved = toml::to_string(&config).unwrap();
            assert!(!saved.contains("music_enabled"));
            let reloaded: InstanceConfig = toml::from_str(&saved).unwrap();
            assert!(reloaded.voice_enabled);
            assert_eq!(reloaded.elevenlabs_voice_id, "test-voice");
        }
    }
}
