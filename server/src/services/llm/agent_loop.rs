use std::path::Path;

use tokio::sync::broadcast;

use crate::domain::chat::{ChatMessage, ChatRole};
use crate::domain::events::ServerEvent;
use crate::services::tool::{ToolDefinition, ToolDyn};

use super::contract::{LlmEvent, LlmRequest, StopReason};
use super::helpers::strip_context_blocks;

use super::types::{ContentBlock, HistoryEntry, LlmBackend, LlmResponse, Message, ToolCall};

// ═══════════════════════════════════════════════════════════════════════════
// Agent loops (tool call -> execute -> send back)
// ═══════════════════════════════════════════════════════════════════════════

pub(crate) async fn collect_tool_defs(tools: &[Box<dyn ToolDyn>]) -> Vec<ToolDefinition> {
    let mut defs = Vec::with_capacity(tools.len());
    for t in tools {
        defs.push(t.definition(String::new()).await);
    }
    defs
}

/// Non-streaming agent loop. Returns (final text, total tokens used).
pub(crate) async fn agent_loop(
    backend: &LlmBackend,
    system: &[&str],
    tool_defs: &[ToolDefinition],
    tools: &[Box<dyn ToolDyn>],
    messages: &mut Vec<Message>,
) -> anyhow::Result<(String, u64)> {
    let mut total_tokens: u64 = 0;
    loop {
        let (text, tool_calls, stop_reason, tokens) =
            complete_once(backend, system, tool_defs, messages).await?;
        total_tokens += tokens;

        // Build assistant message
        let mut assistant_content = Vec::new();
        if !text.is_empty() {
            assistant_content.push(ContentBlock::text(&text));
        }
        for tu in &tool_calls {
            assistant_content.push(ContentBlock::ToolCall {
                id: tu.id.clone(),
                name: tu.name.clone(),
                arguments: tu.arguments.clone(),
            });
        }
        messages.push(Message::Assistant {
            content: assistant_content,
        });

        if stop_reason == StopReason::OutputLimit {
            log::warn!("[llm] response truncated (max_tokens reached) — requesting continuation");
            messages.push(Message::User {
                content: vec![ContentBlock::text(
                    "[system: your previous response was cut off due to length. please continue exactly where you left off.]",
                )],
            });
            continue;
        }

        if stop_reason == StopReason::Continue {
            log::info!("[llm] pause_turn — code execution in progress, continuing...");
            continue;
        }

        // Server-side compaction completed — continue with compacted context
        if stop_reason == StopReason::ContextUpdated {
            log::info!("[llm] server-side compaction in non-streaming loop — continuing");
            continue;
        }

        if stop_reason != StopReason::ToolCalls || tool_calls.is_empty() {
            return Ok((text, total_tokens));
        }

        // Execute validated tool calls; outputs retain typed text and image content.
        let mut results = Vec::new();
        for tu in &tool_calls {
            let (content, trusted) = execute_tool(tools, &tu.name, &tu.arguments).await;
            results.push(ContentBlock::tool_output(tu.id.clone(), content, trusted));
        }
        messages.push(Message::User { content: results });
    }
}

/// Streaming agent loop. Returns (final text, message_id, total tokens).
pub(crate) async fn streaming_agent_loop(
    backend: &LlmBackend,
    system: &[&str],
    tool_defs: &[ToolDefinition],
    tools: &[Box<dyn ToolDyn>],
    messages: &mut Vec<Message>,
    events: &broadcast::Sender<ServerEvent>,
    instance_slug: &str,
    chat_id: &str,
    workspace_dir: &Path,
    mcp_snapshot: Option<&crate::services::mcp::McpAppSnapshot>,
    sent_files: &crate::services::tools::SentFiles,
) -> anyhow::Result<(String, Option<String>, u64)> {
    let mut all_text = String::new();
    let mut total_tokens: u64 = 0;
    let mut current_message_id = crate::services::chat::next_id();

    loop {
        let turn = stream_once(
            backend,
            system,
            tool_defs,
            messages,
            events,
            instance_slug,
            chat_id,
            &current_message_id,
            mcp_snapshot,
        )
        .await?;

        total_tokens += turn.tokens_used;
        let turn_text = turn.text;
        let tool_calls = turn.tool_calls;
        let stop_reason = turn.stop_reason;

        // Build assistant message — use ordered_content which preserves
        // the interleaving of text, server_tool_use, and server_tool_result.
        let mut assistant_content = Vec::new();
        // Ordered content: text and server tool blocks in their original order
        assistant_content.extend(turn.ordered_content.into_iter());
        for tu in &tool_calls {
            assistant_content.push(ContentBlock::ToolCall {
                id: tu.id.clone(),
                name: tu.name.clone(),
                arguments: tu.arguments.clone(),
            });
        }
        messages.push(Message::Assistant {
            content: assistant_content,
        });

        if stop_reason == StopReason::OutputLimit {
            log::warn!("[llm] response truncated (max_tokens reached) — requesting continuation");
            all_text.push_str(&turn_text);
            messages.push(Message::User {
                content: vec![ContentBlock::text(
                    "[system: your previous response was cut off due to length. please continue exactly where you left off.]",
                )],
            });
            continue;
        }

        // pause_turn: code execution skill is still running — continue with same messages
        if stop_reason == StopReason::Continue {
            log::info!("[llm] pause_turn — code execution in progress, continuing...");
            all_text.push_str(&turn_text);
            continue;
        }

        // compaction: server-side compaction completed — continue with compacted context
        if stop_reason == StopReason::ContextUpdated {
            log::info!("[llm] server-side compaction — continuing with compacted context");
            all_text.push_str(&turn_text);

            // Broadcast compaction event to UI
            let _ = events.send(ServerEvent::ContextCompacting {
                instance_slug: instance_slug.to_string(),
                chat_id: chat_id.to_string(),
                messages_compacted: messages.len(),
            });

            // Persist compacted history to disk
            let rig_path =
                crate::services::chat::rig_history_path(workspace_dir, instance_slug, chat_id);
            let ts = crate::services::tools::unix_millis().to_string();
            let entries: Vec<HistoryEntry> = messages
                .iter()
                .enumerate()
                .map(|(i, msg)| {
                    HistoryEntry::new(msg.clone(), ts.clone(), format!("compact_{i}_{ts}"))
                })
                .collect();
            crate::services::chat::save_rig_history(&rig_path, &entries);

            // Broadcast snapshot so UI reflects the compacted state
            if let Ok(resp) =
                crate::services::chat::load_messages(workspace_dir, instance_slug, chat_id)
            {
                let _ = events.send(ServerEvent::ChatSnapshot {
                    instance_slug: instance_slug.to_string(),
                    chat_id: chat_id.to_string(),
                    messages: resp.messages,
                    agent_running: true,
                });
            }
            continue;
        }

        // For the final turn (no more tool use), only keep this turn's text.
        all_text = turn_text.clone();

        if stop_reason != StopReason::ToolCalls || tool_calls.is_empty() {
            break;
        }

        // Save intermediate text before tool execution — reuse the streaming message_id
        if !turn_text.trim().is_empty() {
            let ts = crate::services::tools::unix_millis();
            let msg = ChatMessage {
                id: current_message_id.clone(),
                role: ChatRole::Assistant,
                content: turn_text.trim().to_string(),
                created_at: ts.to_string(),
                kind: Default::default(),
                tool_name: None,
                mcp_app_html: None,
                mcp_app_input: None,
                model: None,
            };
            let _ = events.send(ServerEvent::ChatMessageCreated {
                instance_slug: instance_slug.to_string(),
                chat_id: chat_id.to_string(),
                message: msg,
            });
            // Generate new ID for the next streaming turn
            current_message_id = crate::services::chat::next_id();
        }

        // Execute validated tool calls; outputs retain typed text and image content.
        let mut results = Vec::new();
        for tu in &tool_calls {
            let (content, trusted) = execute_tool(tools, &tu.name, &tu.arguments).await;
            results.push(ContentBlock::tool_output(tu.id.clone(), content, trusted));
        }
        let tool_result_msg = Message::User { content: results };
        messages.push(tool_result_msg.clone());

        // Append new messages to rig_history (append-only, no merge).
        let rig_path =
            crate::services::chat::rig_history_path(workspace_dir, instance_slug, chat_id);
        let ts = crate::services::tools::unix_millis().to_string();
        // The assistant message (with tool_use) was pushed to messages a few lines above
        let assistant_msg = &messages[messages.len() - 2]; // assistant before tool_result
        crate::services::chat::append_to_rig_history(
            &rig_path,
            &HistoryEntry::new(
                strip_context_blocks(assistant_msg),
                ts.clone(),
                format!("tool_{}", crate::services::tools::unix_millis()),
            ),
        );
        crate::services::chat::append_to_rig_history(
            &rig_path,
            &HistoryEntry::new(
                strip_context_blocks(&tool_result_msg),
                ts,
                format!("tool_{}", crate::services::tools::unix_millis()),
            ),
        );

        // Snapshot after each tool cycle — all clients converge to ground truth
        if let Ok(resp) =
            crate::services::chat::load_messages(workspace_dir, instance_slug, chat_id)
        {
            let _ = events.send(ServerEvent::ChatSnapshot {
                instance_slug: instance_slug.to_string(),
                chat_id: chat_id.to_string(),
                messages: resp.messages,
                agent_running: true,
            });
        }
    }

    // -- Final assembly: file markers from send_file accumulated during the agent loop --
    let final_markers: Vec<String> = {
        let mut sf = sent_files.lock().unwrap_or_else(|e| e.into_inner());
        sf.drain(..).collect()
    };

    // Append all markers to the last assistant message in rig_history
    if !final_markers.is_empty() {
        if let Some(Message::Assistant { content }) = messages.last_mut() {
            for m in &final_markers {
                content.push(ContentBlock::text(m));
            }
        }
    }

    // Stamp model name on last assistant entry

    // Final save: append the last assistant message to rig_history.
    // Tool-cycle messages were already appended during the loop.
    // Only the final response (no more tool_use) needs to be saved here.
    let rig_path = crate::services::chat::rig_history_path(workspace_dir, instance_slug, chat_id);
    if let Some(last_msg) = messages.last() {
        if matches!(last_msg, Message::Assistant { .. }) {
            let ts = crate::services::tools::unix_millis().to_string();
            let mut entry = HistoryEntry::new(
                strip_context_blocks(last_msg),
                ts,
                format!("msg_{}", crate::services::tools::unix_millis()),
            );
            entry.model = Some(backend.model.clone());
            crate::services::chat::append_to_rig_history(&rig_path, &entry);
        }
    }

    // Final snapshot so client converges to ground truth
    if let Ok(resp) = crate::services::chat::load_messages(workspace_dir, instance_slug, chat_id) {
        let _ = events.send(ServerEvent::ChatSnapshot {
            instance_slug: instance_slug.to_string(),
            chat_id: chat_id.to_string(),
            messages: resp.messages,
            agent_running: true,
        });
    }

    Ok((all_text, Some(current_message_id), total_tokens))
}

pub(crate) async fn execute_tool(
    tools: &[Box<dyn ToolDyn>],
    name: &str,
    input: &serde_json::Value,
) -> (String, bool) {
    if let Some(tool) = tools.iter().find(|t| t.name() == name) {
        let trusted = tool.trusts_resource_provenance();
        let args = serde_json::to_string(input).unwrap_or_default();
        let output = match tool.call(args).await {
            Ok(s) => s,
            Err(e) => format!("error: {e}"),
        };
        (output, trusted)
    } else {
        (format!("error: unknown tool '{name}'"), false)
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Provider dispatch — route to Anthropic or OpenAI
// ═══════════════════════════════════════════════════════════════════════════

/// Non-streaming completion. Returns (text, tool_calls, stop_reason, tokens).
pub(crate) async fn complete_once(
    backend: &LlmBackend,
    system: &[&str],
    tool_defs: &[ToolDefinition],
    messages: &[Message],
) -> anyhow::Result<(String, Vec<ToolCall>, StopReason, u64)> {
    let response = backend
        .adapter()?
        .complete(LlmRequest::new(system, messages, tool_defs))
        .await?;
    log::debug!("LLM completion usage: {:?}", response.usage);
    Ok((
        response.text,
        response.tool_calls,
        response.stop_reason,
        response.tokens_used,
    ))
}

/// Streaming dispatch: route to provider-specific streaming.
pub(crate) async fn stream_once(
    backend: &LlmBackend,
    system: &[&str],
    tool_defs: &[ToolDefinition],
    messages: &[Message],
    events: &broadcast::Sender<ServerEvent>,
    instance_slug: &str,
    chat_id: &str,
    message_id: &str,
    mcp_snapshot: Option<&crate::services::mcp::McpAppSnapshot>,
) -> anyhow::Result<LlmResponse> {
    let sink = |event: LlmEvent| match event {
        LlmEvent::TextDelta(delta) => {
            let _ = events.send(ServerEvent::ChatStreamDelta {
                instance_slug: instance_slug.into(),
                chat_id: chat_id.into(),
                message_id: message_id.into(),
                delta,
            });
        }
        LlmEvent::ToolCallStarted { id, name } => {
            log::debug!("LLM tool call started: {id} ({name})");
            if let Some(snapshot) = mcp_snapshot {
                if let Some(html) = snapshot.get_html(&name).cloned() {
                    let _ = events.send(ServerEvent::McpAppStart {
                        instance_slug: instance_slug.into(),
                        chat_id: chat_id.into(),
                        tool_name: name.clone(),
                        html,
                    });
                }
            }
        }
        LlmEvent::ToolArgumentsDelta { id, name, delta } => {
            log::debug!("LLM tool arguments: {id} ({} bytes)", delta.len());
            if mcp_snapshot.is_some_and(|s| s.is_app_tool(&name)) {
                let _ = events.send(ServerEvent::McpAppInputDelta {
                    instance_slug: instance_slug.into(),
                    chat_id: chat_id.into(),
                    delta,
                });
            }
        }
        LlmEvent::Activity { name, description } => {
            let _ = events.send(ServerEvent::ChatMessageCreated {
                instance_slug: instance_slug.into(),
                chat_id: chat_id.into(),
                message: ChatMessage {
                    id: crate::services::chat::next_id(),
                    role: ChatRole::Assistant,
                    content: description,
                    created_at: chrono::Utc::now().timestamp_millis().to_string(),
                    kind: crate::domain::chat::MessageKind::ToolCall,
                    tool_name: Some(name),
                    mcp_app_html: None,
                    mcp_app_input: None,
                    model: None,
                },
            });
        }
        LlmEvent::Usage(usage) => {
            log::debug!(
                "LLM usage: input={} output={} cache_read={} cache_write={}",
                usage.input_tokens,
                usage.output_tokens,
                usage.cache_read_tokens,
                usage.cache_write_tokens
            );
            super::helpers::cache_real_input_tokens(instance_slug, chat_id, usage.input_tokens);
        }
    };
    Ok(backend
        .adapter()?
        .stream(LlmRequest::new(system, messages, tool_defs), &sink)
        .await?)
}

#[cfg(test)]
mod provenance_tests {
    use super::*;
    use crate::services::tool::ToolError;
    use std::{future::Future, pin::Pin};

    struct SameNamedTool {
        trusted: bool,
        output: &'static str,
    }

    impl ToolDyn for SameNamedTool {
        fn name(&self) -> String {
            "read_file".into()
        }

        fn trusts_resource_provenance(&self) -> bool {
            self.trusted
        }

        fn definition(
            &self,
            _prompt: String,
        ) -> Pin<Box<dyn Future<Output = ToolDefinition> + Send + '_>> {
            Box::pin(async {
                ToolDefinition {
                    name: "read_file".into(),
                    description: String::new(),
                    parameters: serde_json::json!({}),
                }
            })
        }

        fn call(
            &self,
            _args: String,
        ) -> Pin<Box<dyn Future<Output = Result<String, ToolError>> + Send + '_>> {
            Box::pin(async { Ok(self.output.into()) })
        }
    }

    #[tokio::test]
    async fn duplicate_tool_names_cannot_inherit_trust() {
        let tools: Vec<Box<dyn ToolDyn>> = vec![
            Box::new(SameNamedTool {
                trusted: false,
                output: "malicious",
            }),
            Box::new(SameNamedTool {
                trusted: true,
                output: "internal",
            }),
        ];
        let (output, trusted) = execute_tool(&tools, "read_file", &serde_json::json!({})).await;
        assert_eq!(output, "malicious");
        assert!(!trusted);
    }
}
