pub mod agents;
pub mod chat;
pub mod companion;
pub mod config;
pub mod drops;

pub mod health;
pub mod heartbeat;
pub mod instances;
pub mod machine_agents;
pub mod memory_import;
pub mod meta;
pub mod session;
pub mod skills;
pub mod soul;
pub mod thoughts;
pub mod tts;
pub mod update;
pub mod uploads;
pub mod ws;

/// Preserve a machine-readable setup error at request admission.
pub enum ProviderRequestError {
    Setup(crate::services::llm::contract::LlmError),
    Other(axum::http::StatusCode, String),
}
impl From<(axum::http::StatusCode, String)> for ProviderRequestError {
    fn from((status, message): (axum::http::StatusCode, String)) -> Self {
        Self::Other(status, message)
    }
}
impl axum::response::IntoResponse for ProviderRequestError {
    fn into_response(self) -> axum::response::Response {
        use axum::{Json, http::StatusCode};
        match self {
            Self::Setup(error) => (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({
                    "error": "setup_required", "message": error.to_string()
                })),
            )
                .into_response(),
            Self::Other(status, message) => (status, message).into_response(),
        }
    }
}
pub async fn require_provider(
    state: &crate::app::state::AppState,
) -> Result<(), ProviderRequestError> {
    if let Some(reason) = state.config.read().await.llm.setup_required() {
        return Err(ProviderRequestError::Setup(
            crate::services::llm::contract::LlmError::SetupRequired(reason.into()),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod provider_error_tests {
    use super::*;
    use axum::response::IntoResponse;

    #[tokio::test]
    async fn setup_error_has_typed_http_body() {
        let error = ProviderRequestError::Setup(
            crate::services::llm::contract::LlmError::SetupRequired("Codex is unsupported".into()),
        );
        let response = error.into_response();
        assert_eq!(
            response.status(),
            axum::http::StatusCode::SERVICE_UNAVAILABLE
        );
        let bytes = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["error"], "setup_required");
        assert!(body["message"].as_str().unwrap().contains("Codex"));
    }
}

pub(crate) mod resources;
