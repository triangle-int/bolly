use cua_protocol::*;

fn target() -> WindowTarget {
    WindowTarget {
        pid: 42,
        window_id: 99,
    }
}

fn descriptor(capability: Capability) -> MachineDescriptor {
    MachineDescriptor {
        machine_id: MachineId::try_from("desktop-1").unwrap(),
        location: MachineLocation::Desktop,
        platform: Platform::Macos,
        driver_version: DriverVersion::try_from("0.28.2").unwrap(),
        health: MachineHealth::Healthy,
        permissions: PermissionState {
            accessibility: Permission::Granted,
            screen_capture: Permission::Granted,
        },
        capabilities: vec![capability],
    }
}

#[test]
fn protocol_is_window_only_and_unknown_fields_fail_closed() {
    for json in [
        r#"{"tool":"click","args":{"target":{"pid":0,"window_id":2},"delivery_mode":"background","address":{"kind":"point","x":1,"y":2},"button":"left","action":"press","modifiers":[],"count":1}}"#,
        r#"{"tool":"click","args":{"target":{"kind":"desktop","display_id":"primary"},"delivery_mode":"background","address":{"kind":"point","x":1,"y":2},"button":"left","action":"press","modifiers":[],"count":1}}"#,
        r#"{"tool":"click","args":{"target":{"pid":1,"window_id":2},"delivery_mode":"foreground","address":{"kind":"point","x":1,"y":2},"button":"left","action":"press","modifiers":[],"count":1}}"#,
        r#"{"tool":"click","args":{"target":{"pid":1,"window_id":2},"delivery_mode":"background","address":{"kind":"point","x":1,"y":2},"button":"left","action":"press","modifiers":[],"count":1,"extra":true}}"#,
    ] {
        assert!(CuaAction::from_json(json).is_err(), "accepted {json}");
    }
}

#[test]
fn element_address_variants_are_closed_and_snapshot_bound() {
    for address in [
        r#"{"kind":"element_token","element_token":"opaque/token"}"#,
        r#"{"kind":"element_index","element_index":7,"snapshot_id":"s0123abcd"}"#,
        r#"{"kind":"point","x":10,"y":20}"#,
    ] {
        let json = format!(
            r#"{{"tool":"double_click","args":{{"target":{{"pid":1,"window_id":2}},"delivery_mode":"background","address":{address}}}}}"#
        );
        CuaAction::from_json(&json).unwrap();
    }
    for address in [
        r#"{"kind":"element_index","element_index":7}"#,
        r#"{"kind":"element_token","element_token":"x","x":1,"y":2}"#,
    ] {
        let json = format!(
            r#"{{"tool":"double_click","args":{{"target":{{"pid":1,"window_id":2}},"delivery_mode":"background","address":{address}}}}}"#
        );
        assert!(CuaAction::from_json(&json).is_err());
    }
}

#[test]
fn forbidden_driver_surfaces_and_launch_escape_hatches_are_absent() {
    for tool in [
        "get_desktop_state",
        "clipboard_read",
        "start_recording",
        "kill_app",
        "set_config",
        "bring_to_front",
        "browser_navigate",
    ] {
        assert!(CuaAction::from_json(&format!(r#"{{"tool":"{tool}","args":{{}}}}"#)).is_err());
    }
    for args in [
        r#"{"bundle_id":"com.example.App","additional_arguments":["--shell"]}"#,
        r#"{"bundle_id":"com.example.App","urls":["file:///tmp/x"]}"#,
    ] {
        assert!(
            CuaAction::from_json(&format!(r#"{{"tool":"launch_app","args":{args}}}"#)).is_err()
        );
    }
}

#[test]
fn requests_and_envelopes_are_size_and_depth_bounded() {
    assert!(CuaRequestEnvelope::from_json(&" ".repeat(MAX_REQUEST_BYTES + 1)).is_err());
    assert!(CuaRegistrationEnvelope::from_json(&" ".repeat(MAX_REGISTRATION_BYTES + 1)).is_err());
    assert!(CuaResponseEnvelope::from_json(&" ".repeat(MAX_RESULT_BYTES + 1)).is_err());
    let nested = format!(
        "{}null{}",
        "{\"x\":".repeat(MAX_JSON_DEPTH + 1),
        "}".repeat(MAX_JSON_DEPTH + 1)
    );
    assert!(CuaRequestEnvelope::from_json(&nested).is_err());
}

#[test]
fn capabilities_permissions_and_health_gate_authorization() {
    let click = CuaAction::Click(ClickArgs {
        target: target(),
        session: None,
        delivery_mode: DeliveryMode::Background,
        address: ElementAddress::Point(WindowPoint { x: 1.0, y: 2.0 }),
        button: MouseButton::Left,
        action: ClickAction::Press,
        modifiers: vec![],
        count: Some(1),
    });
    descriptor(Capability::Pointer).authorize(&click).unwrap();
    assert!(descriptor(Capability::Keyboard).authorize(&click).is_err());
    let mut denied = descriptor(Capability::Pointer);
    denied.permissions.accessibility = Permission::Denied;
    assert!(denied.authorize(&click).is_err());
}

#[test]
fn machine_selection_rejects_ambiguity_and_duplicate_ids() {
    let one = vec![descriptor(Capability::Health)];
    assert_eq!(
        select_machine(&one, None).unwrap().machine_id.as_str(),
        "desktop-1"
    );
    let duplicate = vec![
        descriptor(Capability::Health),
        descriptor(Capability::Health),
    ];
    assert_eq!(
        select_machine(&duplicate, None),
        Err(SelectionError::DuplicateMachineId)
    );
}
