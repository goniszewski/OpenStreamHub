use std::{
    error::Error,
    fs::File,
    io::{self, BufReader, Read, Write},
    path::PathBuf,
    process::ExitCode,
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};

use clap::{Parser, Subcommand};
use cooltek_sdk_cli::capture::{
    CaptureReader, EndReason, MAX_REPORT_BYTES, decode_hex, encode_hex,
};
use cooltek_sdk_cli::{
    device::{DevicePath, TIMEOUT_MS_RANGE, parse_path, select_device},
    live_output::TimedStdout,
    live_runtime::{LiveOptions, LiveSummary, run_live},
    service_events::{emit, end_record, plan_report, process_work, start_record},
};
use cooltek_sdk_masterhub::{DeviceRegistry, InputReader, ModuleKind};
use cooltek_sdk_service::{
    ActionDispatcher, DeviceContext, InputSession, Profile, ProfileSession, ServiceModule,
    macos::MacOsDispatcher,
};
use serde_json::json;

#[derive(Parser)]
#[command(
    name = "masterhub-service",
    version,
    about = "Bind MasterHUB gestures to macOS actions",
    long_about = "Bind MasterHUB gestures to macOS actions. Replay always plans actions without HID or macOS access. Live runs default to dry-run; --execute enables actions, while --software-mode independently enables software mode for the IPS dial or keys. Each process owns one explicitly selected interface. Ctrl-C or SIGTERM ends the session and attempts requested mode cleanup; a second Ctrl-C exits immediately."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Validate a JSON profile without accessing HID or running actions.
    Validate { profile: PathBuf },
    /// Plan bindings from a completed diagnostic capture, always without actions.
    Replay {
        #[arg(long)]
        profile: PathBuf,
        capture: PathBuf,
    },
    /// Read one exact interface until interrupted, optionally executing actions.
    ///
    /// JSON records are written to stdout with bounded write deadlines, so stdout
    /// must be a terminal or pipe. Redirecting to a regular file (`> file.jsonl`)
    /// is rejected; to save the log, pipe through tee instead:
    /// `masterhub-service run ... | tee session.jsonl`.
    ///
    /// A failed action is logged as `action_failed` and the session continues,
    /// unless --stop-on-action-error is given. Log-output and device errors
    /// always end the session.
    Run {
        #[arg(long)]
        profile: PathBuf,
        /// Lossless path from masterhub-diag list, without the trailing NUL.
        #[arg(long, value_parser = parse_path)]
        path_hex: DevicePath,
        /// Enable real macOS actions. Without this flag actions are only logged.
        #[arg(long)]
        execute: bool,
        /// Request IPS dial/key software mode and restore hardware mode on exit.
        #[arg(long)]
        software_mode: bool,
        #[arg(long, value_parser = clap::value_parser!(u64).range(1..=86_400))]
        duration_secs: Option<u64>,
        #[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
        max_reports: Option<u64>,
        /// Maximum wait for each read (1 to 1000 ms).
        #[arg(long, default_value_t = 100, value_parser = clap::value_parser!(i32).range(TIMEOUT_MS_RANGE))]
        timeout_ms: i32,
        /// End the session on the first failed action instead of logging it and continuing.
        #[arg(long)]
        stop_on_action_error: bool,
    },
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("masterhub-service: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<(), Box<dyn Error>> {
    match cli.command {
        Command::Validate { profile } => {
            let profile = load_profile(profile)?;
            emit(
                &mut io::stdout().lock(),
                json!({
                    "type":"profile_valid", "schema_version":profile.schema_version,
                    "name":profile.name, "binding_count":profile.binding_count(),
                }),
            )?;
        }
        Command::Replay { profile, capture } => {
            let engine = ProfileSession::new(load_profile(profile)?)?;
            replay(engine, capture, &mut io::stdout().lock())?;
        }
        Command::Run {
            profile,
            path_hex,
            execute,
            software_mode,
            duration_secs,
            max_reports,
            timeout_ms,
            stop_on_action_error,
        } => {
            let profile = load_profile(profile)?;
            let profile_name = profile.name.clone();
            if execute && !cfg!(target_os = "macos") {
                return Err("--execute is supported only on macOS".into());
            }
            // Reject unsupported stdout (such as a regular file) before any HID access.
            let stdout = TimedStdout::new(Duration::from_millis(500))?;
            let stopped = cooltek_sdk_cli::interrupt::install()?;
            let registry = DeviceRegistry::new()?;
            let devices = registry.devices();
            let selected = select_device(&devices, &path_hex, software_mode)?;
            let device = device_context(
                selected.vendor_id,
                selected.product_id,
                selected.serial_number.clone(),
            )?;
            let engine = live_bindings(profile, device.module)?;
            let reader = InputReader::new(registry.open(selected)?);
            let details = json!({
                "source":"hardware", "profile":profile_name, "path_hex":encode_hex(&path_hex),
                "vendor_id":selected.vendor_id, "product_id":selected.product_id,
                "interface_number":selected.interface_number, "usage_page":selected.usage_page,
                "usage":selected.usage, "software_mode_requested":software_mode,
                "queue_capacity":64, "log_timeout_ms":500,
                "drain_timeout_ms":10000, "cancellation_timeout_ms":2000,
                "stop_on_action_error":stop_on_action_error,
            });
            run_live(
                reader,
                engine,
                device,
                execute,
                details,
                LiveOptions {
                    software_mode,
                    duration: duration_secs.map(Duration::from_secs),
                    max_reports,
                    timeout_ms,
                    stop_on_action_error,
                    ..LiveOptions::default()
                },
                stopped,
                move |abort| Ok((Dispatcher::new(execute, abort), stdout)),
            )?;
        }
    }
    Ok(())
}

// Folder pages need display updates before their mappings can be used live.
// Other modules can retain their existing flat bindings from the same profile.
fn live_bindings(mut profile: Profile, module: ServiceModule) -> Result<ProfileSession, String> {
    profile.validate()?;
    if profile.schema_version == 2 {
        if module == ServiceModule::M4315 {
            return Err("nested keypad profiles are offline only until live display integration is implemented".into());
        }
        profile.schema_version = 1;
        profile.keypad = None;
    }
    ProfileSession::new(profile)
}

fn load_profile(path: PathBuf) -> Result<Profile, Box<dyn Error>> {
    let mut bytes = Vec::new();
    File::open(path)?.take(1_048_577).read_to_end(&mut bytes)?;
    Ok(Profile::from_json(&bytes)?)
}

/// Establishes the module from VID/PID; a blank serial is treated as absent.
fn device_context(
    vendor_id: u16,
    product_id: u16,
    serial_number: Option<String>,
) -> Result<DeviceContext, String> {
    let module = match ModuleKind::from_ids(vendor_id, product_id) {
        Some(ModuleKind::M003K) => ServiceModule::M003K,
        Some(ModuleKind::M131E) => ServiceModule::M131E,
        Some(ModuleKind::M4315) => ServiceModule::M4315,
        Some(ModuleKind::M005S) => ServiceModule::M005S,
        Some(ModuleKind::M032LE) => ServiceModule::M032LE,
        _ => return Err("the binding service supports only the five control modules".into()),
    };
    DeviceContext::new(module, serial_number)
}

enum Dispatcher {
    Preview,
    MacOs(MacOsDispatcher),
}

impl Dispatcher {
    fn new(execute: bool, abort: Arc<AtomicBool>) -> Self {
        if execute {
            Self::MacOs(MacOsDispatcher::with_cancellation(abort))
        } else {
            Self::Preview
        }
    }
}

impl ActionDispatcher for Dispatcher {
    fn dispatch(&mut self, action: &cooltek_sdk_service::Action) -> Result<(), String> {
        match self {
            Self::Preview => action.validate_executable(),
            Self::MacOs(dispatcher) => dispatcher.dispatch(action),
        }
    }
}

/// Plans a completed capture through the same report/event path as live runs.
fn replay(
    mut engine: ProfileSession,
    file: PathBuf,
    output: &mut impl Write,
) -> Result<(), String> {
    let (mut reader, session) = CaptureReader::new(BufReader::new(
        File::open(file).map_err(|error| error.to_string())?,
    ))?;
    let device = device_context(
        session.identity.vendor_id,
        session.identity.product_id,
        session.identity.serial_number.clone(),
    )?;
    let mut input = InputSession::new(device.module);
    let mut dispatcher = Dispatcher::Preview;
    let never = AtomicBool::new(false);
    let mut summary = LiveSummary::default();
    let mut matched_actions = 0u64;
    emit(
        output,
        start_record(
            &device,
            false,
            json!({"source":session.source, "identity":session.identity}),
        ),
    )?;
    let result = (|| {
        while let Some(report) = reader.next_report()? {
            summary.report_count += 1;
            let raw = decode_hex(&report.raw_hex, MAX_REPORT_BYTES)?;
            let work = plan_report(
                &mut input,
                &mut engine,
                &device,
                &raw,
                report.timestamp_micros,
            )?;
            summary.queued_report_count += 1;
            summary.decoded_event_count += work.events.len() as u64;
            matched_actions += work.events.iter().filter(|e| e.action().is_some()).count() as u64;
            process_work(
                work,
                &mut dispatcher,
                output,
                &never,
                false,
                false,
                &mut summary,
            )?;
        }
        let end = reader.end().ok_or("capture end missing")?;
        emit(
            output,
            json!({"type":"capture_end", "reason":end.reason, "error":end.error, "report_count":end.report_count}),
        )?;
        if end.reason == EndReason::TransportError {
            return Err(format!(
                "capture ended after a transport error: {}",
                end.error.as_deref().unwrap_or("unknown")
            ));
        }
        Ok(())
    })();
    input.reset();
    engine.reset();
    summary.incomplete_action_count = matched_actions.saturating_sub(summary.action_count);
    let footer = emit(
        output,
        end_record(
            "capture_complete",
            result.as_ref().err().map(String::as_str),
            &summary,
            false,
        ),
    );
    combine(result, footer)
}

fn combine(primary: Result<(), String>, secondary: Result<(), String>) -> Result<(), String> {
    match (primary, secondary) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
        (Err(error), Err(other)) => Err(format!("{error}; {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cooltek_sdk_service::{Action, Gesture, InputEvent, KeypadResolution};

    fn nested_profile() -> Profile {
        Profile::from_json(include_bytes!(
            "../../../../examples/profiles/keypad-folders.json"
        ))
        .unwrap()
    }

    #[test]
    fn live_keypad_folders_are_rejected_before_a_reader_can_be_opened() {
        let error = live_bindings(nested_profile(), ServiceModule::M4315).unwrap_err();
        assert!(error.contains("offline only"));
        assert!(error.contains("display integration"));
    }

    #[test]
    fn other_modules_keep_their_bindings_in_a_folder_profile() {
        let mut engine = live_bindings(nested_profile(), ServiceModule::M003K).unwrap();
        let device = DeviceContext {
            module: ServiceModule::M003K,
            serial_number: None,
        };
        let event = InputEvent {
            timestamp_micros: 0,
            control: 0,
            gesture: Gesture::Clockwise,
            value: None,
        };
        assert_eq!(
            engine.resolve(&device, &event).unwrap(),
            KeypadResolution::Action {
                action: Action::Volume { delta: 5 },
                page_id: None
            }
        );
    }

    #[test]
    fn blank_serials_are_absent_for_live_and_replay_contexts() {
        for serial in [Some(String::new()), Some("  ".into()), None] {
            let device = device_context(0x2516, 0x0202, serial).unwrap();
            assert_eq!(device.serial_number, None);
        }
    }
}
