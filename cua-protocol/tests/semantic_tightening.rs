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
        request_id: RequestId::try_from("req-1").unwrap(),
        machine_id: MachineId::try_from("desktop-1").unwrap(),
        action,
    }
}

fn response_for(request: &CuaRequestEnvelope, result: CuaActionResult) -> CuaResponseEnvelope {
    CuaResponseEnvelope {
        version: request.version,
        request_id: request.request_id.clone(),
        machine_id: request.machine_id.clone(),
        action: request.action.kind(),
        response: CuaResponse::Success {
            result: Box::new(result),
        },
    }
}

fn outcome(effect: ActionEffect, route: ActionRoute) -> ActionOutcome {
    ActionOutcome {
        effect,
        route,
        delivery: Some(ActionDelivery {
            requested: DeliveryMode::Background,
            delivered_count: Some(1),
        }),
        evidence: vec![ActionEvidence::DeliveryReceipt],
        escalation: None,
    }
}

fn health_name(name: &str) -> HealthCheckName {
    HealthCheckName::try_from(name).unwrap()
}

fn health_response(request: &CuaRequestEnvelope, report: &str) -> CuaResponseEnvelope {
    response_for(
        request,
        CuaActionResult::HealthReport(serde_json::from_str(report).unwrap()),
    )
}

#[test]
fn observed_app_window_and_session_payloads_deserialize() {
    let apps = r#"{"apps":[{"active":false,"bundle_id":"com.apple.Safari","kind":"desktop","last_used":"2026-09-03T05:58:02Z","launch_path":"/Applications/Safari.app","name":"Safari","pid":3566,"running":true,"windows":[]}]}"#;
    let parsed: AppsResult = serde_json::from_str(apps).unwrap();
    assert_eq!(
        parsed.apps[0].launch_path.as_ref().unwrap().as_str(),
        "/Applications/Safari.app"
    );

    let windows = r#"{"current_space_id":1,"windows":[{"target":{"pid":3566,"window_id":79},"app_name":"Safari","title":"","bounds":{"x":232.0,"y":381.0,"width":1885.0,"height":960.0},"z_index":41,"is_on_screen":true,"space_ids":[1],"current_space_id":1,"layer":0,"on_current_space":true}]}"#;
    let parsed: WindowsResult = serde_json::from_str(windows).unwrap();
    assert_eq!(parsed.current_space_id, Some(1));
    assert_eq!(parsed.windows[0].title.as_ref().unwrap().as_str(), "");

    let start = r#"{"active":true,"capture_scope":"auto","desktop_capture_authorized":false,"desktop_unlocked":false,"effective_scope":"window","escalation_detail":null,"escalation_reason":null,"revived":false,"session":"review"}"#;
    let _: StartSessionResult = serde_json::from_str(start).unwrap();
    let get = r#"{"client_kind":"cli","cursor_visible":false,"expires_in_seconds":299,"idle_seconds":0,"implicit":false,"recording_active":false,"session":"review","state":"active","transport":"cli"}"#;
    let _: GetSessionResult = serde_json::from_str(get).unwrap();
    let end = r#"{"active":false,"session":"review"}"#;
    let _: EndSessionResult = serde_json::from_str(end).unwrap();
}

#[test]
fn accessibility_elements_preserve_actions_and_empty_values() {
    let json = r#"{"element_index":0,"element_token":"token","role":"AXTextField","label":"Name","value":"","actions":["AXPress","AXConfirm"],"depth":0}"#;
    let element: AccessibilityElement = serde_json::from_str(json).unwrap();
    assert_eq!(element.value.unwrap().as_str(), "");
    assert_eq!(element.actions.len(), 2);
}

#[test]
fn health_data_is_typed_and_unknown_data_is_bounded_without_json_value() {
    let report = r#"{"schema_version":"1","platform":"darwin","driver_version":"0.28.2","overall":"ok","checks":[{"name":"platform_supported","status":"pass","message":"macOS 27.0 (arm64)","data":{"architecture":"arm64","os_version":"27.0"}},{"name":"bundle_identity","status":"pass","message":"Bundle is com.trycua.driver.","data":{"bundle_identifier":"com.trycua.driver","executable_path":"/Applications/CuaDriver.app/Contents/MacOS/cua-driver","identity_source":"current_process"}},{"name":"future_probe","status":"pass","message":"ok","data":{"attempts":1,"modes":["fast","safe"]}}]}"#;
    let result = CuaActionResult::HealthReport(serde_json::from_str(report).unwrap());
    let envelope = CuaResponseEnvelope {
        version: ProtocolVersion::V1,
        request_id: RequestId::try_from("r").unwrap(),
        machine_id: MachineId::try_from("m").unwrap(),
        action: CuaActionKind::HealthReport,
        response: CuaResponse::Success {
            result: Box::new(result),
        },
    };
    envelope.validate().unwrap();
}

#[test]
fn action_result_context_must_match_exact_request() {
    let address = ElementAddress::Point(WindowPoint { x: 1.0, y: 2.0 });
    let req = request(CuaAction::Click(ClickArgs {
        target: target(),
        session: None,
        delivery_mode: DeliveryMode::Background,
        address: address.clone(),
        button: MouseButton::Left,
        action: ClickAction::Press,
        modifiers: vec![],
        count: Some(1),
    }));
    let result = ClickActionResult {
        target: target(),
        session: None,
        address,
        button: MouseButton::Left,
        action: ClickAction::Press,
        count: Some(1),
        outcome: outcome(ActionEffect::Confirmed, ActionRoute::Accessibility),
    };
    response_for(&req, CuaActionResult::Click(result.clone()))
        .validate_response_for(&req)
        .unwrap();
    let mut forged = result;
    forged.address = ElementAddress::Point(WindowPoint { x: 9.0, y: 9.0 });
    assert!(
        response_for(&req, CuaActionResult::Click(forged))
            .validate_response_for(&req)
            .is_err()
    );
}

#[test]
fn native_background_policy_rejects_dom_and_successful_global_routes() {
    assert!(
        outcome(ActionEffect::Confirmed, ActionRoute::Dom)
            .validate()
            .is_err()
    );
    assert!(
        outcome(ActionEffect::Confirmed, ActionRoute::GlobalInput)
            .validate()
            .is_err()
    );
    outcome(ActionEffect::Unverifiable, ActionRoute::GlobalInput)
        .validate()
        .unwrap();
    outcome(ActionEffect::Refused, ActionRoute::TrustedInput)
        .validate()
        .unwrap();
}

#[test]
fn verification_remains_tri_state_and_correlated() {
    let req = request(CuaAction::VerifyState(VerifyStateArgs {
        target: target(),
        session: None,
        expect: vec![VerifyPredicate::Element(ElementPredicate {
            selector: ElementSelector {
                role: Some(BoundedText::try_from("AXTextField").unwrap()),
                label_contains: None,
            },
            condition: ElementCondition::ValueEquals(EmptyValueText::try_from("").unwrap()),
        })],
        include_screenshot: false,
        stable_samples: 1,
        timeout_ms: 0,
    }));
    let result = VerificationResult {
        overall: PredicateStatus::Unknown,
        predicates: vec![PredicateEvaluation {
            predicate_index: 0,
            status: PredicateStatus::Unknown,
        }],
        screenshot: None,
    };
    response_for(&req, CuaActionResult::VerifyState(result))
        .validate_response_for(&req)
        .unwrap();
}

#[test]
fn drag_scroll_and_type_text_results_reject_each_differing_option() {
    let drag_request = request(CuaAction::Drag(DragArgs {
        target: target(),
        session: None,
        delivery_mode: DeliveryMode::Background,
        from: WindowPoint { x: 1.0, y: 2.0 },
        to: WindowPoint { x: 3.0, y: 4.0 },
        duration_ms: 100,
        steps: 4,
        button: MouseButton::Left,
        modifiers: vec![],
    }));
    let drag_result = DragActionResult {
        target: target(),
        session: None,
        from: WindowPoint { x: 1.0, y: 2.0 },
        to: WindowPoint { x: 3.0, y: 4.0 },
        duration_ms: 100,
        steps: 4,
        button: MouseButton::Left,
        outcome: outcome(ActionEffect::Confirmed, ActionRoute::Accessibility),
    };
    response_for(&drag_request, CuaActionResult::Drag(drag_result.clone()))
        .validate_response_for(&drag_request)
        .unwrap();

    let mut wrong_duration = drag_result.clone();
    wrong_duration.duration_ms = 101;
    assert!(
        response_for(&drag_request, CuaActionResult::Drag(wrong_duration))
            .validate_response_for(&drag_request)
            .is_err()
    );
    let mut wrong_steps = drag_result.clone();
    wrong_steps.steps = 5;
    assert!(
        response_for(&drag_request, CuaActionResult::Drag(wrong_steps))
            .validate_response_for(&drag_request)
            .is_err()
    );
    let mut wrong_button = drag_result;
    wrong_button.button = MouseButton::Right;
    assert!(
        response_for(&drag_request, CuaActionResult::Drag(wrong_button))
            .validate_response_for(&drag_request)
            .is_err()
    );

    let address = ElementAddress::Point(WindowPoint { x: 1.0, y: 2.0 });
    let scroll_request = request(CuaAction::Scroll(ScrollArgs {
        target: target(),
        session: None,
        delivery_mode: DeliveryMode::Background,
        address: address.clone(),
        direction: ScrollDirection::Down,
        by: ScrollGranularity::Line,
        amount: 3,
    }));
    let scroll_result = ScrollActionResult {
        target: target(),
        session: None,
        address: address.clone(),
        direction: ScrollDirection::Down,
        by: ScrollGranularity::Line,
        amount: 3,
        outcome: outcome(ActionEffect::Confirmed, ActionRoute::Accessibility),
    };
    response_for(
        &scroll_request,
        CuaActionResult::Scroll(scroll_result.clone()),
    )
    .validate_response_for(&scroll_request)
    .unwrap();

    let mut wrong_granularity = scroll_result.clone();
    wrong_granularity.by = ScrollGranularity::Page;
    assert!(
        response_for(&scroll_request, CuaActionResult::Scroll(wrong_granularity))
            .validate_response_for(&scroll_request)
            .is_err()
    );
    let mut wrong_amount = scroll_result;
    wrong_amount.amount = 4;
    assert!(
        response_for(&scroll_request, CuaActionResult::Scroll(wrong_amount))
            .validate_response_for(&scroll_request)
            .is_err()
    );

    let type_request = request(CuaAction::TypeText(TypeTextArgs {
        target: target(),
        session: None,
        delivery_mode: DeliveryMode::Background,
        address: address.clone(),
        text: BoundedText::try_from("hello").unwrap(),
        delay_ms: 3,
    }));
    let type_result = TypeTextActionResult {
        target: target(),
        session: None,
        address,
        text: BoundedText::try_from("hello").unwrap(),
        delay_ms: 3,
        outcome: outcome(ActionEffect::Confirmed, ActionRoute::Accessibility),
    };
    response_for(
        &type_request,
        CuaActionResult::TypeText(type_result.clone()),
    )
    .validate_response_for(&type_request)
    .unwrap();

    let mut wrong_delay = type_result;
    wrong_delay.delay_ms = 4;
    assert!(
        response_for(&type_request, CuaActionResult::TypeText(wrong_delay))
            .validate_response_for(&type_request)
            .is_err()
    );
}

#[test]
fn canonical_filtered_health_reports_are_correlated_without_dropping_checks() {
    let include_request = request(CuaAction::HealthReport(HealthReportArgs {
        include: vec![health_name("binary_version")],
        skip: vec![],
    }));
    let included = r#"{
        "schema_version":"1","platform":"darwin","driver_version":"0.28.2","overall":"ok",
        "checks":[
            {"name":"binary_version","status":"pass","message":"cua-driver 0.28.2"},
            {"name":"platform_supported","status":"skip","message":"Skipped by include/skip filter."},
            {"name":"session_active","status":"skip","message":"Skipped by include/skip filter."},
            {"name":"bundle_identity","status":"skip","message":"Skipped by include/skip filter."},
            {"name":"tcc_accessibility","status":"skip","message":"Skipped by include/skip filter."},
            {"name":"tcc_screen_recording","status":"skip","message":"Skipped by include/skip filter."},
            {"name":"ax_capability","status":"skip","message":"Skipped by include/skip filter."},
            {"name":"screen_capture_capability","status":"skip","message":"Skipped by include/skip filter."}
        ]
    }"#;
    health_response(&include_request, included)
        .validate_response_for(&include_request)
        .unwrap();

    let skip_request = request(CuaAction::HealthReport(HealthReportArgs {
        include: vec![],
        skip: vec![health_name("bundle_identity")],
    }));
    let skipped = r#"{
        "schema_version":"1","platform":"darwin","driver_version":"0.28.2","overall":"ok",
        "checks":[
            {"name":"binary_version","status":"pass","message":"cua-driver 0.28.2"},
            {"name":"platform_supported","status":"pass","message":"macOS 27.0 (arm64)","data":{"architecture":"arm64","os_version":"27.0"}},
            {"name":"session_active","status":"pass","message":"MCP session is active."},
            {"name":"bundle_identity","status":"skip","message":"Skipped by include/skip filter."},
            {"name":"tcc_accessibility","status":"pass","message":"Accessibility is granted.","data":{"bundle_identifier":"com.trycua.driver"}},
            {"name":"tcc_screen_recording","status":"pass","message":"Screen Recording is granted.","data":{"bundle_identifier":"com.trycua.driver"}},
            {"name":"ax_capability","status":"pass","message":"AX is trusted and reachable."},
            {"name":"screen_capture_capability","status":"skip","message":"Direct capture was not probed."}
        ]
    }"#;
    health_response(&skip_request, skipped)
        .validate_response_for(&skip_request)
        .unwrap();
}

#[test]
fn filtered_health_check_rejects_pass_and_fail_statuses() {
    let request = request(CuaAction::HealthReport(HealthReportArgs {
        include: vec![health_name("binary_version")],
        skip: vec![],
    }));
    let canonical = r#"{
        "schema_version":"1","platform":"darwin","driver_version":"0.28.2","overall":"ok",
        "checks":[
            {"name":"binary_version","status":"pass","message":"cua-driver 0.28.2"},
            {"name":"platform_supported","status":"skip","message":"Skipped by include/skip filter."},
            {"name":"session_active","status":"skip","message":"Skipped by include/skip filter."},
            {"name":"bundle_identity","status":"skip","message":"Skipped by include/skip filter."},
            {"name":"tcc_accessibility","status":"skip","message":"Skipped by include/skip filter."},
            {"name":"tcc_screen_recording","status":"skip","message":"Skipped by include/skip filter."},
            {"name":"ax_capability","status":"skip","message":"Skipped by include/skip filter."},
            {"name":"screen_capture_capability","status":"skip","message":"Skipped by include/skip filter."}
        ]
    }"#;
    let mut report: HealthReportResult = serde_json::from_str(canonical).unwrap();

    report.checks[3].status = HealthCheckStatus::Pass;
    assert_eq!(
        response_for(&request, CuaActionResult::HealthReport(report))
            .validate_response_for(&request)
            .unwrap_err()
            .to_string(),
        "unrequested health check was not marked skipped"
    );

    let mut report: HealthReportResult = serde_json::from_str(canonical).unwrap();
    report.checks[3].status = HealthCheckStatus::Fail;
    report.overall = HealthOverall::Degraded;
    assert_eq!(
        response_for(&request, CuaActionResult::HealthReport(report))
            .validate_response_for(&request)
            .unwrap_err()
            .to_string(),
        "unrequested health check was not marked skipped"
    );
}

#[test]
fn filtered_health_report_cannot_drop_a_canonical_check() {
    let request = request(CuaAction::HealthReport(HealthReportArgs {
        include: vec![health_name("binary_version")],
        skip: vec![],
    }));
    let missing_filtered_check = r#"{
        "schema_version":"1","platform":"darwin","driver_version":"0.28.2","overall":"ok",
        "checks":[
            {"name":"binary_version","status":"pass","message":"cua-driver 0.28.2"},
            {"name":"platform_supported","status":"skip","message":"Skipped by include/skip filter."},
            {"name":"session_active","status":"skip","message":"Skipped by include/skip filter."},
            {"name":"tcc_accessibility","status":"skip","message":"Skipped by include/skip filter."},
            {"name":"tcc_screen_recording","status":"skip","message":"Skipped by include/skip filter."},
            {"name":"ax_capability","status":"skip","message":"Skipped by include/skip filter."},
            {"name":"screen_capture_capability","status":"skip","message":"Skipped by include/skip filter."}
        ]
    }"#;

    assert_eq!(
        health_response(&request, missing_filtered_check)
            .validate_response_for(&request)
            .unwrap_err()
            .to_string(),
        "filtered health report omitted a canonical check"
    );
}
