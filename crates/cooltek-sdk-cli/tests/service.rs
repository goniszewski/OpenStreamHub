use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};

use serde_json::{Value, json};

static NEXT_FILE: AtomicU64 = AtomicU64::new(0);

fn run(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_masterhub-service"))
        .args(arguments)
        .output()
        .unwrap()
}

fn example_profile() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/profiles/macos.json")
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn replay(path: &Path) -> Output {
    run(&[
        "replay",
        "--profile",
        example_profile().to_str().unwrap(),
        path.to_str().unwrap(),
    ])
}

fn output_records(output: &Output) -> Vec<Value> {
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn planned_actions(records: &[Value]) -> Vec<Value> {
    records
        .iter()
        .filter(|record| record["type"] == "event" && record["status"] == "planned")
        .map(|record| record["action"].clone())
        .collect()
}

struct TempFile(PathBuf);

impl TempFile {
    fn new(text: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "masterhub-service-test-{}-{}.json",
            std::process::id(),
            NEXT_FILE.fetch_add(1, Ordering::Relaxed),
        ));
        fs::write(&path, text).unwrap();
        Self(path)
    }

    fn records(records: &[Value]) -> Self {
        Self::new(
            &records
                .iter()
                .map(|record| serde_json::to_string(record).unwrap() + "\n")
                .collect::<String>(),
        )
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn capture_records(name: &str) -> Vec<Value> {
    fs::read_to_string(fixture(name))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn assert_failure(output: &Output, expected: &str) {
    assert!(!output.status.success(), "unexpected success: {output:?}");
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(
        error.contains(expected),
        "expected {expected:?}, got {error}"
    );
    assert!(!error.contains("panicked"));
}

#[test]
fn validates_example_profile_without_starting_a_service() {
    let output = run(&["validate", example_profile().to_str().unwrap()]);
    assert!(output.status.success(), "{output:?}");
    let records = output_records(&output);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["type"], "profile_valid");
    assert_eq!(records[0]["name"], "macOS controls");
    assert_eq!(records[0]["binding_count"], 9);
}

#[test]
fn nested_keypad_profile_validates_and_replay_keeps_navigation_releases_on_the_origin_page() {
    let profile =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/profiles/keypad-folders.json");
    let validated = run(&["validate", profile.to_str().unwrap()]);
    assert!(validated.status.success(), "{validated:?}");
    let summary = output_records(&validated);
    assert_eq!(summary[0]["schema_version"], 2);
    assert_eq!(summary[0]["binding_count"], 5);

    let original = capture_records("m4315-software-derived-synthetic.jsonl");
    let mut records = vec![original[0].clone()];
    // Root key 2 opens Development; its key 2 opens Git tools. Releasing
    // either folder must not run Git tools' release action at the same slot.
    // Key 3 executes its own release action, then key 1 goes Back twice.
    for (index, control) in [
        Some(1),
        None,
        Some(1),
        None,
        Some(2),
        None,
        Some(0),
        None,
        Some(0),
        None,
    ]
    .into_iter()
    .enumerate()
    {
        let mut raw = vec![1, 0x34, 17, 0, 2, 15];
        let mut states = [0u8; 15];
        if let Some(control) = control {
            states[control] = 1;
        }
        raw.extend_from_slice(&states);
        let hex: String = raw.iter().map(|byte| format!("{byte:02x}")).collect();
        records.push(json!({"type":"report","timestamp_micros":(index + 1) * 1000,"raw_hex":hex}));
    }
    records.push(
        json!({"type":"end","timestamp_micros":10001,"report_count":10,"reason":"max_reports"}),
    );
    let capture = TempFile::records(&records);
    let output = run(&[
        "replay",
        "--profile",
        profile.to_str().unwrap(),
        capture.0.to_str().unwrap(),
    ]);
    assert!(output.status.success(), "{output:?}");
    let rows = output_records(&output);
    assert_eq!(
        planned_actions(&rows),
        vec![json!({"type":"shortcut","key":"t","modifiers":["command"]})]
    );
    let depths: Vec<u64> = rows
        .iter()
        .filter(|row| row["status"] == "navigation")
        .map(|row| row["navigation"]["depth"].as_u64().unwrap())
        .collect();
    assert_eq!(depths, [1, 2, 1, 0]);
    let ending = rows.last().unwrap();
    assert_eq!(ending["event_count"], 10);
    assert_eq!(ending["action_count"], 1);
    assert_eq!(ending["execution"], "dry_run");
}

#[test]
fn all_new_modules_replay_offline_and_log_concrete_actions_under_both_pid_generations() {
    let profile =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/profiles/all-modules.json");
    for (name, aliases, expected_events, expected_actions) in [
        (
            "m4315-software-derived-synthetic.jsonl",
            [0x01fe, 0x9997],
            6,
            vec![
                json!({"type":"toggle_mute"}),
                json!({"type":"shortcut","key":"k","modifiers":["command"]}),
            ],
        ),
        (
            "m005s-software-derived-synthetic.jsonl",
            [0x0200, 0x9991],
            7,
            vec![
                json!({"type":"set_volume","percent":0}),
                json!({"type":"set_volume","percent":60}),
                json!({"type":"set_volume","percent":100}),
            ],
        ),
        (
            "m032le-software-derived-synthetic.jsonl",
            [0x0204, 0x9995],
            5,
            vec![
                json!({"type":"volume","delta":5}),
                json!({"type":"shortcut","key":"v","modifiers":["command"]}),
                json!({"type":"volume","delta":5}),
                json!({"type":"shortcut","key":"v","modifiers":["command"]}),
                json!({"type":"shortcut","key":"c","modifiers":["command"]}),
            ],
        ),
    ] {
        for product_id in aliases {
            let mut source = capture_records(name);
            source[0]["identity"]["product_id"] = json!(product_id);
            source[0]["identity"]["module"] = json!("M131E"); // VID/PID controls routing.
            let capture = TempFile::records(&source);
            let output = run(&[
                "replay",
                "--profile",
                profile.to_str().unwrap(),
                capture.0.to_str().unwrap(),
            ]);
            assert!(
                output.status.success(),
                "{name}, PID {product_id:x}: {output:?}"
            );
            let rows = output_records(&output);
            assert_eq!(planned_actions(&rows), expected_actions);
            assert_eq!(rows.last().unwrap()["event_count"], expected_events);
            assert_eq!(rows.last().unwrap()["action_count"], expected_actions.len());
            assert_eq!(
                rows.iter()
                    .filter(|row| row["status"] == "parse_error")
                    .count(),
                2
            );
            assert_eq!(
                rows.iter().filter(|row| row["status"] == "unknown").count(),
                1
            );
            assert!(!rows.iter().any(|row| row["type"] == "action_executed"));
        }
    }
}

#[test]
fn knob_replay_plans_actions_and_deduplicates_press_snapshots() {
    let output = replay(&fixture("m003k-synthetic.jsonl"));
    assert!(output.status.success(), "{output:?}");
    let records = output_records(&output);
    assert_eq!(records[0]["execution"], "dry_run");
    assert_eq!(
        planned_actions(&records),
        vec![
            json!({"type":"volume", "delta":5}),
            json!({"type":"toggle_mute"}),
            json!({"type":"shortcut", "key":"v", "modifiers":["command"]}),
        ],
    );
    assert_eq!(
        records
            .iter()
            .filter(|record| record["type"] == "report")
            .count(),
        7,
    );
    assert_eq!(
        records
            .iter()
            .filter(|record| record["status"] == "parse_error")
            .count(),
        2,
    );
    assert_eq!(
        records
            .iter()
            .filter(|record| record["status"] == "unknown")
            .count(),
        1,
    );
    let end = records.last().unwrap();
    assert_eq!(end["type"], "service_end");
    assert_eq!(end["action_count"], 3);
    assert_eq!(end["event_count"], 6);
    assert!(
        !records
            .iter()
            .any(|record| record["type"] == "action_executed")
    );
}

#[test]
fn dial_replay_repeats_rotations_and_preserves_short_long_gestures_through_bad_reports() {
    let output = replay(&fixture("m131e-software-derived-synthetic.jsonl"));
    assert!(output.status.success(), "{output:?}");
    let records = output_records(&output);
    assert_eq!(records[0]["execution"], "dry_run");
    assert_eq!(
        planned_actions(&records),
        vec![
            json!({"type":"volume", "delta":5}),
            json!({"type":"volume", "delta":5}),
            json!({"type":"volume", "delta":-5}),
            json!({"type":"launch_app", "bundle_id":"com.apple.finder"}),
            json!({"type":"toggle_mute"}),
        ],
    );
    assert_eq!(
        records
            .iter()
            .filter(|record| record["type"] == "report")
            .count(),
        16,
    );
    assert_eq!(
        records
            .iter()
            .filter(|record| record["status"] == "parse_error")
            .count(),
        1,
    );
    assert_eq!(
        records
            .iter()
            .filter(|record| record["status"] == "unknown")
            .count(),
        5,
    );
    let gestures: Vec<_> = records
        .iter()
        .filter(|record| {
            record["type"] == "event"
                && matches!(
                    record["event"]["gesture"].as_str(),
                    Some("short_press" | "long_press")
                )
        })
        .map(|record| {
            (
                record["event"]["gesture"].clone(),
                record["event"]["timestamp_micros"].clone(),
            )
        })
        .collect();
    assert_eq!(
        gestures,
        vec![
            (json!("long_press"), json!(610_000)),
            (json!("short_press"), json!(1_299_999)),
        ],
    );
    let end = records.last().unwrap();
    assert_eq!(end["type"], "service_end");
    assert_eq!(end["report_count"], 16);
    assert_eq!(end["action_count"], 5);
    assert!(
        !records
            .iter()
            .any(|record| record["type"] == "action_executed")
    );
}

#[test]
fn replay_routes_both_module_aliases_from_ids_even_with_misleading_model_labels() {
    for (name, aliases, misleading_label) in [
        ("m003k-synthetic.jsonl", [0x0202, 0x9996], "M131E"),
        (
            "m131e-software-derived-synthetic.jsonl",
            [0x0206, 0x9994],
            "M003K",
        ),
    ] {
        let baseline = planned_actions(&output_records(&replay(&fixture(name))));
        for product_id in aliases {
            let mut records = capture_records(name);
            records[0]["identity"]["product_id"] = json!(product_id);
            records[0]["identity"]["module"] = json!(misleading_label);
            let capture = TempFile::records(&records);
            let output = replay(&capture.0);
            assert!(output.status.success(), "PID {product_id:x}: {output:?}");
            assert_eq!(planned_actions(&output_records(&output)), baseline);
        }
    }
}

#[test]
fn replay_rejects_a_matching_product_id_from_another_vendor() {
    let mut records = capture_records("m003k-synthetic.jsonl");
    records[0]["identity"]["vendor_id"] = json!(0x1234);
    let capture = TempFile::records(&records);
    let output = replay(&capture.0);
    assert_failure(&output, "supports only the five control modules");
    assert!(output.stdout.is_empty());
}

#[test]
fn replay_rejects_truncation_and_never_reports_a_completed_capture() {
    let mut records = capture_records("m003k-synthetic.jsonl");
    records.pop();
    let capture = TempFile::records(&records);
    let output = replay(&capture.0);
    assert_failure(&output, "missing end record");
    let records = output_records(&output);
    assert!(!records.iter().any(|record| record["type"] == "capture_end"));
    assert_eq!(records.last().unwrap()["type"], "service_end");
    assert_eq!(records.last().unwrap()["reason"], "error");
    assert!(
        !records
            .iter()
            .any(|record| record["type"] == "action_executed")
    );
}

#[test]
fn replay_preserves_transport_error_footer_and_exits_unsuccessfully() {
    let mut records = capture_records("m003k-synthetic.jsonl");
    let footer = records.last_mut().unwrap();
    footer["reason"] = json!("transport_error");
    footer["error"] = json!("synthetic disconnect");
    let capture = TempFile::records(&records);
    let output = replay(&capture.0);
    assert_failure(&output, "synthetic disconnect");
    let records = output_records(&output);
    let end = records
        .iter()
        .find(|record| record["type"] == "capture_end")
        .unwrap();
    assert_eq!(end["reason"], "transport_error");
    assert_eq!(end["error"], "synthetic disconnect");
    assert_eq!(records.last().unwrap()["reason"], "error");
}

#[test]
fn replay_rejects_execute_flag() {
    let output = run(&[
        "replay",
        "--profile",
        example_profile().to_str().unwrap(),
        fixture("m003k-synthetic.jsonl").to_str().unwrap(),
        "--execute",
    ]);
    assert_failure(&output, "unexpected argument '--execute'");
    assert!(output.stdout.is_empty());
}

#[test]
fn run_rejects_invalid_profile_before_accessing_the_selected_path() {
    let profile = TempFile::new("{\"schema_version\":999,\"name\":\"invalid\",\"bindings\":[]}");
    let output = run(&[
        "run",
        "--profile",
        profile.0.to_str().unwrap(),
        "--path-hex",
        "ab",
        "--software-mode",
        "--execute",
    ]);
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("schema"), "{error}");
    assert!(
        !error.contains("selected path"),
        "profile must fail before HID: {error}"
    );
    assert!(!error.contains("--execute is supported only"), "{error}");
    assert!(output.stdout.is_empty());
}

#[test]
fn validate_rejects_ambiguous_selectors_and_unsupported_module_gestures() {
    let mut profile: Value = serde_json::from_slice(&fs::read(example_profile()).unwrap()).unwrap();
    let duplicate = profile["bindings"][0].clone();
    profile["bindings"].as_array_mut().unwrap().push(duplicate);
    let file = TempFile::new(&serde_json::to_string(&profile).unwrap());
    assert_failure(
        &run(&["validate", file.0.to_str().unwrap()]),
        "duplicate selector",
    );

    profile["bindings"].as_array_mut().unwrap().pop();
    profile["bindings"][0]["gesture"] = json!("right");
    let file = TempFile::new(&serde_json::to_string(&profile).unwrap());
    assert_failure(
        &run(&["validate", file.0.to_str().unwrap()]),
        "unsupported for this module",
    );
}

#[test]
fn run_rejects_invalid_paths_and_unbounded_timeout_values_during_argument_parsing() {
    for path in ["", "0", "gg", "00"] {
        let output = run(&[
            "run",
            "--profile",
            "unused-profile.json",
            "--path-hex",
            path,
        ]);
        assert_failure(&output, "invalid value");
        assert!(output.stdout.is_empty());
    }
    for arguments in [
        ["--timeout-ms", "0"],
        ["--timeout-ms", "1001"],
        ["--duration-secs", "0"],
        ["--duration-secs", "86401"],
        ["--max-reports", "0"],
    ] {
        let output = run(&[
            "run",
            "--profile",
            "unused-profile.json",
            "--path-hex",
            "ab",
            arguments[0],
            arguments[1],
        ]);
        assert_failure(&output, "invalid value");
        assert!(output.stdout.is_empty());
    }
}

#[test]
fn run_requires_an_explicit_interface_and_help_states_separate_opt_ins() {
    let output = run(&["run", "--profile", "unused-profile.json"]);
    assert_failure(&output, "--path-hex");
    let output = run(&["--help"]);
    assert!(output.status.success());
    let help = String::from_utf8_lossy(&output.stdout);
    assert!(help.contains("Replay always plans actions without HID or macOS access"));
    assert!(help.contains("--execute enables actions"));
    assert!(
        help.contains(
            "--software-mode independently enables software mode for the IPS dial or keys"
        )
    );
}
