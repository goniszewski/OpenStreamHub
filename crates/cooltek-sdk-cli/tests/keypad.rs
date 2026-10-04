use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};

use cooltek_sdk_cli::capture::decode_hex;
use serde_json::{Value, json};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct TempDirectory(PathBuf);

impl TempDirectory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "masterhub-keypad-test-{}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn run(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_masterhub-diag"))
        .args(arguments)
        .output()
        .unwrap()
}

fn jpeg_plan(jpeg: &Path, key: &str, descriptor: &Path, output: &Path) -> Output {
    run(&[
        "keypad",
        "jpeg",
        jpeg.to_str().unwrap(),
        "--key-index",
        key,
        "--descriptor",
        descriptor.to_str().unwrap(),
        "--output",
        output.to_str().unwrap(),
    ])
}

fn success(output: &Output) -> Value {
    assert!(
        output.status.success(),
        "command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn assert_rejected_without_output(output: Output, path: &Path) {
    assert!(!output.status.success());
    assert!(!path.exists());
    assert!(!String::from_utf8_lossy(&output.stderr).contains("panicked"));
}

#[test]
fn original_synthetic_keypad_jpeg_reassembles_with_key_index_and_descriptor_sized_reports() {
    // Both fixtures are synthetic offline data, never hardware captures.
    let jpeg_path = fixture("keypad-72x72-baseline.jpg");
    let jpeg = fs::read(&jpeg_path).unwrap();
    let decoded = image::load_from_memory_with_format(&jpeg, image::ImageFormat::Jpeg).unwrap();
    assert_eq!((decoded.width(), decoded.height()), (72, 72));
    let descriptor_path = fixture("dial-report2-65-byte.hid");
    let descriptor = fs::read(&descriptor_path).unwrap();
    for key in [0u8, 14] {
        let directory = TempDirectory::new();
        let destination = directory.path("keypad-plan.jsonl");
        let summary = success(&jpeg_plan(
            &jpeg_path,
            &key.to_string(),
            &descriptor_path,
            &destination,
        ));
        assert_eq!(summary["type"], "jpeg_plan_created");
        assert_eq!(summary["module"], "m4315");
        assert_eq!(summary["key_index"], key);
        let rows: Vec<Value> = fs::read_to_string(&destination)
            .unwrap()
            .lines()
            .map(|row| serde_json::from_str(row).unwrap())
            .collect();
        assert_eq!(rows[0]["type"], "jpeg_plan");
        assert_eq!(rows[0]["schema_version"], 1);
        assert_eq!(rows[0]["source"], "software_derived");
        assert_eq!(rows[0]["output_report_length"], 65);
        assert_eq!(rows[0]["jpeg_length"], jpeg.len());
        assert_eq!(
            decode_hex(rows[0]["report_descriptor_hex"].as_str().unwrap(), 4096).unwrap(),
            descriptor
        );
        let report_count = rows.len() - 2;
        assert_eq!(report_count, jpeg.len().div_ceil(57));
        assert_eq!(summary["report_count"], report_count);
        let mut reassembled = Vec::new();
        for (sequence, row) in rows[1..rows.len() - 1].iter().enumerate() {
            assert_eq!(row["type"], "output_report");
            assert_eq!(row["sequence"], sequence);
            let packet = decode_hex(row["raw_hex"].as_str().unwrap(), 65).unwrap();
            assert_eq!(packet.len(), 65);
            assert_eq!(&packet[..2], &[2, 0x36]);
            assert_eq!(
                usize::from(u16::from_le_bytes([packet[2], packet[3]])),
                sequence
            );
            let length = usize::from(u16::from_le_bytes([packet[4], packet[5]]));
            assert!((1..=57).contains(&length));
            assert_eq!(packet[6], key);
            assert_eq!(packet[7], u8::from(sequence + 1 == report_count));
            assert!(packet[8 + length..].iter().all(|byte| *byte == 0));
            reassembled.extend_from_slice(&packet[8..8 + length]);
        }
        assert_eq!(reassembled, jpeg);
        assert_eq!(rows.last().unwrap()["type"], "jpeg_plan_end");
        assert_eq!(rows.last().unwrap()["report_count"], report_count);
        for row in rows {
            assert_eq!(row["module"], "m4315");
            assert_eq!(row["key_index"], key);
            assert!(row.get("reserved_byte_6").is_none());
        }
    }
}

#[test]
fn keypad_preparation_entropy_decodes_at_72_pixels_and_preserves_dial_dimensions() {
    let directory = TempDirectory::new();
    for (module, side) in [("keypad", 72), ("dial", 240)] {
        let prepared = directory.path(&format!("{module}.jpg"));
        let metadata = success(&run(&[
            module,
            "prepare-image",
            fixture("dial-240x240-baseline.jpg").to_str().unwrap(),
            "--output",
            prepared.to_str().unwrap(),
            "--fit",
            "cover",
            "--quality",
            "80",
            "--rotate",
            "270",
            "--background",
            "12abFF",
        ]));
        assert_eq!(metadata["type"], "image_prepared");
        assert_eq!(metadata["width"], side);
        assert_eq!(metadata["height"], side);
        assert_eq!(metadata["fit"], "cover");
        assert_eq!(metadata["quality"], 80);
        assert_eq!(metadata["clockwise_rotation_degrees"], 270);
        assert_eq!(metadata["background_rgb"], json!([18, 171, 255]));
        if module == "keypad" {
            assert_eq!(metadata["module"], "m4315");
        } else {
            assert!(metadata.get("module").is_none());
        }
        let jpeg = fs::read(prepared).unwrap();
        let decoded = image::load_from_memory_with_format(&jpeg, image::ImageFormat::Jpeg).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (side, side));
        assert_eq!(metadata["jpeg_length"], jpeg.len());
    }
}

#[test]
fn malformed_or_wrong_size_keypad_jpegs_fail_before_plan_creation() {
    let directory = TempDirectory::new();
    let jpeg = fs::read(fixture("keypad-72x72-baseline.jpg")).unwrap();
    let mut progressive = jpeg.clone();
    let sof = progressive
        .windows(2)
        .position(|pair| pair == [0xff, 0xc0])
        .unwrap();
    progressive[sof + 1] = 0xc2;
    let mut trailing = jpeg.clone();
    trailing.push(0);
    for (index, bytes) in [
        b"invalid JPEG".to_vec(),
        fs::read(fixture("dial-240x240-baseline.jpg")).unwrap(),
        jpeg[..jpeg.len() - 2].to_vec(),
        progressive,
        trailing,
    ]
    .into_iter()
    .enumerate()
    {
        let input = directory.path("invalid.jpg");
        fs::write(&input, bytes).unwrap();
        let destination = directory.path(&format!("invalid-{index}.jsonl"));
        assert_rejected_without_output(
            jpeg_plan(
                &input,
                "0",
                &fixture("dial-report2-65-byte.hid"),
                &destination,
            ),
            &destination,
        );
    }
}

#[test]
fn invalid_keypad_descriptors_fail_before_plan_creation() {
    let directory = TempDirectory::new();
    for (index, bytes) in [
        vec![0x85, 0x02, 0x75],                               // Truncated descriptor.
        vec![0x85, 0x01, 0x75, 8, 0x95, 64, 0x91, 2],         // No output report ID 2.
        vec![0x85, 0x02, 0x75, 8, 0x95, 7, 0x91, 2],          // Only an eight-byte report.
        vec![0x85, 0x02, 0x75, 8, 0x96, 0xff, 0xff, 0x91, 2], // 65,536 bytes.
    ]
    .into_iter()
    .enumerate()
    {
        let descriptor = directory.path("invalid.hid");
        fs::write(&descriptor, bytes).unwrap();
        let destination = directory.path(&format!("invalid-{index}.jsonl"));
        assert_rejected_without_output(
            jpeg_plan(
                &fixture("keypad-72x72-baseline.jpg"),
                "0",
                &descriptor,
                &destination,
            ),
            &destination,
        );
    }
}

#[test]
fn keypad_jpeg_size_and_sequence_limits_are_checked_before_output_creation() {
    let directory = TempDirectory::new();
    let oversized = directory.path("oversized.jpg");
    fs::File::create(&oversized)
        .unwrap()
        .set_len(16 * 1024 * 1024 + 1)
        .unwrap();
    let destination = directory.path("oversized.jsonl");
    assert_rejected_without_output(
        jpeg_plan(
            &oversized,
            "0",
            &fixture("dial-report2-65-byte.hid"),
            &destination,
        ),
        &destination,
    );

    // An original synthetic COM segment keeps JPEG framing valid but causes
    // more than 65,536 chunks when the saved report has one payload byte.
    let mut jpeg = fs::read(fixture("keypad-72x72-baseline.jpg")).unwrap();
    let mut comment = vec![0xff, 0xfe, 0xff, 0xff];
    comment.resize(65_537, 0x55);
    let app0_end = 4 + usize::from(u16::from_be_bytes([jpeg[4], jpeg[5]]));
    jpeg.splice(app0_end..app0_end, comment);
    let overflow = directory.path("sequence-overflow.jpg");
    fs::write(&overflow, jpeg).unwrap();
    let descriptor = directory.path("one-payload-byte.hid");
    fs::write(&descriptor, [0x85, 0x02, 0x75, 8, 0x95, 8, 0x91, 2]).unwrap();
    let destination = directory.path("sequence-overflow.jsonl");
    let output = jpeg_plan(&overflow, "0", &descriptor, &destination);
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("sequence"), "unexpected error: {error}");
    assert!(!destination.exists());
}

#[test]
fn keypad_output_files_are_never_overwritten_and_bad_images_create_no_output() {
    let directory = TempDirectory::new();
    let destination = directory.path("existing.jsonl");
    fs::write(&destination, b"keep exact plan content\n").unwrap();
    assert!(
        !jpeg_plan(
            &fixture("keypad-72x72-baseline.jpg"),
            "0",
            &fixture("dial-report2-65-byte.hid"),
            &destination,
        )
        .status
        .success()
    );
    assert_eq!(
        fs::read(&destination).unwrap(),
        b"keep exact plan content\n"
    );
    let prepared = directory.path("existing.jpg");
    fs::write(&prepared, b"keep exact image content").unwrap();
    assert!(
        !run(&[
            "keypad",
            "prepare-image",
            fixture("keypad-72x72-baseline.jpg").to_str().unwrap(),
            "--output",
            prepared.to_str().unwrap(),
        ])
        .status
        .success()
    );
    assert_eq!(fs::read(&prepared).unwrap(), b"keep exact image content");
    let invalid = directory.path("invalid.png");
    fs::write(&invalid, b"invalid image").unwrap();
    let prepared = directory.path("never-created.jpg");
    assert_rejected_without_output(
        run(&[
            "keypad",
            "prepare-image",
            invalid.to_str().unwrap(),
            "--output",
            prepared.to_str().unwrap(),
        ]),
        &prepared,
    );
}

#[test]
fn keypad_commands_require_offline_inputs_and_reject_live_selection() {
    for arguments in [
        vec![
            "keypad",
            "jpeg",
            "missing.jpg",
            "--descriptor",
            "missing.hid",
            "--output",
            "missing.jsonl",
        ],
        vec![
            "keypad",
            "jpeg",
            "missing.jpg",
            "--key-index",
            "0",
            "--output",
            "missing.jsonl",
        ],
        vec![
            "keypad",
            "jpeg",
            "missing.jpg",
            "--key-index",
            "0",
            "--descriptor",
            "missing.hid",
        ],
    ] {
        assert_eq!(run(&arguments).status.code(), Some(2));
    }
    let directory = TempDirectory::new();
    let destination = directory.path("never-created.jsonl");
    for key in ["15", "255", "-1"] {
        let result = jpeg_plan(
            &fixture("keypad-72x72-baseline.jpg"),
            key,
            &fixture("dial-report2-65-byte.hid"),
            &destination,
        );
        assert_eq!(result.status.code(), Some(2));
        assert!(!destination.exists());
    }
    for command in ["jpeg", "prepare-image"] {
        let mut arguments = vec![
            "keypad",
            command,
            "missing.jpg",
            "--output",
            destination.to_str().unwrap(),
        ];
        if command == "jpeg" {
            arguments.extend(["--key-index", "0", "--descriptor", "missing.hid"]);
        }
        arguments.extend(["--path-hex", "61"]);
        assert_eq!(run(&arguments).status.code(), Some(2));
        assert!(!destination.exists());
        let help = run(&["keypad", command, "--help"]);
        assert!(help.status.success());
        assert!(!String::from_utf8_lossy(&help.stdout).contains("--path-hex"));
    }
    for option in [
        ["--quality", "0"],
        ["--quality", "101"],
        ["--rotate", "45"],
        ["--fit", "invalid"],
        ["--background", "12345"],
    ] {
        assert_eq!(
            run(&[
                "keypad",
                "prepare-image",
                "missing.png",
                "--output",
                destination.to_str().unwrap(),
                option[0],
                option[1],
            ])
            .status
            .code(),
            Some(2)
        );
        assert!(!destination.exists());
    }
}
