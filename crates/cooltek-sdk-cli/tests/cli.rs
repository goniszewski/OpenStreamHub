use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};

use serde_json::Value;

const FIXTURE: &str = include_str!("fixtures/m003k-synthetic.jsonl");
static NEXT_FILE: AtomicU64 = AtomicU64::new(0);

fn run(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_masterhub-diag"))
        .args(arguments)
        .output()
        .unwrap()
}

fn replay(path: &Path) -> Output {
    run(&["replay", path.to_str().unwrap()])
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

struct TempCapture(PathBuf);

impl TempCapture {
    fn new(text: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "masterhub-diag-test-{}-{}.jsonl",
            std::process::id(),
            NEXT_FILE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::write(&path, text).unwrap();
        Self(path)
    }
}

impl Drop for TempCapture {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn records() -> Vec<Value> {
    FIXTURE
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn capture(records: &[Value]) -> TempCapture {
    let text = records
        .iter()
        .map(|record| serde_json::to_string(record).unwrap() + "\n")
        .collect::<String>();
    TempCapture::new(&text)
}

fn assert_invalid(file: &TempCapture, expected: &str) {
    let output = replay(&file.0);
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(
        error.contains(expected),
        "expected {expected:?}, got {error}"
    );
    assert!(!error.contains("panicked"));
}

#[test]
fn help_describes_read_only_defaults_and_explicit_mode() {
    let output = run(&["--help"]);
    assert!(output.status.success());
    let help = String::from_utf8_lossy(&output.stdout);
    assert!(help.contains("Discovery never opens devices"));
    assert!(help.contains("Replay does not initialize HID"));
    let output = run(&["record", "--help"]);
    assert!(output.status.success());
    let help = String::from_utf8_lossy(&output.stdout);
    assert!(help.contains("--path-hex"));
    assert!(help.contains("--software-mode"));
    assert!(help.contains("existing files are never overwritten"));
}

#[test]
fn new_module_diagnostics_use_identity_and_preserve_unknown_and_malformed_reports() {
    for (name, pid, event_count) in [
        ("m4315-software-derived-synthetic.jsonl", 0x9997, 6),
        ("m005s-software-derived-synthetic.jsonl", 0x9991, 7),
        ("m032le-software-derived-synthetic.jsonl", 0x9995, 5),
    ] {
        let mut source: Vec<Value> = fs::read_to_string(fixture(name))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        source[0]["identity"]["product_id"] = serde_json::json!(pid);
        source[0]["identity"]["module"] = serde_json::json!("M003K");
        if pid == 0x9997 {
            source[0]["software_mode_requested"] = serde_json::json!(true);
        }
        let output = replay(&capture(&source).0);
        assert!(output.status.success(), "{name}: {output:?}");
        let rows: Vec<Value> = String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(
            rows.iter()
                .filter_map(|row| row["events"].as_array())
                .map(Vec::len)
                .sum::<usize>(),
            event_count
        );
        assert_eq!(
            rows.iter()
                .filter(|row| row["type"] == "parse_error")
                .count(),
            2
        );
        assert_eq!(
            rows.iter().filter(|row| row["type"] == "unknown").count(),
            1
        );
        assert_eq!(rows.last().unwrap()["type"], "capture_end");
    }
}

#[test]
fn replay_preserves_raw_identity_and_decodes_state_transitions() {
    let output = replay(&fixture("m003k-synthetic.jsonl"));
    assert!(output.status.success(), "{:?}", output);
    let text = String::from_utf8(output.stdout).unwrap();
    let output: Vec<Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(output.len(), 8);
    assert_eq!(output[0]["source"], "synthetic");
    assert_eq!(output[0]["identity"]["module"], "M003K");
    assert_eq!(output[0]["raw_hex"], "012a0500000301ff00");
    assert_eq!(output[0]["timestamp_micros"], 1000);
    assert_eq!(output[0]["events"][0]["knob_index"], 0);
    assert_eq!(output[0]["events"][0]["kind"]["direction"], "clockwise");
    assert_eq!(output[0]["events"][1]["knob_index"], 2);
    assert_eq!(
        output[0]["events"][1]["kind"]["direction"],
        "counter_clockwise"
    );
    assert_eq!(output[1]["events"][0]["kind"]["type"], "pressed");
    assert_eq!(output[2]["events"].as_array().unwrap().len(), 0);
    assert_eq!(output[3]["type"], "parse_error");
    assert_eq!(output[4]["events"][0]["kind"]["type"], "released");
    assert_eq!(output[4]["events"][1]["knob_index"], 2);
    assert_eq!(output[5]["type"], "unknown");
    assert_eq!(output[5]["raw_hex"], "012bffff");
    assert_eq!(output[6]["type"], "parse_error");
    assert_eq!(output[7]["type"], "capture_end");
    assert_eq!(output[7]["report_count"], 7);
    assert_eq!(output[7]["reason"], "max_reports");
}

#[test]
fn dial_reports_and_wrong_vendor_are_never_decoded_as_knobs() {
    let output = replay(&fixture("m131e-synthetic.jsonl"));
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    let item: Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
    assert_eq!(item["type"], "unknown");
    assert_eq!(item["raw_hex"], "012a0500000301ff00");
    assert_eq!(item["identity"]["module"], "M131E");
    let mut data = records();
    data[0]["identity"]["vendor_id"] = serde_json::json!(1);
    let file = capture(&data);
    let output = replay(&file.0);
    assert!(output.status.success());
    for line in String::from_utf8(output.stdout).unwrap().lines().take(7) {
        let item: Value = serde_json::from_str(line).unwrap();
        assert_eq!(item["type"], "unknown");
    }
}

#[test]
fn invalid_record_arguments_fail_in_clap_before_discovery() {
    for (option, value) in [
        ("--duration-secs", "0"),
        ("--duration-secs", "86401"),
        ("--timeout-ms", "0"),
        ("--timeout-ms", "-1"),
        ("--timeout-ms", "1001"),
        ("--max-reports", "0"),
        ("--path-hex", "abc"),
        ("--path-hex", "zz"),
        ("--path-hex", "00"),
        ("--path-hex", ""),
    ] {
        let mut arguments = vec!["record", "--output", "not-created.jsonl"];
        if option != "--path-hex" {
            arguments.extend(["--path-hex", "61"]);
        }
        arguments.extend([option, value]);
        let output = run(&arguments);
        assert_eq!(output.status.code(), Some(2), "{:?}", output);
    }
}

#[test]
fn rejects_missing_header_and_unknown_version() {
    assert_invalid(&TempCapture::new(""), "capture is empty");
    let mut data = records();
    data.remove(0);
    assert_invalid(&capture(&data), "expected a session header");
    let mut data = records();
    data[0]["schema_version"] = serde_json::json!(99);
    assert_invalid(&capture(&data), "unsupported schema_version");
}

#[test]
fn rejects_truncation_repeated_sessions_and_trailing_data() {
    let mut data = records();
    data.pop();
    assert_invalid(&capture(&data), "missing end record");
    assert_invalid(&TempCapture::new(FIXTURE.trim_end()), "missing newline");
    let mut data = records();
    data.insert(2, data[0].clone());
    assert_invalid(&capture(&data), "multiple sessions");
    let mut data = records();
    data.push(data[1].clone());
    assert_invalid(&capture(&data), "data after end record");
}

#[test]
fn rejects_corrupt_json_and_unsupported_schema_fields() {
    assert_invalid(&TempCapture::new("{oops}\n"), "invalid capture record");
    let mut data = records();
    data[1]["unrecognized"] = serde_json::json!(true);
    assert_invalid(&capture(&data), "unknown field");
}

#[test]
fn rejects_invalid_hex_empty_reports_and_nul_paths() {
    for raw_hex in ["f", "zz", "☃", ""] {
        let mut data = records();
        data[1]["raw_hex"] = serde_json::json!(raw_hex);
        let output = replay(&capture(&data).0);
        assert!(!output.status.success());
        assert!(!String::from_utf8_lossy(&output.stderr).contains("panicked"));
    }
    let mut data = records();
    data[0]["identity"]["path_hex"] = serde_json::json!("00");
    assert_invalid(&capture(&data), "no NUL bytes");
}

#[test]
fn rejects_report_descriptor_and_line_size_overflow() {
    let mut data = records();
    data[1]["raw_hex"] = serde_json::json!("ff".repeat(4097));
    assert_invalid(&capture(&data), "4096-byte limit");
    let mut data = records();
    data[0]["report_descriptor"] = serde_json::json!({
        "status":"available", "raw_hex":"ff".repeat(4097)
    });
    assert_invalid(&capture(&data), "4096-byte limit");
    data[0]["report_descriptor"] = serde_json::json!({
        "status":"available", "raw_hex":""
    });
    assert_invalid(&capture(&data), "available descriptor must contain");
    assert_invalid(
        &TempCapture::new(&(" ".repeat(65_537) + "\n")),
        "line-size limit",
    );
}

#[test]
fn rejects_backward_time_bad_counts_and_invalid_mode_metadata() {
    let mut data = records();
    data[2]["timestamp_micros"] = serde_json::json!(0);
    assert_invalid(&capture(&data), "nondecreasing");
    let mut data = records();
    data.last_mut().unwrap()["report_count"] = serde_json::json!(8);
    assert_invalid(&capture(&data), "differs from");
    let mut data = records();
    data[0]["software_mode_requested"] = serde_json::json!(true);
    assert_invalid(&capture(&data), "software mode is supported only");
}

#[test]
fn complete_transport_error_capture_remains_replayable() {
    let mut data = records();
    let footer = data.last_mut().unwrap();
    footer["reason"] = serde_json::json!("transport_error");
    footer["error"] = serde_json::json!("input read failed: device unplugged");
    let output = replay(&capture(&data).0);
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    let end: Value = serde_json::from_str(text.lines().last().unwrap()).unwrap();
    assert_eq!(end["type"], "capture_end");
    assert_eq!(end["reason"], "transport_error");
    assert_eq!(end["error"], "input read failed: device unplugged");
    let mut data = records();
    data.last_mut().unwrap()["reason"] = serde_json::json!("transport_error");
    assert_invalid(&capture(&data), "error is required only");
}

#[test]
fn interrupted_capture_replays_with_its_reason_and_empty_serial_is_accepted() {
    let mut data = records();
    data[0]["identity"]["serial_number"] = serde_json::json!("");
    let footer = data.last_mut().unwrap();
    footer["reason"] = serde_json::json!("interrupted");
    let output = replay(&capture(&data).0);
    assert!(output.status.success(), "{output:?}");
    let rows: Vec<Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let end = rows.last().unwrap();
    assert_eq!(end["type"], "capture_end");
    assert_eq!(end["reason"], "interrupted");
    assert!(end["error"].is_null());
    // An interrupted footer must not carry a transport error message.
    data.last_mut().unwrap()["error"] = serde_json::json!("unexpected");
    assert_invalid(
        &capture(&data),
        "error is required only for transport_error",
    );
}
