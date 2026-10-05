use std::{
    error::Error,
    fs::{File, OpenOptions},
    io::{self, BufReader, BufWriter, Read, Write},
    path::{Path, PathBuf},
    process::ExitCode,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use clap::{Args, Parser, Subcommand, ValueEnum};
use cooltek_sdk_masterhub::{
    DeviceRegistry, InputReader, ModuleIdentity, ModuleKind, ReportTransport, descriptor, dial,
    hidapi::{HidDevice, MAX_REPORT_DESCRIPTOR_SIZE},
    keypad,
};
use cooltek_sdk_service::{Gesture, InputEvent, InputSession, InputUpdate, ServiceModule};
use serde_json::{Value, json};

use cooltek_sdk_cli::capture::{
    CaptureReader, Descriptor, End, EndReason, Identity, MAX_REPORT_BYTES, Record, Report,
    SCHEMA_VERSION, Session, Source, decode_hex, encode_hex, write_record,
};
use cooltek_sdk_cli::device::{DevicePath, TIMEOUT_MS_RANGE, parse_path, select_device};
use cooltek_sdk_cli::image_prep::{
    FitMode, MAX_ENCODED_BYTES, PrepareOptions, prepare_image, prepare_keypad_image,
};

#[derive(Parser)]
#[command(
    name = "masterhub-diag",
    version,
    about = "Discover MasterHUB devices, record raw reports, and replay offline",
    long_about = "Discover MasterHUB devices, record raw reports, and replay offline.\n\nDiscovery never opens devices. Recording does not initialize devices or send configuration reports unless --software-mode is explicitly requested. Replay does not initialize HID. Dial setters preview reports unless --path-hex explicitly selects a device. Images can be converted offline; JPEG packet plans require a saved HID descriptor. Keypad image conversion and JPEG packet planning are offline only."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print a JSON device inventory without opening or initializing devices.
    List,
    /// Record a single exact device path to a new JSONL file.
    ///
    /// Ctrl-C or SIGTERM ends the capture with an `interrupted` end record and
    /// restores hardware mode if --software-mode was requested; a second Ctrl-C
    /// exits immediately without cleanup.
    Record {
        /// Hexadecimal device path copied from `list` (no trailing NUL).
        #[arg(long, value_parser = parse_path)]
        path_hex: DevicePath,
        /// Destination; existing files are never overwritten.
        #[arg(long)]
        output: PathBuf,
        /// Capture deadline, including idle time (1 to 86400 seconds).
        #[arg(long, default_value_t = 30, value_parser = clap::value_parser!(u64).range(1..=86_400))]
        duration_secs: u64,
        /// Maximum wait for each read (1 to 1000 ms); reduced as the deadline approaches.
        #[arg(long, default_value_t = 100, value_parser = clap::value_parser!(i32).range(TIMEOUT_MS_RANGE))]
        timeout_ms: i32,
        /// End capture after this many nonempty reports.
        #[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
        max_reports: Option<u64>,
        /// Explicitly enter software mode (M131E dial or M4315 keys).
        #[arg(long)]
        software_mode: bool,
    },
    /// Decode a completed JSONL capture without opening any hardware.
    Replay { file: PathBuf },
    /// Preview software-derived dial commands or explicitly send to a selected dial.
    Dial {
        #[command(subcommand)]
        command: DialCommand,
    },
    /// Prepare M4315 key images and JPEG packet plans offline, without HID.
    Keypad {
        #[command(subcommand)]
        command: KeypadCommand,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum DialMode {
    Hardware,
    Software,
}

#[derive(Clone, Copy, ValueEnum)]
enum ImageFit {
    Contain,
    Cover,
    Stretch,
}

#[derive(Args)]
struct ImagePreparationArgs {
    file: PathBuf,
    /// New JPEG file; existing files are never overwritten.
    #[arg(long)]
    output: PathBuf,
    /// Preserve the full image, center-crop, or ignore aspect ratio.
    #[arg(long, value_enum, default_value_t = ImageFit::Contain)]
    fit: ImageFit,
    #[arg(long, default_value_t = 100, value_parser = clap::value_parser!(u8).range(1..=100))]
    quality: u8,
    /// Clockwise rotation in degrees, applied after fitting.
    #[arg(long, default_value = "0", value_parser = ["0", "90", "180", "270"])]
    rotate: String,
    /// RGB matte for transparency and contain-mode margins.
    #[arg(long, default_value = "000000", value_parser = parse_color)]
    background: [u8; 3],
}

#[derive(Subcommand)]
enum DialCommand {
    /// Convert a static PNG/JPEG to a 240x240 baseline JFIF JPEG, without HID.
    PrepareImage(ImagePreparationArgs),
    /// Preview mode control; --path-hex explicitly sends the command.
    Mode {
        mode: DialMode,
        #[arg(long, value_parser = parse_path)]
        path_hex: Option<DevicePath>,
    },
    /// Preview backlight control; --path-hex explicitly sends the command.
    Backlight {
        #[arg(value_parser = clap::value_parser!(u8).range(0..=100))]
        value: u8,
        #[arg(long, value_parser = parse_path)]
        path_hex: Option<DevicePath>,
    },
    /// Preview three raw color channels in SDK order (six hexadecimal digits).
    Color {
        #[arg(value_parser = parse_color)]
        channels: [u8; 3],
        #[arg(long, value_parser = parse_path)]
        path_hex: Option<DevicePath>,
    },
    /// Explicitly query current firmware fields and backlight on a selected dial.
    Info {
        #[arg(long, value_parser = parse_path)]
        path_hex: DevicePath,
    },
    /// Plan a 240x240 baseline JFIF JPEG offline, or explicitly send it to a dial.
    Jpeg {
        file: PathBuf,
        /// Saved binary HID descriptor for an offline plan.
        #[arg(
            long,
            required_unless_present = "path_hex",
            conflicts_with = "path_hex"
        )]
        descriptor: Option<PathBuf>,
        /// Exact dial path for a live write; report size comes from its descriptor.
        #[arg(long, value_parser = parse_path, required_unless_present = "descriptor")]
        path_hex: Option<DevicePath>,
        /// New JSONL plan file; existing files are never overwritten.
        #[arg(
            long,
            required_unless_present = "path_hex",
            conflicts_with = "path_hex"
        )]
        output: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum KeypadCommand {
    /// Convert a static PNG/JPEG to a 72x72 baseline JFIF JPEG, without HID.
    PrepareImage(ImagePreparationArgs),
    /// Plan a 72x72 baseline JFIF JPEG for one M4315 key, without HID.
    Jpeg {
        file: PathBuf,
        /// Zero-based key index, from 0 through 14.
        #[arg(long, value_parser = clap::value_parser!(u8).range(0..=14))]
        key_index: u8,
        /// Saved binary HID descriptor; output report ID 2 supplies the size.
        #[arg(long)]
        descriptor: PathBuf,
        /// New JSONL plan file; existing files are never overwritten.
        #[arg(long)]
        output: PathBuf,
    },
}

#[derive(Clone, Copy)]
enum ImagePreparationTarget {
    Dial,
    Keypad,
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("masterhub-diag: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<(), Box<dyn Error>> {
    match cli.command {
        Command::List => {
            let registry = DeviceRegistry::new()?;
            let devices: Vec<Identity> = registry.devices().iter().map(identity).collect();
            let mut output = io::stdout().lock();
            serde_json::to_writer_pretty(
                &mut output,
                &json!({ "schema_version": SCHEMA_VERSION, "devices": devices }),
            )?;
            writeln!(output)?;
            Ok(())
        }
        Command::Record {
            path_hex,
            output,
            duration_secs,
            timeout_ms,
            max_reports,
            software_mode,
        } => record(
            &path_hex,
            output,
            duration_secs,
            timeout_ms,
            max_reports,
            software_mode,
        ),
        Command::Replay { file } => replay(file),
        Command::Dial { command } => dial_command(command),
        Command::Keypad { command } => keypad_command(command),
    }
}

fn parse_color(raw: &str) -> Result<[u8; 3], String> {
    decode_hex(raw, 3)?
        .try_into()
        .map_err(|_| "color must contain exactly six hexadecimal digits".into())
}

fn identity(device: &ModuleIdentity) -> Identity {
    Identity {
        path_hex: encode_hex(device.path.as_bytes()),
        vendor_id: device.vendor_id,
        product_id: device.product_id,
        serial_number: device.serial_number.clone(),
        manufacturer_string: device.manufacturer_string.clone(),
        product_string: device.product_string.clone(),
        release_number: device.release_number,
        interface_number: device.interface_number,
        usage_page: device.usage_page,
        usage: device.usage,
        module: device.module.map(|module| module.name().to_owned()),
    }
}

fn print_json(value: &serde_json::Value) -> Result<(), Box<dyn Error>> {
    let mut output = io::stdout().lock();
    serde_json::to_writer(&mut output, value)?;
    writeln!(output)?;
    Ok(())
}

fn open_dial(path: &[u8]) -> Result<HidDevice, Box<dyn Error>> {
    let registry = DeviceRegistry::new()?;
    let devices = registry.devices();
    let selected = select_dial(&devices, path)?;
    Ok(registry.open(selected)?)
}

fn feature_command(
    report: [u8; dial::FEATURE_REPORT_LENGTH],
    path: Option<DevicePath>,
) -> Result<(), Box<dyn Error>> {
    let sent = if let Some(path) = path {
        open_dial(&path)?.send_feature_report(&report)?;
        true
    } else {
        false
    };
    print_json(&json!({
        "type": if sent { "feature_sent" } else { "feature_preview" },
        "source": "software_derived",
        "raw_hex": encode_hex(&report),
    }))
}

fn read_bounded(path: &Path, limit: usize) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut bytes = Vec::new();
    File::open(path)?
        .take(u64::try_from(limit)? + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(format!("{} exceeds the {limit}-byte limit", path.display()).into());
    }
    Ok(bytes)
}

fn dial_command(command: DialCommand) -> Result<(), Box<dyn Error>> {
    match command {
        DialCommand::PrepareImage(arguments) => {
            prepare_image_command(arguments, ImagePreparationTarget::Dial)
        }
        DialCommand::Mode { mode, path_hex } => feature_command(
            dial::software_mode_report(matches!(mode, DialMode::Software)),
            path_hex,
        ),
        DialCommand::Backlight { value, path_hex } => {
            feature_command(dial::backlight_report(value)?, path_hex)
        }
        DialCommand::Color { channels, path_hex } => {
            feature_command(dial::color_report(channels), path_hex)
        }
        DialCommand::Info { path_hex } => {
            let device = open_dial(&path_hex)?;
            let firmware = dial::read_firmware_info(&device, dial::FirmwareInfoKind::Current)?;
            let backlight = dial::read_backlight(&device)?;
            print_json(&json!({
                "type": "dial_info", "source": "hardware",
                "firmware_raw_hex": encode_hex(&firmware.raw_version),
                "firmware_text_hex": encode_hex(&firmware.version_text),
                "firmware_text": firmware.version_string_lossy(),
                "backlight": backlight,
            }))
        }
        DialCommand::Jpeg {
            file,
            descriptor: descriptor_path,
            path_hex,
            output,
        } => {
            let jpeg = read_bounded(&file, dial::MAX_JPEG_LENGTH)?;
            if let Some(path) = path_hex {
                let device = open_dial(&path)?;
                let mut raw_descriptor = [0; MAX_REPORT_DESCRIPTOR_SIZE];
                let received = device.get_report_descriptor(&mut raw_descriptor)?;
                if received == 0 || received > raw_descriptor.len() {
                    return Err("HID descriptor returned an invalid byte count".into());
                }
                let report_length =
                    descriptor::output_report_length(&raw_descriptor[..received], 2)?;
                let count = dial::display_jpeg(&device, &jpeg, report_length)?;
                print_json(&json!({
                    "type":"jpeg_sent", "source":"software_derived",
                    "output_report_length": report_length, "report_count": count,
                }))
            } else {
                let descriptor_path =
                    descriptor_path.ok_or("offline JPEG planning requires --descriptor")?;
                let output = output.ok_or("offline JPEG planning requires --output")?;
                let raw_descriptor = read_bounded(&descriptor_path, MAX_REPORT_DESCRIPTOR_SIZE)?;
                let report_length = descriptor::output_report_length(&raw_descriptor, 2)?;
                let packets = dial::JpegPackets::new(&jpeg, report_length)?;
                // Validate all inputs before creating a file. A plan is distinct
                // from the hardware/synthetic input-capture schema.
                let mut writer = BufWriter::new(
                    OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&output)?,
                );
                serde_json::to_writer(
                    &mut writer,
                    &json!({
                        "type":"jpeg_plan", "schema_version":1, "source":"software_derived",
                        "output_report_length":report_length, "jpeg_length":jpeg.len(),
                        "report_descriptor_hex":encode_hex(&raw_descriptor),
                        "reserved_byte_6":0,
                    }),
                )?;
                writeln!(writer)?;
                let mut count = 0;
                for (sequence, packet) in packets.enumerate() {
                    serde_json::to_writer(
                        &mut writer,
                        &json!({
                            "type":"output_report", "sequence":sequence, "raw_hex":encode_hex(&packet),
                        }),
                    )?;
                    writeln!(writer)?;
                    count += 1;
                }
                serde_json::to_writer(
                    &mut writer,
                    &json!({"type":"jpeg_plan_end", "report_count":count}),
                )?;
                writeln!(writer)?;
                writer.flush()?;
                print_json(&json!({
                    "type":"jpeg_plan_created", "source":"software_derived",
                    "output":output, "output_report_length":report_length, "report_count":count,
                }))
            }
        }
    }
}

fn prepare_image_command(
    arguments: ImagePreparationArgs,
    target: ImagePreparationTarget,
) -> Result<(), Box<dyn Error>> {
    let ImagePreparationArgs {
        file,
        output,
        fit,
        quality,
        rotate,
        background,
    } = arguments;
    let bytes = read_bounded(&file, MAX_ENCODED_BYTES)?;
    let rotation_degrees = rotate.parse::<u16>()?;
    let options = PrepareOptions {
        quality,
        fit: match fit {
            ImageFit::Contain => FitMode::Contain,
            ImageFit::Cover => FitMode::Cover,
            ImageFit::Stretch => FitMode::Stretch,
        },
        quarter_turns: u8::try_from(rotation_degrees / 90)?,
        background,
    };
    let (jpeg, width, height) = match target {
        ImagePreparationTarget::Dial => (
            prepare_image(&bytes, options)?,
            dial::IMAGE_WIDTH,
            dial::IMAGE_HEIGHT,
        ),
        ImagePreparationTarget::Keypad => (
            prepare_keypad_image(&bytes, options)?,
            keypad::IMAGE_WIDTH,
            keypad::IMAGE_HEIGHT,
        ),
    };
    // Decode, fit, encode and validate before creating an output file.
    let mut writer = BufWriter::new(
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&output)?,
    );
    writer.write_all(&jpeg)?;
    writer.flush()?;
    let mut metadata = json!({
        "type":"image_prepared", "schema_version":1,
        "output":output, "width":width, "height":height,
        "jpeg_length":jpeg.len(), "quality":quality,
        "fit": match fit { ImageFit::Contain => "contain", ImageFit::Cover => "cover", ImageFit::Stretch => "stretch" },
        "clockwise_rotation_degrees":rotation_degrees, "background_rgb":background,
    });
    if matches!(target, ImagePreparationTarget::Keypad) {
        metadata["module"] = json!("m4315");
    }
    print_json(&metadata)
}

fn keypad_command(command: KeypadCommand) -> Result<(), Box<dyn Error>> {
    match command {
        KeypadCommand::PrepareImage(arguments) => {
            prepare_image_command(arguments, ImagePreparationTarget::Keypad)
        }
        KeypadCommand::Jpeg {
            file,
            key_index,
            descriptor: descriptor_path,
            output,
        } => {
            let jpeg = read_bounded(&file, keypad::MAX_JPEG_LENGTH)?;
            let raw_descriptor = read_bounded(&descriptor_path, MAX_REPORT_DESCRIPTOR_SIZE)?;
            let report_length = descriptor::output_report_length(&raw_descriptor, 2)?;
            let packets = keypad::JpegPackets::new(key_index, &jpeg, report_length)?;
            // Complete validation precedes output creation. This is a packet
            // plan for a saved descriptor, with no device discovery or writes.
            let mut writer = BufWriter::new(
                OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&output)?,
            );
            serde_json::to_writer(
                &mut writer,
                &json!({
                    "type":"jpeg_plan", "schema_version":1, "source":"software_derived",
                    "module":"m4315", "key_index":key_index,
                    "output_report_length":report_length, "jpeg_length":jpeg.len(),
                    "report_descriptor_hex":encode_hex(&raw_descriptor),
                }),
            )?;
            writeln!(writer)?;
            let mut count = 0;
            for (sequence, packet) in packets.enumerate() {
                serde_json::to_writer(
                    &mut writer,
                    &json!({
                        "type":"output_report", "module":"m4315", "key_index":key_index,
                        "sequence":sequence, "raw_hex":encode_hex(&packet),
                    }),
                )?;
                writeln!(writer)?;
                count += 1;
            }
            serde_json::to_writer(
                &mut writer,
                &json!({
                    "type":"jpeg_plan_end", "module":"m4315", "key_index":key_index,
                    "report_count":count,
                }),
            )?;
            writeln!(writer)?;
            writer.flush()?;
            print_json(&json!({
                "type":"jpeg_plan_created", "source":"software_derived",
                "module":"m4315", "key_index":key_index,
                "output":output, "output_report_length":report_length, "report_count":count,
            }))
        }
    }
}

fn record(
    path: &[u8],
    output: PathBuf,
    duration_secs: u64,
    timeout_ms: i32,
    max_reports: Option<u64>,
    software_mode: bool,
) -> Result<(), Box<dyn Error>> {
    let interrupted = cooltek_sdk_cli::interrupt::install()?;
    let registry = DeviceRegistry::new()?;
    let devices = registry.devices();
    let selected = select_device(&devices, path, software_mode)?;
    // Fail on an existing output before opening the hardware or changing its mode.
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&output)?;
    let device = match registry.open(selected) {
        Ok(device) => device,
        Err(error) => {
            // This call created the file and wrote nothing; do not leave it behind.
            drop(file);
            let _ = std::fs::remove_file(&output);
            return Err(error.into());
        }
    };
    let mut reader = InputReader::new(device);
    let report_descriptor = descriptor(reader.transport());
    let started_unix_micros =
        u64::try_from(SystemTime::now().duration_since(UNIX_EPOCH)?.as_micros())?;
    let started = Instant::now();
    let mut writer = BufWriter::new(file);
    write_record(
        &mut writer,
        &Record::Session(Session {
            schema_version: SCHEMA_VERSION,
            source: Source::Hardware,
            started_unix_micros,
            identity: identity(selected),
            software_mode_requested: software_mode,
            report_descriptor,
        }),
    )?;
    record_session(
        &mut reader,
        &mut writer,
        RecordSettings {
            module: ModuleKind::from_ids(selected.vendor_id, selected.product_id),
            software_mode,
            started,
            duration: Duration::from_secs(duration_secs),
            timeout_ms,
            max_reports,
        },
        &interrupted,
    )
}

struct RecordSettings {
    module: Option<ModuleKind>,
    software_mode: bool,
    started: Instant,
    duration: Duration,
    timeout_ms: i32,
    max_reports: Option<u64>,
}

/// Enables requested mode, captures until a limit or interrupt, then restores mode.
fn record_session<T: ReportTransport>(
    reader: &mut InputReader<T>,
    writer: &mut impl Write,
    settings: RecordSettings,
    interrupted: &AtomicBool,
) -> Result<(), Box<dyn Error>> {
    let RecordSettings {
        module,
        software_mode,
        started,
        duration,
        timeout_ms,
        max_reports,
    } = settings;
    begin_record_mode(reader.transport(), module, software_mode, writer, started)?;
    let result = capture_input(
        reader,
        writer,
        started,
        duration,
        timeout_ms,
        max_reports,
        interrupted,
    );
    cleanup_record_mode(reader.transport(), module, software_mode, result)
}

fn begin_record_mode<T: ReportTransport>(
    transport: &T,
    module: Option<ModuleKind>,
    software_mode: bool,
    writer: &mut impl Write,
    started: Instant,
) -> Result<(), Box<dyn Error>> {
    if !software_mode {
        return Ok(());
    }
    if let Err(error) = cooltek_sdk_cli::mode::set(transport, module, true) {
        let message = format!("software-mode feature report failed: {error}");
        let failure: Box<dyn Error> = match finish(
            writer,
            started,
            0,
            EndReason::TransportError,
            Some(message.clone()),
        ) {
            Ok(()) => message.into(),
            Err(footer) => {
                format!("{message}; writing capture footer also failed: {footer}").into()
            }
        };
        // A failed write can have changed device mode. Cleanup is authorized by
        // the opt-in flag and must run even if writing the footer also failed.
        return cleanup_record_mode(transport, module, true, Err(failure));
    }
    Ok(())
}

fn cleanup_record_mode<T: ReportTransport>(
    transport: &T,
    module: Option<ModuleKind>,
    software_mode: bool,
    result: Result<(), Box<dyn Error>>,
) -> Result<(), Box<dyn Error>> {
    if !software_mode {
        return result;
    }
    match (result, cooltek_sdk_cli::mode::set(transport, module, false)) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) => Err(error),
        (Ok(()), Err(error)) => {
            Err(format!("capture completed; restoring hardware mode failed: {error}").into())
        }
        (Err(error), Err(cleanup)) => {
            Err(format!("{error}; restoring hardware mode also failed: {cleanup}").into())
        }
    }
}

fn select_dial<'a>(
    devices: &'a [ModuleIdentity],
    path: &[u8],
) -> Result<&'a ModuleIdentity, Box<dyn Error>> {
    let selected = select_device(devices, path, false)?;
    if ModuleKind::from_ids(selected.vendor_id, selected.product_id) != Some(ModuleKind::M131E) {
        return Err("dial commands require an M131E (2516:0206 or 2516:9994)".into());
    }
    Ok(selected)
}

fn capture_input<T: ReportTransport>(
    reader: &mut InputReader<T>,
    writer: &mut impl Write,
    started: Instant,
    duration: Duration,
    timeout_ms: i32,
    max_reports: Option<u64>,
    interrupted: &AtomicBool,
) -> Result<(), Box<dyn Error>> {
    let mut count = 0;
    loop {
        if interrupted.load(Ordering::SeqCst) {
            return finish(writer, started, count, EndReason::Interrupted, None);
        }
        let Some(remaining) = duration.checked_sub(started.elapsed()) else {
            return finish(writer, started, count, EndReason::DurationElapsed, None);
        };
        // HIDAPI takes integer milliseconds. Stop before a sub-millisecond tail;
        // timeout=0 is a poll and timeout=-1 is unbounded, neither is appropriate here.
        let remaining_ms = remaining.as_millis();
        if remaining_ms == 0 {
            return finish(writer, started, count, EndReason::DurationElapsed, None);
        }
        let read_timeout = i32::try_from(remaining_ms.min(timeout_ms as u128))?;
        match reader.read_input(read_timeout) {
            Ok(None) => continue,
            Ok(Some(bytes)) => {
                write_record(
                    writer,
                    &Record::Report(Report {
                        timestamp_micros: timestamp(started),
                        raw_hex: encode_hex(bytes),
                    }),
                )?;
                count += 1;
                if max_reports.is_some_and(|maximum| count >= maximum) {
                    return finish(writer, started, count, EndReason::MaxReports, None);
                }
            }
            Err(error) => {
                finish(
                    writer,
                    started,
                    count,
                    EndReason::TransportError,
                    Some(format!("input read failed: {error}")),
                )?;
                return Err(error.into());
            }
        }
    }
}

fn descriptor(device: &HidDevice) -> Descriptor {
    let mut raw = [0u8; MAX_REPORT_DESCRIPTOR_SIZE];
    match device.get_report_descriptor(&mut raw) {
        Ok(length) if (1..=raw.len()).contains(&length) => Descriptor::Available {
            raw_hex: encode_hex(&raw[..length]),
        },
        Ok(0) => Descriptor::Error {
            message: "HIDAPI returned an empty report descriptor".into(),
        },
        Ok(_) => Descriptor::Error {
            message: "HIDAPI returned an invalid descriptor length".into(),
        },
        Err(error) => Descriptor::Error {
            message: error.to_string(),
        },
    }
}

fn timestamp(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX)
}

fn finish(
    writer: &mut impl Write,
    started: Instant,
    report_count: u64,
    reason: EndReason,
    error: Option<String>,
) -> Result<(), Box<dyn Error>> {
    write_record(
        writer,
        &Record::End(End {
            timestamp_micros: timestamp(started),
            report_count,
            reason,
            error,
        }),
    )?;
    Ok(())
}

fn replay(file: PathBuf) -> Result<(), Box<dyn Error>> {
    // This path constructs no registry or HidApi instance. Every module decodes
    // through the same InputSession as masterhub-service; M003K and M131E keep
    // their established diagnostic event shapes.
    let (mut reader, session) = CaptureReader::new(BufReader::new(File::open(file)?))?;
    let module = ModuleKind::from_ids(session.identity.vendor_id, session.identity.product_id);
    let mut input = match module {
        Some(ModuleKind::M003K) => Some(ServiceModule::M003K),
        Some(ModuleKind::M131E) => Some(ServiceModule::M131E),
        Some(ModuleKind::M4315) => Some(ServiceModule::M4315),
        Some(ModuleKind::M005S) => Some(ServiceModule::M005S),
        Some(ModuleKind::M032LE) => Some(ServiceModule::M032LE),
        _ => None,
    }
    .map(InputSession::new);
    let mut dial_pressed_at = None;
    let mut output = io::stdout().lock();
    while let Some(report) = reader.next_report()? {
        let raw = decode_hex(&report.raw_hex, MAX_REPORT_BYTES)?;
        let mut item = json!({
            "schema_version": SCHEMA_VERSION,
            "source": session.source,
            "identity": session.identity,
            "timestamp_micros": report.timestamp_micros,
            "raw_hex": report.raw_hex,
        });
        match input
            .as_mut()
            .map(|input| input.decode(&raw, report.timestamp_micros))
        {
            None | Some(Ok(InputUpdate::Unknown)) => item["type"] = json!("unknown"),
            // Gesture-timing errors are reported here too, leaving state unchanged.
            Some(Err(error)) => {
                item["type"] = json!("parse_error");
                item["error"] = json!(error);
            }
            Some(Ok(InputUpdate::Events(events))) => {
                item["type"] = json!("events");
                match module {
                    Some(ModuleKind::M003K) => {
                        item["events"] = events.iter().map(knob_event).collect();
                    }
                    Some(ModuleKind::M131E) => {
                        let (decoded, gestures) = dial_events(&events, &mut dial_pressed_at);
                        item["events"] = json!(decoded);
                        item["gestures"] = json!(gestures);
                    }
                    _ => item["events"] = json!(events),
                }
            }
        }
        serde_json::to_writer(&mut output, &item)?;
        writeln!(output)?;
    }
    let end = reader.end().ok_or("capture is missing its end record")?;
    serde_json::to_writer(
        &mut output,
        &json!({
            "type": "capture_end",
            "schema_version": SCHEMA_VERSION,
            "source": session.source,
            "identity": session.identity,
            "timestamp_micros": end.timestamp_micros,
            "report_count": end.report_count,
            "reason": end.reason,
            "error": end.error,
        }),
    )?;
    writeln!(output)?;
    Ok(())
}

fn knob_event(event: &InputEvent) -> Value {
    let kind = match event.gesture {
        Gesture::Clockwise => json!({"type":"rotated", "direction":"clockwise"}),
        Gesture::CounterClockwise => json!({"type":"rotated", "direction":"counter_clockwise"}),
        Gesture::Pressed => json!({"type":"pressed"}),
        _ => json!({"type":"released"}),
    };
    json!({"knob_index":event.control, "kind":kind})
}

/// Splits normalized dial events into wire events and host press gestures.
///
/// The session emits at most one `pressed` per physical press and follows a
/// matched release with its gesture, so the hold time is release minus press.
fn dial_events(events: &[InputEvent], pressed_at: &mut Option<u64>) -> (Vec<Value>, Vec<Value>) {
    let mut decoded = Vec::new();
    let mut gestures = Vec::new();
    for event in events {
        match event.gesture {
            Gesture::ShortPress | Gesture::LongPress => {
                let held = pressed_at
                    .take()
                    .map_or(0, |pressed| event.timestamp_micros.saturating_sub(pressed));
                let kind = if event.gesture == Gesture::ShortPress {
                    "short_press"
                } else {
                    "long_press"
                };
                gestures.push(json!({"type":kind, "held_micros":held}));
            }
            Gesture::Right => decoded.push(json!({"type":"rotated", "direction":"right"})),
            Gesture::Left => decoded.push(json!({"type":"rotated", "direction":"left"})),
            Gesture::Pressed => {
                pressed_at.get_or_insert(event.timestamp_micros);
                decoded.push(json!({"type":"pressed"}));
            }
            _ => decoded.push(json!({"type":"released"})),
        }
    }
    (decoded, gestures)
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, collections::VecDeque, ffi::CString};

    use cooltek_sdk_masterhub::hidapi::{HidError, HidResult};
    use serde_json::Value;

    use super::*;

    #[test]
    fn exact_paths_parse_as_single_required_values_without_opening_hid() {
        let cli = Cli::try_parse_from([
            "masterhub-diag",
            "record",
            "--path-hex",
            "61ff",
            "--output",
            "capture.jsonl",
        ])
        .unwrap();
        let Command::Record { path_hex, .. } = cli.command else {
            panic!("expected record")
        };
        assert_eq!(path_hex.0, vec![0x61, 0xff]);
        assert!(
            Cli::try_parse_from(["masterhub-diag", "record", "--output", "capture.jsonl"]).is_err()
        );
        for arguments in [
            vec![
                "masterhub-diag",
                "dial",
                "mode",
                "software",
                "--path-hex",
                "61ff",
            ],
            vec![
                "masterhub-diag",
                "dial",
                "backlight",
                "50",
                "--path-hex",
                "61ff",
            ],
            vec![
                "masterhub-diag",
                "dial",
                "color",
                "abcdef",
                "--path-hex",
                "61ff",
            ],
            vec!["masterhub-diag", "dial", "info", "--path-hex", "61ff"],
            vec![
                "masterhub-diag",
                "dial",
                "jpeg",
                "picture.jpg",
                "--path-hex",
                "61ff",
            ],
        ] {
            let cli = Cli::try_parse_from(arguments).unwrap();
            let Command::Dial { command } = cli.command else {
                panic!("expected dial")
            };
            let path = match command {
                DialCommand::PrepareImage(_) => panic!("expected a hardware-capable command"),
                DialCommand::Info { path_hex } => path_hex,
                DialCommand::Mode { path_hex, .. }
                | DialCommand::Backlight { path_hex, .. }
                | DialCommand::Color { path_hex, .. }
                | DialCommand::Jpeg { path_hex, .. } => path_hex.unwrap(),
            };
            assert_eq!(path.0, vec![0x61, 0xff]);
        }
        assert!(Cli::try_parse_from(["masterhub-diag", "dial", "info"]).is_err());
    }

    enum Read {
        Data(Vec<u8>),
        Idle,
        Disconnected,
    }

    struct FakeTransport {
        reads: RefCell<VecDeque<Read>>,
        timeouts: RefCell<Vec<i32>>,
    }

    impl FakeTransport {
        fn new(reads: impl IntoIterator<Item = Read>) -> Self {
            Self {
                reads: RefCell::new(reads.into_iter().collect()),
                timeouts: RefCell::new(Vec::new()),
            }
        }
    }

    impl ReportTransport for FakeTransport {
        fn read_timeout(&self, buffer: &mut [u8], timeout_ms: i32) -> HidResult<usize> {
            self.timeouts.borrow_mut().push(timeout_ms);
            match self.reads.borrow_mut().pop_front().unwrap_or(Read::Idle) {
                Read::Data(bytes) => {
                    buffer[..bytes.len()].copy_from_slice(&bytes);
                    Ok(bytes.len())
                }
                Read::Idle => {
                    std::thread::sleep(Duration::from_millis(timeout_ms as u64));
                    Ok(0)
                }
                Read::Disconnected => Err(HidError::HidApiError {
                    message: "synthetic device unplugged".into(),
                }),
            }
        }

        fn get_feature_report(&self, _: &mut [u8]) -> HidResult<usize> {
            panic!("recording must not query vendor feature reports")
        }

        fn send_feature_report(&self, _: &[u8]) -> HidResult<()> {
            panic!("recording must not initialize or configure devices")
        }
    }

    fn output_records(bytes: Vec<u8>) -> Vec<Value> {
        String::from_utf8(bytes)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    fn device(path: &[u8], product_id: u16, interface_number: i32) -> ModuleIdentity {
        ModuleIdentity {
            path: CString::new(path).unwrap(),
            vendor_id: 0x2516,
            product_id,
            serial_number: None,
            manufacturer_string: None,
            product_string: None,
            release_number: 0,
            interface_number,
            usage_page: 0xff00,
            usage: 1,
            module: cooltek_sdk_masterhub::ModuleKind::from_ids(0x2516, product_id),
        }
    }

    #[test]
    fn selection_requires_exact_unambiguous_path_and_explicit_ips_mode() {
        let devices = [device(b"first", 0x0202, 0), device(b"second", 0x0206, 0)];
        assert_eq!(
            select_device(&devices, b"second", false)
                .unwrap()
                .product_id,
            0x0206
        );
        assert!(select_device(&devices, b"absent", false).is_err());
        assert!(select_device(&devices, b"first", true).is_err());
        assert!(select_device(&devices, b"second", true).is_ok());
        let ambiguous = [device(b"same", 0x0206, 0), device(b"same", 0x0206, 1)];
        assert!(select_device(&ambiguous, b"same", false).is_err());
        assert!(select_device(&[device(b"new-dial", 0x9994, 0)], b"new-dial", true).is_ok());
        for product_id in [0x01fe, 0x9997] {
            assert!(select_device(&[device(b"keys", product_id, 0)], b"keys", true).is_ok());
        }
        let mut wrong_vendor = device(b"wrong-vendor", 0x9994, 0);
        wrong_vendor.vendor_id = 1;
        assert!(select_device(&[wrong_vendor], b"wrong-vendor", true).is_err());
    }

    #[test]
    fn dial_selection_accepts_both_m131e_product_ids() {
        for product_id in [0x0206, 0x9994] {
            let mut dial = device(b"dial", product_id, 0);
            dial.module = Some(ModuleKind::M4315);
            let devices = [dial];
            let selected = select_dial(&devices, b"dial").unwrap();
            assert_eq!(selected.product_id, product_id);
        }
    }

    #[test]
    fn dial_selection_rejects_keypad_other_modules_and_wrong_vendor() {
        for product_id in [0x01fe, 0x9997, 0x0202, 0x9991, 0x9995, 0xffff] {
            let mut other_module = device(b"other-module", product_id, 0);
            other_module.module = Some(ModuleKind::M131E);
            let devices = [device(b"dial", 0x0206, 0), other_module];
            let error = select_dial(&devices, b"other-module").unwrap_err();
            assert!(error.to_string().contains("dial commands require an M131E"));
        }
        let mut wrong_vendor = device(b"wrong-vendor", 0x9994, 0);
        wrong_vendor.vendor_id = 1;
        let devices = [wrong_vendor];
        let error = select_dial(&devices, b"wrong-vendor").unwrap_err();
        assert!(error.to_string().contains("dial commands require an M131E"));
    }

    #[test]
    fn dial_selection_requires_an_exact_unambiguous_path() {
        let devices = [device(b"dial", 0x0206, 0), device(b"other-dial", 0x9994, 0)];
        assert!(select_dial(&devices, b"absent").is_err());
        assert!(select_dial(&devices, b"dia").is_err());
        assert_eq!(
            select_dial(&devices, b"other-dial").unwrap().product_id,
            0x9994
        );
        let ambiguous = [device(b"same", 0x0206, 0), device(b"same", 0x9994, 1)];
        assert!(select_dial(&ambiguous, b"same").is_err());
    }

    #[test]
    fn requested_mode_is_restored_after_success_and_capture_failure() {
        struct CleanupTransport {
            writes: RefCell<Vec<Vec<u8>>>,
            fail: bool,
        }
        impl ReportTransport for CleanupTransport {
            fn read_timeout(&self, _: &mut [u8], _: i32) -> HidResult<usize> {
                panic!("cleanup must not read input")
            }
            fn get_feature_report(&self, _: &mut [u8]) -> HidResult<usize> {
                panic!("cleanup must not query features")
            }
            fn send_feature_report(&self, data: &[u8]) -> HidResult<()> {
                self.writes.borrow_mut().push(data.to_vec());
                if self.fail {
                    Err(HidError::HidApiError {
                        message: "cleanup disconnected".into(),
                    })
                } else {
                    Ok(())
                }
            }
        }
        let transport = CleanupTransport {
            writes: RefCell::new(Vec::new()),
            fail: false,
        };
        cleanup_record_mode(&transport, Some(ModuleKind::M131E), false, Ok(())).unwrap();
        assert!(transport.writes.borrow().is_empty());
        cleanup_record_mode(&transport, Some(ModuleKind::M131E), true, Ok(())).unwrap();
        let error = cleanup_record_mode(
            &transport,
            Some(ModuleKind::M131E),
            true,
            Err("capture failed".into()),
        )
        .unwrap_err();
        assert_eq!(error.to_string(), "capture failed");
        assert_eq!(
            *transport.writes.borrow(),
            vec![dial::software_mode_report(false).to_vec(); 2]
        );
        let failed = CleanupTransport {
            writes: RefCell::new(Vec::new()),
            fail: true,
        };
        assert!(
            cleanup_record_mode(&failed, Some(ModuleKind::M131E), true, Ok(()))
                .unwrap_err()
                .to_string()
                .contains("capture completed")
        );
        let error = cleanup_record_mode(
            &failed,
            Some(ModuleKind::M131E),
            true,
            Err("capture failed".into()),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("capture failed") && error.contains("cleanup disconnected"));
    }

    #[test]
    fn failed_mode_enable_attempts_cleanup_even_if_footer_write_fails() {
        struct FailedEnable(RefCell<Vec<Vec<u8>>>);
        impl ReportTransport for FailedEnable {
            fn read_timeout(&self, _: &mut [u8], _: i32) -> HidResult<usize> {
                panic!("must not read")
            }
            fn get_feature_report(&self, _: &mut [u8]) -> HidResult<usize> {
                panic!("must not query")
            }
            fn send_feature_report(&self, data: &[u8]) -> HidResult<()> {
                self.0.borrow_mut().push(data.to_vec());
                if data[2] == 1 {
                    Err(HidError::HidApiError {
                        message: "enable failed".into(),
                    })
                } else {
                    Ok(())
                }
            }
        }
        struct FailedWriter;
        impl Write for FailedWriter {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::Error::other("disk full"))
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        for fail_footer in [false, true] {
            let transport = FailedEnable(RefCell::new(Vec::new()));
            let mut bytes = Vec::new();
            let result = if fail_footer {
                begin_record_mode(
                    &transport,
                    Some(ModuleKind::M131E),
                    true,
                    &mut FailedWriter,
                    Instant::now(),
                )
            } else {
                begin_record_mode(
                    &transport,
                    Some(ModuleKind::M131E),
                    true,
                    &mut bytes,
                    Instant::now(),
                )
            };
            let error = result.unwrap_err().to_string();
            assert!(error.contains("enable failed"));
            assert_eq!(error.contains("disk full"), fail_footer);
            assert_eq!(
                *transport.0.borrow(),
                [
                    dial::software_mode_report(true).to_vec(),
                    dial::software_mode_report(false).to_vec()
                ]
            );
            if !fail_footer {
                assert_eq!(output_records(bytes)[0]["reason"], "transport_error");
            }
        }
    }

    #[test]
    fn recording_is_read_only_and_retains_actual_lengths() {
        let mut reader = InputReader::new(FakeTransport::new([
            Read::Data(vec![1, 2, 3, 4]),
            Read::Data(vec![5]),
        ]));
        let mut output = Vec::new();
        capture_input(
            &mut reader,
            &mut output,
            Instant::now(),
            Duration::from_secs(1),
            100,
            Some(2),
            &AtomicBool::new(false),
        )
        .unwrap();
        let records = output_records(output);
        assert_eq!(records[0]["raw_hex"], "01020304");
        assert_eq!(records[1]["raw_hex"], "05");
        assert_eq!(records[2]["reason"], "max_reports");
        assert_eq!(records[2]["report_count"], 2);
    }

    #[test]
    fn idle_capture_ends_at_deadline_with_positive_capped_timeouts() {
        let mut reader = InputReader::new(FakeTransport::new([Read::Idle]));
        let mut output = Vec::new();
        let started = Instant::now();
        capture_input(
            &mut reader,
            &mut output,
            started,
            Duration::from_millis(10),
            100,
            None,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(started.elapsed() < Duration::from_millis(500));
        assert!(
            reader
                .transport()
                .timeouts
                .borrow()
                .iter()
                .all(|timeout| (1..=10).contains(timeout))
        );
        let records = output_records(output);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0]["reason"], "duration_elapsed");
        assert_eq!(records[0]["report_count"], 0);
    }

    #[test]
    fn interrupt_writes_an_interrupted_footer_and_restores_requested_mode() {
        struct Interrupting {
            reads: RefCell<VecDeque<Vec<u8>>>,
            writes: RefCell<Vec<Vec<u8>>>,
            flag: std::sync::Arc<AtomicBool>,
        }
        impl ReportTransport for Interrupting {
            fn read_timeout(&self, buffer: &mut [u8], _: i32) -> HidResult<usize> {
                // Simulate Ctrl-C arriving while the second report is read.
                let bytes = self.reads.borrow_mut().pop_front().unwrap_or_default();
                if self.reads.borrow().is_empty() {
                    cooltek_sdk_cli::interrupt::signal_received(&self.flag);
                }
                buffer[..bytes.len()].copy_from_slice(&bytes);
                Ok(bytes.len())
            }
            fn get_feature_report(&self, _: &mut [u8]) -> HidResult<usize> {
                panic!("recording must not query vendor feature reports")
            }
            fn send_feature_report(&self, data: &[u8]) -> HidResult<()> {
                self.writes.borrow_mut().push(data.to_vec());
                Ok(())
            }
        }
        let flag = std::sync::Arc::new(AtomicBool::new(false));
        let mut reader = InputReader::new(Interrupting {
            reads: RefCell::new(VecDeque::from([vec![1, 0x44, 3, 0, 3, 0, 0], vec![2]])),
            writes: RefCell::new(Vec::new()),
            flag: std::sync::Arc::clone(&flag),
        });
        let mut output = Vec::new();
        record_session(
            &mut reader,
            &mut output,
            RecordSettings {
                module: Some(ModuleKind::M131E),
                software_mode: true,
                started: Instant::now(),
                duration: Duration::from_secs(60),
                timeout_ms: 100,
                max_reports: None,
            },
            &flag,
        )
        .unwrap();
        assert_eq!(
            *reader.transport().writes.borrow(),
            [
                dial::software_mode_report(true).to_vec(),
                dial::software_mode_report(false).to_vec()
            ]
        );
        let records = output_records(output.clone());
        assert_eq!(records.len(), 3);
        assert_eq!(records[1]["raw_hex"], "02");
        assert_eq!(records[2]["type"], "end");
        assert_eq!(records[2]["reason"], "interrupted");
        assert_eq!(records[2]["report_count"], 2);
        assert!(records[2].get("error").is_none());

        // The footer is a complete capture for both replay paths.
        let mut capture = serde_json::to_vec(&Record::Session(Session {
            schema_version: SCHEMA_VERSION,
            source: Source::Synthetic,
            started_unix_micros: 0,
            identity: identity(&device(b"dial", 0x0206, 0)),
            software_mode_requested: true,
            report_descriptor: Descriptor::Error {
                message: "synthetic".into(),
            },
        }))
        .unwrap();
        capture.push(b'\n');
        capture.extend(output);
        let (mut reader, _) = CaptureReader::new(capture.as_slice()).unwrap();
        while reader.next_report().unwrap().is_some() {}
        assert_eq!(reader.end().unwrap().reason, EndReason::Interrupted);
    }

    #[test]
    fn interrupt_before_any_read_ends_without_reading() {
        let mut reader = InputReader::new(FakeTransport::new([Read::Data(vec![1])]));
        let mut output = Vec::new();
        capture_input(
            &mut reader,
            &mut output,
            Instant::now(),
            Duration::from_secs(1),
            100,
            None,
            &AtomicBool::new(true),
        )
        .unwrap();
        assert!(reader.transport().timeouts.borrow().is_empty());
        let records = output_records(output);
        assert_eq!(records[0]["reason"], "interrupted");
        assert_eq!(records[0]["report_count"], 0);
    }

    #[test]
    fn unplug_preserves_partial_reports_and_writes_error_footer() {
        let mut reader = InputReader::new(FakeTransport::new([
            Read::Data(vec![9]),
            Read::Disconnected,
        ]));
        let mut output = Vec::new();
        assert!(
            capture_input(
                &mut reader,
                &mut output,
                Instant::now(),
                Duration::from_secs(1),
                100,
                None,
                &AtomicBool::new(false),
            )
            .is_err()
        );
        let records = output_records(output);
        assert_eq!(records[0]["raw_hex"], "09");
        assert_eq!(records[1]["reason"], "transport_error");
        assert_eq!(records[1]["report_count"], 1);
        assert!(records[1]["error"].as_str().unwrap().contains("unplugged"));
    }
}
