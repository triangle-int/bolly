//! Guard for #95: no engagement dashboard, no duplicate behavioral aggregate
//! stores, and the remaining rhythm metric is documented with an opt-out.

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
fn stats_dashboard_and_duplicate_aggregates_are_absent() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();

    for removed in [
        "server/src/services/daily_stats.rs",
        "server/src/domain/daily_stats.rs",
        "client/src/lib/components/stats",
        "client/src/routes/[slug]/stats",
    ] {
        assert!(
            !repo.join(removed).exists(),
            "stats dashboard source still exists: {removed}"
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
            "daily_stats",
            "DailyStats",
            "StatsResponse",
            "get_stats",
            "streak",
            "heatmap",
            "mood_counts",
            "daily_history",
            "recompute_rhythm",
            "snapshot_before_clear",
            "/stats\"",
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
            "fetchStats",
            "StatsView",
            "interface Stats ",
            "streak",
            "heatmap",
            "\"stats\"",
        ] {
            if source.contains(token) {
                violations.push(format!("{relative} contains {token:?}"));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "engagement dashboard surface remains:\n{}",
        violations.join("\n")
    );
}

#[test]
fn remaining_behavioral_metric_has_a_consumer_retention_rule_and_opt_out() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let doc = fs::read_to_string(repo.join("docs/behavioral-metrics.md"))
        .expect("docs/behavioral-metrics.md must describe the retained rhythm metric");
    for required in [
        "#95",
        "rhythm.json",
        "heartbeat",
        "retention",
        "opt out",
        "rhythm_tracking",
        "/rhythm",
    ] {
        assert!(
            doc.contains(required),
            "metrics doc is missing {required:?}"
        );
    }
}
