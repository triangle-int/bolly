//! Guard for #93: no child-agent framework, Agents dashboard, call_agent tool,
//! or heartbeat prompt-patching remains; the companion runs curated routines.

#[path = "../test-support/source_scan.rs"]
mod source_scan;

use source_scan::without_cfg_test_items;
use std::{
    fs,
    path::{Path, PathBuf},
};

fn files(root: &Path, extensions: &[&str]) -> Vec<PathBuf> {
    fn visit(dir: &Path, extensions: &[&str], out: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(&path, extensions, out);
            } else if path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| extensions.contains(&ext))
            {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    visit(root, extensions, &mut out);
    out.sort();
    out
}

#[test]
fn child_agent_framework_and_agents_dashboard_are_absent() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();

    for removed in [
        "server/src/services/child_agents.rs",
        "server/src/domain/child_agent.rs",
        "server/src/services/agent_runs.rs",
        "server/src/domain/agent_run.rs",
        "server/src/routes/agents.rs",
        "server/src/routes/heartbeat.rs",
        "client/src/lib/components/agents",
        "client/src/routes/[slug]/agents",
        "client/src/lib/components/chat/HeartbeatUpdateBanner.svelte",
    ] {
        assert!(
            !repo.join(removed).exists(),
            "child-agent source still exists: {removed}"
        );
    }

    let mut violations = Vec::new();
    for path in files(&repo.join("server/src"), &["rs"]) {
        let relative = path
            .strip_prefix(repo)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let production = without_cfg_test_items(&fs::read_to_string(&path).unwrap());
        for token in [
            "child_agents::",
            "mod child_agents",
            "ChildAgentConfig",
            "call_agent",
            "CallAgentTool",
            "tool_groups",
            "agent_runs::",
            "mod agent_runs",
            "agent_interval",
            "reset_agent",
            "explore-code",
            "deep-research",
            "night-maintenance",
            "/agents\"",
            "/agents/{",
            "agent-runs",
            "heartbeat/updates",
        ] {
            if production.contains(token) {
                violations.push(format!("{relative} contains {token:?}"));
            }
        }
    }
    for path in files(&repo.join("client/src"), &["ts", "js", "svelte"]) {
        let relative = path
            .strip_prefix(repo)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let source = fs::read_to_string(&path).unwrap();
        for token in [
            "fetchAgents",
            "triggerAgent",
            "updateAgent(",
            "resetAgent",
            "fetchAgentRun",
            "fetchAgentHistory",
            "AgentsView",
            "ChildAgent",
            "AgentRunSummary",
            "AgentHistoryEntry",
            "\"agents\"",
            "HeartbeatUpdate",
            "heartbeat/updates",
            "tool_groups",
        ] {
            if source.contains(token) {
                violations.push(format!("{relative} contains {token:?}"));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "child-agent surface remains:\n{}",
        violations.join("\n")
    );
}

#[test]
fn routines_are_documented_in_the_proactive_loop_doc() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let doc = fs::read_to_string(repo.join("docs/proactive-loop.md")).unwrap();
    for required in [
        "#93",
        "check-in",
        "reflection_enabled",
        "check_in_interval_hours",
        "heartbeat.md",
        "memory_forget",
    ] {
        assert!(
            doc.contains(required),
            "proactive doc is missing {required:?}"
        );
    }
}
