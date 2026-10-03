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
        .map(|record| record["binding"]["action"].clone())
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
    assert_failure(&output, "supports only M003K and M131E interfaces");
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
    assert!(help.contains("--software-mode independently enables dial software mode"));
}
