//! Shared fixtures that pin the Rust/TypeScript boundary. The frontend tests
//! read the same files. Regenerate with `UPDATE_FIXTURES=1 cargo test`.

use crate::{feed, replay, service, store, test_profiles};
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
