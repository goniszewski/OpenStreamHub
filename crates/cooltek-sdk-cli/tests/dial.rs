use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};

use serde_json::{Value, json};

const DIAL_CAPTURE: &str = include_str!("fixtures/m131e-software-derived-synthetic.jsonl");
static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

fn run(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_masterhub-diag"))
        .args(arguments)
        .output()
        .unwrap()
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

struct TempDirectory(PathBuf);

impl TempDirectory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "masterhub-dial-test-{}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }

    fn capture(&self, records: &[Value]) -> PathBuf {
        let path = self.path("capture.jsonl");
        let text = records
            .iter()
            .map(|record| serde_json::to_string(record).unwrap() + "\n")
            .collect::<String>();
        fs::write(&path, text).unwrap();
        path
    }
}

impl Drop for TempDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn parse_records(text: &str) -> Vec<Value> {
    text.lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn stdout_records(output: &Output) -> Vec<Value> {
    assert!(
        output.status.success(),
        "command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    parse_records(std::str::from_utf8(&output.stdout).unwrap())
}

fn decode_hex(hex: &str) -> Vec<u8> {
    assert!(hex.len().is_multiple_of(2));
    hex.as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}

#[test]
fn dial_replay_preserves_raw_fields_and_button_state_through_unknowns_and_errors() {
    let output = run(&[
        "replay",
        fixture("m131e-software-derived-synthetic.jsonl")
            .to_str()
            .unwrap(),
    ]);
    let records = stdout_records(&output);
    assert_eq!(records.len(), 17);
    assert_eq!(records[0]["source"], "synthetic");
    assert_eq!(records[0]["identity"]["product_id"], 0x9994);
    assert_eq!(records[0]["raw_hex"], "0144040003aa0055");
    assert_eq!(records[0]["timestamp_micros"], 1000);
    let right = json!([{ "type": "rotated", "direction": "right" }]);
    assert_eq!(records[0]["events"], right);
    assert_eq!(records[1]["events"], right);
    assert_eq!(
        records[2]["events"],
        json!([{ "type": "rotated", "direction": "left" }])
    );
    assert_eq!(records[3]["events"], json!([{ "type": "pressed" }]));
    assert_eq!(records[4]["events"], json!([]));
    assert_eq!(records[5]["type"], "unknown");
    assert_eq!(records[5]["raw_hex"], "0144030004cc02");
    assert_eq!(records[6]["type"], "parse_error");
    assert_eq!(records[7]["events"], json!([]));
    assert_eq!(records[8]["events"], json!([{ "type": "released" }]));
    assert_eq!(records[9]["events"], json!([]));
    for index in [12, 13, 14, 15] {
        assert_eq!(records[index]["type"], "unknown");
    }
    assert_eq!(records[16]["type"], "capture_end");
    assert_eq!(records[16]["report_count"], 16);
}

#[test]
fn replay_classifies_host_gestures_at_the_600_ms_boundary() {
    let output = run(&[
        "replay",
        fixture("m131e-software-derived-synthetic.jsonl")
            .to_str()
            .unwrap(),
    ]);
    let records = stdout_records(&output);
    assert_eq!(
        records[8]["gestures"],
        json!([{ "type": "long_press", "held_micros": 600000 }])
    );
    assert_eq!(
        records[11]["gestures"],
        json!([{ "type": "short_press", "held_micros": 599999 }])
    );
    assert_eq!(records[9]["gestures"], json!([]));
}

#[test]
fn dial_aliases_route_by_usb_identity_even_with_a_different_module_label() {
    for product_id in [0x0206, 0x9994] {
        let directory = TempDirectory::new();
        let mut records = parse_records(DIAL_CAPTURE);
        records[0]["identity"]["product_id"] = json!(product_id);
        records[0]["identity"]["module"] = json!("M003K");
        records[0]["software_mode_requested"] = json!(true);
        let path = directory.capture(&records);
        let output = run(&["replay", path.to_str().unwrap()]);
        let decoded = stdout_records(&output);
        assert_eq!(
            decoded[0]["events"],
            json!([{ "type": "rotated", "direction": "right" }])
        );
        assert_eq!(decoded[0]["identity"]["module"], "M003K");
    }
}

#[test]
fn knob_aliases_route_by_usb_identity_even_with_a_different_module_label() {
    for product_id in [0x0202, 0x9996] {
        let directory = TempDirectory::new();
        let mut records = parse_records(include_str!("fixtures/m003k-synthetic.jsonl"));
        records[0]["identity"]["product_id"] = json!(product_id);
        records[0]["identity"]["module"] = json!("M131E");
        let path = directory.capture(&records);
        let output = run(&["replay", path.to_str().unwrap()]);
        let decoded = stdout_records(&output);
        assert_eq!(decoded[0]["events"][0]["knob_index"], 0);
        assert_eq!(decoded[0]["events"][0]["kind"]["direction"], "clockwise");
        assert_eq!(decoded[0]["identity"]["module"], "M131E");
    }
}

#[test]
fn dial_looking_bytes_with_wrong_vendor_or_product_remain_unknown() {
    for (vendor_id, product_id) in [(1, 0x9994), (0x2516, 0x9999), (0x2516, 0x1234)] {
        let directory = TempDirectory::new();
        let mut records = parse_records(DIAL_CAPTURE);
        records[0]["identity"]["vendor_id"] = json!(vendor_id);
        records[0]["identity"]["product_id"] = json!(product_id);
        let path = directory.capture(&records);
        let output = run(&["replay", path.to_str().unwrap()]);
        for record in stdout_records(&output).iter().take(16) {
            assert_eq!(record["type"], "unknown");
        }
    }
}

#[test]
fn mode_backlight_and_color_default_to_offline_feature_previews() {
    for (arguments, prefix) in [
        (vec!["dial", "mode", "software"], "034901"),
        (vec!["dial", "mode", "hardware"], "034900"),
        (vec!["dial", "backlight", "0"], "034400"),
        (vec!["dial", "backlight", "100"], "034464"),
        (vec!["dial", "color", "1234aB"], "03461234ab"),
    ] {
        let output = run(&arguments);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let preview: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(preview["type"], "feature_preview");
        assert_eq!(preview["source"], "software_derived");
        assert_eq!(
            preview["raw_hex"],
            format!("{prefix}{}", "00".repeat(32 - prefix.len() / 2))
        );
    }
}

#[test]
fn invalid_dial_values_and_paths_fail_during_argument_parsing() {
    for arguments in [
        vec!["dial", "mode", "invalid", "--path-hex", "61"],
        vec!["dial", "backlight", "101", "--path-hex", "61"],
        vec!["dial", "backlight", "-1", "--path-hex", "61"],
        vec!["dial", "color", "12345", "--path-hex", "61"],
        vec!["dial", "color", "zzzzzz", "--path-hex", "61"],
        vec!["dial", "mode", "software", "--path-hex", "00"],
        vec!["dial", "backlight", "10", "--path-hex", "abc"],
    ] {
        let output = run(&arguments);
        assert_eq!(output.status.code(), Some(2), "{:?}", output);
        assert!(!String::from_utf8_lossy(&output.stderr).contains("panicked"));
    }
}

fn jpeg_plan(jpeg: &Path, descriptor: &Path, destination: &Path) -> Output {
    run(&[
        "dial",
        "jpeg",
        jpeg.to_str().unwrap(),
        "--descriptor",
        descriptor.to_str().unwrap(),
        "--output",
        destination.to_str().unwrap(),
    ])
}

#[test]
fn offline_jpeg_plan_reconstructs_the_image_with_descriptor_sized_packets() {
    let directory = TempDirectory::new();
    let destination = directory.path("plan.jsonl");
    let jpeg = fixture("dial-240x240-baseline.jpg");
    let output = jpeg_plan(&jpeg, &fixture("dial-report2-65-byte.hid"), &destination);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let records = parse_records(&fs::read_to_string(&destination).unwrap());
    assert_eq!(records[0]["type"], "jpeg_plan");
    assert_eq!(records[0]["source"], "software_derived");
    assert_eq!(records[0]["output_report_length"], 65);
    let report_count = records.len() - 2;
    assert_eq!(report_count, 31);
    let mut reconstructed = Vec::new();
    for (sequence, record) in records[1..records.len() - 1].iter().enumerate() {
        assert_eq!(record["type"], "output_report");
        assert_eq!(record["sequence"], sequence);
        let bytes = decode_hex(record["raw_hex"].as_str().unwrap());
        assert_eq!(bytes.len(), 65);
        assert_eq!(&bytes[..2], &[2, 0x45]);
        assert_eq!(u16::from_le_bytes([bytes[2], bytes[3]]) as usize, sequence);
        let payload_length = u16::from_le_bytes([bytes[4], bytes[5]]) as usize;
        assert!((1..=57).contains(&payload_length));
        assert_eq!(bytes[6], 0);
        assert_eq!(bytes[7], u8::from(sequence + 1 == report_count));
        assert!(bytes[8 + payload_length..].iter().all(|byte| *byte == 0));
        reconstructed.extend_from_slice(&bytes[8..8 + payload_length]);
    }
    assert_eq!(reconstructed, fs::read(&jpeg).unwrap());
    assert_eq!(records.last().unwrap()["type"], "jpeg_plan_end");
    assert_eq!(records.last().unwrap()["report_count"], report_count);
}

#[test]
fn offline_jpeg_plan_never_overwrites_existing_output() {
    let directory = TempDirectory::new();
    let destination = directory.path("existing.jsonl");
    fs::write(&destination, "keep this exact content\n").unwrap();
    let output = jpeg_plan(
        &fixture("dial-240x240-baseline.jpg"),
        &fixture("dial-report2-65-byte.hid"),
        &destination,
    );
    assert!(!output.status.success());
    assert_eq!(
        fs::read_to_string(destination).unwrap(),
        "keep this exact content\n"
    );
}

#[test]
fn jpeg_and_descriptor_validation_finish_before_output_is_created() {
    let directory = TempDirectory::new();
    let invalid_jpeg = directory.path("invalid.jpg");
    fs::write(&invalid_jpeg, b"invalid JPEG").unwrap();
    let output_path = directory.path("invalid-image-plan.jsonl");
    let output = jpeg_plan(
        &invalid_jpeg,
        &fixture("dial-report2-65-byte.hid"),
        &output_path,
    );
    assert!(!output.status.success());
    assert!(!output_path.exists());

    let invalid_descriptor = directory.path("invalid.hid");
    fs::write(&invalid_descriptor, [0x85, 0x02, 0x75]).unwrap();
    let output_path = directory.path("invalid-descriptor-plan.jsonl");
    let output = jpeg_plan(
        &fixture("dial-240x240-baseline.jpg"),
        &invalid_descriptor,
        &output_path,
    );
    assert!(!output.status.success());
    assert!(!output_path.exists());
}

#[test]
fn jpeg_live_path_conflicts_with_offline_descriptor_and_output_before_discovery() {
    let directory = TempDirectory::new();
    let destination = directory.path("not-created.jsonl");
    let output = run(&[
        "dial",
        "jpeg",
        fixture("dial-240x240-baseline.jpg").to_str().unwrap(),
        "--path-hex",
        "61",
        "--descriptor",
        fixture("dial-report2-65-byte.hid").to_str().unwrap(),
        "--output",
        destination.to_str().unwrap(),
    ]);
    assert_eq!(output.status.code(), Some(2), "{:?}", output);
    assert!(!destination.exists());
}
