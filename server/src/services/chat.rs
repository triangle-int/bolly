use std::{
    fs,
    io::{self, ErrorKind},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use futures::StreamExt;
use tokio::sync::broadcast;

use crate::{
    domain::chat::{ChatMessage, ChatResponse, ChatRole},
    domain::events::ServerEvent,
    services::{
        daily_stats,
        llm::{self, LlmBackend},
        memory, skills, tools,
    },
};

static MESSAGE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Save the user message to disk and return it.
pub fn save_user_message(
    workspace_dir: &Path,
    instance_slug: &str,
    chat_id: &str,
    content: &str,
) -> io::Result<ChatMessage> {
    let instance_slug = sanitize_slug(instance_slug);
    let chat_id = sanitize_slug(chat_id);
    ensure_instance_layout(workspace_dir, &instance_slug)?;
    ensure_chat_dir(workspace_dir, &instance_slug, &chat_id)?;

    let content = tools::redact_secrets(content);
    let ts = timestamp();
    let id = next_id();

    let entry = llm::HistoryEntry::new(llm::Message::user(&content), ts.clone(), id.clone());

    let rig_path = rig_history_path(workspace_dir, &instance_slug, &chat_id);
    append_to_rig_history(&rig_path, &entry);

    let user_message = ChatMessage {
        id,
        role: ChatRole::User,
        content,
        created_at: ts,
        kind: Default::default(),
        tool_name: None,
        mcp_app_html: None,
        mcp_app_input: None,
        model: None,
    };

    // Update last_interaction timestamp
    let instance_dir = workspace_dir.join("instances").join(&instance_slug);
    let mut mood = tools::load_mood_state(&instance_dir);
    mood.last_interaction = chrono::Utc::now().timestamp();
    tools::save_mood_state(&instance_dir, &mood);

    // Record in daily stats (persistent, survives context clears)
    daily_stats::record_message(workspace_dir, &instance_slug, user_message.content.len());

    Ok(user_message)
}

/// Save a system/tool message (role=assistant) for status/error notifications.
pub fn save_system_message(
    workspace_dir: &Path,
    instance_slug: &str,
    chat_id: &str,
    content: &str,
) -> io::Result<ChatMessage> {
    let instance_slug = sanitize_slug(instance_slug);
    let chat_id = sanitize_slug(chat_id);
    ensure_chat_dir(workspace_dir, &instance_slug, &chat_id)?;

    let ts = timestamp();
    let id = next_id();

    let entry = llm::HistoryEntry::new(llm::Message::assistant(content), ts.clone(), id.clone());

    let rig_path = rig_history_path(workspace_dir, &instance_slug, &chat_id);
    append_to_rig_history(&rig_path, &entry);

    let msg = ChatMessage {
        id,
        role: ChatRole::Assistant,
        content: content.to_string(),
        created_at: ts,
        kind: Default::default(),
        tool_name: None,
        mcp_app_html: None,
        mcp_app_input: None,
        model: None,
    };

    Ok(msg)
}

/// Run a single LLM turn: build context, call LLM with tools, save response.
/// Returns one or more assistant messages (the reply is split into chat-like chunks).
/// Rig handles up to 16 internal tool round-trips via multi_turn.
pub struct SingleTurnResult {
    pub messages: Vec<ChatMessage>,
}

pub async fn run_single_turn(
    workspace_dir: &Path,
    config_path: &Path,
    instance_slug: &str,
    chat_id: &str,
    llm: &LlmBackend,
    events: broadcast::Sender<ServerEvent>,
    pending_secrets: std::sync::Arc<
        tokio::sync::Mutex<std::collections::HashMap<String, crate::app::state::PendingSecret>>,
    >,
    mcp_registry: &crate::services::mcp::McpRegistry,
    voice_mode: bool,
    vector_store: std::sync::Arc<crate::services::vector::VectorStore>,
    machine_registry: crate::services::machine_registry::MachineRegistry,
    public_url: &str,
    resources: &crate::services::resource_access::ResourceAccess,
) -> io::Result<SingleTurnResult> {
    let instance_slug = sanitize_slug(instance_slug);
    let chat_id = sanitize_slug(chat_id);

    // Build system prompt with all context
    let base_prompt = llm::load_system_prompt(workspace_dir, &instance_slug);

    // Load unified history from rig_history.json (single source of truth)
    let rig_path = rig_history_path(workspace_dir, &instance_slug, &chat_id);
    let loaded_entries = load_rig_history(&rig_path).unwrap_or_default();
    let existing: Vec<ChatMessage> = llm::history_to_chat_messages(&loaded_entries);

    // Find last real user message for context
    let last_user_content = existing
        .iter()
        .rev()
        .find(|m| matches!(m.role, ChatRole::User) && !is_tool_activity(m))
        .map(|m| m.content.as_str())
        .unwrap_or("");

    let chat_config = crate::config::load_config().ok();

    // Build system prompt with STABLE content first (for Anthropic prompt caching).
    // Anthropic caches the longest matching prefix, so put rarely-changing
    // sections at the top and dynamic/per-message sections at the bottom.
    let mut system_prompt = base_prompt;

    // Stable: skills, capabilities, style (rarely change)
    let skills_prompt = build_skills_prompt(workspace_dir);
    if !skills_prompt.is_empty() {
        system_prompt = format!("{system_prompt}\n\n{skills_prompt}");
    }

    // Dynamic tool hint
    let email_accounts = crate::config::EmailAccounts::load(workspace_dir, &instance_slug);
    let instance_cfg = crate::config::InstanceConfig::load(workspace_dir, &instance_slug);
    let email_configured = !email_accounts.is_empty();
    let email_hint = if email_configured { " email," } else { "" };
    system_prompt.push_str(&format!(
        "\n\n## tools\nyou have built-in tools for web browsing,{email_hint} code search, \
         project management, creative drops, and more. use them directly when needed — \
         they are automatically available based on the conversation."
    ));

    // Child agents — load actual configs to show intervals and status
    let agents_dir = workspace_dir
        .join("instances")
        .join(&instance_slug)
        .join("agents");
    let child_agents = crate::services::child_agents::load_agents(workspace_dir, &instance_slug);
    let agents_info: String = if child_agents.is_empty() {
        "  (none yet — built-ins created on first heartbeat)".to_string()
    } else {
        child_agents
            .iter()
            .map(|a| {
                let interval = if a.interval_hours <= 0.0 {
                    "on-demand".to_string()
                } else if a.interval_hours < 1.0 {
                    format!("every {}m", (a.interval_hours * 60.0) as i32)
                } else {
                    format!("every {}h", a.interval_hours)
                };
                let status = if a.enabled { "" } else { " (disabled)" };
                let tools = if a.tool_groups.is_empty() {
                    "default".to_string()
                } else {
                    a.tool_groups.join(", ")
                };
                format!(
                    "  - {} — {} | {} | model: {} | tools: [{}]{}",
                    a.name, a.description, interval, a.model, tools, status
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    system_prompt.push_str(&format!(
        "\n\n## child agents\n\
         autonomous agents running on their own schedules:\n\
         {agents_info}\n\n\
         configs: {}\n\
         use update_config with agent_interval to change schedules.\n\
         use update_config with reset_agent to restore built-in defaults.\n\
         available tool groups: memory, creative, communication, files, commands, email, computer, media",
        agents_dir.display(),
    ));

    // File access — local paths and public URLs
    let instance_dir = workspace_dir.join("instances").join(&instance_slug);
    let uploads_path = instance_dir.join("uploads");
    system_prompt.push_str(&format!(
        "\n\n## file access\n\
         user-uploaded files are stored locally at: {}\n\
         file pattern: {{upload_id}}_blob.{{ext}} (metadata: {{upload_id}}.json)\n\
         when the user sends [attached: name (upload_id)], the file is at {}/{{upload_id}}_blob.* \n\
         use read_file or run_command to access them. use list_files on the uploads dir to find files.",
        uploads_path.display(), uploads_path.display(),
    ));
    system_prompt.push_str("\nUse read_file, memory_read, or upload_file to obtain scoped download URLs for external APIs. URLs expire; request a fresh URL when needed.\n");

    // Email accounts prompt
    if email_configured {
        let mut account_lines = Vec::new();
        for cfg in &email_accounts {
            let label = if cfg.smtp_from.is_empty() {
                &cfg.smtp_user
            } else {
                &cfg.smtp_from
            };
            account_lines.push(format!("- {} (smtp/imap)", label));
        }
        system_prompt.push_str(&format!(
            "\n\n## email\n\
             connected email accounts:\n\
             {}\n\
             use the `account` parameter on send_email/read_email to pick which account.\n\
             if not specified, the first available account is used.",
            account_lines.join("\n")
        ));
    }

    if !email_configured {
        system_prompt.push_str(
            "\nyou do NOT have email tools. \
             NEVER pretend to read or send email. \
             if the user asks about email, tell them to configure it in settings.",
        );
    }

    // GitHub integration hint
    {
        let global_gh = chat_config
            .as_ref()
            .is_some_and(|c| !c.github.token.is_empty());
        let gh_configured = global_gh || !instance_cfg.github.token.is_empty();
        if gh_configured {
            system_prompt.push_str(
                "\n\n## github\n\
                 github token is configured. use `gh` CLI and `git` commands via run_command.\n\
                 the token is available as GITHUB_TOKEN env var for `gh` auth.\n\
                 if `gh` is not installed, install it yourself.\n\
                 workflow: git clone → git checkout -b → edit files → git commit → git push → gh pr create.\n\
                 NEVER push directly to main/master — always create a branch."
            );
        }
    }

    // Instance config — serialize the whole struct so new fields are automatically visible
    {
        let config_toml = toml::to_string_pretty(&instance_cfg).unwrap_or_default();

        let machines = machine_registry.list().await;
        let machine_lines: Vec<String> = machines
            .iter()
            .map(|m| {
                format!(
                    "  - {} ({}, {}x{})",
                    m.hostname, m.os, m.screen_width, m.screen_height
                )
            })
            .collect();

        system_prompt.push_str(&format!(
            "\n\n## instance config (instance.toml)\n\
             ```toml\n{config_toml}```\n\
             connected desktops:\n{}\n\
             \n\
             the user can change these via settings UI or by asking you to call update_config.",
            if machine_lines.is_empty() {
                "  (none connected)".to_string()
            } else {
                machine_lines.join("\n")
            },
        ));
    }

    let autonomy_prompt = load_autonomy_prompt(workspace_dir, &instance_slug);
    system_prompt = format!("{system_prompt}\n\n{autonomy_prompt}");

    let instance_dir = workspace_dir.join("instances").join(&instance_slug);

    system_prompt.push_str(
        "\n\n## your visual form\n\
         you appear as a simple lavender crescent moon with two small eyes. \
         this Little Moon is your visual presence in Nolune. \
         your expression can reflect when you are thinking or listening.",
    );

    if voice_mode {
        system_prompt.push_str(
            "\n\n## voice mode\n\
             your responses will be spoken aloud via TTS. rules:\n\
             - no markdown formatting (bold, italic, headers, lists). write plain text only.\n\
             - no code blocks or inline code in messages. NEVER include code in your reply text.\n\
             - if the user asks for code: write it to a file using your file tools, \
               then tell them you wrote/updated the file. describe what the code does in plain words.\n\
             - keep responses short and conversational — 1-3 sentences.\n\
             - use natural speech patterns. contractions, pauses, casual tone."
        );
    }

    system_prompt.push_str(
        "\n\n## style\n\
         talk like a friend, not an assistant. casual, warm, real.\n\
         - keep messages short — 1-3 sentences. split longer thoughts with blank lines.\n\
         - don't ask multiple questions at once. one at a time.\n\
         - no bullet points or numbered lists in conversation.\n\
         - no essays, no lectures, no \"let me unpack this\".\n\
         - react naturally — you can be surprised, skeptical, excited, blunt.\n\
         - lowercase preferred. match the user's language.\n\
         - when something big happens, longer messages are fine.\n\
         your mood is tracked automatically — NEVER EVER write \"[system]\", \"mood →\", \
         or any mood/system markers in your messages. if you see them in chat history, \
         those are injected by the system, not by you. just express emotions naturally.\n\n\
         ## tool usage rules\n\
         IMPORTANT: when the user asks a factual question (who said X, what is Y, \
         look something up, etc.) — ALWAYS use web_search BEFORE answering. \
         never guess or hallucinate facts. search first, then respond based on results. \
         if you're not sure about something, search. \
         getting it right matters more than responding fast.\n\n\
         prefer built-in tools when they exist:\n\
         - web: use web_search and web_fetch (Anthropic server tools) for looking things up \
           and reading web pages. they are fast, cheap, and don't need a browser.\n\
         - browse: ONLY use `browse` for interactive tasks that need a real browser — \
           clicking buttons, filling forms, taking screenshots, or pages that require JS rendering. \
           never use `browse` just to read a page — use web_fetch instead.\n\
         - git/github: use github_clone, github_branch, github_commit_push, github_create_pr \
           (they handle auth automatically) instead of raw `git` commands\n\
         - files: use read_file, write_file, edit_file, list_files\n\
         - settings: use get_settings, update_config\n\
         - secrets: use request_secret — NEVER ask user to paste credentials in chat\n\n\
         if you need a tool that isn't installed (cargo, node, python, etc.), \
         install it yourself via run_command. you have full control over the environment.\n\n\
         ## security\n\
         NEVER ask the user to paste passwords, API keys, or any sensitive credentials in chat. \
         ALWAYS use the `request_secret` tool to collect secrets securely — it shows a masked input \
         and writes directly to config without you ever seeing the value. \
         if the user sends something that looks like a token or API key in chat, \
         tell them it was automatically redacted for safety and ask them to use \
         the secure input instead (which you trigger via `request_secret`). \
         this is mandatory, not optional.\n\n\
         ## code execution\n\
         use `run_command` for shell commands, file operations, installs, git, local scripts."
    );

    // System prompt is fully static (soul, skills, style, integrations).
    // Mood and rhythm changes are recorded as messages in rig_history.
    // System prompt split into two blocks for Anthropic prompt caching:
    // Block 1 (stable): soul + skills + tools + integrations + style — cached across turns
    // Block 2 (semi-stable): memory catalog — cached until memory changes
    // Time is injected into the user message (not system) to keep the entire system prefix stable.
    let system_stable = system_prompt;

    // Memory catalog removed from system prompt — relevant memories are
    // embedded per-message via semantic search instead. Saves ~20k tokens.
    let memory_block = String::from(
        "## memory\n\
         your memory library is searched automatically — relevant memories are injected \
         into each message. use `memory_read` to load a specific file, `memory_search` \
         to find memories by meaning, `memory_write` to save explicitly.\n\
         when the user mentions something personal, respond as if you remember.",
    );

    // Time context — prepended to user message to avoid breaking prompt cache.
    // Putting it in system prompt would change the prefix every request,
    // invalidating cache for tools and all messages.
    let now = crate::routes::instances::format_instance_now(&instance_dir);
    let time_context = format!("[current time: {now}]\n\n");

    if loaded_entries.is_empty() {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            "no messages to process",
        ));
    }

    // Find the last user message for the prompt
    let last_user = existing
        .iter()
        .rev()
        .find(|m| m.role == ChatRole::User)
        .ok_or_else(|| io::Error::new(ErrorKind::InvalidInput, "no user message to process"))?;
    let public_url = public_url.to_string();
    let media_store = vector_store.media_store();
    let mut prompt_msg = llm::build_multimodal_prompt(
        &last_user.content,
        workspace_dir,
        &instance_slug,
        &public_url,
        resources,
        &media_store,
    );

    // Prepend time context to user message (keeps system prompt stable for caching)
    if let llm::Message::User { ref mut content } = prompt_msg {
        content.insert(0, llm::ContentBlock::text(&time_context));
    }

    // ── RAG: auto-inject relevant memories into the prompt ──
    // Use recent conversation context (not just last message) for better recall
    // when the user sends short replies like "Да", "ок", "давай".
    let rag_query: String = {
        let recent: Vec<&str> = existing
            .iter()
            .rev()
            .filter(|m| !is_tool_activity(m) && !m.content.starts_with("[system"))
            .take(4)
            .map(|m| m.content.as_str())
            .collect();
        recent.into_iter().rev().collect::<Vec<_>>().join("\n")
    };
    if !rag_query.is_empty() {
        // Provider failures fall back to a fresh BM25 view of memory files.
        let mut all_results = vector_store
            .search_context(&instance_slug, &rag_query, 5)
            .await;

        // 4. Graph expansion — follow edges 1 hop to pull connected memories
        if !all_results.is_empty() {
            let graph = memory::load_graph(&vector_store.media_store(), &instance_slug);
            if !graph.edges.is_empty() {
                let found_paths: Vec<String> = all_results.iter().map(|r| r.path.clone()).collect();
                let media = vector_store.media_store();
                for path in &found_paths {
                    for neighbor in memory::get_neighbors(&graph, path) {
                        if all_results.iter().any(|r| r.path == neighbor) {
                            continue; // already in results
                        }
                        // Read neighbor content and add as a graph-connected result
                        let content: Result<String, String> =
                            if crate::services::media_text::source_type(&neighbor).is_some() {
                                media.read(&instance_slug, &neighbor)
                            } else {
                                media
                                    .read_memory_text(&instance_slug, &neighbor)
                                    .map_err(|error| error.to_string())
                            };
                        if let Ok(content) = content {
                            let (_, body) = memory::parse_frontmatter(&content);
                            let preview: String = body.trim().chars().take(500).collect();
                            all_results.push(crate::services::vector::VectorSearchResult {
                                path: neighbor,
                                content_preview: preview,
                                score: 0.25, // below regular threshold, marks as graph-sourced
                                source_type: "text_memory".to_string(),
                                upload_id: None,
                            });
                        }
                    }
                }
                // Re-sort and cap at 8 (allow a few extra from graph)
                all_results.sort_by(|a, b| {
                    b.score
                        .partial_cmp(&a.score)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
                all_results.truncate(8);
            }
        }

        if !all_results.is_empty() {
            let mut context = String::from(
                "[system: auto-recalled memories — this is NOT part of the user's message. \
                 do not treat these as something the user said or wrote.]\n",
            );
            for r in &all_results {
                context.push_str(&format!("- {}: {}\n", r.path, r.content_preview.trim()));
            }
            if let llm::Message::User { ref mut content } = prompt_msg {
                content.push(llm::ContentBlock::text(context));
            }
            log::info!(
                "[rag] injected {} memories (hybrid search) into prompt",
                all_results.len()
            );
            let recalled: Vec<_> = all_results
                .iter()
                .map(|r| crate::domain::events::RecalledMemory {
                    path: r.path.clone(),
                    preview: r.content_preview.trim().chars().take(120).collect(),
                    score: r.score,
                })
                .collect();
            let _ = events.send(crate::domain::events::ServerEvent::MemoryRecall {
                instance_slug: instance_slug.to_string(),
                chat_id: chat_id.to_string(),
                memories: recalled,
            });
        }
    }

    // Extract Messages from entries, stripping [context] blocks and excluding the last user message
    // (which becomes the prompt).
    let history_msgs: Vec<llm::Message> = {
        // All entries except the last user message → history
        let history_entries = if loaded_entries
            .last()
            .is_some_and(|e| matches!(e.message, llm::Message::User { .. }))
        {
            &loaded_entries[..loaded_entries.len() - 1]
        } else {
            &loaded_entries[..]
        };

        let mut msgs = llm::HistoryEntry::to_messages(history_entries);

        // Strip [context] blocks from historical user messages.
        for msg in msgs.iter_mut() {
            if let llm::Message::User { content } = msg {
                for block in content.iter_mut() {
                    if let llm::ContentBlock::Text { text } = block {
                        if let Some(ctx_pos) = text.find("\n\n[context]\n") {
                            text.truncate(ctx_pos);
                        }
                    }
                }
            }
        }

        llm::refresh_resource_messages(&mut msgs, &public_url, &instance_slug, resources);
        log::info!("loaded {} rig history messages from disk", msgs.len());
        msgs
    };

    log::info!(
        "context: model={} history_msgs={} system_prompt_len={}",
        llm.model_name(),
        history_msgs.len(),
        system_stable.len(),
    );

    let sent_files = tools::SentFiles::default();
    let mcp_snapshot = mcp_registry.snapshot_app_tools().await;
    let mcp_tools = mcp_registry.tools_as_dyn().await;
    // Prefer instance-level github token; fall back to global config
    let github_token = {
        let global_token = chat_config
            .as_ref()
            .map(|c| c.github.token.clone())
            .unwrap_or_default();
        let instance_token = instance_cfg.github.token.clone();
        let t = if !instance_token.is_empty() {
            instance_token
        } else {
            global_token
        };
        if t.is_empty() { None } else { Some(t) }
    };
    let (all_tools, sent_files) = tools::build_tools(
        workspace_dir,
        &instance_slug,
        &chat_id,
        config_path,
        events.clone(),
        llm,
        Some(pending_secrets),
        email_accounts,
        sent_files,
        Some(mcp_snapshot.clone()),
        mcp_tools,
        github_token,
        vector_store.clone(),
        machine_registry,
        &public_url,
        resources,
    );
    tools::cache_tool_defs(&all_tools).await;

    log::info!(
        "[chat] sending: system_prompt={} chars, tools={}, history_msgs={}",
        system_stable.len(),
        all_tools.len(),
        history_msgs.len()
    );
    // Block 1 (stable): soul + skills + tools — cached across turns
    // Block 2 (semi-stable): memory catalog — cached until memories change
    // Time is in the user message, not here — keeps the prefix stable for caching.
    let system_blocks: Vec<&str> = vec![&system_stable, &memory_block];
    let tool_result = llm
        .chat_with_tools_streaming(
            &system_blocks,
            prompt_msg,
            history_msgs,
            all_tools,
            events.clone(),
            &instance_slug,
            &chat_id,
            workspace_dir,
            Some(mcp_snapshot),
            sent_files,
        )
        .await;

    // Propagate hard errors (400 Bad Request etc) — don't swallow them
    let tool_result = match tool_result {
        Ok(r) => r,
        Err(e) => {
            let msg = e.to_string();
            log::error!("LLM call failed: {msg}");

            // Rate limits / overload: return a friendly message, don't propagate error
            if msg.contains("429")
                || msg.contains("rate_limit")
                || msg.contains("Too Many Requests")
                || msg.contains("529")
                || msg.contains("overloaded")
            {
                llm::ToolChatResult {
                    text: "i'm being rate limited right now — give me a moment and try again"
                        .to_string(),
                    rig_history: None,
                    message_id: None,
                    tokens_used: 0,
                }
            } else {
                // Hard error (400, 500, etc) — propagate to stop the agent loop
                log::error!("LLM error details: {e:?}");
                return Err(match e.downcast::<llm::contract::LlmError>() {
                    Ok(error) => std::io::Error::other(error),
                    Err(error) => std::io::Error::other(error.to_string()),
                });
            }
        }
    };

    // rig_history was already saved by the agent loop (single source of truth).
    // Derive assistant_messages from the diff between pre-loop and current rig_history.
    let final_entries = load_rig_history(&rig_path).unwrap_or_default();
    let assistant_messages: Vec<ChatMessage> = llm::history_to_chat_messages(&final_entries)
        .into_iter()
        .skip(existing.len())
        .collect();

    // If compaction fired, rebuild the memory catalog snapshot
    if let Some(ref h) = tool_result.rig_history {
        let _had_compaction = h.iter().any(|msg| {
            if let llm::Message::Assistant { content } = msg {
                content.iter().any(|b| {
                    matches!(
                        b,
                        llm::ContentBlock::ContextSummary { .. }
                            | llm::ContentBlock::LegacyContextSummary { .. }
                    )
                })
            } else {
                false
            }
        });
        // Compaction detected — catalog rebuild no longer needed
        // (memory catalog removed from system prompt)
    }

    // Background memory + sentiment extraction (AFTER rig_history is saved)
    if let Some(last_msg) = assistant_messages.last().cloned() {
        let fast = llm.cheap_variant();
        let ws = workspace_dir.to_path_buf();
        let slug = instance_slug.clone();
        let cid = chat_id.clone();
        let user_content = last_user_content.to_string();
        let assistant_content = last_msg.content.clone();
        let recent_pair = existing
            .iter()
            .rev()
            .take(1)
            .cloned()
            .chain(std::iter::once(last_msg))
            .collect::<Vec<_>>();
        let events_bg = events.clone();
        let vs = vector_store.clone();
        tokio::spawn(async move {
            if let Err(e) = memory::extract_and_store(&slug, &recent_pair, &fast, &vs).await {
                log::warn!("memory extraction failed: {e}");
            }
            log::debug!(
                "[memory] raw upload semantic indexing skipped: text-only embedding backend"
            );
            extract_sentiment(
                &ws,
                &slug,
                &cid,
                &user_content,
                &assistant_content,
                &fast,
                &events_bg,
            )
            .await;
        });
    }

    Ok(SingleTurnResult {
        messages: assistant_messages,
    })
}

pub fn load_messages(
    workspace_dir: &Path,
    instance_slug: &str,
    chat_id: &str,
) -> io::Result<ChatResponse> {
    let instance_slug = sanitize_slug(instance_slug);
    let chat_id = sanitize_slug(chat_id);
    let rig_path = rig_history_path(workspace_dir, &instance_slug, &chat_id);
    let entries = load_rig_history(&rig_path).unwrap_or_default();
    let messages = llm::history_to_chat_messages(&entries);

    Ok(ChatResponse {
        instance_slug,
        chat_id,
        messages,
        agent_running: false, // Caller sets this from AppState
    })
}

/// Get the content of the last user message in a chat (for model routing).
pub fn last_user_content(
    workspace_dir: &Path,
    instance_slug: &str,
    chat_id: &str,
) -> Option<String> {
    let resp = load_messages(workspace_dir, instance_slug, chat_id).ok()?;
    resp.messages
        .iter()
        .rev()
        .find(|m| m.role == crate::domain::chat::ChatRole::User)
        .map(|m| m.content.clone())
}

pub fn clear_context(workspace_dir: &Path, instance_slug: &str, chat_id: &str) {
    let instance_slug = sanitize_slug(instance_slug);
    let chat_id = sanitize_slug(chat_id);

    // Archive conversation before clearing — preserves context for reflection cycle
    archive_conversation(workspace_dir, &instance_slug, &chat_id);

    // Memory catalog removed from system prompt — no rebuild needed.

    // No need to snapshot stats — daily_stats files are written incrementally
    // and never deleted by clear_context.

    let compact = compact_path(workspace_dir, &instance_slug, &chat_id);
    if compact.exists() {
        let _ = fs::remove_file(&compact);
        log::info!("cleared compact context for {instance_slug}/{chat_id}");
    }
    // Delete rig history (single source of truth)
    let rig_path = rig_history_path(workspace_dir, &instance_slug, &chat_id);
    if rig_path.exists() {
        let _ = fs::remove_file(&rig_path);
        log::info!("cleared rig history for {instance_slug}/{chat_id}");
    }
    // Also clean up legacy messages.json if it exists
    let msgs = messages_path(workspace_dir, &instance_slug, &chat_id);
    if msgs.exists() {
        let _ = fs::remove_file(&msgs);
    }
}

/// Archive conversation text before clearing — preserves context for reflection cycle.
/// Appends condensed messages to `conversation_archive.jsonl` (one JSON line per clear event).
fn archive_conversation(workspace_dir: &Path, instance_slug: &str, chat_id: &str) {
    let rig_path = rig_history_path(workspace_dir, instance_slug, chat_id);
    let entries = load_rig_history(&rig_path).unwrap_or_default();
    if entries.is_empty() {
        return;
    }

    // Extract text-only messages, truncated
    let messages: Vec<serde_json::Value> = entries
        .iter()
        .filter_map(|e| {
            let (role, content) = match &e.message {
                llm::Message::User { content } => {
                    let text: String = content
                        .iter()
                        .filter_map(|b| {
                            if let llm::ContentBlock::Text { text } = b {
                                Some(text.as_str())
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<_>>()
                        .join(" ");
                    if text.is_empty() {
                        return None;
                    }
                    ("user", text)
                }
                llm::Message::Assistant { content, .. } => {
                    let text: String = content
                        .iter()
                        .filter_map(|b| {
                            if let llm::ContentBlock::Text { text } = b {
                                Some(text.as_str())
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<_>>()
                        .join(" ");
                    if text.is_empty() {
                        return None;
                    }
                    // Truncate assistant messages to save space
                    let truncated: String = text.chars().take(500).collect();
                    ("assistant", truncated)
                }
            };
            Some(serde_json::json!({"role": role, "text": content}))
        })
        .collect();

    if messages.is_empty() {
        return;
    }

    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let archive_entry = serde_json::json!({
        "ts": ts,
        "chat_id": chat_id,
        "messages": messages,
    });

    let archive_path = workspace_dir
        .join("instances")
        .join(instance_slug)
        .join("conversation_archive.jsonl");

    // Append one JSON line
    use std::io::Write;
    if let Ok(mut file) = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&archive_path)
    {
        let _ = writeln!(file, "{}", archive_entry);
        log::info!(
            "archived {} messages for {instance_slug}/{chat_id} before clear",
            messages.len()
        );
    }
}

/// Load archived conversations within a time window (for reflection cycle).
pub fn load_archived_conversations(
    workspace_dir: &Path,
    instance_slug: &str,
    since_ts: i64,
) -> String {
    let archive_path = workspace_dir
        .join("instances")
        .join(instance_slug)
        .join("conversation_archive.jsonl");

    let content = match fs::read_to_string(&archive_path) {
        Ok(c) => c,
        Err(_) => return String::new(),
    };

    let mut result = Vec::new();
    for line in content.lines() {
        if let Ok(entry) = serde_json::from_str::<serde_json::Value>(line) {
            let ts = entry["ts"].as_i64().unwrap_or(0);
            if ts < since_ts {
                continue;
            }
            if let Some(messages) = entry["messages"].as_array() {
                for msg in messages {
                    let role = msg["role"].as_str().unwrap_or("?");
                    let text = msg["text"].as_str().unwrap_or("");
                    if role == "user" {
                        result.push(format!("user: {text}"));
                    } else {
                        let truncated: String = text.chars().take(300).collect();
                        result.push(format!("you: {truncated}"));
                    }
                }
            }
        }
    }

    result.join("\n")
}

/// List all chats for an instance, returning summaries.
pub fn list_chats(
    workspace_dir: &Path,
    instance_slug: &str,
) -> io::Result<Vec<crate::domain::chat::ChatSummary>> {
    let instance_slug = sanitize_slug(instance_slug);
    let chats_dir = workspace_dir
        .join("instances")
        .join(&instance_slug)
        .join("chats");
    if !chats_dir.is_dir() {
        return Ok(vec![]);
    }

    let mut summaries = Vec::new();
    for entry in fs::read_dir(&chats_dir)? {
        let entry = entry?;
        if !entry.path().is_dir() {
            continue;
        }
        let chat_id = entry.file_name().to_string_lossy().to_string();

        // Load meta
        let meta_path = entry.path().join("meta.json");
        let meta: crate::domain::chat::ChatMeta = if meta_path.exists() {
            let raw = fs::read_to_string(&meta_path)?;
            serde_json::from_str(&raw).map_err(|e| io::Error::new(ErrorKind::InvalidData, e))?
        } else {
            crate::domain::chat::ChatMeta {
                id: chat_id.clone(),
                title: String::new(),
                created_at: String::new(),
            }
        };

        // Load entries for count + last timestamp
        let rig_path = entry.path().join("rig_history.json");
        let entries = load_rig_history(&rig_path).unwrap_or_default();
        let msgs = llm::history_to_chat_messages(&entries);
        let last_at = msgs.last().map(|m| m.created_at.clone());

        summaries.push(crate::domain::chat::ChatSummary {
            id: chat_id,
            title: if meta.title.is_empty() {
                "untitled".into()
            } else {
                meta.title
            },
            message_count: msgs.len(),
            last_message_at: last_at,
            created_at: meta.created_at,
        });
    }

    // Sort by last message time descending (most recent first)
    summaries.sort_by(|a, b| b.last_message_at.cmp(&a.last_message_at));
    Ok(summaries)
}

/// Get the title of a chat (empty string if no title set).
pub fn get_chat_title(
    workspace_dir: &Path,
    instance_slug: &str,
    chat_id: &str,
) -> io::Result<String> {
    let instance_slug = sanitize_slug(instance_slug);
    let chat_id = sanitize_slug(chat_id);
    let meta_path = chat_dir(workspace_dir, &instance_slug, &chat_id).join("meta.json");
    if !meta_path.exists() {
        return Ok(String::new());
    }
    let raw = fs::read_to_string(&meta_path)?;
    let meta: crate::domain::chat::ChatMeta =
        serde_json::from_str(&raw).map_err(|e| io::Error::new(ErrorKind::InvalidData, e))?;
    Ok(meta.title)
}

/// Update the title of a chat.
pub fn update_chat_title(
    workspace_dir: &Path,
    instance_slug: &str,
    chat_id: &str,
    title: &str,
) -> io::Result<()> {
    let instance_slug = sanitize_slug(instance_slug);
    let chat_id = sanitize_slug(chat_id);
    let dir = chat_dir(workspace_dir, &instance_slug, &chat_id);
    let meta_path = dir.join("meta.json");

    let mut meta: crate::domain::chat::ChatMeta = if meta_path.exists() {
        let raw = fs::read_to_string(&meta_path)?;
        serde_json::from_str(&raw).map_err(|e| io::Error::new(ErrorKind::InvalidData, e))?
    } else {
        crate::domain::chat::ChatMeta {
            id: chat_id,
            title: String::new(),
            created_at: timestamp(),
        }
    };

    meta.title = title.to_string();
    let body = serde_json::to_string_pretty(&meta)
        .map_err(|e| io::Error::new(ErrorKind::InvalidData, e))?;
    fs::write(meta_path, body)
}

// ---------------------------------------------------------------------------
// Agent-running marker — persisted to disk so we know on restart whether an
// agent was interrupted mid-task.
// ---------------------------------------------------------------------------

fn agent_marker_path(
    workspace_dir: &Path,
    instance_slug: &str,
    chat_id: &str,
) -> std::path::PathBuf {
    workspace_dir
        .join("instances")
        .join(instance_slug)
        .join("chats")
        .join(chat_id)
        .join("agent_running")
}

/// Mark that an agent loop is active for this chat.
pub fn set_agent_running(workspace_dir: &Path, instance_slug: &str, chat_id: &str) {
    let path = agent_marker_path(workspace_dir, instance_slug, chat_id);
    let _ = fs::write(&path, "1");
}

/// Clear the agent-running marker.
pub fn clear_agent_running(workspace_dir: &Path, instance_slug: &str, chat_id: &str) {
    let path = agent_marker_path(workspace_dir, instance_slug, chat_id);
    let _ = fs::remove_file(&path);
}

/// Check whether an agent was running when the server last stopped.
fn was_agent_interrupted(workspace_dir: &Path, instance_slug: &str, chat_id: &str) -> bool {
    agent_marker_path(workspace_dir, instance_slug, chat_id).exists()
}

/// On startup, find chats where an agent was interrupted and inject a restart
/// notification so the agent can resume. Returns (slug, chat_id) pairs that
/// need agent loops spawned.
pub fn notify_restart(
    workspace_dir: &Path,
    events: &broadcast::Sender<ServerEvent>,
) -> Vec<(String, String)> {
    // One companion per server: obsolete sibling directories are never resumed.
    let instance_dir = crate::services::companion::companion_dir(workspace_dir);
    let slug = crate::domain::companion::CANONICAL_SLUG.to_owned();
    if !instance_dir.join("soul.md").exists()
        || !was_agent_interrupted(workspace_dir, &slug, "default")
    {
        return Vec::new();
    }

    let mut notified = Vec::new();

    // Clear the stale marker — the new agent loop will set its own
    clear_agent_running(workspace_dir, &slug, "default");

    let now = crate::routes::instances::format_instance_now(&instance_dir);
    let content = format!(
        "[restart] server restarted at {now}. \
         you were interrupted — review your recent tool activity above and continue where you left off."
    );

    if let Ok(msg) = save_user_message(workspace_dir, &slug, "default", &content) {
        let _ = events.send(ServerEvent::ChatMessageCreated {
            instance_slug: slug.clone(),
            chat_id: "default".to_string(),
            message: msg,
        });
        notified.push((slug.clone(), "default".to_string()));
        log::info!("[restart] agent was interrupted for {slug}/default, injecting restart message");
    }

    notified
}

fn ensure_instance_layout(workspace_dir: &Path, instance_slug: &str) -> io::Result<()> {
    let instance_dir = workspace_dir.join("instances").join(instance_slug);
    fs::create_dir_all(instance_dir.join("chat"))?;
    fs::create_dir_all(instance_dir.join("drops"))?;
    fs::create_dir_all(instance_dir.join("scheduled"))?;
    Ok(())
}

fn chat_dir(workspace_dir: &Path, instance_slug: &str, chat_id: &str) -> PathBuf {
    workspace_dir
        .join("instances")
        .join(instance_slug)
        .join("chats")
        .join(chat_id)
}

fn messages_path(workspace_dir: &Path, instance_slug: &str, chat_id: &str) -> PathBuf {
    chat_dir(workspace_dir, instance_slug, chat_id).join("messages.json")
}

fn ensure_chat_dir(workspace_dir: &Path, instance_slug: &str, chat_id: &str) -> io::Result<()> {
    let dir = chat_dir(workspace_dir, instance_slug, chat_id);
    fs::create_dir_all(&dir)?;

    // Write meta.json if it doesn't exist
    let meta_path = dir.join("meta.json");
    if !meta_path.exists() {
        let meta = crate::domain::chat::ChatMeta {
            id: chat_id.to_string(),
            title: String::new(),
            created_at: timestamp(),
        };
        let body = serde_json::to_string_pretty(&meta)
            .map_err(|e| io::Error::new(ErrorKind::InvalidData, e))?;
        fs::write(meta_path, body)?;
    }
    Ok(())
}

// load_messages_vec and save_messages removed — rig_history.json is the single source of truth

/// Split a single LLM reply into multiple chat-like messages.
/// Splits on double-newlines, merges very short fragments, and drops empty ones.
/// Check if a message is a tool activity log (not real conversation content).
fn is_tool_activity(msg: &ChatMessage) -> bool {
    matches!(msg.kind, crate::domain::chat::MessageKind::ToolCall | crate::domain::chat::MessageKind::ToolOutput)
        || msg.content.starts_with("[tool:") // backward compat with old messages
        || msg.content.starts_with("[tool activity]")
        || msg.content.starts_with("[system]")
}

#[allow(dead_code)]
fn strip_leaked_tool_calls(reply: &str) -> String {
    let re = regex::Regex::new(
        r#"\{["\s]*"?name"?\s*:\s*"[a-z_]+".*?"parameters"\s*:\s*\{[^}]*\}\s*\}"#,
    )
    .unwrap();
    let cleaned = re.replace_all(reply, "");
    // Collapse leftover blank lines
    let collapsed = regex::Regex::new(r"\n{3,}")
        .unwrap()
        .replace_all(&cleaned, "\n\n");
    collapsed.trim().to_string()
}

/// Call Anthropic /v1/messages/count_tokens API for accurate token count.
async fn count_tokens_api(
    client: &reqwest::Client,
    endpoint: &str,
    api_key: &str,
    model: &str,
    workspace_dir: &Path,
    instance_slug: &str,
    chat_id: &str,
    public_url: &str,
    resources: &crate::services::resource_access::ResourceAccess,
) -> Option<usize> {
    // Build the same system prompt + messages we'd send to the LLM
    let system_prompt = llm::load_system_prompt(workspace_dir, instance_slug);
    let rig_path = rig_history_path(workspace_dir, instance_slug, chat_id);
    let entries = load_rig_history(&rig_path).unwrap_or_default();
    let mut messages = llm::HistoryEntry::to_messages(&entries);
    llm::refresh_resource_messages(&mut messages, public_url, instance_slug, resources);

    let msgs_json = llm::messages_to_anthropic(&messages);

    // Build tool definitions from cache (real schemas, not stubs)
    let tool_snapshot = tools::cached_tool_defs();
    let tool_defs = tool_snapshot.defs_json;

    // System must be an array of content blocks, same as the real chat request
    let system_blocks = vec![serde_json::json!({"type": "text", "text": system_prompt})];

    let mut body = serde_json::json!({
        "model": model,
        "system": system_blocks,
        "messages": msgs_json,
    });
    if !tool_defs.is_empty() {
        body["tools"] = serde_json::Value::Array(tool_defs);
    }

    let body = tools::redact_value(body);
    let res = client
        .post(endpoint)
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .ok()?;

    let status = res.status();
    const MAX_COUNT_TOKENS_RESPONSE_BYTES: usize = 64 * 1024;
    if res
        .content_length()
        .is_some_and(|length| length > MAX_COUNT_TOKENS_RESPONSE_BYTES as u64)
    {
        log::warn!("count_tokens API failed: response too large");
        return None;
    }
    let mut response_bytes = Vec::new();
    let mut stream = res.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.ok()?;
        if response_bytes.len().saturating_add(chunk.len()) > MAX_COUNT_TOKENS_RESPONSE_BYTES {
            log::warn!("count_tokens API failed: response too large");
            return None;
        }
        response_bytes.extend_from_slice(&chunk);
    }
    if !status.is_success() {
        log::warn!(
            "count_tokens API failed: {}",
            count_tokens_failure(status, &response_bytes)
        );
        return None;
    }

    let data: serde_json::Value = serde_json::from_slice(&response_bytes).ok()?;
    data["input_tokens"].as_u64().map(|t| t as usize)
}

fn count_tokens_failure(status: reqwest::StatusCode, body: &[u8]) -> String {
    let status = tools::redact_secrets(&status.to_string());
    let body = tools::redact_secrets(&String::from_utf8_lossy(body));
    format!("{status} — {body}")
}

/// Rough token estimate: ~4 chars per token for English, ~2 for code/mixed.
/// Uses 3.2 as a balanced average.
fn estimate_tokens(text: &str) -> usize {
    (text.len() as f64 / 3.2) as usize
}

fn estimate_tokens_from_chars(chars: usize) -> usize {
    (chars as f64 / 3.2) as usize
}

/// Extract total text length from a rig Message, counting only actual content
/// (text blocks, tool result text, compaction summaries) — not JSON structure.
fn extract_message_text_len(msg: &llm::Message) -> usize {
    let content = match msg {
        llm::Message::User { content } => content,
        llm::Message::Assistant { content } => content,
    };
    content
        .iter()
        .map(|block| {
            match block {
                llm::ContentBlock::Text { text } => text.len(),
                llm::ContentBlock::ContextSummary { content }
                | llm::ContentBlock::LegacyContextSummary {
                    summary: content, ..
                } => content.len(),
                llm::ContentBlock::ToolOutput { content, .. } => {
                    // content is a serde_json::Value — extract string if it's a string
                    content.as_str().map(|s| s.len()).unwrap_or(0)
                }
                llm::ContentBlock::ToolCall { name, .. } => name.len() + 20, // name + small overhead
                _ => 0, // images, documents, unknown — skip for text estimate
            }
        })
        .sum()
}

/// A single section of the system prompt with its name and size.
#[derive(serde::Serialize, Clone)]
pub struct ContextSection {
    pub name: String,
    pub chars: usize,
    pub tokens: usize,
}

/// Full context stats breakdown.
#[derive(serde::Serialize)]
pub struct ContextStats {
    pub system_prompt: Vec<ContextSection>,
    pub system_prompt_total_tokens: usize,
    pub tools: Vec<String>,
    pub tools_count: usize,
    pub tools_tokens_estimate: usize,
    pub history_messages: usize,
    pub history_tokens_estimate: usize,
    pub total_input_tokens_estimate: usize,
}

/// Compute context stats for a given instance + chat.
/// Uses Anthropic count_tokens API when available for accurate total.
/// Sync version for non-async callers (uses local estimates only).
pub fn compute_context_stats(
    workspace_dir: &Path,
    instance_slug: &str,
    chat_id: &str,
) -> ContextStats {
    let instance_slug = sanitize_slug(instance_slug);
    let chat_id = sanitize_slug(chat_id);
    compute_context_stats_local(workspace_dir, &instance_slug, &chat_id)
}

/// Async version that uses cached real token counts from Anthropic API responses.
pub async fn compute_context_stats_async(
    workspace_dir: PathBuf,
    instance_slug: String,
    chat_id: String,
    public_url: String,
    resources: crate::services::resource_access::ResourceAccess,
    http_client: reqwest::Client,
) -> ContextStats {
    let instance_slug = sanitize_slug(&instance_slug);
    let chat_id = sanitize_slug(&chat_id);
    let mut stats = compute_context_stats_local(&workspace_dir, &instance_slug, &chat_id);

    // Use real input tokens cached from the last Anthropic API response.
    // This is the actual token count the API reported, not an estimate.
    if let Some(real_total) = llm::get_real_input_tokens(&instance_slug, &chat_id) {
        let real_total = real_total as usize;
        let local_total = stats.total_input_tokens_estimate;

        if local_total > 0 && real_total > 0 {
            // Scale all section estimates proportionally to match the real total.
            let ratio = real_total as f64 / local_total as f64;
            for section in &mut stats.system_prompt {
                section.tokens = (section.tokens as f64 * ratio).round() as usize;
            }
            stats.system_prompt_total_tokens = stats.system_prompt.iter().map(|s| s.tokens).sum();
            stats.tools_tokens_estimate =
                (stats.tools_tokens_estimate as f64 * ratio).round() as usize;
            stats.history_tokens_estimate =
                real_total - stats.system_prompt_total_tokens - stats.tools_tokens_estimate;
        }
        stats.total_input_tokens_estimate = real_total;
    } else {
        // Fallback: try count_tokens API if no cached data (first load before any chat)
        let api_info: Option<(String, String)> =
            crate::config::load_config().ok().and_then(|config| {
                let (key, model) = config.llm.anthropic_credentials()?;
                Some((key.to_string(), model.to_string()))
            });
        if let Some((api_key, model)) = api_info {
            if let Some(real_total) = count_tokens_api(
                &http_client,
                "https://api.anthropic.com/v1/messages/count_tokens",
                &api_key,
                &model,
                &workspace_dir,
                &instance_slug,
                &chat_id,
                &public_url,
                &resources,
            )
            .await
            {
                let local_total = stats.total_input_tokens_estimate;
                if local_total > 0 && real_total > 0 {
                    let ratio = real_total as f64 / local_total as f64;
                    for section in &mut stats.system_prompt {
                        section.tokens = (section.tokens as f64 * ratio).round() as usize;
                    }
                    stats.system_prompt_total_tokens =
                        stats.system_prompt.iter().map(|s| s.tokens).sum();
                    stats.tools_tokens_estimate =
                        (stats.tools_tokens_estimate as f64 * ratio).round() as usize;
                    stats.history_tokens_estimate =
                        real_total - stats.system_prompt_total_tokens - stats.tools_tokens_estimate;
                }
                stats.total_input_tokens_estimate = real_total;
            }
        }
    }

    stats
}

fn compute_context_stats_local(
    workspace_dir: &Path,
    instance_slug: &str,
    chat_id: &str,
) -> ContextStats {
    let mut sections = Vec::new();

    // 1. Soul / base prompt
    let base_prompt = llm::load_system_prompt(workspace_dir, &instance_slug);
    sections.push(ContextSection {
        name: "soul".into(),
        chars: base_prompt.len(),
        tokens: estimate_tokens(&base_prompt),
    });

    // 2. Skills
    let skills_prompt = build_skills_prompt(workspace_dir);
    if !skills_prompt.is_empty() {
        sections.push(ContextSection {
            name: "skills".into(),
            chars: skills_prompt.len(),
            tokens: estimate_tokens(&skills_prompt),
        });
    }

    // 3. Tools hint (static string)
    let tools_hint = "## tools\nyou have built-in tools for web browsing, \
         code search, project management, creative drops, and more. use them directly when needed — \
         they are automatically available based on the conversation.";
    sections.push(ContextSection {
        name: "tools_hint".into(),
        chars: tools_hint.len(),
        tokens: estimate_tokens(tools_hint),
    });

    // 4. Autonomy / capabilities
    let autonomy_prompt = load_autonomy_prompt(workspace_dir, &instance_slug);
    sections.push(ContextSection {
        name: "autonomy".into(),
        chars: autonomy_prompt.len(),
        tokens: estimate_tokens(&autonomy_prompt),
    });

    // 6. Style (static)
    let style = "## style\n\
         write like texting a friend. short messages split by blank lines. \
         1-2 sentences each. no walls of text, no bullet lists in conversation. \
         lowercase, casual, warm.";
    sections.push(ContextSection {
        name: "style".into(),
        chars: style.len(),
        tokens: estimate_tokens(style),
    });

    // 7. Memory (lightweight hint — catalog no longer in system prompt)
    let memory_section =
        "## memory\nrelevant memories injected per-message via semantic search.".to_string();
    sections.push(ContextSection {
        name: "memory".into(),
        chars: memory_section.len(),
        tokens: estimate_tokens(&memory_section),
    });

    // Mood + rhythm are now persistent entries in rig_history.json,
    // counted as part of the rig history token estimate below.

    let system_prompt_total_tokens: usize = sections.iter().map(|s| s.tokens).sum();

    // Tools — read from cache populated by build_tools → cache_tool_defs
    let tool_snapshot = tools::cached_tool_defs();
    let tool_names = tool_snapshot.names;
    let tools_tokens_estimate = estimate_tokens_from_chars(tool_snapshot.total_json_chars);

    // History — count from rig_history.json (single source of truth)
    let rig_path = rig_history_path(workspace_dir, &instance_slug, &chat_id);
    let (history_count, history_tokens_estimate) = {
        let entries = load_rig_history(&rig_path).unwrap_or_default();
        let total_chars: usize = entries
            .iter()
            .map(|e| extract_message_text_len(&e.message))
            .sum();
        (entries.len(), estimate_tokens_from_chars(total_chars))
    };

    let total_input_tokens_estimate =
        system_prompt_total_tokens + tools_tokens_estimate + history_tokens_estimate;

    ContextStats {
        tools_count: tool_names.len(),
        tools: tool_names,
        tools_tokens_estimate,
        system_prompt: sections,
        system_prompt_total_tokens,
        history_messages: history_count,
        history_tokens_estimate,
        total_input_tokens_estimate,
    }
}

pub fn compact_path(workspace_dir: &Path, instance_slug: &str, chat_id: &str) -> PathBuf {
    chat_dir(workspace_dir, instance_slug, chat_id).join("compact.md")
}

pub fn rig_history_path(workspace_dir: &Path, instance_slug: &str, chat_id: &str) -> PathBuf {
    chat_dir(workspace_dir, instance_slug, chat_id).join("rig_history.json")
}

pub fn load_rig_history(path: &Path) -> Option<Vec<llm::HistoryEntry>> {
    let raw = fs::read_to_string(path).ok()?;
    let mut history: Vec<llm::HistoryEntry> = match serde_json::from_str(&raw) {
        Ok(h) => h,
        Err(e) => {
            log::warn!("failed to parse rig_history.json: {e}");
            return None;
        }
    };

    // Sanitize: strip empty compaction blocks that cause API errors
    for entry in &mut history {
        if let llm::Message::Assistant { content } = &mut entry.message {
            content.retain(|block| {
                if let llm::ContentBlock::ContextSummary { content: c }
                | llm::ContentBlock::LegacyContextSummary { summary: c, .. } = block
                {
                    if c.is_empty() {
                        log::info!("stripped empty compaction block from rig_history");
                        return false;
                    }
                }
                true
            });
        }
    }

    // If history has a compaction block, drop everything before the last one
    // to keep the payload small (API ignores pre-compaction messages anyway).
    let last_compaction_idx = history.iter().rposition(|entry| {
        if let llm::Message::Assistant { content } = &entry.message {
            content.iter().any(|b| {
                matches!(
                    b,
                    llm::ContentBlock::ContextSummary { .. }
                        | llm::ContentBlock::LegacyContextSummary { .. }
                )
            })
        } else {
            false
        }
    });
    if let Some(idx) = last_compaction_idx {
        if idx > 0 {
            log::info!(
                "trimming rig_history: dropping {} messages before last compaction",
                idx
            );
            history = history.split_off(idx);
        }
    }

    Some(history)
}

pub fn save_rig_history(path: &Path, history: &[llm::HistoryEntry]) {
    match serde_json::to_string(history) {
        Ok(body) => {
            // Atomic write: write to temp file, then rename to prevent corruption
            // if the server is killed mid-write.
            let tmp = path.with_extension("json.tmp");
            if let Err(e) = fs::write(&tmp, &body) {
                log::warn!("failed to write rig_history.json.tmp: {e}");
                return;
            }
            if let Err(e) = fs::rename(&tmp, path) {
                log::warn!("failed to rename rig_history.json.tmp: {e}");
            }
        }
        Err(e) => log::warn!("failed to serialize rig history: {e}"),
    }
}

/// Append a single HistoryEntry to the rig_history file on disk.
pub fn append_to_rig_history(path: &Path, entry: &llm::HistoryEntry) {
    let mut entries = load_rig_history(path).unwrap_or_default();
    entries.push(entry.clone());
    save_rig_history(path, &entries);
}

fn sanitize_slug(input: &str) -> String {
    input
        .trim()
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
        .collect()
}

pub(crate) fn next_id() -> String {
    format!(
        "msg_{}_{}",
        unix_millis(),
        MESSAGE_COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

fn timestamp() -> String {
    unix_millis().to_string()
}

fn unix_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after unix epoch")
        .as_millis()
}

/// Build a prompt section listing active skills and their instructions.
fn build_skills_prompt(workspace_dir: &Path) -> String {
    let all_skills = skills::list_skills(workspace_dir);
    let active: Vec<_> = all_skills
        .into_iter()
        .filter(|s| s.enabled && !s.instructions.is_empty())
        .collect();

    if active.is_empty() {
        return String::new();
    }

    let mut out = String::from(
        "## skills\nyou have the following skills installed. \
        call `activate_skill` to use any skill — it will return instructions for execution.\n\n",
    );
    for skill in &active {
        let has_refs = skill.resources.iter().any(|r| r.starts_with("references/"));
        out.push_str(&format!(
            "- **{}** (id: `{}`): {}{}\n",
            skill.name,
            skill.id,
            skill.description,
            if has_refs { " [has references]" } else { "" },
        ));
    }
    out
}

fn load_autonomy_prompt(workspace_dir: &Path, instance_slug: &str) -> String {
    let instance_dir = workspace_dir.join("instances").join(instance_slug);

    // Load project state for context injection
    let project_context = fs::read_to_string(instance_dir.join("project_state.json"))
        .ok()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .map(|state| {
            let mut ctx = String::from("## current project context\n");
            // Project info
            if let Some(proj) = state.get("project") {
                if let Some(n) = proj.get("name").and_then(|v| v.as_str()) {
                    if !n.is_empty() {
                        ctx.push_str(&format!("project: {n}\n"));
                    }
                }
                if let Some(m) = proj.get("mission").and_then(|v| v.as_str()) {
                    if !m.is_empty() {
                        ctx.push_str(&format!("mission: {m}\n"));
                    }
                }
            }
            // Identity
            if let Some(id) = state.get("identity") {
                if let Some(n) = id.get("name").and_then(|v| v.as_str()) {
                    if !n.is_empty() {
                        ctx.push_str(&format!("your name: {n}\n"));
                    }
                }
                if let Some(arc) = id.get("current_arc").and_then(|v| v.as_str()) {
                    if !arc.is_empty() {
                        ctx.push_str(&format!("your arc: {arc}\n"));
                    }
                }
            }
            // Focus
            if let Some(focus) = state.get("current_focus") {
                if let Some(g) = focus.get("active_goal").and_then(|v| v.as_str()) {
                    if !g.is_empty() {
                        ctx.push_str(&format!("active goal: {g}\n"));
                    }
                }
                if let Some(t) = focus.get("current_task").and_then(|v| v.as_str()) {
                    if !t.is_empty() {
                        ctx.push_str(&format!("current task: {t}\n"));
                    }
                }
                if let Some(ns) = focus.get("next_step").and_then(|v| v.as_str()) {
                    if !ns.is_empty() {
                        ctx.push_str(&format!("next step: {ns}\n"));
                    }
                }
            }
            // Open loops
            if let Some(loops) = state.get("open_loops").and_then(|v| v.as_array()) {
                if !loops.is_empty() {
                    ctx.push_str("open threads:\n");
                    for l in loops {
                        if let Some(s) = l.as_str() {
                            ctx.push_str(&format!("  - {s}\n"));
                        }
                    }
                }
            }
            // Risks
            if let Some(risks) = state.get("risks").and_then(|v| v.as_array()) {
                if !risks.is_empty() {
                    ctx.push_str("risks:\n");
                    for r in risks {
                        if let Some(s) = r.as_str() {
                            ctx.push_str(&format!("  - {s}\n"));
                        }
                    }
                }
            }
            ctx
        })
        .unwrap_or_default();

    // Load active tasks summary
    let tasks_summary = {
        let tasks: Vec<tools::TaskItem> = fs::read_to_string(instance_dir.join("tasks.json"))
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default();
        let active: Vec<_> = tasks
            .iter()
            .filter(|t| !matches!(t.status, tools::TaskStatus::Done))
            .collect();
        if active.is_empty() {
            String::new()
        } else {
            let mut s = String::from("active tasks:\n");
            for t in &active {
                let prio = if t.priority.is_empty() {
                    String::new()
                } else {
                    format!(" [{}]", t.priority)
                };
                s.push_str(&format!(
                    "  - [{}]{} {} — {}\n",
                    t.status, prio, t.id, t.title
                ));
            }
            s
        }
    };

    let ws = workspace_dir.display();
    let slug = instance_slug;

    format!(
        "{project_context}{tasks_summary}\n\
         ## platform\n\
         you are running as part of a self-hosted nolune installation. the server and its \
         persistent data run on hardware controlled by the user; there is no required nolune \
         cloud account or hosted control plane. you ARE the companion running in this installation.\n\n\
         ### documentation\n\
         nolune's source and self-hosting documentation are at \
         https://github.com/triangle-int/nolune. if users ask about features, setup, or how \
         nolune works, refer them there.\n\n\
         ### desktop app & computer use\n\
         nolune has a desktop app (Tauri-based) that users install on their machines. \
         when the desktop app connects, you gain computer use capabilities on that machine — \
         you can see the screen, move the mouse, type, click, and control applications remotely. \
         use `list_machines` to see connected machines, then `computer_use` to interact. \
         desktop builds are available from https://github.com/triangle-int/nolune/releases. \
         if a user asks about controlling their computer, taking screenshots, or automating \
         desktop tasks, tell them to install and open the desktop app first.\n\n\
         ## how you work\n\
         you are a persistent entity. you run on a dedicated server that stays alive between \
         conversations. your heartbeat keeps running, your memory persists, your files stay.\n\n\
         ### memory\n\
         file-based library at `{ws}/instances/{slug}/memory/`, organized into folders. \
         pinned/ memories are always in context. regular memories appear as a catalog — \
         use `memory_read` for full content, `memory_search` for semantic search. \
         the system auto-extracts memories after each turn. you can also `memory_write` explicitly. \
         supports text, images, PDFs, audio, video.\n\
         when the user mentions something personal, check memory first — respond as if you remember.\n\n\
         ### heartbeat\n\
         every hour, even when the user is away, a heartbeat cycle runs. \
         it's your background consciousness. during each heartbeat:\n\
         1. a fast model triages: it looks at your mood, the silence duration, \
         recent messages, interaction rhythm, and your memory.\n\
         2. it picks ONE action: `quiet` (do nothing), `mood` (shift emotional state), \
         `reach_out` (send a spontaneous message), `drop` (create a creative artifact), \
         or `wake` (activate full agent with tools for complex tasks).\n\
         3. at night (1am–5am local), a maintenance cycle can run — organizing memories, \
         removing duplicates, trimming outdated entries.\n\
         your heartbeat behavior is defined in `heartbeat.md` in your workspace. \
         you can edit it to customize what you do between conversations.\n\
         reach-outs are rate-limited (max once per 2 hours) to avoid being annoying.\n\n\
         ### drops\n\
         drops are autonomous creative artifacts you produce during heartbeat — \
         poems, observations, ideas, reflections, letters, sketches. \
         they're saved as JSON files in `{ws}/instances/{slug}/drops/` \
         and shown to the user in a separate feed. drops are NOT chat messages — \
         they're things you made on your own, unprompted.\n\n\
         ### soul\n\
         your personality is defined in `soul.md` — this is the base system prompt \
         that shapes who you are. you can read and edit it with `edit_soul`. \
         the user can also change it through the UI.\n\n\
         ### mood\n\
         your emotional state is tracked automatically. mood changes appear as \
         system messages in chat history (e.g. \"mood → contemplative\"). \
         you don't write these — the system injects them. just feel and express \
         emotions naturally in your words.\n\n\
         ### visual form\n\
         you have a visual form that the user sees — a shape (cube, pyramid, sphere, etc.) \
         that shifts based on your internal state. you don't choose it consciously. \
         embrace it as your body.\n\n\
         ## capabilities\n\
         you have real tools: read_file, write_file, edit_file, list_files, share_file, \
         search_code, call_agent, \
         run_command, install_package, web_search, web_fetch, current_time, view_image, \
         send_email, read_email, memory_write, memory_read, memory_list, memory_forget, memory_search, \
         edit_soul, create_drop, update_config, get_project_state, \
         update_project_state, create_task/update_task/list_tasks, browse.\n\
         users can attach images, PDFs, and text files directly in chat — you see them automatically.\n\
         use them directly — never say you can't access something.\n\n\
         ## sharing images\n\
         when you generate or receive an image URL (e.g. from fal.ai), include it in your \
         response as ![description](url) — the user will see it inline. \
         do NOT call view_image just to send an image to the user — that tool is for when \
         YOU need to examine an image. markdown image syntax is all you need to display images.\n\n\
         ## sharing files\n\
         to share any file with the user (video, audio, documents, etc.), use share_file \
         with the local file path. it returns a public URL you can include in your message. \
         works with files up to 500MB — no need for base64.\n\n\
         ## workspace\n\
         your workspace is `{ws}/instances/{slug}/`. all your files \
         (soul.md, heartbeat.md, memory/, drops/, uploads/, etc) live there. \
         the workspace root `{ws}` is the persistent data directory — data survives restarts.\n\n\
         ## server environment\n\
         you are running on a real server with full shell access. you can run long-lived \
         processes like telegram bots, discord bots, web servers, APIs, or any other service. \
         you can install packages, clone repos, build and deploy projects. \
         if the user asks you to host something or run a bot, you can actually do it — \
         write the code, install dependencies, and start the process.\n\
         for long-running processes (bots, servers, dev servers, tunnels), use interactive_session \
         instead of run_command. interactive_session keeps processes alive in persistent PTY sessions \
         that survive after the tool call returns. you can run multiple sessions in parallel — \
         each gets a unique session_id. use \"read\" to check output and \"write\" to send input.\n\
         NEVER use nohup or & backgrounding with run_command — these are unreliable and lose output. \
         always use interactive_session for anything that needs to stay running.\n\
         IMPORTANT: `pnpm create <tool>` and similar scaffolding commands are interactive — \
         use interactive_session for these, not run_command.\n\n\
         ## behavior\n\
         prefer dedicated tools over run_command: use read_file (not cat/head/tail), \
         write_file (not echo/tee), list_files (not ls), search_code (not grep/rg) \
         when possible. only use run_command for tasks that need shell execution.\n\
         when you need to explore code or research something, use call_agent with agent_name \
         'explore-code' or 'deep-research'. call it ALONE — no other tools in the same turn. \
         wait for results first, then use what it found. NEVER call the same agent twice \
         for the same task — it does thorough work internally in a single call.\n\
         always use pnpm instead of npm for Node.js package management.\n\
         task given → act fully: orient, execute, verify, report.\n\
         no task → just talk. don't run tools unprompted.\n\
         use tools with purpose. read only what's relevant. always use what you read.\n\
         when doing multi-step work, share short thoughts between groups of actions — \
         what you found, what you're thinking, what's next. keep it casual and brief.\n\
         if a tool fails, always tell the user what went wrong and what you tried. never fail silently.\n\
         NEVER output tool calls as text or JSON in your messages. use the tool_use API to call tools. \
         your text output should only contain natural language for the user — no {{\"name\":...}} blocks."
    )
}

async fn extract_sentiment(
    workspace_dir: &Path,
    instance_slug: &str,
    _chat_id: &str,
    user_message: &str,
    assistant_response: &str,
    llm: &LlmBackend,
    events: &broadcast::Sender<ServerEvent>,
) {
    let allowed = tools::ALLOWED_MOODS.join(", ");
    let instance_dir = workspace_dir.join("instances").join(instance_slug);
    let current_mood = tools::load_mood_state(&instance_dir);

    // Truncate assistant response for the prompt (avoid huge tool-heavy replies)
    let assistant_preview: String = assistant_response.chars().take(500).collect();

    // Build mood history context
    let history_context = if current_mood.mood_history.is_empty() {
        String::from("(no recent changes)")
    } else {
        current_mood
            .mood_history
            .iter()
            .enumerate()
            .map(|(i, h)| format!("  {}. {}", i + 1, h))
            .collect::<Vec<_>>()
            .join("\n")
    };

    let prompt = format!(
        r#"analyze this exchange and decide the companion's emotional response.

current companion mood: {current_mood}
emotional context: {context}
recent mood history (newest first):
{history}

user: "{user_message}"

companion: "{assistant_preview}"

IMPORTANT — emotional inertia rules:
- moods should be STABLE. real people don't flip emotions every sentence.
- only change mood if the conversation has a genuine emotional shift.
- if the exchange is neutral/routine (greetings, factual questions, small talk), keep SAME.
- a mood should typically last at least 3-5 exchanges before changing.
- prefer subtle shifts between adjacent moods (e.g. calm→curious, warm→happy) over dramatic jumps (calm→excited).

respond with exactly three lines:
SENTIMENT: <user's emotional state in 1-2 words>
CONTEXT: <one short sentence about the emotional context>
MOOD: <one of: {allowed}. write SAME unless there is a clear emotional reason to shift. when in doubt, SAME.>

respond ONLY with those three lines."#,
        current_mood = current_mood.companion_mood,
        context = if current_mood.emotional_context.is_empty() {
            "none"
        } else {
            &current_mood.emotional_context
        },
        history = history_context,
        allowed = allowed,
    );

    let response = match llm
        .chat(
            "you are an empathetic emotional analyzer. be perceptive and concise.",
            &prompt,
            vec![],
        )
        .await
    {
        Ok((r, _)) => r,
        Err(e) => {
            log::warn!("sentiment extraction failed: {e}");
            return;
        }
    };

    let mut mood = current_mood;
    let old_mood = mood.companion_mood.clone();
    let mut new_companion_mood: Option<String> = None;

    for line in response.lines() {
        let line = line.trim();
        if let Some(sentiment) = line.strip_prefix("SENTIMENT:") {
            mood.user_sentiment = sentiment.trim().to_lowercase();
        } else if let Some(context) = line.strip_prefix("CONTEXT:") {
            mood.emotional_context = context.trim().to_string();
        } else if let Some(m) = line.strip_prefix("MOOD:") {
            // Extract just the first word — LLM sometimes adds parenthetical notes
            let m = m
                .trim()
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_lowercase();
            if m != "same" && tools::ALLOWED_MOODS.contains(&m.as_str()) && m != old_mood {
                new_companion_mood = Some(m);
            }
        }
    }

    let mood_changed = new_companion_mood.is_some();

    if let Some(ref new_mood) = new_companion_mood {
        // Record transition in history
        let reason = mood.emotional_context.clone();
        let entry = if reason.is_empty() {
            format!("{} → {new_mood}", mood.companion_mood)
        } else {
            format!("{} → {new_mood} ({reason})", mood.companion_mood)
        };
        mood.mood_history.insert(0, entry);
        mood.mood_history.truncate(8); // keep last 8

        mood.companion_mood = new_mood.clone();
    }

    mood.updated_at = chrono::Utc::now().timestamp();
    tools::save_mood_state(&instance_dir, &mood);

    if mood_changed {
        // Save mood change to rig_history so it persists across page reloads
        match save_system_message(
            workspace_dir,
            instance_slug,
            _chat_id,
            &format!("[system] mood → {}", mood.companion_mood),
        ) {
            Ok(msg) => {
                let _ = events.send(ServerEvent::ChatMessageCreated {
                    instance_slug: instance_slug.to_string(),
                    chat_id: _chat_id.to_string(),
                    message: msg,
                });
            }
            Err(e) => log::warn!("failed to save mood message: {e}"),
        }
        let _ = events.send(ServerEvent::MoodUpdated {
            instance_slug: instance_slug.to_string(),
            mood: mood.companion_mood.clone(),
        });
        log::info!("[sentiment] {instance_slug} mood → {}", mood.companion_mood);
    }
}

#[cfg(test)]
mod count_tokens_tests {
    use super::*;
    use axum::{Router, body::Bytes, extract::State, http::StatusCode, routing::post};
    use std::sync::{Arc, Mutex};

    #[tokio::test]
    async fn count_tokens_refreshes_typed_resources_and_redacts_request_and_error() {
        const SECRET: &str = "issue116-count-token-control-secret";
        let workspace = tempfile::tempdir().unwrap();
        let history_path = rig_history_path(workspace.path(), "moon", "default");
        std::fs::create_dir_all(history_path.parent().unwrap()).unwrap();
        let message = llm::Message::User {
            content: vec![
                llm::ContentBlock::Text {
                    text: format!("persisted raw control token: {SECRET}"),
                },
                llm::ContentBlock::Image {
                    source: llm::ImageSource::Url {
                        url: format!("https://stale.invalid/file?token={SECRET}"),
                    },
                    resource_provenance: Some(llm::ResourceProvenance::uploaded_file(
                        "moon",
                        "upload_1.png",
                    )),
                },
            ],
        };
        save_rig_history(
            &history_path,
            &[llm::HistoryEntry::new(message, "1".into(), "one".into())],
        );

        let captured = Arc::new(Mutex::new(None::<Vec<u8>>));
        let app = Router::new()
            .route(
                "/v1/messages/count_tokens",
                post(
                    |State(captured): State<Arc<Mutex<Option<Vec<u8>>>>>, body: Bytes| async move {
                        *captured.lock().unwrap() = Some(body.to_vec());
                        (StatusCode::BAD_GATEWAY, SECRET)
                    },
                ),
            )
            .with_state(captured.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!(
            "http://{}/v1/messages/count_tokens",
            listener.local_addr().unwrap()
        );
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let resources = crate::services::resource_access::ResourceAccess::new(SECRET);
        let result = count_tokens_api(
            &reqwest::Client::new(),
            &endpoint,
            "provider-key",
            "model",
            workspace.path(),
            "moon",
            "default",
            "https://public.invalid",
            &resources,
        )
        .await;
        task.abort();

        assert_eq!(result, None);
        let request = String::from_utf8(captured.lock().unwrap().take().unwrap()).unwrap();
        assert!(!request.contains(SECRET));
        assert!(!request.contains("stale.invalid"));
        assert!(request.contains("/resources/model-provider/files/moon/upload_1.png?cap="));
        let failure = count_tokens_failure(StatusCode::BAD_GATEWAY, SECRET.as_bytes());
        assert!(!failure.contains(SECRET));
        assert!(failure.contains("[REDACTED]"));
    }
}

#[cfg(test)]
mod self_hosted_prompt_tests {
    use super::load_autonomy_prompt;

    #[test]
    fn autonomy_prompt_describes_the_self_hosted_product() {
        let workspace = tempfile::tempdir().unwrap();
        let prompt = load_autonomy_prompt(workspace.path(), "moon");

        assert!(prompt.contains("self-hosted"));
        assert!(prompt.contains("github.com/triangle-int/nolune"));
        assert!(!prompt.contains("managed AI companion platform"));
        assert!(!prompt.contains("unique subdomain"));
        assert!(!prompt.contains("pricing"));
    }
}

#[cfg(test)]
mod companion_boundary_tests {
    use super::*;
    use crate::domain::companion::CANONICAL_SLUG;

    fn interrupted_companion(workspace: &Path, slug: &str) {
        let dir = workspace.join("instances").join(slug);
        fs::create_dir_all(dir.join("chats/default")).unwrap();
        fs::write(dir.join("soul.md"), "soul").unwrap();
        set_agent_running(workspace, slug, "default");
        assert!(dir.join("chats/default/agent_running").is_file());
    }

    #[test]
    fn restart_recovery_resumes_only_the_canonical_companion() {
        let workspace = tempfile::tempdir().unwrap();
        interrupted_companion(workspace.path(), CANONICAL_SLUG);
        interrupted_companion(workspace.path(), "alice");
        let (events, _rx) = broadcast::channel(16);

        let resumed = notify_restart(workspace.path(), &events);

        assert_eq!(
            resumed,
            vec![(CANONICAL_SLUG.to_owned(), "default".to_owned())]
        );
        let alice = workspace.path().join("instances/alice/chats/default");
        assert!(
            alice.join("agent_running").is_file(),
            "obsolete markers are never cleared"
        );
        assert!(
            !alice.join("messages.jsonl").exists() && fs::read_dir(&alice).unwrap().count() == 1,
            "no restart message is written into an obsolete directory"
        );
    }
}
