use std::time::Duration;

use futures::StreamExt;

use crate::services::tool::ToolDefinition;

use super::contract::{
    Capabilities, EventSink, LlmError, LlmEvent, LlmRequest, ProviderAdapter, StopReason, Usage,
};
use super::types::LlmBackend;
use super::types::{ContentBlock, LlmResponse, Message, ToolCall};

// ═══════════════════════════════════════════════════════════════════════════
// Anthropic API
// ═══════════════════════════════════════════════════════════════════════════

pub(crate) fn anthropic_headers(api_key: &str) -> Result<reqwest::header::HeaderMap, LlmError> {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        "x-api-key",
        api_key
            .parse()
            .map_err(|_| LlmError::SetupRequired("invalid Anthropic API key header".into()))?,
    );
    headers.insert(
        "anthropic-beta",
        "interleaved-thinking-2025-05-14,compact-2026-01-12"
            .parse()
            .unwrap(),
    );
    headers.insert("anthropic-version", "2023-06-01".parse().unwrap());
    headers.insert("content-type", "application/json".parse().unwrap());
    Ok(headers)
}

pub(crate) fn build_anthropic_request(
    model: &str,
    system: &[&str],
    tool_defs: &[ToolDefinition],
    messages: &[Message],
    max_tokens: u64,
    stream: bool,
    _api_key: &str,
) -> serde_json::Value {
    // System blocks — all blocks are stable now (time moved to user message).
    // Each block gets cache_control to maximize prefix caching.
    let system_blocks: Vec<serde_json::Value> = system
        .iter()
        .enumerate()
        .filter(|(_, s)| !s.is_empty())
        .map(|(i, s)| {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            s.hash(&mut hasher);
            let hash = hasher.finish();
            log::info!(
                "[llm] system block[{i}]: {} chars, hash={:x}",
                s.len(),
                hash
            );
            serde_json::json!({
                "type": "text",
                "text": *s,
                "cache_control": {"type": "ephemeral"},
            })
        })
        .collect();

    // Tool definitions
    let tool_count = tool_defs.len();
    let tools: Vec<serde_json::Value> = tool_defs
        .iter()
        .enumerate()
        .map(|(i, td)| {
            let mut tool = serde_json::json!({
                "name": td.name,
                "description": td.description,
                "input_schema": td.parameters,
            });
            // Cache breakpoint on last tool — caches all tools as one prefix
            if i == tool_count - 1 {
                tool["cache_control"] = serde_json::json!({"type": "ephemeral"});
            }
            tool
        })
        .collect();

    // Messages — strip any legacy oversized base64 images
    let mut msgs = messages_to_anthropic(messages);
    if let Some(arr) = msgs.as_array_mut() {
        for msg in arr.iter_mut() {
            if let Some(content_arr) = msg.get_mut("content").and_then(|c| c.as_array_mut()) {
                // Compaction blocks are passed through as-is — the API
                // understands them natively with the compact-2026-01-12 beta.
                content_arr.retain(|block| {
                    let block_type = block.get("type").and_then(|t| t.as_str());
                    // Strip oversized base64 images
                    if block_type == Some("image") {
                        if let Some(data) = block.pointer("/source/data").and_then(|d| d.as_str()) {
                            if data.len() > 5 * 1024 * 1024 {
                                log::info!(
                                    "stripping oversized base64 image ({} bytes)",
                                    data.len()
                                );
                                return false;
                            }
                        }
                    }
                    // Strip blocks with no recognized type (Unknown variant)
                    if block_type.is_none() {
                        log::info!("stripping block with no type");
                        return false;
                    }
                    true
                });
                // Remove empty content arrays (can happen after stripping)
                if content_arr.is_empty() {
                    content_arr.push(serde_json::json!({"type": "text", "text": "(continued)"}));
                }
            }
        }
    }

    // Strip orphaned tool_result blocks — can happen when server-side compaction
    // replaces tool_use with summary text but leaves the tool_result in place.
    if let Some(arr) = msgs.as_array_mut() {
        // Collect all tool_use IDs from assistant messages
        let mut tool_use_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
        for msg in arr.iter() {
            if msg.get("role").and_then(|r| r.as_str()) == Some("assistant") {
                if let Some(content) = msg.get("content").and_then(|c| c.as_array()) {
                    for block in content {
                        if block.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                            if let Some(id) = block.get("id").and_then(|i| i.as_str()) {
                                tool_use_ids.insert(id.to_string());
                            }
                        }
                    }
                }
            }
        }
        // Remove tool_result blocks that reference non-existent tool_use IDs
        for msg in arr.iter_mut() {
            if msg.get("role").and_then(|r| r.as_str()) == Some("user") {
                if let Some(content) = msg.get_mut("content").and_then(|c| c.as_array_mut()) {
                    let before = content.len();
                    content.retain(|block| {
                        if block.get("type").and_then(|t| t.as_str()) == Some("tool_result") {
                            if let Some(id) = block.get("tool_use_id").and_then(|i| i.as_str()) {
                                if !tool_use_ids.contains(id) {
                                    log::warn!("[llm] stripping orphaned tool_result for {id} (tool_use lost, likely compaction)");
                                    return false;
                                }
                            }
                        }
                        true
                    });
                    if content.is_empty() && before > 0 {
                        content.push(serde_json::json!({"type": "text", "text": "(tool result removed — original tool call was compacted)"}));
                    }
                }
            }
        }
    }

    // Merge consecutive same-role messages (API requires strict alternation)
    if let Some(arr) = msgs.as_array_mut() {
        let mut merged: Vec<serde_json::Value> = Vec::with_capacity(arr.len());
        for msg in arr.drain(..) {
            let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");
            let last_role = merged
                .last()
                .and_then(|m| m.get("role"))
                .and_then(|r| r.as_str())
                .unwrap_or("");
            if role == last_role && !role.is_empty() {
                // Merge content arrays
                if let Some(last) = merged.last_mut() {
                    if let (Some(existing), Some(new_content)) = (
                        last.get_mut("content").and_then(|c| c.as_array_mut()),
                        msg.get("content").and_then(|c| c.as_array()),
                    ) {
                        existing.extend(new_content.iter().cloned());
                    }
                }
            } else {
                merged.push(msg);
            }
        }
        *arr = merged;
    }

    // Top-level cache_control: Anthropic automatically places a cache breakpoint
    // on the last cacheable block, so the entire conversation history (system +
    // tools + all prior messages) is cached. No manual per-message breakpoints needed.
    let mut req = serde_json::json!({
        "model": model,
        "max_tokens": max_tokens,
        "cache_control": {"type": "ephemeral"},
        "system": system_blocks,
        "messages": msgs,
        "context_management": {
            "edits": [{
                "type": "compact_20260112",
                "trigger": {"type": "input_tokens", "value": 100000},
            }]
        },
    });

    if !tools.is_empty() {
        req["tools"] = serde_json::Value::Array(tools);
    }

    // Anthropic server tools (always added for streaming chat with tools)
    if stream && !tool_defs.is_empty() {
        let tools_arr = req["tools"].as_array_mut().unwrap();

        // Web search + fetch (native Anthropic)
        // allowed_callers: ["direct"] disables dynamic filtering (code_execution for search)
        tools_arr.push(serde_json::json!({
            "type": "web_search_20260209",
            "name": "web_search",
            "allowed_callers": ["direct"]
        }));
        tools_arr.push(serde_json::json!({
            "type": "web_fetch_20260209",
            "name": "web_fetch",
            "allowed_callers": ["direct"]
        }));
    }
    req["stream"] = serde_json::json!(stream);
    req
}

/// Non-streaming Anthropic call. Returns (text, tool_calls, stop_reason, tokens_used).
pub(crate) async fn anthropic_complete(
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
    let mut body = build_anthropic_request(
        model, system, tool_defs, messages, max_tokens, false, api_key,
    );
    if let Some(schema) = json_schema {
        body["output_config"] =
            serde_json::json!({"format": {"type": "json_schema", "schema": schema}});
    }

    let resp = http
        .post(&format!("{}/v1/messages", base_url))
        .headers(anthropic_headers(api_key)?)
        .json(&body)
        .send()
        .await?;

    let status = resp.status();
    let resp_text = resp.text().await?;
    if !status.is_success() {
        log::error!(
            "[llm] API {status} — model={model}, msgs={}, body_chars={}",
            messages.len(),
            serde_json::to_string(&body).map(|s| s.len()).unwrap_or(0),
        );
        return Err(LlmError::Http {
            status: status.as_u16(),
            message: resp_text,
        }
        .into());
    }

    let resp_json: serde_json::Value = serde_json::from_str(&resp_text)?;
    if !resp_json["content"].is_array() {
        return Err(LlmError::InvalidResponse("missing response content".into()).into());
    }
    let stop_reason = resp_json["stop_reason"]
        .as_str()
        .ok_or_else(|| LlmError::InvalidResponse("missing stop reason".into()))?
        .to_string();

    let tokens_used = if let Some(usage) = resp_json.get("usage") {
        let input = usage["input_tokens"].as_u64().unwrap_or(0);
        let output = usage["output_tokens"].as_u64().unwrap_or(0);
        let cache_read = usage["cache_read_input_tokens"].as_u64().unwrap_or(0);
        let cache_write = usage["cache_creation_input_tokens"].as_u64().unwrap_or(0);
        log::info!(
            "anthropic usage: input={} cache_read={} cache_write={} output={}",
            input,
            cache_read,
            cache_write,
            output,
        );
        // Normalize to output-equivalent tokens by cost ratio (Sonnet 4.6 pricing):
        // Output: $15/M (1.0x), Input: $3/M (0.2x), Cache write: $3.75/M (0.25x), Cache read: $0.30/M (0.02x)
        let normalized = (output as f64)
            + (input as f64 * 0.2)
            + (cache_write as f64 * 0.25)
            + (cache_read as f64 * 0.02);
        if json_schema.is_some() {
            input + output
        } else {
            normalized as u64
        }
    } else {
        0
    };

    let mut text = String::new();
    let mut tool_calls = Vec::new();

    if let Some(blocks) = resp_json["content"].as_array() {
        for b in blocks {
            match b["type"].as_str() {
                Some("text") => {
                    if let Some(s) = b["text"].as_str() {
                        text.push_str(s);
                    }
                }
                Some("compaction") => {
                    // Server-side compaction — will be handled by caller
                    log::info!("[compaction] non-streaming compaction block received");
                }
                Some("tool_use") => {
                    let id = ToolCall::required_string(b, "id")?;
                    let name = ToolCall::required_string(b, "name")?;
                    let arguments = b["input"].clone();
                    ToolCall::validate_arguments(&arguments)?;
                    tool_calls.push(ToolCall {
                        id,
                        name,
                        arguments,
                    });
                }
                _ => {}
            }
        }
    }

    let raw = &resp_json["usage"];
    let usage = Usage {
        input_tokens: raw["input_tokens"].as_u64().unwrap_or(0)
            + raw["cache_read_input_tokens"].as_u64().unwrap_or(0)
            + raw["cache_creation_input_tokens"].as_u64().unwrap_or(0),
        output_tokens: raw["output_tokens"].as_u64().unwrap_or(0),
        cache_read_tokens: raw["cache_read_input_tokens"].as_u64().unwrap_or(0),
        cache_write_tokens: raw["cache_creation_input_tokens"].as_u64().unwrap_or(0),
    };
    Ok(LlmResponse {
        ordered_content: vec![ContentBlock::text(&text)],
        text,
        tool_calls,
        stop_reason: anthropic_stop_reason(&stop_reason)?,
        tokens_used,
        usage,
    })
}

/// Streaming Anthropic call. Broadcasts text deltas, returns (text, tool_calls, stop_reason, tokens_used, ordered_content).
pub(crate) async fn anthropic_stream(
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
    let body = build_anthropic_request(
        model, system, tool_defs, messages, max_tokens, true, api_key,
    );

    let headers = anthropic_headers(api_key)?;
    let resp = http
        .post(&format!("{}/v1/messages", base_url))
        .headers(headers)
        .json(&body)
        .send()
        .await?;

    let status = resp.status();
    if !status.is_success() {
        let err_text = resp.text().await.unwrap_or_default();
        log::error!(
            "[llm] streaming API {status} — model={model}, msgs={}",
            messages.len(),
        );
        // Log the message types to help debug which content block is invalid
        for (i, msg) in messages.iter().enumerate() {
            let (role, blocks) = match msg {
                Message::User { content } => ("user", content),
                Message::Assistant { content } => ("assistant", content),
            };
            let types: Vec<&str> = blocks
                .iter()
                .map(|b| match b {
                    ContentBlock::Text { .. } => "text",
                    ContentBlock::Image { .. } => "image",
                    ContentBlock::Document { .. } => "document",
                    ContentBlock::ToolCall { .. } => "tool_use",
                    ContentBlock::ToolOutput { .. } => "tool_result",
                    ContentBlock::ContextSummary { .. }
                    | ContentBlock::LegacyContextSummary { .. } => "compaction",
                    ContentBlock::Unknown(_) | ContentBlock::ProviderData { .. } => "provider_data",
                })
                .collect();
            log::error!("[llm] msg[{i}] {role}: {:?}", types);
        }
        return Err(LlmError::Http {
            status: status.as_u16(),
            message: err_text,
        }
        .into());
    }

    let mut text = String::new();
    let mut tool_calls: Vec<ToolCall> = Vec::new();
    let mut stop_reason = String::new();
    let mut input_tokens: u64 = 0;
    let mut output_tokens: u64 = 0;
    let mut cache_read_tokens: u64 = 0;
    let mut cache_write_tokens: u64 = 0;
    let mut current_server_block: Option<serde_json::Value> = None;

    // Ordered content blocks — preserves interleaving of text and server tools
    let mut ordered_content: Vec<ContentBlock> = Vec::new();
    let mut current_text_block = String::new();

    // Current block being built
    let mut current_block_type = String::new();
    let mut current_tool_id = String::new();
    let mut current_tool_name = String::new();
    let mut current_tool_input_json = String::new();

    // SSE parser
    let mut stream = resp.bytes_stream();
    let mut buf = Vec::new();
    let mut event_type = String::new();
    let mut completed = false;

    const STREAM_TIMEOUT: Duration = Duration::from_secs(480);

    loop {
        let chunk = tokio::time::timeout(STREAM_TIMEOUT, stream.next()).await;
        let chunk = match chunk {
            Ok(Some(Ok(c))) => c,
            Ok(Some(Err(e))) => return Err(e.into()),
            Ok(None) => break,
            Err(_) => {
                log::warn!("stream timed out after {}s", STREAM_TIMEOUT.as_secs());
                return Err(LlmError::Timeout.into());
            }
        };

        buf.extend_from_slice(&chunk);

        // Process complete lines
        while let Some(newline_pos) = buf.iter().position(|&b| b == b'\n') {
            let line = String::from_utf8_lossy(&buf[..newline_pos]).to_string();
            buf = buf[newline_pos + 1..].to_vec();

            if line.is_empty() {
                // End of event — process it
                // (event_type is set from the "event: " line)
                event_type.clear();
                continue;
            }

            if let Some(e) = line.strip_prefix("event: ") {
                event_type = e.to_string();
                continue;
            }

            let Some(data) = line.strip_prefix("data: ") else {
                continue;
            };

            let ev: serde_json::Value = serde_json::from_str(data)?;

            match event_type.as_str() {
                "message_start" => {
                    if let Some(msg) = ev.get("message") {
                        if let Some(usage) = msg.get("usage") {
                            input_tokens = usage["input_tokens"].as_u64().unwrap_or(0);
                            cache_read_tokens =
                                usage["cache_read_input_tokens"].as_u64().unwrap_or(0);
                            cache_write_tokens =
                                usage["cache_creation_input_tokens"].as_u64().unwrap_or(0);
                            let real_total = input_tokens + cache_read_tokens + cache_write_tokens;
                            log::info!(
                                "anthropic cache: read={} write={} input={} real_total={}",
                                cache_read_tokens,
                                cache_write_tokens,
                                input_tokens,
                                real_total,
                            );
                        }
                    }
                }
                "content_block_start" => {
                    if current_block_type == "tool_use" {
                        return Err(LlmError::InvalidResponse(
                            "tool call missing block stop".into(),
                        )
                        .into());
                    }
                    if let Some(block) = ev.get("content_block") {
                        current_block_type = block["type"].as_str().unwrap_or("").to_string();

                        // Flush accumulated text before server tool blocks
                        if (current_block_type == "server_tool_use"
                            || current_block_type.ends_with("_tool_result"))
                            && !current_text_block.is_empty()
                        {
                            ordered_content.push(ContentBlock::text(&current_text_block));
                            current_text_block.clear();
                        }

                        // Save server tool blocks as raw JSON for rig_history
                        if current_block_type == "server_tool_use"
                            || current_block_type.ends_with("_tool_result")
                        {
                            current_server_block = Some(block.clone());
                        }

                        // Broadcast server tool activity (web_search, code_execution, etc.)
                        if current_block_type == "server_tool_use" {
                            let tool_name = block["name"].as_str().unwrap_or("server_tool");
                            let summary = match tool_name {
                                "web_search" => "searching the web".to_string(),
                                "web_fetch" => "fetching web page".to_string(),
                                "bash_code_execution" | "code_execution" => {
                                    "executing code".to_string()
                                }
                                "text_editor_code_execution" => {
                                    "editing file in sandbox".to_string()
                                }
                                other => format!("server: {other}"),
                            };
                            events(LlmEvent::Activity {
                                name: tool_name.to_string(),
                                description: summary,
                            });
                        }

                        // Compaction blocks — server-side context summary
                        if current_block_type == "compaction" {
                            // Flush any pending text
                            if !current_text_block.is_empty() {
                                ordered_content.push(ContentBlock::text(&current_text_block));
                                current_text_block.clear();
                            }
                            // Content will arrive via text_delta; we'll assemble it at block_stop
                            log::info!("[compaction] server-side compaction block started");
                        }

                        if current_block_type == "tool_use" {
                            current_tool_id = ToolCall::required_string(block, "id")?;
                            current_tool_name = ToolCall::required_string(block, "name")?;
                            current_tool_input_json.clear();

                            events(LlmEvent::ToolCallStarted {
                                id: current_tool_id.clone(),
                                name: current_tool_name.clone(),
                            });
                        }
                    }
                }
                "content_block_delta" => {
                    if let Some(delta) = ev.get("delta") {
                        match delta["type"].as_str() {
                            Some("text_delta") => {
                                if let Some(t) = delta["text"].as_str() {
                                    current_text_block.push_str(t);
                                    // Compaction text is metadata, not user-visible
                                    if current_block_type != "compaction" {
                                        text.push_str(t);
                                        events(LlmEvent::TextDelta(t.to_string()));
                                    }
                                }
                            }
                            Some("input_json_delta") => {
                                let partial = delta["partial_json"].as_str().ok_or_else(|| {
                                    LlmError::InvalidResponse("missing tool argument delta".into())
                                })?;
                                current_tool_input_json.push_str(partial);
                                events(LlmEvent::ToolArgumentsDelta {
                                    id: current_tool_id.clone(),
                                    name: current_tool_name.clone(),
                                    delta: partial.to_string(),
                                });
                            }
                            _ => {}
                        }
                    }
                }
                "content_block_stop" => {
                    // Commit completed server tool block (preserves order)
                    if let Some(block) = current_server_block.take() {
                        ordered_content.push(ContentBlock::ProviderData {
                            provider: crate::config::LlmProvider::Anthropic,
                            data: block,
                        });
                    }
                    // Compaction block complete — flush accumulated text as Compaction
                    if current_block_type == "compaction" {
                        let summary = std::mem::take(&mut current_text_block);
                        if !summary.is_empty() {
                            log::info!("[compaction] server-side summary: {} chars", summary.len());
                            ordered_content.push(ContentBlock::ContextSummary { content: summary });
                        }
                        // Don't include compaction text in the response text
                        // (it's metadata, not user-visible content)
                        current_block_type.clear();
                    }
                    if current_block_type == "tool_use" {
                        let arguments = ToolCall::parse_arguments(&current_tool_input_json)?;
                        tool_calls.push(ToolCall {
                            id: current_tool_id.clone(),
                            name: current_tool_name.clone(),
                            arguments,
                        });
                        current_block_type.clear();
                    }
                }
                "message_delta" => {
                    if let Some(delta) = ev.get("delta") {
                        if let Some(sr) = delta["stop_reason"].as_str() {
                            stop_reason = sr.to_string();
                        }
                    }
                    if let Some(usage) = ev.get("usage") {
                        output_tokens = usage["output_tokens"].as_u64().unwrap_or(0);
                        log::info!("anthropic output tokens: {}", output_tokens);
                    }
                }
                "message_stop" => {
                    completed = true;
                }
                "error" => {
                    let error_msg = ev["error"]["message"].as_str().unwrap_or("unknown error");
                    return Err(anyhow::anyhow!("Anthropic stream error: {error_msg}"));
                }
                _ => {}
            }
        }
    }

    if !completed {
        return Err(LlmError::InvalidResponse("stream ended before message_stop".into()).into());
    }

    // Normalize to output-equivalent tokens by cost ratio
    let tokens_used = {
        let normalized = (output_tokens as f64)
            + (input_tokens as f64 * 0.2)
            + (cache_write_tokens as f64 * 0.25)
            + (cache_read_tokens as f64 * 0.02);
        normalized as u64
    };
    // Flush remaining text
    if !current_text_block.is_empty() {
        ordered_content.push(ContentBlock::text(&current_text_block));
    }

    if current_block_type == "tool_use" {
        return Err(LlmError::InvalidResponse("unfinished tool call".into()).into());
    }
    let usage = Usage {
        input_tokens: input_tokens + cache_read_tokens + cache_write_tokens,
        output_tokens,
        cache_read_tokens,
        cache_write_tokens,
    };
    events(LlmEvent::Usage(usage));
    Ok(LlmResponse {
        text,
        tool_calls,
        stop_reason: anthropic_stop_reason(&stop_reason)?,
        tokens_used,
        ordered_content,
        usage,
    })
}

pub(super) const CAPABILITIES: Capabilities = Capabilities {
    vision: true,
    documents: true,
    tools: true,
    streaming: true,
    reasoning_controls: false,
    model_discovery: false,
};

/// The transport implementation is private to this adapter.
pub(super) struct AnthropicAdapter(pub LlmBackend);
impl ProviderAdapter for AnthropicAdapter {
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
                result = anthropic_complete(&b.http, &b.api_key, &b.model, request.system, request.tools, request.messages, request.max_tokens, &b.base_url, request.json_schema) => result.map_err(LlmError::from),
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
                result = anthropic_stream(&b.http, &b.api_key, &b.model, request.system, request.tools, request.messages, request.max_tokens, events, &b.base_url) => result.map_err(LlmError::from),
            }
        })
    }
}

/// Explicit wire conversion: persisted history is not an API request schema.
pub(crate) fn messages_to_anthropic(messages: &[Message]) -> serde_json::Value {
    use crate::config::LlmProvider;
    use serde_json::json;
    fn block_to_wire(block: &ContentBlock) -> Option<serde_json::Value> {
        Some(match block {
            ContentBlock::Text { text } => json!({"type": "text", "text": text}),
            ContentBlock::Image { source } => json!({"type": "image", "source": source}),
            ContentBlock::Document { source } => json!({"type": "document", "source": source}),
            ContentBlock::ToolCall {
                id,
                name,
                arguments,
            } => json!({"type": "tool_use", "id": id, "name": name, "input": arguments}),
            ContentBlock::ToolOutput { call_id, content } => {
                json!({"type": "tool_result", "tool_use_id": call_id, "content": content})
            }
            ContentBlock::ContextSummary { content }
            | ContentBlock::LegacyContextSummary {
                summary: content, ..
            } => json!({"type": "compaction", "content": content}),
            ContentBlock::ProviderData {
                provider: LlmProvider::Anthropic,
                data,
            } => data.clone(),
            ContentBlock::ProviderData { .. } => return None,
            // Before provider tagging existed, unknown blocks came from Anthropic.
            ContentBlock::Unknown(data) => data.clone(),
        })
    }
    json!(
        messages
            .iter()
            .filter_map(|message| {
                let (role, blocks) = match message {
                    Message::User { content } => ("user", content),
                    Message::Assistant { content } => ("assistant", content),
                };
                let content: Vec<_> = blocks.iter().filter_map(block_to_wire).collect();
                if content.is_empty() {
                    None
                } else {
                    Some(json!({"role": role, "content": content}))
                }
            })
            .collect::<Vec<_>>()
    )
}

fn anthropic_stop_reason(value: &str) -> Result<StopReason, LlmError> {
    match value {
        "end_turn" | "stop_sequence" => Ok(StopReason::Complete),
        "tool_use" => Ok(StopReason::ToolCalls),
        "max_tokens" => Ok(StopReason::OutputLimit),
        "pause_turn" => Ok(StopReason::Continue),
        "compaction" => Ok(StopReason::ContextUpdated),
        _ => Err(LlmError::InvalidResponse(format!(
            "unknown stop reason: {value}"
        ))),
    }
}
