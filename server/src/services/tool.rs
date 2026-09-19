use std::fmt;
use std::future::Future;
use std::pin::Pin;

use serde::{Deserialize, Serialize};

/// A tool definition with name, description, and JSON Schema parameters.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// Errors during tool execution.
#[derive(Debug)]
pub enum ToolError {
    ToolCallError(Box<dyn std::error::Error + Send + Sync>),
    JsonError(serde_json::Error),
}

impl fmt::Display for ToolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ToolError::ToolCallError(e) => {
                let s = e.to_string();
                if s.starts_with("ToolCallError: ") {
                    write!(f, "{s}")
                } else {
                    write!(f, "ToolCallError: {s}")
                }
            }
            ToolError::JsonError(e) => write!(f, "JsonError: {e}"),
        }
    }
}

impl std::error::Error for ToolError {}

impl From<serde_json::Error> for ToolError {
    fn from(e: serde_json::Error) -> Self {
        ToolError::JsonError(e)
    }
}

/// Dynamic-dispatch tool trait (object-safe).
pub trait ToolDyn: Send + Sync {
    fn name(&self) -> String;
    /// Whether this concrete tool is trusted to emit locally-authenticated
    /// resource provenance. External ToolDyn implementations default closed.
    fn trusts_resource_provenance(&self) -> bool {
        false
    }
    fn definition<'a>(
        &'a self,
        prompt: String,
    ) -> Pin<Box<dyn Future<Output = ToolDefinition> + Send + 'a>>;
    fn call<'a>(
        &'a self,
        args: String,
    ) -> Pin<Box<dyn Future<Output = Result<String, ToolError>> + Send + 'a>>;
}

/// Typed tool trait. Implement this for concrete tools.
pub trait Tool: Send + Sync + 'static {
    const NAME: &'static str;
    const TRUSTS_RESOURCE_PROVENANCE: bool = false;
    type Error: std::error::Error + Send + Sync + 'static;
    type Args: for<'de> Deserialize<'de> + Send + Sync;
    type Output: Serialize;

    fn name(&self) -> String {
        Self::NAME.to_string()
    }

    fn definition(&self, prompt: String) -> impl Future<Output = ToolDefinition> + Send;
    fn call(
        &self,
        args: Self::Args,
    ) -> impl Future<Output = Result<Self::Output, Self::Error>> + Send;
}

/// Blanket impl: any Tool is automatically a ToolDyn.
impl<T: Tool> ToolDyn for T {
    fn name(&self) -> String {
        Tool::name(self)
    }

    fn trusts_resource_provenance(&self) -> bool {
        T::TRUSTS_RESOURCE_PROVENANCE
    }

    fn definition<'a>(
        &'a self,
        prompt: String,
    ) -> Pin<Box<dyn Future<Output = ToolDefinition> + Send + 'a>> {
        Box::pin(Tool::definition(self, prompt))
    }

    fn call<'a>(
        &'a self,
        args: String,
    ) -> Pin<Box<dyn Future<Output = Result<String, ToolError>> + Send + 'a>> {
        Box::pin(async move {
            let parsed: T::Args = serde_json::from_str(&args)?;
            Tool::call(self, parsed)
                .await
                .map_err(|e| ToolError::ToolCallError(Box::new(e)))
                .and_then(|output| serde_json::to_string(&output).map_err(ToolError::JsonError))
        })
    }
}
