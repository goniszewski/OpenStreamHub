//! Shared fixtures that pin the Rust/TypeScript boundary. The frontend tests
//! read the same files. Regenerate with `UPDATE_FIXTURES=1 cargo test`.

use crate::{artwork, diagnostics, feed, replay, service, store, test_profiles};
use cooltek_sdk_service::Profile;
use serde_json::{Value, json};
use std::{fs, path::PathBuf};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../fixtures")
        .join(name)
}

fn check_or_update(name: &str, actual: &Value) {
    let path = fixture(name);
    if std::env::var_os("UPDATE_FIXTURES").is_some() {
        fs::write(&path, serde_json::to_string_pretty(actual).unwrap() + "\n").unwrap();
        return;
    }
    let expected: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(
        actual, &expected,
        "{name} is stale; regenerate with UPDATE_FIXTURES=1 cargo test and review the frontend contracts"
    );
}

#[test]
fn legacy_upgrade_matches_the_shared_parity_fixture() {
    let input = fs::read(fixture("legacy-upgrade-input.json")).unwrap();
    let upgraded = Profile::from_json(&input).unwrap().into_v2().unwrap();
    check_or_update(
        "legacy-upgrade-expected.json",
        &serde_json::to_value(&upgraded).unwrap(),
    );
}

#[test]
fn serialized_ipc_shapes_match_the_shared_contract_fixture() {
    let directory = tempfile::tempdir().unwrap();
    let profiles = store::ProfileStore::open(directory.path().join("profiles")).unwrap();
    let saved = profiles.save(None, test_profiles::folders(), None).unwrap();
    fs::write(
        directory
            .path()
            .join("profiles/00000000-0000-4000-8000-000000000001.json"),
        b"corrupt",
    )
    .unwrap();
    let bootstrap = profiles.bootstrap().unwrap();
    let keypad =
        replay::preview_keypad(test_profiles::folders(), vec!["root".into()], 0, None).unwrap();
    let replayed = replay::replay_demo(test_profiles::folders(), replay::DemoKind::Keys).unwrap();

    // Volatile identity and timestamps are normalized; every field is kept.
    let normalize = |mut value: Value| {
        let text = value
            .to_string()
            .replace(&saved.id, "00000000-0000-4000-8000-000000000000");
        value = serde_json::from_str(&text).unwrap();
        fn walk(value: &mut Value) {
            match value {
                Value::Object(map) => {
                    for (key, item) in map.iter_mut() {
                        if key == "modified_at_ms" {
                            *item = json!(0);
                        } else if key == "storage_path" {
                            *item = json!("/storage/profiles");
                        } else {
                            walk(item);
                        }
                    }
                }
                Value::Array(items) => items.iter_mut().for_each(walk),
                _ => {}
            }
        }
        walk(&mut value);
        value
    };
    check_or_update(
        "ipc-contracts.json",
        &normalize(json!({
            "stored_profile": saved,
            "app_bootstrap": bootstrap,
            "profile_listing": profiles.list().unwrap(),
            "keypad_preview": keypad,
            "replay_result": replayed,
        })),
    );
}

#[test]
fn device_feed_payloads_match_the_shared_fixtures() {
    let collect = |demo, stopped| {
        let source = feed::FeedSource::Demo { demo };
        let id = feed::feed_id(&source).unwrap();
        let mut messages = Vec::new();
        let stop = std::sync::atomic::AtomicBool::new(stopped);
        feed::run_feed(&id, &source, &stop, |message| messages.push(message));
        messages
    };
    let faders = collect(replay::DemoKind::Faders, false);
    let knobs = collect(replay::DemoKind::Knobs, false);
    let stopped = collect(replay::DemoKind::Dial, true);
    let first_input = |messages: &[feed::FeedMessage]| {
        messages
            .iter()
            .find_map(|message| match message {
                feed::FeedMessage::Input(input) => Some(input.clone()),
                feed::FeedMessage::Status(_) => None,
            })
            .unwrap()
    };
    let status = |message: &feed::FeedMessage| match message {
        feed::FeedMessage::Status(status) => status.clone(),
        feed::FeedMessage::Input(_) => panic!("expected a status payload"),
    };
    check_or_update(
        "feed-input-events.json",
        &json!([first_input(&faders), first_input(&knobs)]),
    );
    let failed = feed::StatusPayload {
        feed_id: "live:2f6465762f6869647261773130".into(),
        state: feed::FeedState::Failed,
        source: feed::SourceKind::Live,
        module: None,
        error: Some("could not open the module: example. Another program (for example masterhub-service) may be holding the device.".into()),
    };
    check_or_update(
        "feed-status-events.json",
        &json!([
            status(&faders[0]),
            status(faders.last().unwrap()),
            status(stopped.last().unwrap()),
            failed,
        ]),
    );
    let inventory = [(0x0202, Some("K1"), 0xff00), (0x0200, Some(" "), 0xff00)].map(
        |(product_id, serial, usage_page): (u16, Option<&str>, u16)| {
            cooltek_sdk_masterhub::ModuleIdentity {
                path: std::ffi::CString::new(format!("/dev/hidraw-{product_id:04x}")).unwrap(),
                vendor_id: 0x2516,
                product_id,
                serial_number: serial.map(str::to_owned).filter(|s| !s.trim().is_empty()),
                manufacturer_string: None,
                product_string: None,
                release_number: 0,
                interface_number: 0,
                usage_page,
                usage: 1,
                module: cooltek_sdk_masterhub::ModuleKind::from_ids(0x2516, product_id),
            }
        },
    );
    let live = feed::FeedSource::Live {
        path_hex: cooltek_sdk_cli::capture::encode_hex(b"/dev/hidraw-0202"),
    };
    check_or_update(
        "feed-command-results.json",
        &json!({
            "list_live_devices": feed::live_devices(&inventory),
            "start_feed": [
                feed::feed_id(&feed::FeedSource::Demo { demo: replay::DemoKind::Faders }).unwrap(),
                feed::feed_id(&live).unwrap(),
            ],
            "stop_feed": null,
        }),
    );
    // `device://inventory` payloads: two devices, then everything unplugged.
    check_or_update(
        "feed-inventory-event.json",
        &json!([
            feed::InventoryPayload {
                devices: feed::live_devices(&inventory),
            },
            feed::InventoryPayload { devices: vec![] },
        ]),
    );
}

#[test]
fn service_payloads_match_the_shared_fixtures() {
    let path_hex = cooltek_sdk_cli::capture::encode_hex(b"/dev/hidraw-0202");
    let request = json!({
        "profile": test_profiles::folders_json(),
        "path_hex": path_hex.to_uppercase(),
        "execute": true,
        "software_mode": false,
    });
    // The request deserializes as the command argument and ids are lowercase.
    let parsed: service::StartRequest = serde_json::from_value(request.clone()).unwrap();
    let id = service::service_id(&parsed.path_hex).unwrap();
    check_or_update("service-start-request.json", &request);
    check_or_update(
        "service-command-results.json",
        &json!({ "start_service": id, "stop_service": null }),
    );
    let status = |state| service::StatusPayload::new(&id, state, true);
    check_or_update(
        "service-status-events.json",
        &json!([
            status(service::ServiceState::Starting),
            status(service::ServiceState::Running),
            status(service::ServiceState::Stopping),
            service::StatusPayload::exited(&id, true, Some(0), String::new()),
            service::StatusPayload::exited(
                &id,
                true,
                Some(1),
                "masterhub-service: no HID interface matches the selected path\n".into(),
            ),
            service::StatusPayload::exited(&id, true, None, String::new()),
            service::StatusPayload::failed(
                &id,
                true,
                "could not find masterhub-service. Checked: /Applications/OpenStreamHub.app/Contents/MacOS/masterhub-service. Build it with `cargo build -p cooltek-sdk-cli --bin masterhub-service` or set OPEN_STREAM_HUB_SERVICE to its path.".into(),
            ),
        ]),
    );
    let record = |line: &[u8], truncated| service::record_payload(&id, line, truncated).unwrap();
    check_or_update(
        "service-record-events.json",
        &json!([
            record(br#"{"type":"service_start","mode":"dry_run"}"#, false),
            record(b"not json", false),
            record(&[b'a'; 600], true),
        ]),
    );
}

#[test]
fn session_payloads_match_the_shared_fixtures() {
    use crate::{power::PowerEvent, session};
    use cooltek_sdk_service::ServiceModule;
    use session::fake::{FakeRunner, device, exit, request};
    let dial = |path: &str| device(ServiceModule::M131E, Some("ABCDEF123456"), path);
    let start = |execute| {
        let mut driver = session::Driver::new(FakeRunner::default());
        driver.inventory(vec![dial("/dev/hidraw-0202")], 0);
        let id = driver
            .start(request(&dial("/dev/hidraw-0202"), execute), 0)
            .unwrap();
        (driver, id)
    };
    // Dry run: running, sleep, settle, wake, unplug, replug on a new path,
    // a crash with backoff, then a user stop.
    let (mut dry, id) = start(false);
    dry.power(PowerEvent::Suspend, 1_000);
    exit(&mut dry, &id, "", 1_100);
    dry.power(PowerEvent::Resume, 5_000);
    dry.tick(7_000);
    dry.inventory(vec![], 8_000);
    exit(
        &mut dry,
        &id,
        "masterhub-service: input read failed\n",
        8_100,
    );
    // The exit while unplugged counts as attempt 1 (1 s backoff).
    dry.inventory(vec![dial("/dev/hidraw-0303")], 9_100);
    let moved = format!(
        "service:{}",
        cooltek_sdk_cli::capture::encode_hex(b"/dev/hidraw-0303")
    );
    exit(
        &mut dry,
        &moved,
        "masterhub-service: no HID interface matches the selected path\n",
        10_000,
    );
    dry.tick(12_000);
    dry.stop(&id, true, 13_000);
    exit(&mut dry, &moved, "", 13_100);
    // Execute: a crash waits for the user's confirmation.
    let (mut executing, executing_id) = start(true);
    exit(&mut executing, &executing_id, "", 500);
    let mut events = dry.runner().emitted.clone();
    events.extend(executing.runner().emitted.iter().skip(1).cloned());
    check_or_update(
        "session-state-events.json",
        &serde_json::to_value(&events).unwrap(),
    );
    // Tauri maps snake_case command arguments to camelCase.
    check_or_update(
        "session-command-args.json",
        &json!({
            "resume_session": { "sessionId": executing_id, "profileGeneration": 0 },
            "stop_session": { "sessionId": id },
        }),
    );
    let stale = executing.resume(&executing_id, 1, 600).unwrap_err();
    executing.resume(&executing_id, 0, 600).unwrap();
    // `start_service` returns the session id. A start while asleep on a path
    // whose service id still names a session that moved away gets a
    // `session:<uuid>` name; its first state has no process yet.
    let (mut moved, moved_id) = start(false);
    moved.inventory(vec![], 1);
    exit(&mut moved, &moved_id, "", 2);
    moved.inventory(vec![dial("/dev/hidraw-0303")], 5_000);
    let other = device(
        ServiceModule::M131E,
        Some("FEDCBA654321"),
        "/dev/hidraw-0202",
    );
    moved.inventory(vec![dial("/dev/hidraw-0303"), other.clone()], 5_001);
    moved.power(PowerEvent::Suspend, 5_002);
    let asleep_id = moved.start(request(&other, false), 5_003).unwrap();
    let asleep_state = moved.runner().emitted.last().unwrap().clone();
    assert!(asleep_id.starts_with("session:") && asleep_state.session_id == asleep_id);
    let normalized = |value: Value| -> Value {
        serde_json::from_str(
            &value
                .to_string()
                .replace(&asleep_id, "session:00000000-0000-4000-8000-000000000000"),
        )
        .unwrap()
    };
    check_or_update(
        "session-command-results.json",
        &normalized(json!({
            "resume_session": null,
            // Refused: the profile changed after the confirmation was shown.
            "resume_session_profile_changed": stale,
            "start_service_asleep_collision": {
                "result": asleep_id,
                "first_state": asleep_state,
            },
            "stop_session": null,
        })),
    );
}

#[test]
fn diagnostics_payloads_match_the_shared_fixtures() {
    let fader_hex =
        cooltek_sdk_cli::capture::encode_hex(br"\\?\HID#VID_2516&PID_0200#FADER-SERIAL-0042");
    let request = json!({
        "redact_serials": true,
        "include_capture": true,
        "capture_seconds": 1,
        "include_profile": test_profiles::folders_json(),
        "client": {
            "activity": [
                { "kind": "service_status", "service_id": format!("service:{fader_hex}"),
                  "state": "exited", "exit_code": 1,
                  "stderr_tail": "masterhub-service: could not open FADER-SERIAL-0042\n" },
                { "kind": "feed_status", "feed_id": "demo:faders", "state": "finished" },
                { "kind": "service_record", "record": { "type": "binding_matched", "serial_number": "FADER-SERIAL-0042" } }
            ],
            "notes": "Faders stopped responding after replugging the hub; log at /Users/tester/Desktop/notes.txt",
        },
    });
    check_or_update("diagnostics-request.json", &request);
    let parsed: diagnostics::DiagnosticsRequest = serde_json::from_value(request).unwrap();
    let validated = diagnostics::validate(parsed).unwrap();
    let running = diagnostics::Running {
        feeds: vec!["demo:faders".into()],
        services: vec![format!("service:{fader_hex}")],
    };
    let held = |path: &[u8]| path == b"DevSrvsID:4294970002";
    let mut probe = diagnostics::fake::rig();
    let bundle = diagnostics::build_bundle(
        &diagnostics::BundleInputs {
            request: &validated,
            generated_at: 1_791_201_600,
            app: json!({
                "name": "open-stream-hub-app",
                "version": "0.1.0",
                "build": "debug",
                "target_os": "macos",
                "target_arch": "aarch64",
                "tauri_version": "2.12.1",
                "service_binary": { "path": "/Users/tester/cooltek-sdk/target/debug/masterhub-service" },
            }),
            environment: json!({
                "os": "macos", "os_family": "unix", "arch": "aarch64", "os_version": "macOS 27.0",
            }),
            home: Some("/Users/tester".into()),
            running: &running,
            in_use: &held,
        },
        &mut probe,
    );
    assert!(probe.commands.is_empty());
    check_or_update("diagnostics-bundle-redacted.json", &bundle.value);
    let (_, stamp) = diagnostics::utc_stamps(1_791_201_600);
    check_or_update(
        "diagnostics-result.json",
        &json!({
            "create_diagnostics": diagnostics::DiagnosticsResult {
                path: format!(
                    "/storage/{}/{}",
                    diagnostics::DIRECTORY,
                    diagnostics::file_name(&stamp)
                ),
                bytes: (serde_json::to_vec_pretty(&bundle.value).unwrap().len() + 1) as u64,
                summary: bundle.summary,
            },
        }),
    );
}

#[test]
fn profile_export_payloads_match_the_shared_fixtures() {
    let request = json!({ "profile": test_profiles::folders_json() });
    check_or_update("profile-export-request.json", &request);
    let directory = tempfile::tempdir().unwrap();
    let result =
        crate::export::export(&test_profiles::folders(), directory.path(), 1_791_201_600).unwrap();
    check_or_update(
        "profile-export-result.json",
        &json!({
            "export_profile": {
                "path": format!("/storage/{}", result.path.rsplit_once("/exports/").map(|(_, name)| format!("exports/{name}")).unwrap()),
                "bytes": result.bytes,
            },
        }),
    );
}

#[test]
fn artwork_payloads_match_the_shared_fixtures() {
    use base64::{Engine, engine::general_purpose::STANDARD};
    use image::{ExtendedColorType, ImageEncoder, codecs::png::PngEncoder};
    // A 2x2 PNG: red, green / blue, white.
    let pixels = [255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255];
    let mut source = Vec::new();
    PngEncoder::new(&mut source)
        .write_image(&pixels, 2, 2, ExtendedColorType::Rgb8)
        .unwrap();
    let prepare = json!({
        "image_base64": STANDARD.encode(&source),
        "target": "keypad",
        "quality": 90,
        "fit": "contain",
        "quarter_turns": 1,
        "background": [0, 0, 0],
    });
    check_or_update(
        "artwork-prepare-request.json",
        &json!({ "request": prepare }),
    );
    let request: artwork::PrepareRequest = serde_json::from_value(prepare.clone()).unwrap();
    let prepared = artwork::prepare_artwork(&request).unwrap();
    check_or_update(
        "artwork-prepare-result.json",
        &json!({ "prepare_artwork": prepared }),
    );
    let mut save = prepare;
    save["name"] = json!("Four squares");
    check_or_update("artwork-save-request.json", &json!({ "request": save }));
    let request: artwork::SaveRequest = serde_json::from_value(save).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let saved = artwork::save_artwork(&request, directory.path(), 1_791_201_600).unwrap();
    check_or_update(
        "artwork-save-result.json",
        &json!({
            "save_artwork": {
                "path": format!(
                    "/storage/{}",
                    saved.path.rsplit_once("/artwork/").map(|(_, name)| format!("artwork/{name}")).unwrap()
                ),
                "bytes": saved.bytes,
                "width": saved.width,
                "height": saved.height,
            }
        }),
    );
}

#[test]
fn focus_rule_payloads_match_the_shared_fixtures() {
    use crate::{focus, session};
    use cooltek_sdk_service::ServiceModule;
    use session::fake::{FakeRunner, device, exit, request};
    let profile_id = "00000000-0000-4000-8000-000000000000";
    let rule_id = "00000000-0000-4000-8000-0000000000f1";
    let rule = json!({ "id": null, "app_bundle_id": "com.apple.Safari", "profile_id": profile_id });
    // The argument deserializes as the command's input.
    let parsed: focus::RuleInput = serde_json::from_value(rule.clone()).unwrap();
    let saved = focus::upsert(focus::FocusRules::default(), parsed).unwrap();
    assert_eq!(saved.rules[0].app_bundle_id, "com.apple.Safari");
    let rules = focus::FocusRules {
        enabled: true,
        rules: vec![focus::FocusRule {
            id: rule_id.into(),
            app_bundle_id: "com.apple.Safari".into(),
            profile_id: profile_id.into(),
        }],
    };
    let session_id = format!(
        "service:{}",
        cooltek_sdk_cli::capture::encode_hex(b"/dev/hidraw-0202")
    );
    // Tauri maps snake_case command arguments to camelCase.
    check_or_update(
        "focus-command-args.json",
        &json!({
            "save_focus_rule": { "rule": rule },
            "save_focus_rule_edit": { "rule": { "id": rule_id, "app_bundle_id": "com.apple.Terminal", "profile_id": profile_id } },
            "delete_focus_rule": { "id": rule_id },
            "set_focus_rules_enabled": { "enabled": true },
            "switch_session_profile": { "sessionId": session_id, "profileId": profile_id },
        }),
    );
    let state = focus::FocusState::open(Err("unused".into()));
    let unmatched = state.observe(Some("com.apple.Terminal".into()), |_| true);
    let none = state.observe(None, |_| true);
    let matched = focus::FocusChanged {
        bundle_id: Some("com.apple.Safari".into()),
        rule_id: Some(rule_id.into()),
        profile_id: Some(profile_id.into()),
    };
    assert_eq!(
        focus::match_rule(&rules, Some("com.apple.Safari"), |_| true).map(|rule| &rule.id),
        Some(&rule_id.to_string())
    );
    check_or_update(
        "focus-changed-events.json",
        &json!([matched, unmatched, none]),
    );
    check_or_update(
        "focus-command-results.json",
        &json!({
            "list_focus_rules": rules,
            "list_focus_rules_default": focus::FocusRules::default(),
            // `supported` is false off macOS; the fixture pins the macOS shape.
            "focus_current_app": focus::CurrentApp { supported: true, ..state.current_app() },
            "delete_focus_rule": focus::FocusRules { enabled: true, rules: vec![] },
        }),
    );
    // An executing session paused by a profile switch.
    let dial = device(
        ServiceModule::M131E,
        Some("ABCDEF123456"),
        "/dev/hidraw-0202",
    );
    let mut driver = session::Driver::new(FakeRunner::default());
    driver.inventory(vec![dial.clone()], 0);
    let id = driver.start(request(&dial, true), 0).unwrap();
    let profile_generation = driver
        .switch_profile(&id, profile_id.into(), test_profiles::folders(), 100)
        .unwrap();
    exit(&mut driver, &id, "", 200);
    check_or_update(
        "focus-switch-result.json",
        &json!({
            "switch_session_profile": crate::SwitchedProfile {
                stored: store::StoredProfile {
                    id: profile_id.into(),
                    profile: test_profiles::folders(),
                    modified_at_ms: 0,
                },
                profile_generation,
            },
        }),
    );
    check_or_update(
        "session-switch-events.json",
        &serde_json::to_value(&driver.runner().emitted).unwrap(),
    );
}
