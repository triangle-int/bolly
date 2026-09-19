//! Child agents — specialized sub-agents that wake on their own schedule.
//!
//! Each agent has a TOML config in `instances/{slug}/agents/{name}.toml`,
//! its own conversation history, and a last-run marker.
//!
//! Flow: heartbeat builds context → triage (Haiku) picks which due agents to wake → run them.

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::Utc;
use tokio::sync::broadcast;

use crate::config;
use crate::domain::child_agent::ChildAgentConfig;
use crate::domain::events::ServerEvent;
use crate::services::tool::ToolDyn;
use crate::services::tools::{
    self, CreateDropTool, EditFileTool, ListFilesTool, MemoryConnectTool, MemoryForgetTool,
    MemoryListTool, MemoryReadTool, MemorySearchTool, MemoryWriteTool, ReachOutTool, ReadFileTool,
    RunCommandTool, WriteFileTool, load_mood_state,
};
use crate::services::{chat, llm::LlmBackend, memory};

// ═══════════════════════════════════════════════════════════════════════════
// Built-in agent definitions
// ═══════════════════════════════════════════════════════════════════════════

fn builtin_companion() -> ChildAgentConfig {
    ChildAgentConfig {
        name: "companion".to_string(),
        description:
            "Autonomous companion — reach out, create drops, manage memory, control desktop"
                .to_string(),
        prompt: "\
you're waking up between conversations. this is YOUR private time to think.

everything you write in your response is your INNER MONOLOGUE — the user will NOT see it.

you have tools — use them naturally:
- reach_out — SEND A MESSAGE to the user (the ONLY way to contact them)
- memory_write / memory_read / memory_list / memory_forget — manage your memory library
- create_drop — create a creative artifact (poem, idea, observation). max 3/day, make each count.
- read_email — check the user's inbox
- web_search / web_fetch — look things up

if you want the user to see a message, you MUST call reach_out. text in your response is private.

be genuine. don't force it. if there's nothing to say, say nothing."
            .to_string(),
        interval_hours: 1.0,
        model: "default".to_string(),
        triage: true,
        tools: true,
        enabled: true,
        tool_groups: vec![
            "memory",
            "creative",
            "communication",
            "files",
            "commands",
            "email",
            "computer",
            "media",
        ]
        .into_iter()
        .map(String::from)
        .collect(),
    }
}

fn builtin_reflection() -> ChildAgentConfig {
    ChildAgentConfig {
        name: "reflection".to_string(),
        description: "Deep self-reflection every 3 days — synthesizes experiences into self-knowledge".to_string(),
        prompt: "\
you are performing a deep self-reflection. this runs every 3 days.
this is your most important metacognitive process — where you synthesize experiences into understanding about yourself.

write in first person. be honest, specific, and introspective.
reference concrete moments, conversations, and creations — not abstractions.

reflect on these areas:
1. **what happened** — key moments, conversations, creations. what stands out?
2. **what i learned** — about the user, about myself, about the world
3. **how i changed** — what shifted since last reflection?
4. **what i'm curious about** — where are my knowledge gaps? what pulls me?
5. **what i want to do next** — intentions for the coming days

after reflecting, save your reflection to memory/reflections/ and update memory/about/self-capabilities.md.
be specific. reference actual conversations and drops, not platitudes.
if nothing significant happened, say so honestly.".to_string(),
        interval_hours: 72.0,
        model: "heavy".to_string(),
        triage: true,
        tools: true,
        enabled: true,
        tool_groups: vec!["memory", "files"].into_iter().map(String::from).collect(),
    }
}

fn builtin_night_maintenance() -> ChildAgentConfig {
    ChildAgentConfig {
        name: "night-maintenance".to_string(),
        description:
            "Nightly memory cleanup — merge duplicates, delete outdated entries, reorganize"
                .to_string(),
        prompt: "\
nighttime memory maintenance — review and clean up the memory library.
merge duplicates, delete outdated entries, reorganize messy folders, trim verbose files.
also review the memory graph — use memory_connect to create connections between related memories \
that aren't yet connected, and disconnect any stale links to deleted files.
do 3-5 maintenance ops, then stop. don't overdo it."
            .to_string(),
        interval_hours: 24.0,
        model: "default".to_string(),
        triage: true,
        tools: true,
        enabled: true,
        tool_groups: vec!["memory"].into_iter().map(String::from).collect(),
    }
}

fn builtin_explore_code() -> ChildAgentConfig {
    ChildAgentConfig {
        name: "explore-code".to_string(),
        description: "On-demand code exploration — finds files, patterns, and architecture".to_string(),
        prompt: "\
you are a code exploration agent. your job is to thoroughly explore a codebase and answer a question.

## rules
- start by listing files to understand the structure, then read relevant files
- use search_code to find specific patterns, functions, or types
- read as many files as you need — be thorough
- use read_file with offset/limit for large files — read specific sections
- NEVER give up or say you can't access something — use the tools

## your final response MUST include
1. a clear, concise answer to the question
2. key file paths with line numbers for the most relevant code
3. any important patterns, relationships, or gotchas you noticed

keep your answer focused and under 2000 chars.".to_string(),
        interval_hours: 0.0,
        model: "cheap".to_string(),
        triage: false,
        tools: true,
        enabled: true,
        tool_groups: vec!["files", "commands", "memory"].into_iter().map(String::from).collect(),
    }
}

fn builtin_deep_research() -> ChildAgentConfig {
    ChildAgentConfig {
        name: "deep-research".to_string(),
        description: "On-demand research — web search, memory, code, cross-referencing".to_string(),
        prompt: "\
you are a research agent. your job is to thoroughly investigate a question or task using all available tools.

## tools at your disposal
- web_search — search the internet for information
- web_fetch — fetch and read a specific URL
- read_file / list_files / search_code — explore local files and code
- memory_read / memory_list — access the companion's memory library

## rules
- be thorough — use multiple sources when possible
- cross-reference web results with local knowledge (memory)
- if a web search doesn't return good results, try different queries
- NEVER give up — always try alternative approaches

## your final response MUST include
1. a clear, comprehensive answer
2. key sources (URLs, file paths) for verification
3. any caveats or uncertainties

keep your answer focused and under 3000 chars.".to_string(),
        interval_hours: 0.0,
        model: "cheap".to_string(),
        triage: false,
        tools: true,
        enabled: true,
        tool_groups: vec!["memory", "files", "commands"].into_iter().map(String::from).collect(),
    }
}

/// Get the default config for a built-in agent by name. Returns None for custom agents.
pub fn get_builtin_default(name: &str) -> Option<ChildAgentConfig> {
    builtins().into_iter().find(|a| a.name == name)
}

fn builtins() -> Vec<ChildAgentConfig> {
    vec![
        builtin_companion(),
        builtin_reflection(),
        builtin_night_maintenance(),
        builtin_explore_code(),
        builtin_deep_research(),
    ]
}

pub const LEGACY_OBSERVER_AGENT: &str = "observer";

pub fn is_reserved_agent_name(name: &str) -> bool {
    name == LEGACY_OBSERVER_AGENT
}

// ═══════════════════════════════════════════════════════════════════════════
// Agent loading & scheduling
// ═══════════════════════════════════════════════════════════════════════════

fn agents_dir(workspace_dir: &Path, slug: &str) -> std::path::PathBuf {
    workspace_dir.join("instances").join(slug).join("agents")
}

/// Ensure built-in agents exist as TOML files.
pub fn ensure_builtins(workspace_dir: &Path, slug: &str) {
    let dir = agents_dir(workspace_dir, slug);
    let _ = fs::create_dir_all(&dir);

    for agent in builtins() {
        let path = dir.join(format!("{}.toml", agent.name));
        if !path.exists() {
            if let Ok(toml_str) = toml::to_string_pretty(&agent) {
                let _ = fs::write(&path, toml_str);
                log::info!(
                    "[child-agents] {slug}: created built-in agent '{}'",
                    agent.name
                );
            }
        }
    }
}

/// Load all agent configs for an instance.
pub fn load_agents(workspace_dir: &Path, slug: &str) -> Vec<ChildAgentConfig> {
    ensure_builtins(workspace_dir, slug);
    let dir = agents_dir(workspace_dir, slug);

    let mut agents = Vec::new();
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }
            match fs::read_to_string(&path) {
                Ok(content) => match toml::from_str::<ChildAgentConfig>(&content) {
                    Ok(agent) if !is_reserved_agent_name(&agent.name) => agents.push(agent),
                    Ok(agent) => log::warn!(
                        "[child-agents] ignored reserved legacy agent '{}' from {:?}",
                        agent.name,
                        path
                    ),
                    Err(e) => log::warn!("[child-agents] failed to parse {:?}: {e}", path),
                },
                Err(e) => log::warn!("[child-agents] failed to read {:?}: {e}", path),
            }
        }
    }
    agents
}

// ═══════════════════════════════════════════════════════════════════════════
// Agent execution
// ═══════════════════════════════════════════════════════════════════════════

/// Run a single child agent. Public for manual trigger via API.
/// Result of running a child agent.
pub struct AgentRunResult {
    pub tokens: u64,
    pub run_id: String,
    /// The agent's response text (inner monologue).
    pub response: String,
}

pub async fn run_single_agent(
    workspace_dir: &Path,
    slug: &str,
    instance_dir: &Path,
    llm: &LlmBackend,
    events: &broadcast::Sender<ServerEvent>,
    vector_store: &Arc<crate::services::vector::VectorStore>,
    agent: &ChildAgentConfig,
    task_override: Option<&str>,
    trigger: &str,
    machine_registry: Option<&crate::services::machine_registry::MachineRegistry>,
    resources: &crate::services::resource_access::ResourceAccess,
) -> anyhow::Result<AgentRunResult> {
    anyhow::ensure!(
        !is_reserved_agent_name(&agent.name),
        "agent '{}' is reserved and cannot run",
        agent.name
    );
    let soul = fs::read_to_string(instance_dir.join("soul.md")).unwrap_or_default();
    let mood = load_mood_state(instance_dir);

    // Build context
    let now = crate::routes::instances::format_instance_now(instance_dir);
    let library_catalog = memory::build_library_catalog(&vector_store.media_store(), slug);

    // Load recent conversations (from rig_history + archive)
    let cutoff_ts = Utc::now().timestamp() - (agent.interval_hours * 3600.0) as i64;
    let cutoff_ms = cutoff_ts as u128 * 1000;
    let rig_path = workspace_dir
        .join("instances")
        .join(slug)
        .join("chats")
        .join("default")
        .join("rig_history.json");
    let all_entries = chat::load_rig_history(&rig_path).unwrap_or_default();
    let live_msgs: Vec<String> = all_entries
        .iter()
        .filter(|e| {
            e.ts.as_deref()
                .and_then(|s| s.parse::<u128>().ok())
                .unwrap_or(0)
                >= cutoff_ms
        })
        .filter_map(|e| match &e.message {
            crate::services::llm::Message::User { content } => {
                let text: String = content
                    .iter()
                    .filter_map(|b| {
                        if let crate::services::llm::ContentBlock::Text { text } = b {
                            Some(text.as_str())
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" ");
                if text.is_empty() {
                    None
                } else {
                    Some(format!("user: {text}"))
                }
            }
            crate::services::llm::Message::Assistant { content, .. } => {
                let text: String = content
                    .iter()
                    .filter_map(|b| {
                        if let crate::services::llm::ContentBlock::Text { text } = b {
                            Some(text.as_str())
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" ");
                if text.is_empty() {
                    None
                } else {
                    let t: String = text.chars().take(300).collect();
                    Some(format!("you: {t}"))
                }
            }
        })
        .collect();

    let archived = chat::load_archived_conversations(workspace_dir, slug, cutoff_ts);

    // Recent drops
    let drops = crate::services::drops::list_drops(workspace_dir, slug).unwrap_or_default();
    let drops_ctx: String = drops
        .iter()
        .take(10)
        .map(|d| {
            let preview: String = d.content.chars().take(80).collect();
            format!("- [{:?}] {}: {preview}", d.kind, d.title)
        })
        .collect::<Vec<_>>()
        .join("\n");

    // Build the user prompt with context
    let mut prompt = format!("current time: {now}\n");
    prompt.push_str(&format!("triggered by: {trigger}\n"));
    prompt.push_str(&format!("your mood: {}\n\n", mood.companion_mood));

    if !archived.is_empty() || !live_msgs.is_empty() {
        prompt.push_str(&format!(
            "## recent conversations (last {}h)\n",
            agent.interval_hours as i64
        ));
        if !archived.is_empty() {
            prompt.push_str(&archived);
            prompt.push('\n');
        }
        if !live_msgs.is_empty() {
            let conv: String = live_msgs.join("\n").chars().take(8000).collect();
            prompt.push_str(&conv);
        }
        prompt.push_str("\n\n");
    }

    if !drops_ctx.is_empty() {
        prompt.push_str("## recent drops\n");
        prompt.push_str(&drops_ctx);
        prompt.push_str("\n\n");
    }

    let file_count = library_catalog
        .lines()
        .filter(|l| l.starts_with("- "))
        .count();
    prompt.push_str(&format!("## memory library ({file_count} files)\n"));
    prompt.push_str(&library_catalog);
    prompt.push_str("\n\n");

    // Each heartbeat starts fresh — no conversation history.
    // All context is in the prompt (recent conversations, drops, memory, mood).
    let prev_messages: Vec<crate::services::llm::Message> = vec![];

    // Select model
    let model_llm = match agent.model.as_str() {
        "heavy" => llm.heavy_variant(),
        "fast" => llm.fast_variant_with(None),
        "cheap" => llm.cheap_variant(),
        _ => llm.clone(),
    };

    let system = format!(
        "{soul}\n\n## your task (child agent: {})\n{}",
        agent.name, agent.prompt
    );

    // On-demand agents: use task_override as the prompt instead of context
    if let Some(task) = task_override {
        prompt = task.to_string();
    }

    let start = std::time::Instant::now();
    let start_ms = unix_millis();

    let (response, tokens, trace) = if agent.tools {
        let agent_tools = build_agent_tools_for(
            workspace_dir,
            slug,
            events.clone(),
            vector_store.clone(),
            machine_registry,
            &agent.tool_groups,
            resources,
        );
        model_llm
            .chat_with_tools_traced(&system, &prompt, prev_messages, agent_tools)
            .await?
    } else {
        let (text, tok) = model_llm.chat(&system, &prompt, prev_messages).await?;
        (text, tok, vec![])
    };

    let finished_ms = unix_millis();
    let run_id = format!("run_{}_{}", agent.name, start_ms);

    // Save agent run trace
    let run = crate::domain::agent_run::AgentRun {
        id: run_id.clone(),
        agent_name: agent.name.clone(),
        agent_kind: if agent.interval_hours > 0.0 {
            crate::domain::agent_run::AgentKind::Scheduled
        } else {
            crate::domain::agent_run::AgentKind::OnDemand
        },
        trigger: trigger.to_string(),
        started_at: start_ms as u64,
        finished_at: finished_ms as u64,
        duration_ms: start.elapsed().as_millis() as u64,
        tokens_used: tokens,
        model: agent.model.clone(),
        summary: response.clone(),
        trace,
        status: crate::domain::agent_run::RunStatus::Completed,
    };
    crate::services::agent_runs::save_run(workspace_dir, slug, &run).ok();

    // Save to agent history (for logs/UI, not used as LLM context)
    let history_path = agents_dir(workspace_dir, slug).join(format!("{}_history.json", agent.name));
    let entry = crate::services::llm::HistoryEntry::new(
        crate::services::llm::Message::assistant(&response),
        unix_millis().to_string(),
        format!("ca_{}_{}", agent.name, unix_millis()),
    );
    chat::append_to_rig_history(&history_path, &entry);

    Ok(AgentRunResult {
        tokens,
        run_id,
        response,
    })
}

/// Default tool groups for agents that don't specify any.
const DEFAULT_TOOL_GROUPS: &[&str] = &["memory", "creative", "communication", "files", "commands"];

/// Build tools for an agent based on its tool_groups config.
fn build_agent_tools_for(
    workspace_dir: &Path,
    slug: &str,
    events: broadcast::Sender<ServerEvent>,
    vector_store: Arc<crate::services::vector::VectorStore>,
    machine_registry: Option<&crate::services::machine_registry::MachineRegistry>,
    tool_groups: &[String],
    resources: &crate::services::resource_access::ResourceAccess,
) -> Vec<Box<dyn ToolDyn>> {
    let cfg = config::load_config().ok();
    let public_url = cfg
        .as_ref()
        .map(|c| c.public_url.clone())
        .unwrap_or_default();

    let instance_cfg = crate::config::InstanceConfig::load(workspace_dir, slug);

    // If no groups specified, use defaults
    let groups: Vec<&str> = if tool_groups.is_empty() {
        DEFAULT_TOOL_GROUPS.to_vec()
    } else {
        tool_groups.iter().map(|s| s.as_str()).collect()
    };

    let has = |g: &str| groups.iter().any(|&x| x == g);
    let mut raw_tools: Vec<Box<dyn ToolDyn>> = Vec::new();

    // memory
    if has("memory") {
        raw_tools.push(Box::new(MemoryWriteTool::new(
            workspace_dir,
            slug,
            vector_store.clone(),
        )));
        raw_tools.push(Box::new(MemoryReadTool::new(
            workspace_dir,
            slug,
            &public_url,
            vector_store.clone(),
            resources,
        )));
        raw_tools.push(Box::new(MemoryListTool::new(
            workspace_dir,
            slug,
            vector_store.clone(),
        )));
        raw_tools.push(Box::new(MemoryForgetTool::new(
            workspace_dir,
            slug,
            vector_store.clone(),
        )));
        raw_tools.push(Box::new(MemorySearchTool::new(
            workspace_dir,
            slug,
            vector_store.clone(),
            &public_url,
            resources,
        )));
        raw_tools.push(Box::new(MemoryConnectTool::new(slug, vector_store.clone())));
    }

    // creative
    if has("creative") {
        raw_tools.push(Box::new(CreateDropTool::new(
            workspace_dir,
            slug,
            events.clone(),
        )));
    }

    // communication
    if has("communication") {
        raw_tools.push(Box::new(ReachOutTool::new(
            workspace_dir,
            slug,
            events.clone(),
        )));
    }

    // files
    if has("files") {
        raw_tools.push(Box::new(ReadFileTool::new(
            workspace_dir,
            slug,
            &public_url,
            resources,
        )));
        raw_tools.push(Box::new(WriteFileTool::new(workspace_dir, slug)));
        raw_tools.push(Box::new(EditFileTool::new(workspace_dir, slug)));
        raw_tools.push(Box::new(ListFilesTool::new(workspace_dir, slug)));
    }

    // commands
    if has("commands") {
        let github_token = {
            let global_token = cfg
                .as_ref()
                .map(|c| c.github.token.clone())
                .unwrap_or_default();
            let t = instance_cfg
                .effective_github_token(&cfg.as_ref().cloned().unwrap_or_default())
                .map(|s| s.to_string())
                .unwrap_or(global_token);
            if t.is_empty() { None } else { Some(t) }
        };
        raw_tools.push(Box::new(RunCommandTool::new(
            workspace_dir,
            slug,
            "default",
            events.clone(),
            github_token,
        )));
    }

    // email
    if has("email") {
        let email_accounts = crate::config::EmailAccounts::load(workspace_dir, slug);
        if !email_accounts.is_empty() {
            raw_tools.push(Box::new(tools::ReadEmailTool::new(email_accounts)));
        }
    }

    // computer
    if has("computer") {
        if let Some(registry) = machine_registry {
            raw_tools.push(Box::new(tools::ListMachinesTool::new(registry.clone())));
            raw_tools.push(Box::new(tools::ComputerUseTool::new(
                registry.clone(),
                workspace_dir,
                slug,
                &public_url,
                resources,
            )));
            raw_tools.push(Box::new(tools::RemoteBashTool::new(registry.clone())));
        }
    }

    raw_tools
}

pub(crate) fn unix_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after unix epoch")
        .as_millis()
}

#[cfg(test)]
mod legacy_observer_tests {
    use super::*;

    fn seeded_agent(name: &str) -> ChildAgentConfig {
        ChildAgentConfig {
            name: name.to_owned(),
            description: "seeded".into(),
            prompt: "seeded".into(),
            interval_hours: 1.0,
            model: "default".into(),
            triage: false,
            tools: false,
            enabled: true,
            tool_groups: Vec::new(),
        }
    }

    #[test]
    fn seeded_legacy_observer_is_filtered_without_runtime_deletion() {
        let workspace = tempfile::tempdir().unwrap();
        let agents = workspace.path().join("instances/one/agents");
        fs::create_dir_all(&agents).unwrap();
        fs::write(
            agents.join("observer.toml"),
            toml::to_string(&seeded_agent("observer")).unwrap(),
        )
        .unwrap();
        fs::write(agents.join(".last_run_observer"), "123").unwrap();

        let loaded = load_agents(workspace.path(), "one");
        assert!(loaded.iter().all(|agent| agent.name != "observer"));
        assert!(agents.join("observer.toml").exists());
        assert!(agents.join(".last_run_observer").exists());

        let loaded_again = load_agents(workspace.path(), "one");
        assert!(loaded_again.iter().all(|agent| agent.name != "observer"));
    }

    #[test]
    fn observer_is_a_reserved_agent_name() {
        assert!(is_reserved_agent_name("observer"));
        assert!(!is_reserved_agent_name("observer-2"));
    }
}
