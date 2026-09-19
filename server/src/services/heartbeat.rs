//! Heartbeat — independent loops for each child agent.
//!
//! Each scheduled agent gets its own tokio task running on its own interval.
//! No triage — agents wake up on their own schedule and run directly.

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::Utc;
use tokio::sync::{RwLock, broadcast};

use crate::domain::child_agent::ChildAgentConfig;
use crate::domain::companion::CANONICAL_SLUG;
use crate::domain::events::ServerEvent;
use crate::domain::thought::Thought;
use crate::services::machine_registry::MachineRegistry;
use crate::services::tools::load_mood_state;
use crate::services::{chat, companion, llm::LlmBackend, rhythm, thoughts};

pub fn start(
    workspace_dir: &Path,
    llm: Arc<RwLock<Option<LlmBackend>>>,
    events: broadcast::Sender<ServerEvent>,
    vector_store: Arc<crate::services::vector::VectorStore>,
    google_ai_key: String,
    machine_registry: MachineRegistry,
    resources: crate::services::resource_access::ResourceAccess,
) {
    // One companion per server: only the canonical identity has an inner life.
    let slug = CANONICAL_SLUG.to_owned();
    if !companion::companion_dir(workspace_dir)
        .join("soul.md")
        .exists()
    {
        log::info!("heartbeat: companion not onboarded yet; no loops spawned");
        return;
    }
    crate::services::child_agents::ensure_builtins(workspace_dir, &slug);

    // Spawn one independent loop per scheduled agent of the companion
    let agents = crate::services::child_agents::load_agents(workspace_dir, &slug);

    for agent in agents {
        if agent.interval_hours <= 0.0 {
            continue; // skip on-demand agents
        }

        let ws = workspace_dir.to_path_buf();
        let s = slug.clone();
        let l = llm.clone();
        let ev = events.clone();
        let vs = vector_store.clone();
        let gai = google_ai_key.clone();
        let mr = machine_registry.clone();
        let resources = resources.clone();
        let agent_name = agent.name.clone();
        let agent_hours = agent.interval_hours;
        let agent_clone = agent.clone();

        tokio::spawn(async move {
            run_agent_loop(&ws, &s, &agent_clone, l, ev, vs, &gai, mr, resources).await;
        });

        log::info!(
            "heartbeat: spawned '{agent_name}' for instance '{slug}' (every {agent_hours}h)"
        );
    }
}

/// Independent loop for a single child agent.
async fn run_agent_loop(
    workspace_dir: &Path,
    slug: &str,
    agent: &ChildAgentConfig,
    llm: Arc<RwLock<Option<LlmBackend>>>,
    events: broadcast::Sender<ServerEvent>,
    vector_store: Arc<crate::services::vector::VectorStore>,
    google_ai_key: &str,
    machine_registry: MachineRegistry,
    resources: crate::services::resource_access::ResourceAccess,
) {
    let interval_secs = (agent.interval_hours * 3600.0) as u64;
    let instance_dir = workspace_dir.join("instances").join(slug);

    // Initial delay: wait until the agent is due
    let marker_path = workspace_dir
        .join("instances")
        .join(slug)
        .join("agents")
        .join(format!(".last_run_{}", agent.name));
    let last_run: i64 = fs::read_to_string(&marker_path)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);
    let now = Utc::now().timestamp();
    let elapsed = (now - last_run).max(0) as u64;
    if elapsed < interval_secs {
        let wait = interval_secs - elapsed;
        log::info!(
            "[heartbeat] {slug}/{}: next run in {}m",
            agent.name,
            wait / 60
        );
        tokio::time::sleep(Duration::from_secs(wait)).await;
    }

    let mut interval = tokio::time::interval(Duration::from_secs(interval_secs));
    interval.tick().await; // first tick is immediate

    loop {
        // Check that soul.md still exists (instance might have been deleted)
        if !instance_dir.join("soul.md").exists() {
            log::info!("[heartbeat] {slug}/{}: soul.md gone, stopping", agent.name);
            break;
        }

        // Re-read agent config each tick to pick up enabled/disabled changes
        let current_config = {
            let path = workspace_dir
                .join("instances")
                .join(slug)
                .join("agents")
                .join(format!("{}.toml", agent.name));
            std::fs::read_to_string(&path)
                .ok()
                .and_then(|raw| toml::from_str::<ChildAgentConfig>(&raw).ok())
        };

        let is_enabled = current_config
            .as_ref()
            .map(|c| c.enabled)
            .unwrap_or(agent.enabled);

        if !is_enabled {
            // Agent disabled — skip this tick but keep the loop alive
            interval.tick().await;
            continue;
        }

        let llm_guard = llm.read().await;
        if let Some(backend) = llm_guard.as_ref() {
            run_agent_tick(
                workspace_dir,
                slug,
                &instance_dir,
                backend,
                &events,
                &vector_store,
                google_ai_key,
                agent,
                &machine_registry,
                &resources,
            )
            .await;
        }
        drop(llm_guard);

        interval.tick().await;
    }
}

/// Single tick of an agent's heartbeat.
async fn run_agent_tick(
    workspace_dir: &Path,
    slug: &str,
    instance_dir: &Path,
    llm: &LlmBackend,
    events: &broadcast::Sender<ServerEvent>,
    vector_store: &Arc<crate::services::vector::VectorStore>,
    google_ai_key: &str,
    agent: &ChildAgentConfig,
    machine_registry: &MachineRegistry,
    resources: &crate::services::resource_access::ResourceAccess,
) {
    log::info!("[heartbeat] {slug}: running '{}'", agent.name);

    // ── Rhythm update (companion only) ──
    if agent.name == "companion" {
        // Incremental aggregate only; nothing rescans history here (#95).
        let rhythm_insights = if rhythm::tracking_enabled(workspace_dir, slug) {
            let rhythm_data = rhythm::load_rhythm(instance_dir);
            rhythm::build_rhythm_insights(workspace_dir, slug, &rhythm_data)
        } else {
            String::new()
        };
        if !rhythm_insights.trim().is_empty() {
            let label = format!("[system] rhythm update\n{rhythm_insights}");
            let _ = chat::save_system_message(workspace_dir, slug, "default", &label);
        }
    }

    // Run the agent
    match crate::services::child_agents::run_single_agent(
        workspace_dir,
        slug,
        instance_dir,
        llm,
        events,
        vector_store,
        google_ai_key,
        agent,
        None,
        "heartbeat",
        Some(machine_registry),
        resources,
    )
    .await
    {
        Ok(r) => {
            // Mark as run
            let marker = workspace_dir
                .join("instances")
                .join(slug)
                .join("agents")
                .join(format!(".last_run_{}", agent.name));
            let _ = fs::write(&marker, Utc::now().timestamp().to_string());

            let _ = chat::save_system_message(
                workspace_dir,
                slug,
                "default",
                &format!(
                    "[system] child agent '{}' ran ({} tokens)",
                    agent.name, r.tokens
                ),
            );

            log::info!(
                "[heartbeat] {slug}/{}: done ({} tokens)",
                agent.name,
                r.tokens
            );

            // Save thought with the agent's actual inner monologue
            let final_mood = load_mood_state(instance_dir).companion_mood;
            let thought = Thought {
                id: format!("thought_{}", unix_millis()),
                raw: r.response,
                actions: vec![format!("wake:{}", agent.name)],
                mood: final_mood,
                created_at: unix_millis().to_string(),
            };
            let _ = thoughts::save_thought(workspace_dir, slug, &thought);
            let _ = events.send(ServerEvent::HeartbeatThought {
                instance_slug: slug.to_string(),
                thought,
            });
        }
        Err(e) => {
            log::warn!("[heartbeat] {slug}/{}: failed: {e}", agent.name);
        }
    }
}

fn unix_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after unix epoch")
        .as_millis()
}
