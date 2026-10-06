use cooltek_sdk_cli::capture::{CaptureReader, EndReason, Source, decode_hex};
use cooltek_sdk_masterhub::ModuleKind;
use cooltek_sdk_service::{
    Action, ActionDispatcher, DeviceContext, DryRunDispatcher, Gesture, InputEvent, InputSession,
    InputUpdate, InputValue, KeypadNavigation, KeypadResolution, Profile, ProfileSession,
    ServiceModule,
};
use serde::{Deserialize, Serialize};
use std::io::Cursor;

const KNOBS: &str =
    include_str!("../../../../crates/cooltek-sdk-cli/tests/fixtures/m003k-synthetic.jsonl");
const DIAL: &str = include_str!(
    "../../../../crates/cooltek-sdk-cli/tests/fixtures/m131e-software-derived-synthetic.jsonl"
);
const KEYS: &str = include_str!(
    "../../../../crates/cooltek-sdk-cli/tests/fixtures/m4315-software-derived-synthetic.jsonl"
);
const FADERS: &str = include_str!(
    "../../../../crates/cooltek-sdk-cli/tests/fixtures/m005s-software-derived-synthetic.jsonl"
);
const ROLLERS: &str = include_str!(
    "../../../../crates/cooltek-sdk-cli/tests/fixtures/m032le-software-derived-synthetic.jsonl"
);

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DemoKind {
    Knobs,
    Dial,
    Keys,
    Faders,
    Rollers,
}

#[derive(Clone, Debug, Serialize)]
pub struct ReplayEvent {
    pub timestamp_us: u64,
    pub module: ServiceModule,
    pub control: u8,
    pub gesture: Gesture,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<InputValue>,
    pub serial_number: Option<String>,
    pub action: Option<Action>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub navigation: Option<KeypadNavigation>,
}

#[derive(Clone, Debug, Serialize)]
pub struct KeypadPreview {
    pub path: Vec<String>,
    pub depth: usize,
    pub events: [ReplayEvent; 2],
    pub planned_action_count: usize,
}

#[derive(Debug, Serialize)]
pub struct Diagnostic {
    pub timestamp_us: u64,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct ReplayResult {
    pub source: String,
    pub synthetic: bool,
    pub status: &'static str,
    pub report_count: u64,
    pub event_count: usize,
    pub planned_action_count: usize,
    pub events: Vec<ReplayEvent>,
    pub diagnostics: Vec<Diagnostic>,
}

/// Returns the bundled capture text and its display source for a demo.
pub fn demo_capture(kind: DemoKind) -> (&'static str, &'static str) {
    match kind {
        DemoKind::Knobs => (KNOBS, "Bundled M003K synthetic capture"),
        DemoKind::Dial => (DIAL, "Bundled M131E software-derived synthetic capture"),
        DemoKind::Keys => (KEYS, "Bundled M4315 software-derived synthetic capture"),
        DemoKind::Faders => (FADERS, "Bundled M005S software-derived synthetic capture"),
        DemoKind::Rollers => (ROLLERS, "Bundled M032LE software-derived synthetic capture"),
    }
}

/// Maps a recognized MasterHUB model to a module with service input support.
pub fn service_module(kind: Option<ModuleKind>) -> Option<ServiceModule> {
    match kind {
        Some(ModuleKind::M003K) => Some(ServiceModule::M003K),
        Some(ModuleKind::M131E) => Some(ServiceModule::M131E),
        Some(ModuleKind::M4315) => Some(ServiceModule::M4315),
        Some(ModuleKind::M005S) => Some(ServiceModule::M005S),
        Some(ModuleKind::M032LE) => Some(ServiceModule::M032LE),
        _ => None,
    }
}

pub fn replay_demo(profile: Profile, kind: DemoKind) -> Result<ReplayResult, String> {
    let (capture, source) = demo_capture(kind);
    replay_capture(profile, capture, source)
}

/// One bundled capture decoded by the shared service input session.
pub struct DecodedCapture {
    pub context: DeviceContext,
    pub report_count: u64,
    /// Each report's timestamp and decode outcome, in capture order.
    pub reports: Vec<(u64, Result<InputUpdate, String>)>,
}

/// Validates and decodes a complete synthetic capture without planning actions.
///
/// Malformed reports are retained as per-report errors; capture framing,
/// hex, and transport-error footers fail the whole capture.
pub fn decode_capture(capture: &str) -> Result<DecodedCapture, String> {
    let (mut reader, header) = CaptureReader::new(Cursor::new(capture.as_bytes()))?;
    if !matches!(header.source, Source::Synthetic) {
        return Err("demo replay requires a bundled synthetic capture".into());
    }
    let module = service_module(ModuleKind::from_ids(
        header.identity.vendor_id,
        header.identity.product_id,
    ))
    .ok_or("demo replay requires a supported MasterHUB module identity")?;
    let mut input = InputSession::new(module);
    let mut reports = Vec::new();
    while let Some(report) = reader.next_report()? {
        let raw = decode_hex(&report.raw_hex, InputSession::MAX_REPORT_BYTES)?;
        reports.push((
            report.timestamp_micros,
            input.decode(&raw, report.timestamp_micros),
        ));
    }
    let footer = reader
        .end()
        .ok_or_else(|| "capture is missing its final record".to_string())?;
    if footer.reason == EndReason::TransportError {
        return Err(format!(
            "capture ended with a transport error: {}",
            footer.error.as_deref().unwrap_or("unknown error")
        ));
    }
    Ok(DecodedCapture {
        context: DeviceContext {
            module,
            serial_number: header.identity.serial_number,
        },
        report_count: reports.len() as u64,
        reports,
    })
}

// Internal only: callers cannot choose a path, URL, or external capture text.
fn replay_capture(profile: Profile, capture: &str, source: &str) -> Result<ReplayResult, String> {
    validate_profile(&profile)?;
    let mut session = ProfileSession::new(profile)?;
    let decoded = decode_capture(capture)?;
    let context = decoded.context;
    let mut dispatcher = DryRunDispatcher::default();
    let mut events = Vec::new();
    let mut diagnostics = Vec::new();
    for (timestamp_us, update) in decoded.reports {
        match update {
            Ok(InputUpdate::Events(decoded)) => {
                for event in decoded {
                    events.push(plan_event(&mut session, &context, event, &mut dispatcher)?);
                }
            }
            Ok(InputUpdate::Unknown) => diagnostics.push(Diagnostic {
                timestamp_us,
                message: "Unknown input report; no action planned".into(),
            }),
            Err(message) => diagnostics.push(Diagnostic {
                timestamp_us,
                message: format!("Malformed input report: {message}"),
            }),
        }
    }
    Ok(ReplayResult {
        source: source.into(),
        synthetic: true,
        status: "completed",
        report_count: decoded.report_count,
        event_count: events.len(),
        planned_action_count: dispatcher.actions.len(),
        events,
        diagnostics,
    })
}

pub fn preview_binding(
    profile: Profile,
    module: ServiceModule,
    control: u8,
    gesture: Gesture,
    serial_number: Option<String>,
    value: Option<InputValue>,
) -> Result<ReplayEvent, String> {
    validate_profile(&profile)?;
    reject_blank_serial(&serial_number)?;
    let event = InputEvent {
        timestamp_micros: 0,
        control,
        gesture,
        value,
    };
    event.validate_for(module)?;
    let prime_release = profile.schema_version == 2
        && module == ServiceModule::M4315
        && gesture == Gesture::Released;
    let mut session = ProfileSession::new(profile)?;
    let context = DeviceContext {
        module,
        serial_number,
    };
    if prime_release {
        // The single-gesture preview has no preceding HID press. Prime the
        // shared origin tracking, then return only the requested release plan.
        // No dispatcher sees the priming action.
        session.resolve(
            &context,
            &InputEvent {
                gesture: Gesture::Pressed,
                ..event
            },
        )?;
    }
    plan_event(
        &mut session,
        &context,
        event,
        &mut DryRunDispatcher::default(),
    )
}

/// Preview serials are user-selected, so a blank one is a mistake. The service
/// crate would normalise it to "any serial", which would hide that mistake.
fn reject_blank_serial(serial_number: &Option<String>) -> Result<(), String> {
    match serial_number {
        Some(serial) if serial.trim().is_empty() => Err("serial number must not be blank".into()),
        _ => Ok(()),
    }
}

pub fn preview_keypad(
    profile: Profile,
    path: Vec<String>,
    control: u8,
    serial_number: Option<String>,
) -> Result<KeypadPreview, String> {
    validate_profile(&profile)?;
    reject_blank_serial(&serial_number)?;
    let mut session = ProfileSession::new(profile.into_v2()?)?;
    session.set_path(&path)?;
    let context = DeviceContext {
        module: ServiceModule::M4315,
        serial_number,
    };
    let mut dispatcher = DryRunDispatcher::default();
    let pressed = plan_event(
        &mut session,
        &context,
        InputEvent {
            timestamp_micros: 0,
            control,
            gesture: Gesture::Pressed,
            value: None,
        },
        &mut dispatcher,
    )?;
    let released = plan_event(
        &mut session,
        &context,
        InputEvent {
            timestamp_micros: 1,
            control,
            gesture: Gesture::Released,
            value: None,
        },
        &mut dispatcher,
    )?;
    Ok(KeypadPreview {
        path: session.path().to_vec(),
        depth: session.depth(),
        events: [pressed, released],
        planned_action_count: dispatcher.actions.len(),
    })
}

fn plan_event(
    session: &mut ProfileSession,
    context: &DeviceContext,
    event: InputEvent,
    dispatcher: &mut DryRunDispatcher,
) -> Result<ReplayEvent, String> {
    let (action, page_id, navigation) = match session.resolve(context, &event)? {
        KeypadResolution::Action { action, page_id } => (Some(action), page_id, None),
        KeypadResolution::Navigation { navigation } => (None, None, Some(navigation)),
        KeypadResolution::Unbound => (None, None, None),
    };
    if let Some(action) = &action {
        dispatcher.dispatch(action)?;
    }
    Ok(ReplayEvent {
        timestamp_us: event.timestamp_micros,
        module: context.module,
        control: event.control,
        gesture: event.gesture,
        value: event.value,
        serial_number: context.serial_number.clone(),
        action,
        page_id,
        navigation,
    })
}

pub fn validate_profile(profile: &Profile) -> Result<(), String> {
    profile.validate()?;
    let bytes = serde_json::to_vec(profile)
        .map_err(|error| format!("could not encode profile: {error}"))?;
    if bytes.len() > Profile::MAX_JSON_BYTES {
        return Err("profile exceeds the size limit".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_profiles;
    use serde_json::{Value, json};

    fn sample() -> Profile {
        Profile::from_json(include_bytes!("../../../../examples/profiles/macos.json")).unwrap()
    }

    fn rewrite(capture: &str, change: impl FnOnce(&mut Vec<Value>)) -> String {
        let mut records = capture
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        change(&mut records);
        records
            .iter()
            .map(|record| serde_json::to_string(record).unwrap() + "\n")
            .collect()
    }

    #[test]
    fn raw_knob_demo_uses_shared_decoder_deduplication_and_binding_engine() {
        let result = replay_demo(sample(), DemoKind::Knobs).unwrap();
        assert_eq!(
            (
                result.report_count,
                result.event_count,
                result.planned_action_count,
                result.diagnostics.len()
            ),
            (7, 6, 3, 3)
        );
        assert_eq!(
            result
                .events
                .iter()
                .filter_map(|event| event.action.clone())
                .collect::<Vec<_>>(),
            vec![
                Action::Volume { delta: 5 },
                Action::ToggleMute,
                Action::Shortcut {
                    key: "v".into(),
                    modifiers: vec![cooltek_sdk_service::Modifier::Command]
                },
            ]
        );
        assert_eq!(
            result
                .events
                .iter()
                .filter(|event| event.gesture == Gesture::Pressed)
                .count(),
            2
        );
        assert_eq!(result.status, "completed");
        assert!(result.synthetic);
    }

    #[test]
    fn raw_dial_demo_retains_threshold_timing_and_diagnostics() {
        let result = replay_demo(sample(), DemoKind::Dial).unwrap();
        assert_eq!(
            (
                result.report_count,
                result.event_count,
                result.planned_action_count,
                result.diagnostics.len()
            ),
            (16, 9, 5, 6)
        );
        let releases: Vec<_> = result
            .events
            .iter()
            .filter(|event| {
                matches!(
                    event.gesture,
                    Gesture::Released | Gesture::ShortPress | Gesture::LongPress
                )
            })
            .map(|event| (event.timestamp_us, event.gesture))
            .collect();
        assert_eq!(
            releases,
            vec![
                (610_000, Gesture::Released),
                (610_000, Gesture::LongPress),
                (1_299_999, Gesture::Released),
                (1_299_999, Gesture::ShortPress)
            ]
        );
        assert_eq!(
            result
                .events
                .iter()
                .filter(|event| event.gesture == Gesture::Right)
                .count(),
            2
        );
    }

    #[test]
    fn invalid_capture_never_returns_a_completed_result() {
        for bad in [
            rewrite(KNOBS, |records| {
                records.pop();
            }),
            rewrite(KNOBS, |records| {
                records[1]["timestamp_micros"] = json!(8000);
            }),
            rewrite(KNOBS, |records| {
                records.last_mut().unwrap()["report_count"] = json!(8);
            }),
            rewrite(KNOBS, |records| {
                records.last_mut().unwrap()["reason"] = json!("transport_error");
                records.last_mut().unwrap()["error"] = json!("disconnect");
            }),
            rewrite(KNOBS, |records| {
                records[0]["source"] = json!("hardware");
            }),
            rewrite(KNOBS, |records| {
                records[0]["identity"]["vendor_id"] = json!(0x1234);
            }),
            rewrite(KNOBS, |records| {
                records[1]["raw_hex"] = json!("zz");
            }),
            format!("{KNOBS}\n"),
            KNOBS.trim_end().to_owned(),
        ] {
            assert!(
                replay_capture(sample(), &bad, "test").is_err(),
                "unexpectedly completed: {bad}"
            );
        }
    }

    #[test]
    fn preview_preserves_serial_precedence_and_unbound_events() {
        let mut profile = sample();
        let mut exact = profile.bindings[0].clone();
        exact.serial_number = Some("unit-1".into());
        exact.action = Action::Volume { delta: 11 };
        profile.bindings.push(exact);
        for (serial, delta) in [(None, 5), (Some("other"), 5), (Some("unit-1"), 11)] {
            let result = preview_binding(
                profile.clone(),
                ServiceModule::M003K,
                0,
                Gesture::Clockwise,
                serial.map(str::to_owned),
                None,
            )
            .unwrap();
            assert_eq!(result.action, Some(Action::Volume { delta }));
        }
        assert!(
            preview_binding(
                profile.clone(),
                ServiceModule::M003K,
                1,
                Gesture::Clockwise,
                None,
                None
            )
            .unwrap()
            .action
            .is_none()
        );
        assert!(
            preview_binding(
                profile.clone(),
                ServiceModule::M003K,
                3,
                Gesture::Clockwise,
                None,
                None
            )
            .is_err()
        );
        assert!(
            preview_binding(
                profile.clone(),
                ServiceModule::M003K,
                0,
                Gesture::Right,
                None,
                None
            )
            .is_err()
        );
        assert!(
            preview_binding(
                profile,
                ServiceModule::M003K,
                0,
                Gesture::Clockwise,
                Some(String::new()),
                None
            )
            .is_err()
        );
    }

    #[test]
    fn invalid_profiles_cannot_replay_or_preview() {
        let mut profile = sample();
        profile.schema_version = 999;
        assert!(replay_demo(profile.clone(), DemoKind::Knobs).is_err());
        assert!(
            preview_binding(
                profile,
                ServiceModule::M003K,
                0,
                Gesture::Clockwise,
                None,
                None
            )
            .is_err()
        );
    }

    #[test]
    fn serialized_events_match_the_frontend_contract() {
        let event = preview_binding(
            sample(),
            ServiceModule::M003K,
            0,
            Gesture::Pressed,
            None,
            None,
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(event).unwrap(),
            json!({
                "timestamp_us": 0, "module": "m003k", "control": 0, "gesture": "pressed",
                "serial_number": null, "action": { "type": "toggle_mute" },
            })
        );
        assert!(serde_json::from_str::<DemoKind>("\"external\"").is_err());
    }

    fn all_modules() -> Profile {
        Profile::from_json(include_bytes!(
            "../../../../examples/profiles/all-modules.json"
        ))
        .unwrap()
    }

    #[test]
    fn new_module_captures_use_shared_decoders_and_resolved_actions() {
        for (kind, expected) in [
            (DemoKind::Keys, (8, 6, 2, 3)),
            (DemoKind::Faders, (8, 7, 3, 3)),
            (DemoKind::Rollers, (7, 5, 5, 3)),
        ] {
            let result = replay_demo(all_modules(), kind).unwrap();
            assert_eq!(
                (
                    result.report_count,
                    result.event_count,
                    result.planned_action_count,
                    result.diagnostics.len()
                ),
                expected
            );
            assert!(
                result
                    .events
                    .iter()
                    .all(|event| event.action != Some(Action::VolumeFromPosition))
            );
            assert!(result.synthetic);
        }
        let result = replay_demo(all_modules(), DemoKind::Faders).unwrap();
        let planned = result
            .events
            .iter()
            .filter_map(|event| event.action.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            planned,
            [
                Action::SetVolume { percent: 0 },
                Action::SetVolume { percent: 60 },
                Action::SetVolume { percent: 100 }
            ]
        );
    }

    #[test]
    fn fader_preview_validates_scale_and_returns_a_resolved_absolute_action() {
        let value = Some(InputValue::Position {
            raw: 37,
            maximum: 100,
        });
        let event = preview_binding(
            all_modules(),
            ServiceModule::M005S,
            0,
            Gesture::PositionChanged,
            None,
            value,
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(event).unwrap(),
            json!({
                "timestamp_us": 0, "module": "m005s", "control": 0, "gesture": "position_changed",
                "value": {"type": "position", "raw": 37, "maximum": 100},
                "serial_number": null, "action": {"type": "set_volume", "percent": 37},
            })
        );
        for invalid in [
            None,
            Some(InputValue::Position {
                raw: 101,
                maximum: 100,
            }),
            Some(InputValue::Position {
                raw: 37,
                maximum: 255,
            }),
        ] {
            assert!(
                preview_binding(
                    all_modules(),
                    ServiceModule::M005S,
                    0,
                    Gesture::PositionChanged,
                    None,
                    invalid
                )
                .is_err()
            );
        }
        assert!(
            preview_binding(
                all_modules(),
                ServiceModule::M4315,
                0,
                Gesture::Pressed,
                None,
                value
            )
            .is_err()
        );
        assert!(
            preview_binding(
                all_modules(),
                ServiceModule::M005S,
                5,
                Gesture::PositionChanged,
                None,
                value
            )
            .is_err()
        );
        assert!(
            preview_binding(
                all_modules(),
                ServiceModule::M005S,
                0,
                Gesture::PositionChanged,
                Some(String::new()),
                value
            )
            .is_err()
        );
    }

    #[test]
    fn folder_preview_and_back_use_complete_pairs_without_child_release_actions() {
        let child = preview_keypad(test_profiles::folders(), vec!["root".into()], 0, None).unwrap();
        assert_eq!(child.path, ["root", "tools"]);
        assert_eq!(child.depth, 1);
        assert_eq!(child.planned_action_count, 0);
        let navigation = child.events[0].navigation.as_ref().unwrap();
        assert_eq!(navigation.from_page, "root");
        assert_eq!(navigation.to_page, "tools");
        assert!(child.events[1].action.is_none());
        assert!(child.events[1].navigation.is_none());
        let deeper = preview_keypad(test_profiles::folders(), child.path, 1, None).unwrap();
        assert_eq!(deeper.path, ["root", "tools", "deeper"]);
        assert_eq!(deeper.depth, 2);
        let back = preview_keypad(test_profiles::folders(), deeper.path, 0, None).unwrap();
        assert_eq!(back.path, ["root", "tools"]);
        assert_eq!(back.depth, 1);
        assert_eq!(back.planned_action_count, 0);
        assert_eq!(back.events[0].navigation.as_ref().unwrap().to_page, "tools");
        assert!(back.events[1].navigation.is_none());
        assert!(back.events.iter().all(|event| event.action.is_none()));
    }

    #[test]
    fn keypad_preview_plans_both_origin_actions_and_exact_serial_precedence() {
        for (serial, pressed_action) in [
            (
                None,
                Action::LaunchApp {
                    bundle_id: "com.apple.calculator".into(),
                },
            ),
            (Some("unit-1"), Action::Volume { delta: 11 }),
        ] {
            let result = preview_keypad(
                test_profiles::folders(),
                vec!["root".into(), "tools".into()],
                2,
                serial.map(str::to_owned),
            )
            .unwrap();
            assert_eq!(result.planned_action_count, 2);
            assert_eq!(result.events[0].action, Some(pressed_action));
            assert_eq!(result.events[1].action, Some(Action::Volume { delta: -2 }));
            assert!(
                result
                    .events
                    .iter()
                    .all(|event| event.page_id.as_deref() == Some("tools"))
            );
            assert!(result.events.iter().all(|event| event.navigation.is_none()));
            assert_eq!(result.path, ["root", "tools"]);
        }
        let value = serde_json::to_value(
            preview_keypad(test_profiles::folders(), vec!["root".into()], 0, None).unwrap(),
        )
        .unwrap();
        assert_eq!(
            value["events"][0]["navigation"],
            json!({
                "from_page": "root", "to_page": "tools", "path": ["root", "tools"], "depth": 1,
            })
        );
        assert_eq!(value["events"][1]["gesture"], "released");
        assert_eq!(value["planned_action_count"], 0);
    }

    #[test]
    fn invalid_preview_paths_selectors_and_serials_fail_before_planning() {
        for path in [
            vec![],
            vec!["tools".into()],
            vec!["root".into(), "deeper".into()],
            vec!["root".into(), "missing".into()],
            vec!["root".into(), "tools".into(), "root".into()],
        ] {
            assert!(preview_keypad(test_profiles::folders(), path, 0, None).is_err());
        }
        assert!(preview_keypad(test_profiles::folders(), vec!["root".into()], 15, None).is_err());
        assert!(
            preview_keypad(
                test_profiles::folders(),
                vec!["root".into()],
                0,
                Some(String::new())
            )
            .is_err()
        );
    }

    #[test]
    fn single_release_preview_primes_shared_origin_without_planning_the_press() {
        let event = preview_binding(
            test_profiles::folders(),
            ServiceModule::M4315,
            14,
            Gesture::Released,
            None,
            None,
        )
        .unwrap();
        assert_eq!(event.action, Some(Action::ToggleMute));
        assert_eq!(event.page_id.as_deref(), Some("root"));
        assert!(event.navigation.is_none());
        let folder_release = preview_binding(
            test_profiles::folders(),
            ServiceModule::M4315,
            0,
            Gesture::Released,
            None,
            None,
        )
        .unwrap();
        assert!(folder_release.action.is_none());
        assert!(folder_release.navigation.is_none());
    }

    fn key_capture(snapshots: &[&[usize]]) -> String {
        let mut records =
            vec![serde_json::from_str::<Value>(KEYS.lines().next().unwrap()).unwrap()];
        for (index, snapshot) in snapshots.iter().enumerate() {
            let mut raw = vec![1, 0x34, 17, 0, 2, 15];
            raw.extend_from_slice(&[0; 15]);
            for &key in snapshot.iter() {
                raw[6 + key] = 1;
            }
            records.push(
                json!({ "type": "report", "timestamp_micros": (index + 1) * 1000,
                "raw_hex": raw.iter().map(|byte| format!("{byte:02x}")).collect::<String>() }),
            );
        }
        records.push(
            json!({ "type": "end", "timestamp_micros": snapshots.len() * 1000 + 1,
            "report_count": snapshots.len(), "reason": "max_reports" }),
        );
        records
            .iter()
            .map(|record| serde_json::to_string(record).unwrap() + "\n")
            .collect()
    }

    #[test]
    fn raw_replay_retains_origin_release_after_back_navigation() {
        let capture = key_capture(&[&[0], &[], &[2], &[0, 2], &[]]);
        let result =
            replay_capture(test_profiles::folders(), &capture, "synthetic folder test").unwrap();
        assert_eq!(result.report_count, 5);
        assert_eq!(result.event_count, 6);
        assert_eq!(result.planned_action_count, 2);
        assert!(result.diagnostics.is_empty());
        let navigation: Vec<_> = result
            .events
            .iter()
            .filter_map(|event| event.navigation.as_ref())
            .map(|navigation| navigation.to_page.as_str())
            .collect();
        assert_eq!(navigation, ["tools", "root"]);
        let actions: Vec<_> = result
            .events
            .iter()
            .filter_map(|event| {
                event
                    .action
                    .as_ref()
                    .map(|action| (action, event.page_id.as_deref()))
            })
            .collect();
        assert_eq!(
            actions,
            [
                (
                    &Action::LaunchApp {
                        bundle_id: "com.apple.calculator".into()
                    },
                    Some("tools")
                ),
                (&Action::Volume { delta: -2 }, Some("tools")),
            ]
        );
        assert_eq!(result.status, "completed");
    }

    #[test]
    fn migrated_root_replay_preserves_flat_actions_and_capture_diagnostics() {
        let flat = replay_demo(all_modules(), DemoKind::Keys).unwrap();
        let migrated = replay_demo(all_modules().into_v2().unwrap(), DemoKind::Keys).unwrap();
        assert_eq!(migrated.report_count, flat.report_count);
        assert_eq!(migrated.event_count, flat.event_count);
        assert_eq!(migrated.planned_action_count, flat.planned_action_count);
        assert_eq!(migrated.diagnostics.len(), flat.diagnostics.len());
        assert_eq!(
            migrated
                .events
                .iter()
                .filter_map(|event| event.action.clone())
                .collect::<Vec<_>>(),
            flat.events
                .iter()
                .filter_map(|event| event.action.clone())
                .collect::<Vec<_>>()
        );
        assert!(
            migrated
                .events
                .iter()
                .filter(|event| event.action.is_some())
                .all(|event| event.page_id.as_deref() == Some("root"))
        );
    }
}
