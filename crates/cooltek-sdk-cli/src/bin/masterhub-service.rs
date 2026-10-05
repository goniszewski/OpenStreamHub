use std::{
    error::Error,
    fs::File,
    io::{self, BufReader, Read, Write},
    path::PathBuf,
    process::ExitCode,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use clap::{Parser, Subcommand};
use cooltek_sdk_cli::capture::{
    CaptureReader, EndReason, MAX_REPORT_BYTES, decode_hex, encode_hex,
};
use cooltek_sdk_cli::{
    live_output::TimedStdout,
    live_runtime::{LiveOptions, run_live},
};
use cooltek_sdk_masterhub::{DeviceRegistry, InputReader, ModuleIdentity, ModuleKind};
use cooltek_sdk_service::{
    ActionDispatcher, BindingEngine, DeviceContext, InputSession, InputUpdate, KeypadResolution,
    Profile, ProfileSession, ServiceModule, macos::MacOsDispatcher,
};
use serde_json::{Value, json};

#[derive(Parser)]
#[command(
    name = "masterhub-service",
    version,
    about = "Bind MasterHUB gestures to macOS actions",
    long_about = "Bind MasterHUB gestures to macOS actions. Replay always plans actions without HID or macOS access. Live runs default to dry-run; --execute enables actions, while --software-mode independently enables software mode for the IPS dial or keys. Each process owns one explicitly selected interface. Ctrl-C or SIGTERM ends the session and attempts requested mode cleanup."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Debug)]
struct DevicePath(Vec<u8>);

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
        #[arg(long, default_value_t = 100, value_parser = clap::value_parser!(i32).range(1..=1000))]
        timeout_ms: i32,
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
        } => {
            let path_hex = path_hex.0;
            let profile = load_profile(profile)?;
            let profile_name = profile.name.clone();
            if execute && !cfg!(target_os = "macos") {
                return Err("--execute is supported only on macOS".into());
            }
            let stopped = Arc::new(AtomicBool::new(false));
            let signal = Arc::clone(&stopped);
            ctrlc::set_handler(move || signal.store(true, Ordering::SeqCst))?;
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
                    ..LiveOptions::default()
                },
                stopped,
                move |abort| {
                    Ok((
                        Dispatcher::new(execute, abort),
                        TimedStdout::new(Duration::from_millis(500))?,
                    ))
                },
            )?;
        }
    }
    Ok(())
}

// Folder pages need display updates before their mappings can be used live.
// Other modules can retain their existing flat bindings from the same profile.
fn live_bindings(mut profile: Profile, module: ServiceModule) -> Result<BindingEngine, String> {
    profile.validate()?;
    if profile.schema_version == 2 {
        if module == ServiceModule::M4315 {
            return Err("nested keypad profiles are offline only until live display integration is implemented".into());
        }
        profile.schema_version = 1;
        profile.keypad = None;
    }
    BindingEngine::new(profile)
}

fn load_profile(path: PathBuf) -> Result<Profile, Box<dyn Error>> {
    let mut bytes = Vec::new();
    File::open(path)?.take(1_048_577).read_to_end(&mut bytes)?;
    Ok(Profile::from_json(&bytes)?)
}

fn parse_path(raw: &str) -> Result<DevicePath, String> {
    let path = decode_hex(raw, MAX_REPORT_BYTES)?;
    if path.is_empty() || path.contains(&0) {
        return Err("device path must be nonempty and contain no NUL bytes".into());
    }
    Ok(DevicePath(path))
}

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
    Ok(DeviceContext {
        module,
        serial_number,
    })
}

fn select_device<'a>(
    devices: &'a [ModuleIdentity],
    path: &[u8],
    software_mode: bool,
) -> Result<&'a ModuleIdentity, String> {
    let mut matches = devices
        .iter()
        .filter(|device| device.path.as_bytes() == path);
    let selected = matches
        .next()
        .ok_or("selected path is absent; refresh masterhub-diag list")?;
    if matches.next().is_some() {
        return Err("selected path identifies multiple interfaces".into());
    }
    let context = device_context(
        selected.vendor_id,
        selected.product_id,
        selected.serial_number.clone(),
    )?;
    if software_mode && !matches!(context.module, ServiceModule::M131E | ServiceModule::M4315) {
        return Err("--software-mode is supported only for M131E and M4315".into());
    }
    Ok(selected)
}

fn emit(writer: &mut impl Write, value: Value) -> Result<(), String> {
    serde_json::to_writer(&mut *writer, &value).map_err(|error| error.to_string())?;
    writer.write_all(b"\n").map_err(|error| error.to_string())?;
    writer.flush().map_err(|error| error.to_string())
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

struct Runtime<'a, D, W> {
    engine: ProfileSession,
    device: DeviceContext,
    input: InputSession,
    dispatcher: &'a mut D,
    output: &'a mut W,
    execute: bool,
    report_count: u64,
    event_count: u64,
    action_count: u64,
}

impl<'a, D: ActionDispatcher, W: Write> Runtime<'a, D, W> {
    fn new(
        engine: ProfileSession,
        device: DeviceContext,
        dispatcher: &'a mut D,
        output: &'a mut W,
        execute: bool,
    ) -> Self {
        Self {
            input: InputSession::new(device.module),
            engine,
            device,
            dispatcher,
            output,
            execute,
            report_count: 0,
            event_count: 0,
            action_count: 0,
        }
    }

    fn start(&mut self, details: Value) -> Result<(), String> {
        emit(
            self.output,
            json!({
                "type":"service_start", "schema_version":1,
                "device":{"module":self.device.module,"serial_number":self.device.serial_number},
                "execution":if self.execute {"macos"} else {"dry_run"}, "details":details,
            }),
        )
    }

    fn process(
        &mut self,
        raw: &[u8],
        timestamp_micros: u64,
        stopped: Option<&AtomicBool>,
    ) -> Result<bool, String> {
        self.report_count += 1;
        let update = self.input.decode(raw, timestamp_micros);
        let (status, error, events) = match update {
            Ok(InputUpdate::Events(events)) => ("decoded", None, events),
            Ok(InputUpdate::Unknown) => ("unknown", None, Vec::new()),
            Err(error) => ("parse_error", Some(error), Vec::new()),
        };
        emit(
            self.output,
            json!({
                "type":"report", "schema_version":1, "timestamp_micros":timestamp_micros,
                "raw_hex":encode_hex(raw), "status":status, "error":error,
            }),
        )?;
        for event in events {
            if stopped.is_some_and(|flag| flag.load(Ordering::SeqCst)) {
                return Ok(false);
            }
            self.event_count += 1;
            let binding = self.engine.binding_for(&self.device, &event).cloned();
            let resolution = self.engine.resolve(&self.device, &event)?;
            let (action, navigation, page_id) = match resolution {
                KeypadResolution::Action { action, page_id } => (Some(action), None, page_id),
                KeypadResolution::Navigation { navigation } => (None, Some(navigation), None),
                KeypadResolution::Unbound => (None, None, None),
            };
            // Publish the intended action before invoking the host. A failed log
            // write must never cause an unrecorded action.
            emit(
                self.output,
                json!({
                    "type":"event", "schema_version":1, "event":event,
                    "binding":binding, "action":action, "navigation":navigation, "page_id":page_id,
                    "status":if action.is_some() {"planned"} else if navigation.is_some() {"navigation"} else {"unbound"},
                }),
            )?;
            if let Some(action) = action {
                if stopped.is_some_and(|flag| flag.load(Ordering::SeqCst)) {
                    return Ok(false);
                }
                if let Err(error) = self.dispatcher.dispatch(&action) {
                    let log = emit(
                        self.output,
                        json!({
                            "type":"action_failed", "event":event, "action":action, "error":error,
                        }),
                    );
                    return combine(Err(format!("action failed: {error}")), log).map(|_| true);
                }
                self.action_count += 1;
                if self.execute {
                    emit(
                        self.output,
                        json!({"type":"action_executed", "event":event, "action":action}),
                    )?;
                }
            }
        }
        Ok(true)
    }

    fn end(&mut self, reason: &str, error: Option<&str>) -> Result<(), String> {
        self.input.reset();
        self.engine.reset();
        emit(
            self.output,
            json!({
                "type":"service_end", "schema_version":1, "reason":reason, "error":error,
                "report_count":self.report_count, "event_count":self.event_count,
                "action_count":self.action_count, "execution":if self.execute {"macos"} else {"dry_run"},
            }),
        )
    }
}

fn replay(engine: ProfileSession, file: PathBuf, output: &mut impl Write) -> Result<(), String> {
    let (mut reader, session) = CaptureReader::new(BufReader::new(
        File::open(file).map_err(|error| error.to_string())?,
    ))?;
    let device = device_context(
        session.identity.vendor_id,
        session.identity.product_id,
        session.identity.serial_number.clone(),
    )?;
    let mut dispatcher = Dispatcher::Preview;
    let mut runtime = Runtime::new(engine, device, &mut dispatcher, output, false);
    runtime.start(json!({"source":session.source, "identity":session.identity}))?;
    let result = (|| {
        while let Some(report) = reader.next_report()? {
            runtime.process(
                &decode_hex(&report.raw_hex, MAX_REPORT_BYTES)?,
                report.timestamp_micros,
                None,
            )?;
        }
        let end = reader.end().ok_or("capture end missing")?;
        emit(
            runtime.output,
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
    let footer = runtime.end(
        if result.is_ok() {
            "capture_complete"
        } else {
            "error"
        },
        result.as_ref().err().map(String::as_str),
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
    use cooltek_sdk_service::{Action, Gesture, InputEvent};

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
        let engine = live_bindings(nested_profile(), ServiceModule::M003K).unwrap();
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
            engine.action_for(&device, &event).unwrap(),
            Some(Action::Volume { delta: 5 })
        );
    }
}
