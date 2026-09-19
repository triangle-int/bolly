use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use cua_protocol::*;

fn target() -> WindowTarget {
    WindowTarget {
        pid: 42,
        window_id: 99,
    }
}

fn request(action: CuaAction) -> CuaRequestEnvelope {
    CuaRequestEnvelope {
        version: ProtocolVersion::V1,
        request_id: RequestId::try_from("req-final").unwrap(),
        machine_id: MachineId::try_from("desktop-1").unwrap(),
        action,
    }
}

fn descriptor(health: MachineHealth) -> MachineDescriptor {
    MachineDescriptor {
        machine_id: MachineId::try_from("desktop-1").unwrap(),
        location: MachineLocation::Desktop,
        platform: Platform::Macos,
        driver_version: DriverVersion::try_from("0.28.2").unwrap(),
        health,
        permissions: PermissionState {
            accessibility: Permission::Granted,
            screen_capture: Permission::Granted,
        },
        capabilities: vec![Capability::Pointer],
    }
}

fn click_request() -> CuaRequestEnvelope {
    request(CuaAction::Click(ClickArgs {
        target: target(),
        session: None,
        delivery_mode: DeliveryMode::Background,
        address: ElementAddress::Point(WindowPoint { x: 1.0, y: 2.0 }),
        button: MouseButton::Left,
        action: ClickAction::Press,
        modifiers: vec![],
        count: Some(1),
    }))
}

fn error_response(req: &CuaRequestEnvelope) -> CuaResponseEnvelope {
    CuaResponseEnvelope {
        version: req.version,
        request_id: req.request_id.clone(),
        machine_id: req.machine_id.clone(),
        action: req.action.kind(),
        response: CuaResponse::Error {
            error: CuaRuntimeError {
                code: RuntimeErrorCode::RuntimeUnavailable,
                message: BoundedText::try_from("unavailable").unwrap(),
                retryable: true,
            },
        },
    }
}

fn block_on<F: std::future::Future>(mut future: F) -> F::Output {
    use std::{
        pin::Pin,
        task::{Context, Poll, Waker},
    };
    let mut cx = Context::from_waker(Waker::noop());
    // SAFETY: the future is pinned on this stack and never moved before completion.
    let mut future = unsafe { Pin::new_unchecked(&mut future) };
    loop {
        match future.as_mut().poll(&mut cx) {
            Poll::Ready(value) => return value,
            Poll::Pending => std::thread::yield_now(),
        }
    }
}

#[test]
fn degraded_driver_fixture_is_representable_without_invented_modalities() {
    let json = include_str!("fixtures/degraded-window-state.json");
    let state: WindowStateResult = serde_json::from_str(json).unwrap();
    state.validate().unwrap();
    assert!(state.window_bounds.is_none());
    assert!(state.snapshot_id.is_none());
    assert!(state.screenshot.is_none());
}

#[test]
fn empty_values_titles_and_markdown_are_allowed_but_roles_are_not() {
    EmptyValueText::try_from("").unwrap();
    EmptyTitleText::try_from("").unwrap();
    TreeMarkdown::try_from("").unwrap();
    assert!(BoundedText::try_from("").is_err());
}

#[test]
fn exact_key_click_and_hotkey_grammars_fail_closed() {
    for valid in ["a", "Z", "0", "f12", "return"] {
        KeyName::try_from(valid).unwrap();
    }
    for invalid in ["f13", "enter", "aa", "cmd"] {
        assert!(KeyName::try_from(invalid).is_err(), "accepted {invalid}");
    }
    let invalid_ax_count = r#"{"tool":"click","args":{"target":{"pid":1,"window_id":2},"delivery_mode":"background","address":{"kind":"element_token","element_token":"tok"},"button":"left","action":"press","modifiers":[],"count":2}}"#;
    assert!(CuaAction::from_json(invalid_ax_count).is_err());
    let invalid_point_action = r#"{"tool":"click","args":{"target":{"pid":1,"window_id":2},"delivery_mode":"background","address":{"kind":"point","x":1,"y":2},"button":"left","action":"show_menu","modifiers":[]}}"#;
    assert!(CuaAction::from_json(invalid_point_action).is_err());
    let bad_hotkey = r#"{"tool":"hotkey","args":{"target":{"pid":1,"window_id":2},"delivery_mode":"background","address":{"kind":"point","x":1,"y":2},"keys":["c","cmd"]}}"#;
    assert!(CuaAction::from_json(bad_hotkey).is_err());
}

#[test]
fn degraded_machines_are_authorized_but_unavailable_are_not() {
    descriptor(MachineHealth::Degraded)
        .authorize(&click_request().action)
        .unwrap();
    assert!(
        descriptor(MachineHealth::Unavailable)
            .authorize(&click_request().action)
            .is_err()
    );
}

#[test]
fn checked_adapter_rejects_malicious_callback_responses() {
    let req = click_request();
    let adapter = CheckedCuaAdapter::new(descriptor(MachineHealth::Healthy), move |request| {
        let mut response = error_response(&request);
        response.request_id = RequestId::try_from("forged").unwrap();
        Box::pin(async move { response })
    })
    .unwrap();
    assert!(block_on(adapter.execute(&req)).is_err());

    let wrong_machine = CuaRequestEnvelope {
        machine_id: MachineId::try_from("other-machine").unwrap(),
        ..req
    };
    assert!(block_on(adapter.execute(&wrong_machine)).is_err());
}

#[test]
fn screenshot_validation_checks_signatures_headers_and_dimensions() {
    let png = BASE64_STANDARD.encode(include_bytes!("fixtures/tiny.png"));
    let valid_png = Screenshot {
        media_type: ImageMediaType::Png,
        base64: Base64Image::try_from(png).unwrap(),
        width: 1,
        height: 1,
    };
    valid_png.validate().unwrap();
    let mut wrong_dimensions = valid_png.clone();
    wrong_dimensions.width = 2;
    assert!(wrong_dimensions.validate().is_err());

    let arbitrary = Screenshot {
        media_type: ImageMediaType::Png,
        base64: Base64Image::try_from("YWJjZA==").unwrap(),
        width: 1,
        height: 1,
    };
    assert!(arbitrary.validate().is_err());

    let jpeg = BASE64_STANDARD.encode(include_bytes!("fixtures/tiny.jpg"));
    Screenshot {
        media_type: ImageMediaType::Jpeg,
        base64: Base64Image::try_from(jpeg).unwrap(),
        width: 1,
        height: 1,
    }
    .validate()
    .unwrap();
}
