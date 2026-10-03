use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};

use image::ImageFormat;
use serde_json::Value;

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct TempDirectory(PathBuf);
impl TempDirectory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "masterhub-image-test-{}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn file(&self, name: &str) -> String {
        self.0.join(name).to_str().unwrap().to_owned()
    }
}
impl Drop for TempDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn fixture(name: &str) -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
        .to_str()
        .unwrap()
        .to_owned()
}

fn run(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_masterhub-diag"))
        .args(arguments)
        .output()
        .unwrap()
}

fn success(output: Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn prepared_image_entropy_decodes_and_plans_losslessly_without_hardware() {
    let directory = TempDirectory::new();
    let prepared = directory.file("prepared image.jpg");
    let input = fixture("dial-240x240-baseline.jpg");
    let metadata = success(run(&[
        "dial",
        "prepare-image",
        &input,
        "--output",
        &prepared,
    ]));
    assert_eq!(metadata["type"], "image_prepared");
    assert_eq!(metadata["fit"], "contain");
    assert_eq!(metadata["quality"], 100);
    assert_eq!(metadata["clockwise_rotation_degrees"], 0);
    let jpeg = fs::read(&prepared).unwrap();
    let decoded = image::load_from_memory_with_format(&jpeg, ImageFormat::Jpeg).unwrap();
    assert_eq!((decoded.width(), decoded.height()), (240, 240));
    assert_eq!(metadata["jpeg_length"], jpeg.len());
    let plan_path = directory.file("packets.jsonl");
    let descriptor = fixture("dial-report2-65-byte.hid");
    success(run(&[
        "dial",
        "jpeg",
        &prepared,
        "--descriptor",
        &descriptor,
        "--output",
        &plan_path,
    ]));
    let rows: Vec<Value> = fs::read_to_string(plan_path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let mut reconstructed = Vec::new();
    for row in rows.iter().filter(|row| row["type"] == "output_report") {
        let hex = row["raw_hex"].as_str().unwrap();
        let packet: Vec<u8> = (0..hex.len())
            .step_by(2)
            .map(|offset| u8::from_str_radix(&hex[offset..offset + 2], 16).unwrap())
            .collect();
        let length = u16::from_le_bytes([packet[4], packet[5]]) as usize;
        reconstructed.extend_from_slice(&packet[8..8 + length]);
    }
    assert_eq!(reconstructed, jpeg);
    assert_eq!(rows.last().unwrap()["type"], "jpeg_plan_end");
}

#[test]
fn explicit_conversion_options_and_270_degree_rotation_are_accepted() {
    let directory = TempDirectory::new();
    let prepared = directory.file("rotated.jpg");
    let metadata = success(run(&[
        "dial",
        "prepare-image",
        &fixture("dial-240x240-baseline.jpg"),
        "--output",
        &prepared,
        "--fit",
        "cover",
        "--quality",
        "80",
        "--rotate",
        "270",
        "--background",
        "12abFF",
    ]));
    assert_eq!(metadata["fit"], "cover");
    assert_eq!(metadata["quality"], 80);
    assert_eq!(metadata["clockwise_rotation_degrees"], 270);
    assert_eq!(
        metadata["background_rgb"],
        serde_json::json!([18, 171, 255])
    );
}

#[test]
fn invalid_images_do_not_create_output_and_existing_files_are_preserved() {
    let directory = TempDirectory::new();
    let bad_input = directory.file("invalid.png");
    fs::write(&bad_input, b"invalid image").unwrap();
    let output = directory.file("output.jpg");
    assert!(
        !run(&["dial", "prepare-image", &bad_input, "--output", &output])
            .status
            .success()
    );
    assert!(!PathBuf::from(&output).exists());
    fs::write(&output, b"existing content").unwrap();
    assert!(
        !run(&[
            "dial",
            "prepare-image",
            &fixture("dial-240x240-baseline.jpg"),
            "--output",
            &output,
        ])
        .status
        .success()
    );
    assert_eq!(fs::read(output).unwrap(), b"existing content");
}

#[test]
fn conversion_arguments_reject_invalid_values_and_hardware_selection() {
    let directory = TempDirectory::new();
    let output = directory.file("never-created.jpg");
    for option in [
        ["--quality", "0"],
        ["--quality", "101"],
        ["--rotate", "45"],
        ["--fit", "invalid"],
        ["--background", "12345"],
        ["--path-hex", "61"],
    ] {
        let result = run(&[
            "dial",
            "prepare-image",
            "missing.png",
            "--output",
            &output,
            option[0],
            option[1],
        ]);
        assert_eq!(result.status.code(), Some(2));
        assert!(!PathBuf::from(&output).exists());
    }
}
