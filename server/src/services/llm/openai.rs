use futures::StreamExt;

use crate::services::tool::ToolDefinition;

use super::contract::{
    Capabilities, EventSink, LlmError, LlmEvent, LlmRequest, ProviderAdapter, StopReason, Usage,
};
use super::types::LlmBackend;
use super::types::{ContentBlock, ImageSource, LlmResponse, Message, ToolCall};

// ═══════════════════════════════════════════════════════════════════════════
// OpenAI Responses API
// ═══════════════════════════════════════════════════════════════════════════

/// Convert a canonical image source to an OpenAI `image_url` content part.
fn image_source_to_openai(source: &ImageSource) -> serde_json::Value {
    let url = match source {
        ImageSource::Base64 { media_type, data } => {
            format!("data:{media_type};base64,{data}")
        }
        ImageSource::Url { url } => url.clone(),
    };
    serde_json::json!({"type": "input_image", "image_url": url})
}

fn tool_output_to_string(content: &super::types::ToolOutputContent) -> String {
    use super::types::ToolOutputContent;
    match content {
        ToolOutputContent::Text(s) => s.clone(),
        ToolOutputContent::Blocks(blocks) => blocks
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
        ToolOutputContent::Legacy(value) => value.to_string(),
    }
}

fn stop_reason(response: &serde_json::Value) -> Result<StopReason, LlmError> {
    match response["status"].as_str() {
        Some("completed") => Ok(StopReason::Complete),
        Some("incomplete") => Ok(
            if response["incomplete_details"]["reason"] == "max_output_tokens" {
                StopReason::OutputLimit
            } else {
                StopReason::Incomplete
            },
        ),
        _ => Err(LlmError::InvalidResponse(
            "missing or unsuccessful response status".into(),
        )),
    }
}

/// Convert our internal Message format to OpenAI Responses API input items.
pub(crate) fn messages_to_openai(
    system: &[&str],
    messages: &[Message],
) -> (String, Vec<serde_json::Value>) {
    let instructions = system.join("\n\n");
    let mut input = Vec::new();

    for msg in messages {
        match msg {
            Message::User { content } => {
                for block in content {
                    match block {
                        ContentBlock::ToolOutput {
                            call_id, content, ..
                        } => {
                            // Function call output item
                            input.push(serde_json::json!({
                                "type": "function_call_output",
                                "call_id": call_id,
                                "output": tool_output_to_string(content),
                            }));
                            if let super::types::ToolOutputContent::Blocks(blocks) = content {
                                for block in blocks {
                                    if let ContentBlock::Image { source, .. } = block {
                                        input.push(serde_json::json!({"type": "message", "role": "user", "content": [image_source_to_openai(&source)]}));
                                    }
                                }
                            }
                        }
                        ContentBlock::Text { text } => {
                            input.push(serde_json::json!({
                                "type": "message",
                                "role": "user",
                                "content": text,
                            }));
                        }
                        ContentBlock::Image { source, .. } => {
                            input.push(serde_json::json!({
                                "type": "message",
                                "role": "user",
                                "content": [image_source_to_openai(source)],
                            }));
                        }
                        ContentBlock::ContextSummary { content }
                        | ContentBlock::LegacyContextSummary {
                            summary: content, ..
                        } => {
                            input.push(serde_json::json!({"type": "message", "role": "user", "content": format!("Conversation summary:\n{content}")}));
                        }
                        _ => {}
                    }
                }
            }
            Message::Assistant { content } => {
                // Collect text into a message item
                let mut text = String::new();
                for block in content {
                    match block {
                        ContentBlock::Text { text: t } => text.push_str(t),
                        ContentBlock::ToolCall {
                            id,
                            name,
                            arguments: args,
                        } => {
                            // Flush any text before tool calls
                            if !text.is_empty() {
                                input.push(serde_json::json!({
                                    "type": "message",
                                    "role": "assistant",
                                    "content": text,
                                }));
                                text.clear();
                            }
                            // Function call item
                            input.push(serde_json::json!({
                                "type": "function_call",
                                "call_id": id,
                                "name": name,
                                "arguments": serde_json::to_string(args).unwrap_or_default(),
                            }));
                        }
                        ContentBlock::ContextSummary { content }
                        | ContentBlock::LegacyContextSummary {
                            summary: content, ..
                        } => {
                            text.push_str(&format!("\nConversation summary:\n{content}"));
                        }
                        _ => {}
                    }
                }
                if !text.is_empty() {
                    input.push(serde_json::json!({
                        "type": "message",
                        "role": "assistant",
                        "content": text,
                    }));
                }
            }
        }
    }

    (
        crate::services::tools::redact_secrets(&instructions),
        input
            .into_iter()
            .map(crate::services::tools::redact_value)
            .collect(),
    )
}

/// Convert tool definitions to OpenAI Responses API format.
pub(crate) fn tools_to_openai(
    tool_defs: &[ToolDefinition],
    stream: bool,
) -> Vec<serde_json::Value> {
    let mut tools: Vec<serde_json::Value> = tool_defs
        .iter()
        .map(|t| {
            serde_json::json!({
                "type": "function",
                "name": t.name,
                "description": t.description,
                "parameters": t.parameters,
                "strict": false,
            })
        })
        .collect();

    // Add web search for streaming (interactive) requests
    if stream {
        tools.push(serde_json::json!({"type": "web_search"}));
    }

    tools
}

/// Non-streaming OpenAI Responses API call.
pub(crate) async fn openai_complete(
    http: &reqwest::Client,
    api_key: &str,
    model: &str,
    system: &[&str],
    tool_defs: &[ToolDefinition],
    messages: &[Message],
    max_tokens: u64,
    base_url: &str,
    json_schema: Option<&serde_json::Value>,
) -> anyhow::Result<LlmResponse> {
    let (instructions, input) = messages_to_openai(system, messages);
    let tools = tools_to_openai(tool_defs, false);

    let mut body = serde_json::json!({
        "model": model,
        "max_output_tokens": max_tokens,
        "input": input,
        "store": false,
        "stream": false,
    });
    if !instructions.is_empty() {
        body["instructions"] = serde_json::Value::String(instructions);
    }
    if !tools.is_empty() {
        body["tools"] = serde_json::Value::Array(tools);
    }

    if let Some(schema) = json_schema {
        body["instructions"] = serde_json::json!(format!(
            "{}\n\nRespond with ONLY valid JSON matching this schema:\n{}",
            system.join("\n\n"),
            schema
        ));
        body["text"] = serde_json::json!({"format": {"type": "json_object"}});
    }
    let resp = http
        .post(&format!("{base_url}/v1/responses"))
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Content-Type", "application/json")
        .json(&crate::services::tools::redact_value(body.clone()))
        .send()
        .await?;

    let status = resp.status();
    let resp_text = resp.text().await?;
    if !status.is_success() {
        return Err(LlmError::Http {
            status: status.as_u16(),
            message: crate::services::tools::redact_secrets(&resp_text),
        }
        .into());
    }

    let resp_json: serde_json::Value = serde_json::from_str(&resp_text)?;

    if !resp_json["output"].is_array() {
        return Err(LlmError::InvalidResponse("missing response output".into()).into());
    }

    let mut stop_reason = stop_reason(&resp_json)?;

    // Parse output items
    let mut text = String::new();
    let mut tool_calls = Vec::new();
    let mut has_function_calls = false;

    if let Some(output) = resp_json["output"].as_array() {
        for item in output {
            match item["type"].as_str() {
                Some("message") => {
                    if let Some(content) = item["content"].as_array() {
                        for part in content {
                            if part["type"].as_str() == Some("output_text") {
                                if let Some(t) = part["text"].as_str() {
                                    text.push_str(t);
                                }
                            }
                        }
                    }
                }
                Some("function_call") => {
                    has_function_calls = true;
                    tool_calls.push(ToolCall::from_json_strings(item, "call_id", "arguments")?);
                }
                _ => {}
            }
        }
    }

    // If there are function calls, set stop_reason to tool_use
    if has_function_calls && stop_reason == StopReason::Complete {
        stop_reason = StopReason::ToolCalls;
    }

    let input_tokens = resp_json["usage"]["input_tokens"].as_u64().unwrap_or(0);
    let output_tokens = resp_json["usage"]["output_tokens"].as_u64().unwrap_or(0);
    let cached_tokens = resp_json["usage"]["input_tokens_details"]["cached_tokens"]
        .as_u64()
        .unwrap_or(0);
    let uncached_tokens = input_tokens.saturating_sub(cached_tokens);
    let tokens_used = if json_schema.is_some() {
        input_tokens + output_tokens
    } else {
        (output_tokens as f64 + uncached_tokens as f64 * 0.2 + cached_tokens as f64 * 0.1) as u64
    };

    let usage = Usage {
        input_tokens,
        output_tokens,
        cache_read_tokens: cached_tokens,
        cache_write_tokens: 0,
    };
    Ok(LlmResponse {
        ordered_content: vec![ContentBlock::text(&text)],
        text,
        tool_calls,
        stop_reason,
        tokens_used,
        usage,
    })
}

/// Streaming OpenAI Responses API call.
pub(crate) async fn openai_stream(
    http: &reqwest::Client,
    api_key: &str,
    model: &str,
    system: &[&str],
    tool_defs: &[ToolDefinition],
    messages: &[Message],
    max_tokens: u64,
    events: &EventSink<'_>,
    base_url: &str,
) -> anyhow::Result<LlmResponse> {
    let (instructions, input) = messages_to_openai(system, messages);
    let tools = tools_to_openai(tool_defs, true);

    let mut body = serde_json::json!({
        "model": model,
        "max_output_tokens": max_tokens,
        "input": input,
        "store": false,
        "stream": true,
        "context_management": [{"type": "compaction", "compact_threshold": 100000}],
    });
    if !instructions.is_empty() {
        body["instructions"] = serde_json::Value::String(instructions);
    }
    if !tools.is_empty() {
        body["tools"] = serde_json::Value::Array(tools);
    }

    let resp = http
        .post(&format!("{base_url}/v1/responses"))
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Content-Type", "application/json")
        .json(&crate::services::tools::redact_value(body.clone()))
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(LlmError::Http {
            status: status.as_u16(),
            message: crate::services::tools::redact_secrets(&text),
        }
        .into());
    }

    let mut text = String::new();
    // Track function calls by output_index
    let mut fn_calls: std::collections::HashMap<usize, (String, String, String)> =
        std::collections::HashMap::new(); // idx -> (call_id, name, args)
    let mut stop_reason = StopReason::Complete;
    let mut ordered_content: Vec<ContentBlock> = Vec::new();
    let mut tokens_used: u64 = 0;
    let mut usage = Usage::default();
    let mut completed = false;
    let mut final_calls = Vec::new();

    let mut stream = resp.bytes_stream();
    let mut buf = Vec::new();
    const STREAM_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(480);

    loop {
        let chunk = tokio::time::timeout(STREAM_TIMEOUT, stream.next()).await;
        let chunk = match chunk {
            Ok(Some(Ok(c))) => c,
            Ok(Some(Err(e))) => return Err(e.into()),
            Ok(None) => break,
            Err(_) => return Err(LlmError::Timeout.into()),
        };

        buf.extend_from_slice(&chunk);

        while let Some(newline_pos) = buf.iter().position(|&b| b == b'\n') {
            let line = String::from_utf8_lossy(&buf[..newline_pos]).to_string();
            buf = buf[newline_pos + 1..].to_vec();

            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            // Responses API uses "event: <type>" + "data: <json>" format
            // We need to track event types
            if line.starts_with("event:") {
                continue; // We parse from the data payload which has "type" field
            }

            let Some(data) = line.strip_prefix("data: ") else {
                continue;
            };
            if data == "[DONE]" {
                break;
            }
            let ev: serde_json::Value = serde_json::from_str(data)?;

            let event_type = ev["type"].as_str().unwrap_or("");

            match event_type {
                // Text content streaming
                "response.output_text.delta" => {
                    if let Some(delta) = ev["delta"].as_str() {
                        text.push_str(delta);
                        events(LlmEvent::TextDelta(delta.to_string()));
                    }
                }

                // Function call arguments streaming
                "response.function_call_arguments.delta" => {
                    let delta = ev["delta"].as_str().ok_or_else(|| {
                        LlmError::InvalidResponse("missing argument delta".into())
                    })?;
                    let idx = ev["output_index"].as_u64().ok_or_else(|| {
                        LlmError::InvalidResponse("missing tool output index".into())
                    })? as usize;
                    let entry = fn_calls.get_mut(&idx).ok_or_else(|| {
                        LlmError::InvalidResponse("arguments without tool metadata".into())
                    })?;
                    entry.2.push_str(delta);
                    events(LlmEvent::ToolArgumentsDelta {
                        id: entry.0.clone(),
                        name: entry.1.clone(),
                        delta: delta.to_string(),
                    });
                }

                // New output item added — capture function call metadata
                "response.output_item.added" => {
                    if let Some(item) = ev.get("item") {
                        let idx = ev["output_index"].as_u64().ok_or_else(|| {
                            LlmError::InvalidResponse("missing output index".into())
                        })? as usize;
                        if item["type"].as_str() == Some("function_call") {
                            let call_id = ToolCall::required_string(item, "call_id")?;
                            let name = ToolCall::required_string(item, "name")?;
                            fn_calls.insert(idx, (call_id, name, String::new()));

                            events(LlmEvent::ToolCallStarted {
                                id: ToolCall::required_string(item, "call_id")?,
                                name: ToolCall::required_string(item, "name")?,
                            });
                        }
                        // Web search activity
                        if item["type"].as_str() == Some("web_search_call") {
                            events(LlmEvent::Activity {
                                name: "web_search".into(),
                                description: "searching the web".into(),
                            });
                        }
                    }
                }

                // Response completed — extract final status and usage
                "response.completed" | "response.incomplete" => {
                    completed = true;
                    if let Some(response) = ev.get("response") {
                        stop_reason = self::stop_reason(response)?;
                        let output = response["output"].as_array().ok_or_else(|| {
                            LlmError::InvalidResponse("missing final output".into())
                        })?;
                        for item in output {
                            if item["type"].as_str() == Some("function_call") {
                                final_calls.push(ToolCall::from_json_strings(
                                    item,
                                    "call_id",
                                    "arguments",
                                )?);
                            }
                        }

                        // Usage — includes prompt caching details
                        if let Some(raw_usage) = response.get("usage") {
                            let input_t = raw_usage["input_tokens"].as_u64().unwrap_or(0);
                            let output_t = raw_usage["output_tokens"].as_u64().unwrap_or(0);
                            let cached_t = raw_usage["input_tokens_details"]["cached_tokens"]
                                .as_u64()
                                .unwrap_or(0);
                            let uncached_t = input_t.saturating_sub(cached_t);
                            log::info!(
                                "openai usage: input={} (cached={} uncached={}) output={}",
                                input_t,
                                cached_t,
                                uncached_t,
                                output_t,
                            );
                            usage = Usage {
                                input_tokens: input_t,
                                output_tokens: output_t,
                                cache_read_tokens: cached_t,
                                cache_write_tokens: 0,
                            };
                            // Normalize: cached input is ~50% cheaper on OpenAI
                            tokens_used =
                                (output_t as f64 + uncached_t as f64 * 0.2 + cached_t as f64 * 0.1)
                                    as u64;
                        }
                    }
                }

                "error" | "response.failed" => {
                    return Err(LlmError::InvalidResponse(ev.to_string()).into());
                }
                _ => {}
            }
        }
    }

    if !completed {
        return Err(
            LlmError::InvalidResponse("stream ended before response completion".into()).into(),
        );
    }

    // Build tool use blocks from collected function calls
    let mut tool_calls: Vec<ToolCall> = Vec::new();
    let mut indices: Vec<usize> = fn_calls.keys().copied().collect();
    indices.sort();
    for idx in indices {
        let (call_id, name, args_str) = &fn_calls[&idx];
        let arguments = ToolCall::parse_arguments(args_str)?;
        tool_calls.push(ToolCall {
            id: call_id.clone(),
            name: name.clone(),
            arguments,
        });
    }

    if final_calls.len() != tool_calls.len()
        || final_calls.iter().any(|call| {
            !tool_calls.iter().any(|streamed| {
                streamed.id == call.id
                    && streamed.name == call.name
                    && streamed.arguments == call.arguments
            })
        })
    {
        return Err(LlmError::InvalidResponse(
            "final tool calls differ from streamed calls".into(),
        )
        .into());
    }

    if !text.is_empty() {
        ordered_content.push(ContentBlock::Text { text: text.clone() });
    }

    if !tool_calls.is_empty() && stop_reason == StopReason::Complete {
        stop_reason = StopReason::ToolCalls;
    }

    events(LlmEvent::Usage(usage));
    Ok(LlmResponse {
        text,
        tool_calls,
        stop_reason,
        tokens_used,
        ordered_content,
        usage,
    })
}

pub(super) const CAPABILITIES: Capabilities = Capabilities {
    vision: true,
    documents: false,
    tools: true,
    streaming: true,
    reasoning_controls: false,
    model_discovery: false,
};

/// The transport implementation is private to this adapter.
pub(super) struct OpenaiAdapter(pub LlmBackend);
impl ProviderAdapter for OpenaiAdapter {
    fn capabilities(&self) -> Capabilities {
        CAPABILITIES
    }
    fn complete<'a>(
        &'a self,
        request: LlmRequest<'a>,
    ) -> futures::future::BoxFuture<'a, Result<LlmResponse, LlmError>> {
        Box::pin(async move {
            request.validate(self.capabilities(), false)?;
            let b = &self.0;
            tokio::select! {
                biased;
                _ = request.cancellation.cancelled() => Err(LlmError::Cancelled),
                result = openai_complete(&b.http, &b.api_key, &b.model, request.system, request.tools, request.messages, request.max_tokens, &b.base_url, request.json_schema) => result.map_err(LlmError::from),
            }
        })
    }
    fn stream<'a>(
        &'a self,
        request: LlmRequest<'a>,
        events: &'a EventSink<'a>,
    ) -> futures::future::BoxFuture<'a, Result<LlmResponse, LlmError>> {
        Box::pin(async move {
            request.validate(self.capabilities(), true)?;
            let b = &self.0;
            tokio::select! {
                biased;
                _ = request.cancellation.cancelled() => Err(LlmError::Cancelled),
                result = openai_stream(&b.http, &b.api_key, &b.model, request.system, request.tools, request.messages, request.max_tokens, events, &b.base_url) => result.map_err(LlmError::from),
            }
        })
    }
}
