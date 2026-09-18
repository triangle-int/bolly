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
    #[serde(default)]
    pub embedding: EmbeddingConfig,
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

/// Independent text embedding configuration. Credentials stay in llm.tokens.OPEN_AI.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct EmbeddingConfig {
    pub version: u32,
    pub enabled: bool,
    /// `openai` or an explicitly configured `openai_compatible` backend.
    pub provider: String,
    pub model: String,
    pub dimensions: u32,
    pub base_url: String,
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
            version: 1,
            enabled: true,
            provider: "openai".into(),
            model: "text-embedding-3-small".into(),
            dimensions: 768,
            base_url: "https://api.openai.com/v1".into(),
        }
    }
}

impl EmbeddingConfig {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.version != 1 {
            return Err("unsupported embedding config version");
        }
        if !matches!(self.provider.as_str(), "openai" | "openai_compatible") {
            return Err("embedding provider is unconfigured or unsupported");
        }
        if self.model.trim().is_empty() || self.model.len() > 1024 {
            return Err("invalid embedding model");
        }
        if self.dimensions == 0 || self.dimensions > 16384 {
            return Err("invalid embedding dimensions");
        }
        let url = self.endpoint()?;
        if self.provider == "openai" {
            if self.base_url != "https://api.openai.com/v1" {
                return Err("OpenAI embeddings require https://api.openai.com/v1 exactly");
            }
        } else {
            let host = url.host_str().unwrap_or_default();
            let ip_literal = host
                .strip_prefix('[')
                .and_then(|host| host.strip_suffix(']'))
                .unwrap_or(host);
            let loopback = host.eq_ignore_ascii_case("localhost")
                || ip_literal
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|address| address.is_loopback());
            if !loopback {
                return Err("OpenAI-compatible embeddings require a loopback endpoint");
            }
        }
        Ok(())
    }

    fn endpoint(&self) -> Result<reqwest::Url, &'static str> {
        let url = reqwest::Url::parse(&self.base_url).map_err(|_| "invalid embedding base URL")?;
        if !matches!(url.scheme(), "https" | "http")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err("embedding base URL must not contain credentials, query, or fragment");
        }
        Ok(url)
    }

    pub fn unavailable_reason(&self, key: &str) -> Option<&'static str> {
        if !self.enabled {
            return Some("embeddings disabled");
        }
        if let Err(reason) = self.validate() {
            return Some(reason);
        }
        if self.provider == "openai" && key.trim().is_empty() {
            return Some("OpenAI embedding API key is missing");
        }
        None
    }

    /// Never return credentials accidentally placed in a URL.
    pub fn safe_status(&self, key: &str) -> serde_json::Value {
        let reason = self.unavailable_reason(key);
        serde_json::json!({
            "version": self.version, "enabled": self.enabled,
            "provider": self.provider, "model": self.model, "dimensions": self.dimensions,
            "base_url": self.endpoint().ok().map(|url| url.to_string()),
            "authentication": if self.provider == "openai" { "OpenAI bearer key" } else { "none" },
            "configured": reason.is_none(), "reason": reason,
            "status": if reason.is_some() { "unavailable" } else { "unverified" },
            "fallback": "bm25", "changes_require_restart": true,
            "update_semantics": "full_replacement",
        })
    }
}

impl Config {
    pub fn embedding_status(&self) -> serde_json::Value {
        self.embedding.safe_status(&self.llm.tokens.open_ai)
    }
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

fn default_smtp_port() -> u16 {
    587
}
fn default_imap_port() -> u16 {
    993
}

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

fn default_skin() -> String {
    "orb".to_string()
}

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
                return wrapper
                    .accounts
                    .into_iter()
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
    pub fn save(
        accounts: &[EmailConfig],
        workspace_dir: &Path,
        instance_slug: &str,
    ) -> anyhow::Result<()> {
        let dir = workspace_dir.join("instances").join(instance_slug);
        fs::create_dir_all(&dir)?;
        let wrapper = EmailAccounts {
            accounts: accounts.to_vec(),
        };
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

/// Model names belong to configuration, not provider identity.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ProviderProfile {
    pub heavy: String,
    pub fast: String,
    pub cheap: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct ProviderProfiles {
    #[serde(deserialize_with = "anthropic_profile")]
    pub anthropic: ProviderProfile,
    #[serde(deserialize_with = "openai_profile")]
    pub openai: ProviderProfile,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct PartialProfile {
    heavy: Option<String>,
    fast: Option<String>,
    cheap: Option<String>,
}
fn read_profile<'de, D: serde::Deserializer<'de>>(
    d: D,
    mut base: ProviderProfile,
) -> Result<ProviderProfile, D::Error> {
    let p = PartialProfile::deserialize(d)?;
    if let Some(v) = p.heavy.filter(|s| !s.is_empty()) {
        base.heavy = v;
    }
    if let Some(v) = p.fast.filter(|s| !s.is_empty()) {
        base.fast = v;
    }
    if let Some(v) = p.cheap.filter(|s| !s.is_empty()) {
        base.cheap = v;
    }
    Ok(base)
}
fn anthropic_profile<'de, D: serde::Deserializer<'de>>(d: D) -> Result<ProviderProfile, D::Error> {
    read_profile(d, ProviderProfiles::default().anthropic)
}
fn openai_profile<'de, D: serde::Deserializer<'de>>(d: D) -> Result<ProviderProfile, D::Error> {
    read_profile(d, ProviderProfiles::default().openai)
}

impl Default for ProviderProfiles {
    fn default() -> Self {
        Self {
            anthropic: ProviderProfile {
                heavy: "claude-opus-4-6".into(),
                fast: "claude-sonnet-4-6".into(),
                cheap: "claude-haiku-4-5-20251001".into(),
            },
            openai: ProviderProfile {
                heavy: "gpt-5.4".into(),
                fast: "gpt-5.4".into(),
                cheap: "gpt-5.4-mini".into(),
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LlmProvider {
    /// Direct Anthropic API (requires API key). Format: Anthropic Messages.
    Anthropic,
    /// OpenAI API (requires API key). Format: OpenAI Responses.
    Openai,
    /// Legacy selection retained for setup; no Codex adapter is implemented.
    Codex,
}

// Read legacy names, but always serialize the canonical provider name.
impl<'de> serde::Deserialize<'de> for LlmProvider {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        match s.as_str() {
            "api" | "anthropic" | "claude_cli" | "cli" => Ok(Self::Anthropic),
            "openai" => Ok(Self::Openai),
            "codex" => Ok(Self::Codex),
            other => Err(serde::de::Error::unknown_variant(
                other,
                &["anthropic", "openai", "codex"],
            )),
        }
    }
}

impl Default for LlmProvider {
    fn default() -> Self {
        LlmProvider::Anthropic
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelMode {
    Auto,
    Fast,
    Heavy,
}

impl Default for ModelMode {
    fn default() -> Self {
        ModelMode::Auto
    }
}

fn default_heavy_multiplier() -> f32 {
    1.7
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(from = "LegacyLlmConfig")]
pub struct LlmConfig {
    #[serde(default)]
    pub provider: LlmProvider,
    #[serde(default)]
    pub tokens: LlmTokens,
    #[serde(default)]
    pub model_mode: ModelMode,
    #[serde(default = "default_heavy_multiplier")]
    pub heavy_multiplier: f32,
    #[serde(default)]
    pub profiles: ProviderProfiles,
    #[serde(flatten)]
    pub extra: std::collections::BTreeMap<String, toml::Value>,
}

#[derive(Deserialize)]
struct LegacyLlmConfig {
    #[serde(default)]
    pub provider: LlmProvider,
    #[serde(default)]
    pub tokens: LlmTokens,
    #[serde(default)]
    pub model_mode: ModelMode,
    #[serde(default = "default_heavy_multiplier")]
    pub heavy_multiplier: f32,
    #[serde(default)]
    pub profiles: ProviderProfiles,
    /// Preserve legacy model overrides (including the old example config).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(flatten)]
    pub extra: std::collections::BTreeMap<String, toml::Value>,
}

impl From<LegacyLlmConfig> for LlmConfig {
    fn from(mut old: LegacyLlmConfig) -> Self {
        if let Some(model) = old.model.take().filter(|s| !s.is_empty()) {
            // Claude overrides originate from Anthropic even if the provider was
            // subsequently switched in an old config. Unknown names use that
            // config's selected provider as their origin.
            let profile = if model.starts_with("claude-")
                || (old.provider != LlmProvider::Openai
                    && !["gpt-", "chatgpt-", "o1", "o3", "o4"]
                        .iter()
                        .any(|prefix| model.starts_with(prefix)))
            {
                &mut old.profiles.anthropic
            } else {
                &mut old.profiles.openai
            };
            profile.heavy = model;
        }
        Self {
            provider: old.provider,
            tokens: old.tokens,
            model_mode: old.model_mode,
            heavy_multiplier: old.heavy_multiplier,
            profiles: old.profiles,
            extra: old.extra,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct LlmTokens {
    #[serde(default, rename = "OPEN_AI", alias = "open_ai", alias = "openai")]
    pub open_ai: String,
    #[serde(default, rename = "ANTHROPIC", alias = "anthropic")]
    pub anthropic: String,
    #[serde(
        default,
        rename = "BRAVE_SEARCH",
        alias = "brave_search",
        alias = "brave"
    )]
    pub brave_search: String,
    #[serde(
        default,
        rename = "OPENROUTER",
        alias = "open_router",
        alias = "openrouter"
    )]
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
            embedding: EmbeddingConfig::default(),
            registry_url: default_registry_url(),
            plan: String::new(),
            mcp_servers: Vec::new(),
            github: GithubConfig::default(),
        }
    }
}

impl LlmConfig {
    pub fn profile(&self) -> &ProviderProfile {
        match self.provider {
            LlmProvider::Openai => &self.profiles.openai,
            // Unconfigured legacy selections cannot construct a backend.
            _ => &self.profiles.anthropic,
        }
    }

    pub fn setup_required(&self) -> Option<&'static str> {
        if self.provider == LlmProvider::Codex {
            Some(
                "Codex requires setup and is not supported yet. Select Anthropic or OpenAI and configure its API key.",
            )
        } else if !self.is_configured() {
            Some("Configure an API key for the selected provider.")
        } else {
            None
        }
    }

    /// The heavy model for the current provider.
    pub fn model_name(&self) -> &str {
        if self.provider == LlmProvider::Codex {
            ""
        } else {
            &self.profile().heavy
        }
    }

    /// The fast model for the current provider.
    pub fn fast_model_name(&self) -> &str {
        if self.provider == LlmProvider::Codex {
            ""
        } else {
            &self.profile().fast
        }
    }

    /// The Anthropic API key, or None if not configured.
    pub fn api_key(&self) -> Option<&str> {
        if self.tokens.anthropic.is_empty() {
            None
        } else {
            Some(&self.tokens.anthropic)
        }
    }

    /// Whether the LLM is fully configured.
    pub fn is_configured(&self) -> bool {
        match self.provider {
            LlmProvider::Anthropic => self.api_key().is_some(),
            LlmProvider::Openai => !self.tokens.open_ai.is_empty(),
            LlmProvider::Codex => false,
        }
    }

    /// List of service names that have API keys set.
    pub fn configured_providers(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if !self.tokens.anthropic.is_empty() {
            out.push("anthropic");
        }
        if !self.tokens.open_ai.is_empty() {
            out.push("openai");
        }
        if !self.tokens.open_router.is_empty() {
            out.push("openrouter");
        }
        if !self.tokens.brave_search.is_empty() {
            out.push("brave_search");
        }
        out
    }

    /// Get Anthropic API key + model (for count_tokens API etc.).
    pub fn anthropic_credentials(&self) -> Option<(&str, &str)> {
        if self.provider != LlmProvider::Anthropic {
            return None;
        }
        let key = if self.tokens.anthropic.is_empty() {
            return None;
        } else {
            &self.tokens.anthropic
        };
        Some((
            key,
            if self.model_mode == ModelMode::Fast {
                self.fast_model_name()
            } else {
                self.model_name()
            },
        ))
    }
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            provider: LlmProvider::default(),
            tokens: LlmTokens::default(),
            model_mode: ModelMode::default(),
            heavy_multiplier: default_heavy_multiplier(),
            profiles: ProviderProfiles::default(),
            extra: Default::default(),
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

/// Merge updates into the existing document so unrelated/forward-compatible keys
/// survive a settings save. Arrays are replaced intentionally (e.g. MCP removal).
pub fn serialize_config_preserving_keys(config: &Config, original: &str) -> anyhow::Result<String> {
    fn merge(existing: &mut toml::Value, updated: toml::Value) {
        match (existing, updated) {
            (toml::Value::Table(existing), toml::Value::Table(updated)) => {
                for (key, value) in updated {
                    if let Some(old) = existing.get_mut(&key) {
                        merge(old, value);
                    } else {
                        existing.insert(key, value);
                    }
                }
            }
            (existing, updated) => *existing = updated,
        }
    }
    let mut document: toml::Value = toml::from_str(original)?;
    // Avoid duplicate fields when an old token alias and its canonical spelling
    // would otherwise coexist after merging the serialized config.
    if let Some(tokens) = document
        .get_mut("llm")
        .and_then(|v| v.get_mut("tokens"))
        .and_then(toml::Value::as_table_mut)
    {
        for alias in [
            "open_ai",
            "openai",
            "anthropic",
            "brave_search",
            "brave",
            "open_router",
            "openrouter",
            "elevenlabs",
            "google_ai",
            "gemini",
        ] {
            tokens.remove(alias);
        }
    }
    if let Some(llm) = document.get_mut("llm").and_then(toml::Value::as_table_mut) {
        llm.remove("model");
    }
    merge(&mut document, toml::Value::try_from(config)?);
    Ok(toml::to_string_pretty(&document)?)
}

#[cfg(test)]
mod llm_config_tests {
    use super::*;

    #[test]
    fn legacy_names_migrate_without_losing_config_keys() {
        for name in ["api", "anthropic", "cli", "claude_cli"] {
            let original = format!(
                r#"
custom_global = "retained"
[llm]
provider = "{name}"
model_mode = "fast"
heavy_multiplier = 2.5
model = "custom-model"
custom_llm = "retained"
[llm.tokens]
ANTHROPIC = "anthropic-secret"
OPEN_AI = "openai-secret"
OPENROUTER = "router-secret"
custom_token = "retained"
[github]
custom_github = "retained"
"#
            );
            let config: Config = toml::from_str(&original).unwrap();
            assert_eq!(config.llm.provider, LlmProvider::Anthropic);
            assert_eq!(config.llm.model_name(), "custom-model");
            let serialized = serialize_config_preserving_keys(&config, &original).unwrap();
            let result: toml::Value = toml::from_str(&serialized).unwrap();
            let source: toml::Value = toml::from_str(&original).unwrap();
            assert_eq!(result["llm"]["provider"].as_str(), Some("anthropic"));
            for (key, value) in source["llm"]["tokens"].as_table().unwrap() {
                assert_eq!(&result["llm"]["tokens"][key], value);
            }
            for key in ["model_mode", "heavy_multiplier", "custom_llm"] {
                assert_eq!(result["llm"][key], source["llm"][key]);
            }
            assert_eq!(result["custom_global"], source["custom_global"]);
            assert_eq!(
                result["github"]["custom_github"],
                source["github"]["custom_github"]
            );
            let roundtrip: Config = toml::from_str(&serialized).unwrap();
            assert_eq!(roundtrip.llm.tokens.open_ai, "openai-secret");
        }
    }

    #[test]
    fn codex_stays_unconfigured_even_with_openai_key() {
        let config: Config =
            toml::from_str("[llm]\nprovider='codex'\n[llm.tokens]\nOPEN_AI='key'").unwrap();
        assert_eq!(config.llm.provider, LlmProvider::Codex);
        assert!(!config.llm.is_configured());
        assert!(config.llm.setup_required().unwrap().contains("Codex"));
        assert!(matches!(
            crate::services::llm::LlmBackend::from_config(&config)
                .unwrap()
                .adapter(),
            Err(crate::services::llm::contract::LlmError::SetupRequired(_))
        ));
        let roundtrip: Config = toml::from_str(&toml::to_string(&config).unwrap()).unwrap();
        assert_eq!(roundtrip.llm.provider, LlmProvider::Codex);
    }

    #[test]
    fn legacy_models_are_scoped_before_provider_switch_and_save() {
        for selected in ["api", "openai"] {
            let raw = format!(
                "[llm]\nprovider='{selected}'\nmodel='claude-historical'\n[llm.tokens]\nANTHROPIC='key'\nOPEN_AI='key'"
            );
            let mut config: Config = toml::from_str(&raw).unwrap();
            assert_eq!(config.llm.profiles.anthropic.heavy, "claude-historical");
            config.llm.provider = LlmProvider::Openai;
            assert_eq!(
                config.llm.model_name(),
                ProviderProfiles::default().openai.heavy
            );
            let saved = serialize_config_preserving_keys(&config, &raw).unwrap();
            let saved_value: toml::Value = toml::from_str(&saved).unwrap();
            assert!(saved_value["llm"].get("model").is_none());
            let mut restored: Config = toml::from_str(&saved).unwrap();
            restored.llm.provider = LlmProvider::Anthropic;
            assert_eq!(
                restored.llm.anthropic_credentials().unwrap().1,
                restored.llm.model_name()
            );
            assert_eq!(restored.llm.model_name(), "claude-historical");
            restored.llm.model_mode = ModelMode::Fast;
            assert_eq!(
                restored.llm.anthropic_credentials().unwrap().1,
                restored.llm.fast_model_name()
            );
        }
    }

    #[test]
    fn partial_profiles_use_their_own_provider_defaults() {
        let config: Config = toml::from_str("[llm.profiles.openai]\nfast='custom-fast'\n[llm.profiles.anthropic]\ncheap='custom-cheap'").unwrap();
        let defaults = ProviderProfiles::default();
        assert_eq!(config.llm.profiles.openai.heavy, defaults.openai.heavy);
        assert_eq!(config.llm.profiles.openai.cheap, defaults.openai.cheap);
        assert_eq!(config.llm.profiles.openai.fast, "custom-fast");
        assert_eq!(config.llm.profiles.anthropic.fast, defaults.anthropic.fast);
        assert_eq!(config.llm.profiles.anthropic.cheap, "custom-cheap");
    }

    #[test]
    fn profiles_drive_all_backend_variants() {
        let config: Config = toml::from_str(
            r#"
[llm]
provider = "openai"
[llm.tokens]
openai = "key"
[llm.profiles.openai]
heavy = "custom-heavy"
fast = "custom-fast"
cheap = "custom-cheap"
"#,
        )
        .unwrap();
        let backend = crate::services::llm::LlmBackend::from_config(&config).unwrap();
        assert_eq!(backend.model_name(), "custom-heavy");
        assert_eq!(backend.fast_variant_with(None).model_name(), "custom-fast");
        assert_eq!(backend.cheap_variant().model_name(), "custom-cheap");
        assert_eq!(backend.heavy_variant().model_name(), "custom-heavy");
        assert_eq!(
            backend.fast_variant_with(Some("override")).model_name(),
            "override"
        );
        assert_eq!(
            config.llm.profiles.anthropic.heavy,
            ProviderProfiles::default().anthropic.heavy
        );
    }

    #[test]
    fn token_aliases_do_not_create_duplicate_keys_on_save() {
        let original = "[llm]\nprovider='api'\n[llm.tokens]\nopenai='o'\nanthropic='a'\nbrave='b'\nopenrouter='r'\nelevenlabs='e'\ngemini='g'";
        let config: Config = toml::from_str(original).unwrap();
        let serialized = serialize_config_preserving_keys(&config, original).unwrap();
        let restored: Config = toml::from_str(&serialized).unwrap();
        assert_eq!(restored.llm.tokens, config.llm.tokens);
    }

    #[test]
    fn unknown_provider_is_not_silently_replaced() {
        assert!(toml::from_str::<Config>("[llm]\nprovider='openrouter'").is_err());
    }
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

#[cfg(test)]
mod embedding_config_tests {
    use super::*;

    #[test]
    fn embedding_defaults_roundtrip_and_safe_status_are_independent_of_chat() {
        let mut cfg: Config = toml::from_str("").unwrap();
        let saved = toml::to_string(&cfg).unwrap();
        assert!(saved.contains("[embedding]"));
        let value = serde_json::to_value(&cfg).unwrap();
        assert_eq!(value["embedding"]["version"], 1);
        assert_eq!(value["embedding"]["provider"], "openai");
        assert_eq!(value["embedding"]["model"], "text-embedding-3-small");
        assert_eq!(value["embedding"]["dimensions"], 768);
        assert_eq!(value["embedding"]["base_url"], "https://api.openai.com/v1");
        for chat in [
            LlmProvider::Anthropic,
            LlmProvider::Codex,
            LlmProvider::Openai,
        ] {
            cfg.llm.provider = chat;
            cfg.llm.tokens.open_ai = "secret-openai-token".into();
            let status = cfg.embedding_status();
            assert_eq!(status["configured"], true);
            assert_eq!(status["provider"], "openai");
            assert!(!status.to_string().contains("secret-openai-token"));
        }
        let restored: Config = toml::from_str(&saved).unwrap();
        assert_eq!(restored.embedding_status()["configured"], false);
    }

    #[test]
    fn invalid_disabled_or_unconfigured_embeddings_report_bm25_without_secrets() {
        for raw in [
            "[embedding]\nenabled=false",
            "[embedding]\nprovider='google'",
            "[embedding]\nversion=99",
            "[embedding]\ndimensions=0",
            "[embedding]\nbase_url='https://user:secret@example.test/v1?key=secret'",
        ] {
            let mut cfg: Config = toml::from_str(raw).unwrap();
            cfg.llm.tokens.open_ai = "secret-token".into();
            let status = cfg.embedding_status();
            assert_eq!(status["configured"], false, "{raw}");
            assert_eq!(status["fallback"], "bm25");
            assert!(!status.to_string().contains("secret"));
        }
    }

    #[test]
    fn compatible_embeddings_allow_only_unauthenticated_loopback_endpoints() {
        for allowed in [
            "http://localhost:11434/v1",
            "http://127.0.0.1:11434/v1",
            "http://127.200.3.4:11434/v1",
            "http://[::1]:11434/v1",
        ] {
            let config = EmbeddingConfig {
                provider: "openai_compatible".into(),
                base_url: allowed.into(),
                ..EmbeddingConfig::default()
            };
            assert_eq!(config.validate(), Ok(()), "{allowed}");
            let status = config.safe_status("");
            assert_eq!(status["configured"], true);
            assert_eq!(status["authentication"], "none");
        }

        for rejected in [
            "http://example.com/v1",
            "https://10.0.0.1/v1",
            "http://192.168.1.2/v1",
            "http://169.254.169.254/latest/meta-data",
            "http://[fe80::1]/v1",
            "http://user:password@localhost:11434/v1",
            "http://localhost:11434/v1?key=value",
            "http://localhost:11434/v1#fragment",
        ] {
            let config = EmbeddingConfig {
                provider: "openai_compatible".into(),
                base_url: rejected.into(),
                ..EmbeddingConfig::default()
            };
            assert!(config.validate().is_err(), "{rejected}");
        }
    }

    #[test]
    fn official_openai_requires_the_exact_endpoint_and_key() {
        let mut config = EmbeddingConfig::default();
        assert_eq!(config.validate(), Ok(()));
        assert_eq!(
            config.unavailable_reason(""),
            Some("OpenAI embedding API key is missing")
        );
        config.base_url = "https://api.openai.com/v1/".into();
        assert!(config.validate().is_err());
    }
}
