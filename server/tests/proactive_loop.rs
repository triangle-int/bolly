//! Guard for #92: every proactive trigger passes through the one companion
//! loop, and the loop's record and policy are documented.

use std::{fs, path::Path};

#[test]
fn every_proactive_trigger_routes_through_the_companion_loop() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let mut violations = Vec::new();
    for (file, must_contain) in [
        ("server/src/services/heartbeat.rs", "Routine::CheckIn"),
        ("server/src/services/scheduler.rs", "Trigger::Schedule"),
        (
            "server/src/routes/machine_agents.rs",
            "Trigger::MachineConnected",
        ),
        ("server/src/main.rs", "recover_on_restart"),
    ] {
        let source = fs::read_to_string(repo.join(file)).unwrap();
        if !source.contains(must_contain) {
            violations.push(format!(
                "{file} does not go through the proactive loop ({must_contain:?})"
            ));
        }
    }
    // reach_out asks the loop before messaging the user.
    let communication =
        fs::read_to_string(repo.join("server/src/services/tools/communication.rs")).unwrap();
    if !communication.contains("approve_side_effect") {
        violations.push("reach_out does not ask for side-effect approval".into());
    }
    assert!(violations.is_empty(), "{}", violations.join("\n"));
}

#[test]
fn proactive_loop_is_documented() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let doc = fs::read_to_string(repo.join("docs/proactive-loop.md"))
        .expect("docs/proactive-loop.md must describe the execution record and policy");
    for required in [
        "#92",
        "activity/",
        "quiet_hours",
        "cooldown",
        "attention",
        "retention",
        "cancel",
        "retry",
        "#85",
        "#82",
    ] {
        assert!(
            doc.contains(required),
            "proactive doc is missing {required:?}"
        );
    }
}
