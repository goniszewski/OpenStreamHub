use cooltek_sdk_cli::capture::{CaptureReader, EndReason, Source, decode_hex};
use cooltek_sdk_masterhub::ModuleKind;
use cooltek_sdk_service::{
    Action, ActionDispatcher, Binding, BindingEngine, DeviceContext, DryRunDispatcher, Gesture,
    InputEvent, InputSession, InputUpdate, InputValue, Profile, ServiceModule,
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

#[derive(Clone, Copy, Debug, Deserialize)]
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

pub fn replay_demo(profile: Profile, kind: DemoKind) -> Result<ReplayResult, String> {
    let (capture, source) = match kind {
        DemoKind::Knobs => (KNOBS, "Bundled M003K synthetic capture"),
        DemoKind::Dial => (DIAL, "Bundled M131E software-derived synthetic capture"),
        DemoKind::Keys => (KEYS, "Bundled M4315 software-derived synthetic capture"),
        DemoKind::Faders => (FADERS, "Bundled M005S software-derived synthetic capture"),
        DemoKind::Rollers => (ROLLERS, "Bundled M032LE software-derived synthetic capture"),
    };
    replay_capture(profile, capture, source)
}

// Internal only: callers cannot choose a path, URL, or external capture text.
fn replay_capture(profile: Profile, capture: &str, source: &str) -> Result<ReplayResult, String> {
    validate_profile(&profile)?;
    let engine = BindingEngine::new(profile)?;
    let (mut reader, header) = CaptureReader::new(Cursor::new(capture.as_bytes()))?;
    if !matches!(header.source, Source::Synthetic) {
        return Err("demo replay requires a bundled synthetic capture".into());
    }
    let module = match ModuleKind::from_ids(header.identity.vendor_id, header.identity.product_id) {
        Some(ModuleKind::M003K) => ServiceModule::M003K,
        Some(ModuleKind::M131E) => ServiceModule::M131E,
        Some(ModuleKind::M4315) => ServiceModule::M4315,
        Some(ModuleKind::M005S) => ServiceModule::M005S,
        Some(ModuleKind::M032LE) => ServiceModule::M032LE,
        _ => return Err("demo replay requires a supported MasterHUB module identity".into()),
    };
    let context = DeviceContext {
        module,
        serial_number: header.identity.serial_number,
    };
    let mut input = InputSession::new(module);
    let mut dispatcher = DryRunDispatcher::default();
    let mut events = Vec::new();
    let mut diagnostics = Vec::new();
    let mut report_count = 0;
    while let Some(report) = reader.next_report()? {
        report_count += 1;
        let raw = decode_hex(&report.raw_hex, InputSession::MAX_REPORT_BYTES)?;
        match input.decode(&raw, report.timestamp_micros) {
            Ok(InputUpdate::Events(decoded)) => {
                for event in decoded {
                    events.push(plan_event(&engine, &context, event, &mut dispatcher)?);
                }
            }
            Ok(InputUpdate::Unknown) => diagnostics.push(Diagnostic {
                timestamp_us: report.timestamp_micros,
                message: "Unknown input report; no action planned".into(),
            }),
            Err(message) => diagnostics.push(Diagnostic {
                timestamp_us: report.timestamp_micros,
                message: format!("Malformed input report: {message}"),
            }),
        }
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
    Ok(ReplayResult {
        source: source.into(),
        synthetic: true,
        status: "completed",
        report_count,
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
    let event = InputEvent {
        timestamp_micros: 0,
        control,
        gesture,
        value,
    };
    event.validate_for(module)?;
    if serial_number.is_some() {
        // Keep exact-serial constraints owned by the shared profile validator.
        Profile {
            schema_version: 1,
            name: "Preview selector".into(),
            bindings: vec![Binding {
                module,
                control,
                gesture,
                serial_number: serial_number.clone(),
                action: if module == ServiceModule::M005S {
                    Action::VolumeFromPosition
                } else {
                    Action::ToggleMute
                },
            }],
        }
        .validate()?;
    }
    let engine = BindingEngine::new(profile)?;
    let context = DeviceContext {
        module,
        serial_number,
    };
    plan_event(&engine, &context, event, &mut DryRunDispatcher::default())
}

fn plan_event(
    engine: &BindingEngine,
    context: &DeviceContext,
    event: InputEvent,
    dispatcher: &mut DryRunDispatcher,
) -> Result<ReplayEvent, String> {
    let action = engine.action_for(context, &event)?;
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
}
