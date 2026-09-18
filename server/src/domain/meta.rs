use serde::Serialize;

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    /// Unix timestamp (seconds) when this response was generated.
    pub timestamp: u64,
    /// How long the server has been running, in seconds.
    pub uptime_secs: u64,
}

#[derive(Serialize)]
pub struct ServerMetaResponse {
    pub app: &'static str,
    pub version: &'static str,
    pub commit: &'static str,
    pub port: u16,
    pub workspace_dir: String,
    pub instances_count: usize,
    pub skills_count: usize,
    pub llm: LlmSummary,
}

#[derive(Serialize)]
pub struct LlmSummary {
    pub provider: crate::config::LlmProvider,
    pub setup_required: Option<&'static str>,
    pub model: Option<String>,
    pub configured: bool,
}
