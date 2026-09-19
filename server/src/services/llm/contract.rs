//! Provider boundary. Conversation storage and the agent loop do not own API payloads.
use futures::future::BoxFuture;
use tokio_util::sync::CancellationToken;

use super::types::{ContentBlock, LlmResponse, Message};
use crate::services::tool::ToolDefinition;

#[derive(Clone, Copy, Debug, serde::Serialize)]
pub struct Capabilities {
    pub vision: bool,
    pub documents: bool,
    pub tools: bool,
    pub streaming: bool,
    pub reasoning_controls: bool,
    pub model_discovery: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Usage {
    /// Total input, including cache reads and writes.
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
}

#[derive(Debug)]
pub enum LlmError {
    UnsupportedCapability(&'static str),
    SetupRequired(String),
    Cancelled,
    Timeout,
    Http { status: u16, message: String },
    Transport(String),
    InvalidResponse(String),
}

impl std::fmt::Display for LlmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedCapability(c) => write!(f, "selected provider does not support {c}"),
            Self::SetupRequired(s) => write!(f, "provider setup required: {s}"),
            Self::Cancelled => write!(f, "LLM request cancelled"),
            Self::Timeout => write!(f, "LLM stream timed out"),
            Self::Http { status, message } => write!(f, "LLM API error {status}: {message}"),
            Self::Transport(s) => write!(f, "LLM transport error: {s}"),
            Self::InvalidResponse(s) => write!(f, "invalid LLM response: {s}"),
        }
    }
}
impl std::error::Error for LlmError {}
impl From<anyhow::Error> for LlmError {
    fn from(error: anyhow::Error) -> Self {
        match error.downcast::<Self>() {
            Ok(error) => error,
            Err(error) => {
                if error.is::<reqwest::Error>() {
                    Self::Transport(error.to_string())
                } else {
                    Self::InvalidResponse(error.to_string())
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopReason {
    Complete,
    ToolCalls,
    OutputLimit,
    Incomplete,
    Continue,
    ContextUpdated,
}
/// Provider-independent stream notifications. UI/MCP routing stays in the caller.
#[derive(Debug)]
pub enum LlmEvent {
    TextDelta(String),
    ToolCallStarted {
        id: String,
        name: String,
    },
    ToolArgumentsDelta {
        id: String,
        name: String,
        delta: String,
    },
    Activity {
        name: String,
        description: String,
    },
    Usage(Usage),
}
pub type EventSink<'a> = dyn Fn(LlmEvent) + Send + Sync + 'a;

pub struct LlmRequest<'a> {
    pub system: &'a [&'a str],
    pub messages: &'a [Message],
    pub tools: &'a [ToolDefinition],
    pub max_tokens: u64,
    pub json_schema: Option<&'a serde_json::Value>,
    /// Reserved explicitly, so unsupported controls cannot be silently ignored.
    pub reasoning: Option<&'a str>,
    pub cancellation: CancellationToken,
}
impl<'a> LlmRequest<'a> {
    pub fn new(
        system: &'a [&'a str],
        messages: &'a [Message],
        tools: &'a [ToolDefinition],
    ) -> Self {
        Self {
            system,
            messages,
            tools,
            max_tokens: 16384,
            json_schema: None,
            reasoning: None,
            cancellation: CancellationToken::new(),
        }
    }

    pub fn validate(&self, capabilities: Capabilities, streaming: bool) -> Result<(), LlmError> {
        if self.cancellation.is_cancelled() {
            return Err(LlmError::Cancelled);
        }
        if streaming && !capabilities.streaming {
            return Err(LlmError::UnsupportedCapability("streaming"));
        }
        if streaming && self.json_schema.is_some() {
            return Err(LlmError::UnsupportedCapability(
                "streaming structured output",
            ));
        }
        if self.reasoning.is_some() && !capabilities.reasoning_controls {
            return Err(LlmError::UnsupportedCapability("reasoning controls"));
        }
        if !self.tools.is_empty() && !capabilities.tools {
            return Err(LlmError::UnsupportedCapability("tools"));
        }
        fn validate_blocks(blocks: &[ContentBlock], c: Capabilities) -> Result<(), LlmError> {
            for block in blocks {
                match block {
                    ContentBlock::Image { .. } if !c.vision => {
                        return Err(LlmError::UnsupportedCapability("vision"));
                    }
                    ContentBlock::Document { .. } if !c.documents => {
                        return Err(LlmError::UnsupportedCapability("documents"));
                    }
                    ContentBlock::ToolCall { .. } | ContentBlock::ToolOutput { .. } if !c.tools => {
                        return Err(LlmError::UnsupportedCapability("tools"));
                    }
                    ContentBlock::ToolOutput {
                        content: super::types::ToolOutputContent::Blocks(blocks),
                        ..
                    } => {
                        validate_blocks(blocks, c)?;
                    }
                    _ => {}
                }
            }
            Ok(())
        }
        for message in self.messages {
            let (Message::User { content } | Message::Assistant { content }) = message;
            validate_blocks(content, capabilities)?;
        }
        Ok(())
    }
}

/// Both adapters implement this same object-safe contract. Dropping a future or
/// cancelling its token drops the HTTP request/stream; no background task survives.
pub trait ProviderAdapter: Send + Sync {
    fn capabilities(&self) -> Capabilities;
    fn complete<'a>(
        &'a self,
        request: LlmRequest<'a>,
    ) -> BoxFuture<'a, Result<LlmResponse, LlmError>>;
    fn stream<'a>(
        &'a self,
        request: LlmRequest<'a>,
        events: &'a EventSink<'a>,
    ) -> BoxFuture<'a, Result<LlmResponse, LlmError>>;
    // Reserved extension point; current adapters advertise discovery as unsupported.
    #[allow(dead_code)]
    fn discover_models(&self) -> BoxFuture<'_, Result<Vec<String>, LlmError>> {
        Box::pin(async { Err(LlmError::UnsupportedCapability("model discovery")) })
    }
}

#[cfg(test)]
mod tests {
    use super::super::{
        anthropic::messages_to_anthropic,
        openai::messages_to_openai,
        types::{DocumentSource, HistoryEntry, ImageSource, LlmBackend},
    };
    use super::*;
    use crate::config::{Config, LlmProvider};
    use serde_json::{Value, json};
    use std::sync::{Arc, Mutex};

    fn backend(provider: LlmProvider, url: &str) -> LlmBackend {
        let mut config = Config::default();
        config.llm.provider = provider;
        config.llm.tokens.anthropic = "test".into();
        config.llm.tokens.open_ai = "test".into();
        let mut backend = LlmBackend::from_config(&config).unwrap();
        backend.base_url = url.into();
        backend
    }

    #[test]
    fn stored_history_converts_to_both_providers_without_mutation() {
        // Existing flattened history with tool calls, results, summary and metadata.
        let raw = json!([
            {"role":"user", "content":[{"type":"text","text":"hello"}],"ts":"1","id":"u"},
            {"role":"assistant", "content":[{"type":"tool_use","id":"call1","name":"search","input":{"q":"rust"}}],"model":"old-model"},
            {"role":"user", "content":[{"type":"tool_result","tool_use_id":"call1","content":"found"}]},
            {"role":"assistant", "content":[{"type":"compaction","content":"summary"}]}
        ]);
        let entries: Vec<HistoryEntry> = serde_json::from_value(raw.clone()).unwrap();
        let messages = HistoryEntry::to_messages(&entries);
        let anthropic = messages_to_anthropic(&messages);
        let (_, openai) = messages_to_openai(&[], &messages);
        assert_eq!(anthropic[1]["content"][0]["id"], "call1");
        assert_eq!(anthropic[2]["content"][0]["tool_use_id"], "call1");
        assert_eq!(openai[1]["call_id"], "call1");
        assert_eq!(openai[2]["call_id"], "call1");
        assert_eq!(openai[2]["output"], "found");
        assert!(openai[3]["content"].as_str().unwrap().contains("summary"));
        assert_eq!(serde_json::to_value(entries).unwrap(), raw);
    }

    #[test]
    fn opaque_metadata_only_returns_to_originating_provider() {
        let messages = [Message::Assistant {
            content: vec![
                ContentBlock::text("answer"),
                ContentBlock::ProviderData {
                    provider: LlmProvider::Anthropic,
                    data: json!({"type":"server_tool_use","id":"srv","name":"web_search"}),
                },
            ],
        }];
        let saved = serde_json::to_value(&messages).unwrap();
        let restored: Vec<Message> = serde_json::from_value(saved).unwrap();
        assert_eq!(
            messages_to_anthropic(&restored)[0]["content"][1]["id"],
            "srv"
        );
        let (_, openai) = messages_to_openai(&[], &restored);
        assert_eq!(openai.len(), 1);
        assert_eq!(openai[0]["content"], "answer");
    }

    #[test]
    fn images_and_multimodal_tool_results_convert_without_losing_images() {
        let image = ContentBlock::Image {
            source: ImageSource::Base64 {
                media_type: "image/png".into(),
                data: "abc".into(),
            },
            resource_provenance: None,
        };
        let messages = [Message::User {
            content: vec![
                image.clone(),
                ContentBlock::ToolOutput {
                    call_id: "call1".into(),
                    content: super::super::types::ToolOutputContent::Blocks(vec![
                        ContentBlock::text("screenshot"),
                        image,
                    ]),
                },
            ],
        }];
        let (_, input) = messages_to_openai(&[], &messages);
        assert_eq!(input[0]["content"][0]["type"], "input_image");
        assert_eq!(
            input[0]["content"][0]["image_url"],
            "data:image/png;base64,abc"
        );
        assert_eq!(input[1]["output"], "screenshot");
        assert_eq!(input[2]["content"][0]["type"], "input_image");
        assert_eq!(
            messages_to_anthropic(&messages)[0]["content"][1]["content"][1]["type"],
            "image"
        );
    }

    #[tokio::test]
    async fn capabilities_and_cancellation_reject_before_network() {
        let adapter = backend(LlmProvider::Openai, "http://127.0.0.1:1")
            .adapter()
            .unwrap();
        let messages = [Message::User {
            content: vec![ContentBlock::Document {
                source: DocumentSource::Url {
                    url: "https://example.test/doc.pdf".into(),
                },
                resource_provenance: None,
            }],
        }];
        assert!(matches!(
            adapter.complete(LlmRequest::new(&[], &messages, &[])).await,
            Err(LlmError::UnsupportedCapability("documents"))
        ));
        let mut request = LlmRequest::new(&[], &[], &[]);
        request.reasoning = Some("high");
        assert!(matches!(
            adapter.complete(request).await,
            Err(LlmError::UnsupportedCapability("reasoning controls"))
        ));
        assert!(matches!(
            adapter.discover_models().await,
            Err(LlmError::UnsupportedCapability("model discovery"))
        ));
        for provider in [LlmProvider::Anthropic, LlmProvider::Openai] {
            let adapter = backend(provider, "http://127.0.0.1:1").adapter().unwrap();
            let request = LlmRequest::new(&[], &[], &[]);
            request.cancellation.cancel();
            assert!(matches!(
                adapter.stream(request, &|_| {}).await,
                Err(LlmError::Cancelled)
            ));
        }
    }

    #[test]
    fn capability_validation_includes_nested_tool_results() {
        let capabilities = Capabilities {
            vision: false,
            documents: false,
            tools: true,
            streaming: false,
            reasoning_controls: false,
            model_discovery: false,
        };
        let messages = [Message::User {
            content: vec![ContentBlock::ToolOutput {
                call_id: "id".into(),
                content: super::super::types::ToolOutputContent::Blocks(vec![
                    ContentBlock::Image {
                        source: ImageSource::Url {
                            url: "https://example.test/image".into(),
                        },
                        resource_provenance: None,
                    },
                ]),
            }],
        }];
        assert!(matches!(
            LlmRequest::new(&[], &messages, &[]).validate(capabilities, false),
            Err(LlmError::UnsupportedCapability("vision"))
        ));
        assert!(matches!(
            LlmRequest::new(&[], &[], &[]).validate(capabilities, true),
            Err(LlmError::UnsupportedCapability("streaming"))
        ));
        let capabilities = Capabilities {
            tools: false,
            ..capabilities
        };
        assert!(matches!(
            LlmRequest::new(&[], &messages, &[]).validate(capabilities, false),
            Err(LlmError::UnsupportedCapability("tools"))
        ));
    }

    async fn mock_server(
        status: u16,
        body: String,
    ) -> (String, Arc<Mutex<Vec<Value>>>, tokio::task::JoinHandle<()>) {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = requests.clone();
        let app = axum::Router::new().fallback(axum::routing::post(
            move |axum::Json(request): axum::Json<Value>| {
                let captured = captured.clone();
                let body = body.clone();
                async move {
                    captured.lock().unwrap().push(request);
                    (axum::http::StatusCode::from_u16(status).unwrap(), body)
                }
            },
        ));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (url, requests, task)
    }

    #[tokio::test]
    async fn both_adapters_complete_with_tools_and_usage_through_same_contract() {
        for (provider, body) in [
            (
                LlmProvider::Anthropic,
                json!({"content":[{"type":"text","text":"hello"},{"type":"tool_use","id":"call1","name":"search","input":{"q":"rust"}}],"stop_reason":"tool_use","usage":{"input_tokens":10,"output_tokens":4,"cache_read_input_tokens":2,"cache_creation_input_tokens":3}}),
            ),
            (
                LlmProvider::Openai,
                json!({"status":"completed","output":[{"type":"message","content":[{"type":"output_text","text":"hello"}]},{"type":"function_call","call_id":"call1","name":"search","arguments":"{\"q\":\"rust\"}"}],"usage":{"input_tokens":15,"output_tokens":4,"input_tokens_details":{"cached_tokens":2}}}),
            ),
        ] {
            let (url, requests, task) = mock_server(200, body.to_string()).await;
            let adapter = backend(provider, &url).adapter().unwrap();
            let messages = [Message::user("hello")];
            let response = adapter
                .complete(LlmRequest::new(&["system"], &messages, &[]))
                .await
                .unwrap();
            assert_eq!(response.text, "hello");
            assert_eq!(response.stop_reason, StopReason::ToolCalls);
            assert_eq!(response.tool_calls[0].id, "call1");
            assert_eq!(response.tool_calls[0].arguments["q"], "rust");
            assert_eq!(response.usage.input_tokens, 15);
            assert_eq!(response.usage.output_tokens, 4);
            assert_eq!(response.usage.cache_read_tokens, 2);
            assert_eq!(
                response.usage.cache_write_tokens,
                if provider == LlmProvider::Anthropic {
                    3
                } else {
                    0
                }
            );
            assert_eq!(requests.lock().unwrap().len(), 1);
            task.abort();
        }
    }

    #[tokio::test]
    async fn structured_output_stays_in_adapters() {
        for (provider, body) in [
            (
                LlmProvider::Anthropic,
                json!({"content":[{"type":"text","text":"{}"}],"stop_reason":"end_turn","usage":{"input_tokens":10,"output_tokens":4}}),
            ),
            (
                LlmProvider::Openai,
                json!({"status":"completed","output":[{"type":"message","content":[{"type":"output_text","text":"{}"}]}],"usage":{"input_tokens":10,"output_tokens":4}}),
            ),
        ] {
            let (url, requests, task) = mock_server(200, body.to_string()).await;
            let backend = backend(provider, &url);
            let schema = json!({"type":"object","properties":{}});
            let (text, tokens) = backend
                .chat_json("system", "prompt", schema.clone())
                .await
                .unwrap();
            assert_eq!(text, "{}");
            assert_eq!(tokens, 14);
            let requests = requests.lock().unwrap();
            if provider == LlmProvider::Anthropic {
                assert_eq!(requests[0]["output_config"]["format"]["schema"], schema);
            } else {
                assert_eq!(requests[0]["text"]["format"]["type"], "json_object");
                assert_eq!(requests[0]["store"], false);
            }
            task.abort();
        }
    }

    #[tokio::test]
    async fn both_adapters_stream_canonical_events_and_tool_calls() {
        let anthropic = concat!(
            "event: message_start\ndata: {\"message\":{\"usage\":{\"input_tokens\":5}}}\n\n",
            "event: content_block_start\ndata: {\"content_block\":{\"type\":\"text\"}}\n\n",
            "event: content_block_delta\ndata: {\"delta\":{\"type\":\"text_delta\",\"text\":\"hello\"}}\n\n",
            "event: content_block_start\ndata: {\"content_block\":{\"type\":\"tool_use\",\"id\":\"call1\",\"name\":\"search\"}}\n\n",
            "event: content_block_delta\ndata: {\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{}\"}}\n\n",
            "event: content_block_stop\ndata: {}\n\n",
            "event: message_delta\ndata: {\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"output_tokens\":3}}\n\n",
            "event: message_stop\ndata: {}\n\n"
        );
        let openai = concat!(
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"hello\"}\n\n",
            "data: {\"type\":\"response.output_item.added\",\"output_index\":1,\"item\":{\"type\":\"function_call\",\"call_id\":\"call1\",\"name\":\"search\"}}\n\n",
            "data: {\"type\":\"response.function_call_arguments.delta\",\"output_index\":1,\"delta\":\"{}\"}\n\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"output\":[{\"type\":\"function_call\",\"call_id\":\"call1\",\"name\":\"search\",\"arguments\":\"{}\"}],\"usage\":{\"input_tokens\":5,\"output_tokens\":3}}}\n\n"
        );
        for (provider, body) in [
            (LlmProvider::Anthropic, anthropic),
            (LlmProvider::Openai, openai),
        ] {
            let (url, _, task) = mock_server(200, body.into()).await;
            let events = Mutex::new(Vec::new());
            let sink = |event| events.lock().unwrap().push(event);
            let adapter = backend(provider, &url).adapter().unwrap();
            let response = adapter
                .stream(LlmRequest::new(&[], &[], &[]), &sink)
                .await
                .unwrap();
            assert_eq!(response.text, "hello");
            assert_eq!(response.stop_reason, StopReason::ToolCalls);
            assert_eq!(response.tool_calls[0].id, "call1");
            assert_eq!(response.usage.output_tokens, 3);
            let events = events.lock().unwrap();
            assert!(
                !events.iter().any(
                    |event| matches!(event, LlmEvent::Activity { name, .. } if name == "search")
                ),
                "local tool activity is emitted by the agent loop only"
            );
            assert!(
                events
                    .iter()
                    .any(|e| matches!(e, LlmEvent::TextDelta(s) if s == "hello"))
            );
            assert!(events.iter().any(|e| matches!(e, LlmEvent::ToolCallStarted { id, name } if id == "call1" && name == "search")));
            assert!(events.iter().any(|e| matches!(e, LlmEvent::ToolArgumentsDelta { id, delta, .. } if id == "call1" && delta == "{}")));
            assert!(
                events
                    .iter()
                    .any(|e| matches!(e, LlmEvent::Usage(u) if u.input_tokens == 5))
            );
            task.abort();
        }
    }

    #[tokio::test]
    async fn adapters_return_typed_http_and_truncated_stream_errors() {
        for provider in [LlmProvider::Anthropic, LlmProvider::Openai] {
            let (url, _, task) = mock_server(429, "rate limited".into()).await;
            let adapter = backend(provider, &url).adapter().unwrap();
            assert!(matches!(
                adapter.complete(LlmRequest::new(&[], &[], &[])).await,
                Err(LlmError::Http { status: 429, .. })
            ));
            assert!(matches!(
                adapter
                    .stream(LlmRequest::new(&[], &[], &[]), &|_| {})
                    .await,
                Err(LlmError::Http { status: 429, .. })
            ));
            task.abort();
            let (url, _, task) = mock_server(200, "data: {}\n\n".into()).await;
            let adapter = backend(provider, &url).adapter().unwrap();
            assert!(matches!(
                adapter
                    .stream(LlmRequest::new(&[], &[], &[]), &|_| {})
                    .await,
                Err(LlmError::InvalidResponse(_))
            ));
            task.abort();
        }
    }

    #[tokio::test]
    async fn cancellation_interrupts_in_flight_http_request() {
        for provider in [LlmProvider::Anthropic, LlmProvider::Openai] {
            let entered = Arc::new(tokio::sync::Notify::new());
            let notify = entered.clone();
            let app = axum::Router::new().fallback(axum::routing::post(move || {
                let notify = notify.clone();
                async move {
                    notify.notify_one();
                    std::future::pending::<String>().await
                }
            }));
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let adapter = backend(
                provider,
                &format!("http://{}", listener.local_addr().unwrap()),
            )
            .adapter()
            .unwrap();
            let server = tokio::spawn(async move {
                axum::serve(listener, app).await.unwrap();
            });
            let request = LlmRequest::new(&[], &[], &[]);
            let token = request.cancellation.clone();
            let cancel = async {
                entered.notified().await;
                token.cancel();
            };
            let (result, ()) = tokio::time::timeout(std::time::Duration::from_secs(5), async {
                tokio::join!(adapter.stream(request, &|_| {}), cancel)
            })
            .await
            .unwrap();
            assert!(matches!(result, Err(LlmError::Cancelled)));
            server.abort();
        }
    }

    struct MustNotExecute;
    impl crate::services::tool::ToolDyn for MustNotExecute {
        fn name(&self) -> String {
            "search".into()
        }
        fn definition<'a>(&'a self, _: String) -> BoxFuture<'a, ToolDefinition> {
            Box::pin(async {
                ToolDefinition {
                    name: "search".into(),
                    description: "test".into(),
                    parameters: json!({"type":"object"}),
                }
            })
        }
        fn call<'a>(
            &'a self,
            _: String,
        ) -> BoxFuture<'a, Result<String, crate::services::tool::ToolError>> {
            Box::pin(async { panic!("invalid provider response reached tool execution") })
        }
    }

    #[tokio::test]
    async fn invalid_tool_calls_are_rejected_in_both_modes() {
        for provider in [LlmProvider::Anthropic, LlmProvider::Openai] {
            let id_key = if provider == LlmProvider::Anthropic {
                "id"
            } else {
                "call_id"
            };
            let args_key = if provider == LlmProvider::Anthropic {
                "input"
            } else {
                "arguments"
            };
            for field in [id_key, "name", args_key] {
                for bad in [
                    None,
                    Some(json!("")),
                    Some(json!(null)),
                    Some(json!(7)),
                    Some(json!("broken{")),
                    Some(json!([])),
                ] {
                    let mut call = if provider == LlmProvider::Anthropic {
                        json!({"type":"tool_use","id":"id","name":"search","input":{}})
                    } else {
                        json!({"type":"function_call","call_id":"id","name":"search","arguments":"{}"})
                    };
                    // Nonempty strings are valid IDs/names.
                    if field != args_key && bad == Some(json!("broken{")) {
                        continue;
                    }
                    match &bad {
                        Some(value) => {
                            call[field] = value.clone();
                        }
                        None => {
                            call.as_object_mut().unwrap().remove(field);
                        }
                    }
                    let complete = if provider == LlmProvider::Anthropic {
                        json!({"content":[call.clone()],"stop_reason":"tool_use"})
                    } else {
                        json!({"status":"completed","output":[call.clone()]})
                    };
                    let stream = if provider == LlmProvider::Anthropic {
                        let arguments = if field == args_key {
                            bad.as_ref()
                                .map(|v| {
                                    v.as_str()
                                        .map(str::to_owned)
                                        .unwrap_or_else(|| v.to_string())
                                })
                                .unwrap_or_default()
                        } else {
                            "{}".into()
                        };
                        format!(
                            "event: content_block_start\ndata: {}\n\nevent: content_block_delta\ndata: {}\n\nevent: content_block_stop\ndata: {{}}\n\nevent: message_delta\ndata: {{\"delta\":{{\"stop_reason\":\"tool_use\"}}}}\n\nevent: message_stop\ndata: {{}}\n\n",
                            json!({"content_block":call}),
                            json!({"delta":{"type":"input_json_delta","partial_json":arguments}})
                        )
                    } else {
                        let args = call.get("arguments").and_then(Value::as_str).unwrap_or("");
                        format!(
                            "data: {}\n\ndata: {}\n\ndata: {}\n\n",
                            json!({"type":"response.output_item.added","output_index":0,"item":call}),
                            json!({"type":"response.function_call_arguments.delta","output_index":0,"delta":args}),
                            json!({"type":"response.completed","response":complete})
                        )
                    };
                    for (streaming, body) in [(false, complete.to_string()), (true, stream)] {
                        let (url, _, task) = mock_server(200, body).await;
                        let adapter = backend(provider, &url).adapter().unwrap();
                        let request = LlmRequest::new(&[], &[], &[]);
                        let result = if streaming {
                            adapter.stream(request, &|_| {}).await
                        } else {
                            adapter.complete(request).await
                        };
                        assert!(
                            matches!(result, Err(LlmError::InvalidResponse(_))),
                            "{provider:?} streaming={streaming} field={field} bad={bad:?}: {result:?}"
                        );
                        // Exercise the real agent boundary as well: no tool may run.
                        let backend = backend(provider, &url);
                        let tools: Vec<Box<dyn crate::services::tool::ToolDyn>> =
                            vec![Box::new(MustNotExecute)];
                        let result = if streaming {
                            backend
                                .chat_with_tools_streaming(
                                    &[],
                                    Message::user("test"),
                                    vec![],
                                    tools,
                                    tokio::sync::broadcast::channel(32).0,
                                    "test",
                                    "test",
                                    &std::env::temp_dir(),
                                    None,
                                    Default::default(),
                                )
                                .await
                                .map(|_| ())
                        } else {
                            backend
                                .chat_with_tools_traced("", "test", vec![], tools)
                                .await
                                .map(|_| ())
                        };
                        let error = result.unwrap_err();
                        assert!(matches!(
                            error.downcast_ref::<LlmError>(),
                            Some(LlmError::InvalidResponse(_))
                        ));
                        task.abort();
                    }
                }
            }
        }
    }

    #[tokio::test]
    async fn codex_runtime_returns_typed_setup_required() {
        let mut config = Config::default();
        config.llm.provider = LlmProvider::Codex;
        let backend = LlmBackend::from_config(&config).unwrap();
        let error = backend.chat("system", "hello", vec![]).await.unwrap_err();
        assert!(matches!(
            error.downcast_ref::<LlmError>(),
            Some(LlmError::SetupRequired(_))
        ));
        let error = backend
            .chat_with_tools_traced("system", "hello", vec![], vec![])
            .await
            .unwrap_err();
        assert!(matches!(
            error.downcast_ref::<LlmError>(),
            Some(LlmError::SetupRequired(_))
        ));
    }

    #[test]
    fn legacy_history_fixtures_roundtrip_exactly() {
        for fixture in [
            include_str!("fixtures/history-text.json"),
            include_str!("fixtures/history-multimodal.json"),
        ] {
            let original: Value = serde_json::from_str(fixture).unwrap();
            let entries: Vec<HistoryEntry> = serde_json::from_str(fixture).unwrap();
            assert_eq!(serde_json::to_value(&entries).unwrap(), original);
            let messages = HistoryEntry::to_messages(&entries);
            let _ = messages_to_anthropic(&messages);
            let _ = messages_to_openai(&[], &messages);
            assert_eq!(serde_json::to_value(&entries).unwrap(), original);
        }
    }
}
