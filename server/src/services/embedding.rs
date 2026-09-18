//! Provider-neutral text embeddings with an independent OpenAI-compatible HTTP adapter.

use crate::config::{Config, EmbeddingConfig};
use futures::StreamExt;
use futures::future::BoxFuture;

const MAX_INPUT_COUNT: usize = 64;
const MAX_INPUT_BYTES: usize = 8 * 1024;
const MAX_TOTAL_INPUT_BYTES: usize = 64 * 1024;
const MAX_RESPONSE_BYTES: usize = 40 * 1024 * 1024;

/// Text-only contract. Chat provider selection and raw media are deliberately absent.
pub trait EmbeddingProvider: Send + Sync {
    fn documents<'a>(
        &'a self,
        inputs: &'a [String],
    ) -> BoxFuture<'a, Result<Vec<Vec<f32>>, String>>;
    fn query<'a>(&'a self, input: &'a str) -> BoxFuture<'a, Result<Vec<f32>, String>>;
}

struct OpenAiEmbeddingProvider {
    config: EmbeddingConfig,
    key: Option<String>,
    http: reqwest::Client,
}

impl OpenAiEmbeddingProvider {
    fn from_config(config: &EmbeddingConfig, key: &str) -> Result<Self, String> {
        config
            .unavailable_reason(key)
            .map_or(Ok(()), |reason| Err(reason.to_owned()))?;
        Ok(Self {
            config: config.clone(),
            key: (config.provider == "openai").then(|| key.to_owned()),
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .expect("embedding HTTP client"),
        })
    }

    fn build_request(&self, inputs: &[String]) -> Result<reqwest::RequestBuilder, String> {
        if inputs.len() > MAX_INPUT_COUNT {
            return Err("embedding input count exceeds limit".into());
        }
        if inputs.iter().any(|input| input.len() > MAX_INPUT_BYTES) {
            return Err("embedding input exceeds byte limit".into());
        }
        let total = inputs.iter().try_fold(0usize, |total, input| {
            total.checked_add(input.len()).ok_or(())
        });
        if total.is_err() || total.unwrap() > MAX_TOTAL_INPUT_BYTES {
            return Err("embedding total input bytes exceed limit".into());
        }
        let request = self
            .http
            .post(format!(
                "{}/embeddings",
                self.config.base_url.trim_end_matches('/')
            ))
            .json(&serde_json::json!({"model":self.config.model, "input":inputs, "dimensions":self.config.dimensions}));
        Ok(match &self.key {
            Some(key) => request.bearer_auth(key),
            None => request,
        })
    }

    async fn request(&self, inputs: &[String]) -> Result<Vec<Vec<f32>>, String> {
        if inputs.is_empty() {
            return Ok(vec![]);
        }
        let response = self
            .build_request(inputs)?
            .send()
            .await
            .map_err(|_| "embedding provider is unreachable".to_string())?;
        if !response.status().is_success() {
            // Never include the provider body, request URL, or credentials in diagnostics.
            return Err(format!(
                "embedding provider HTTP {}",
                response.status().as_u16()
            ));
        }
        let response_limit = 4096usize
            .saturating_add(
                inputs.len().saturating_mul(
                    (self.config.dimensions as usize)
                        .saturating_mul(32)
                        .saturating_add(1024),
                ),
            )
            .min(MAX_RESPONSE_BYTES);
        if response
            .content_length()
            .is_some_and(|length| length > response_limit as u64)
        {
            return Err("embedding response is too large".into());
        }
        let mut body = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| "invalid embedding response".to_string())?;
            if body.len().saturating_add(chunk.len()) > response_limit {
                return Err("embedding response is too large".into());
            }
            body.extend_from_slice(&chunk);
        }
        #[derive(serde::Deserialize)]
        struct Item {
            index: usize,
            embedding: Vec<f32>,
        }
        #[derive(serde::Deserialize)]
        struct Response {
            #[serde(default)]
            data: Vec<Item>,
            error: Option<serde_json::Value>,
        }
        let parsed: Response =
            serde_json::from_slice(&body).map_err(|_| "invalid embedding response".to_string())?;
        if parsed.error.is_some() {
            return Err("embedding provider returned an error".into());
        }
        if parsed.data.len() != inputs.len() {
            return Err("embedding response count mismatch".into());
        }
        let mut vectors = vec![None; inputs.len()];
        for item in parsed.data {
            if item.index >= vectors.len() || vectors[item.index].is_some() {
                return Err("invalid embedding response indexes".into());
            }
            if item.embedding.len() != self.config.dimensions as usize
                || item.embedding.iter().any(|v| !v.is_finite())
                || item.embedding.iter().all(|v| *v == 0.)
            {
                return Err("invalid embedding dimensions or values".into());
            }
            vectors[item.index] = Some(item.embedding);
        }
        vectors
            .into_iter()
            .map(|v| v.ok_or_else(|| "missing embedding response index".into()))
            .collect()
    }
}

impl EmbeddingProvider for OpenAiEmbeddingProvider {
    fn documents<'a>(
        &'a self,
        inputs: &'a [String],
    ) -> BoxFuture<'a, Result<Vec<Vec<f32>>, String>> {
        Box::pin(self.request(inputs))
    }
    fn query<'a>(&'a self, input: &'a str) -> BoxFuture<'a, Result<Vec<f32>, String>> {
        Box::pin(async move {
            if input.len() > MAX_INPUT_BYTES {
                return Err("embedding input exceeds byte limit".into());
            }
            Ok(self.request(&[input.to_owned()]).await?.remove(0))
        })
    }
}

/// One immutable configuration for one index lifetime. Failures remain retryable.
pub struct EmbeddingService {
    config: EmbeddingConfig,
    key: String,
    provider: Option<Box<dyn EmbeddingProvider>>,
    health: std::sync::Mutex<Option<Result<(), String>>>,
}

impl EmbeddingService {
    pub fn from_config(config: &Config) -> Self {
        let settings = config.embedding.clone();
        let key = config.llm.tokens.open_ai.clone();
        let provider = OpenAiEmbeddingProvider::from_config(&settings, &key)
            .ok()
            .map(|provider| Box::new(provider) as Box<dyn EmbeddingProvider>);
        Self {
            config: settings,
            key,
            provider,
            health: std::sync::Mutex::new(None),
        }
    }

    pub fn needs_restart(&self, config: &Config) -> bool {
        self.config != config.embedding || self.key != config.llm.tokens.open_ai
    }

    pub fn status(&self) -> serde_json::Value {
        let mut status = self.config.safe_status(&self.key);
        if let Some(health) = &*self.health.lock().unwrap_or_else(|e| e.into_inner()) {
            status["status"] = serde_json::json!(if health.is_ok() {
                "available"
            } else {
                "unavailable"
            });
            status["reason"] = serde_json::json!(health.as_ref().err());
        }
        status
    }

    pub fn ensure_configured(&self) -> Result<(), String> {
        self.config
            .unavailable_reason(&self.key)
            .map_or(Ok(()), |reason| Err(reason.into()))
    }

    async fn embed(&self, input: &str, query: bool) -> Result<Vec<f32>, String> {
        self.ensure_configured()?;
        let provider = self
            .provider
            .as_ref()
            .expect("validated embedding provider");
        let result = if query {
            provider.query(input).await
        } else {
            provider
                .documents(&[input.to_owned()])
                .await
                .map(|mut vectors| vectors.remove(0))
        };
        *self.health.lock().unwrap_or_else(|e| e.into_inner()) =
            Some(result.as_ref().map(|_| ()).map_err(Clone::clone));
        result
    }

    pub async fn document(&self, input: &str) -> Result<Vec<f32>, String> {
        self.embed(input, false).await
    }
    pub async fn query(&self, input: &str) -> Result<Vec<f32>, String> {
        self.embed(input, true).await
    }
}

#[cfg(test)]
#[path = "embedding_tests.rs"]
pub(crate) mod tests;
