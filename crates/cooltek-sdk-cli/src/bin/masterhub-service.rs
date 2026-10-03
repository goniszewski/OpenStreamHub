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
    time::{Duration, Instant},
};

use clap::{Parser, Subcommand};
use cooltek_sdk_cli::capture::{
    CaptureReader, EndReason, MAX_REPORT_BYTES, decode_hex, encode_hex,
};
use cooltek_sdk_masterhub::{
    DeviceRegistry, InputReader, ModuleIdentity, ModuleKind, ReportTransport, dial,
};
use cooltek_sdk_service::{
    ActionDispatcher, BindingEngine, DeviceContext, InputSession, InputUpdate, Profile,
    ServiceModule, macos::MacOsDispatcher,
};
use serde_json::{Value, json};

#[derive(Parser)]
#[command(
    name = "masterhub-service",
    version,
    about = "Bind MasterHUB gestures to macOS actions",
    long_about = "Bind MasterHUB gestures to macOS actions. Replay always plans actions without HID or macOS access. Live runs default to dry-run; --execute enables actions, while --software-mode independently enables dial software mode. Each process owns one explicitly selected interface. Ctrl-C or SIGTERM ends the session and attempts requested dial cleanup."
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
        /// Explicitly request dial software mode and restore hardware mode on exit.
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
                    "type":"profile_valid", "schema_version":1,
                    "name":profile.name, "binding_count":profile.bindings.len(),
                }),
            )?;
        }
        Command::Replay { profile, capture } => {
            let engine = BindingEngine::new(load_profile(profile)?)?;
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
            let engine = BindingEngine::new(profile)?;
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
            let mut reader = InputReader::new(registry.open(selected)?);
            let mut output = io::stdout().lock();
            let mut dispatcher = Dispatcher::new(execute);
            let mut runtime = Runtime::new(engine, device, &mut dispatcher, &mut output, execute);
            runtime.start(json!({
                "source":"hardware", "profile":profile_name, "path_hex":encode_hex(&path_hex),
                "vendor_id":selected.vendor_id, "product_id":selected.product_id,
                "interface_number":selected.interface_number, "usage_page":selected.usage_page,
                "usage":selected.usage, "software_mode_requested":software_mode,
            }))?;
            live_session(
                &mut reader,
                &mut runtime,
                LiveOptions {
                    software_mode,
                    duration: duration_secs.map(Duration::from_secs),
                    max_reports,
                    timeout_ms,
                },
                &stopped,
            )?;
        }
    }
    Ok(())
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
        _ => return Err("the binding service supports only M003K and M131E interfaces".into()),
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
    if software_mode && context.module != ServiceModule::M131E {
        return Err("--software-mode is supported only for the M131E dial".into());
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
    fn new(execute: bool) -> Self {
        if execute {
            Self::MacOs(MacOsDispatcher::new())
        } else {
            Self::Preview
        }
    }
}

impl ActionDispatcher for Dispatcher {
    fn dispatch(&mut self, action: &cooltek_sdk_service::Action) -> Result<(), String> {
        match self {
            Self::Preview => action.validate(),
            Self::MacOs(dispatcher) => dispatcher.dispatch(action),
        }
    }
}

struct Runtime<'a, D, W> {
    engine: BindingEngine,
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
        engine: BindingEngine,
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
            let binding = self.engine.binding_for(&self.device, &event);
            // Publish the intended action before invoking the host. A failed log
            // write must never cause an unrecorded action.
            emit(
                self.output,
                json!({
                    "type":"event", "schema_version":1, "event":event,
                    "binding":binding, "status":if binding.is_some() {"planned"} else {"unbound"},
                }),
            )?;
            if let Some(binding) = binding {
                if stopped.is_some_and(|flag| flag.load(Ordering::SeqCst)) {
                    return Ok(false);
                }
                if let Err(error) = self.dispatcher.dispatch(&binding.action) {
                    let log = emit(
                        self.output,
                        json!({
                            "type":"action_failed", "event":event, "action":binding.action, "error":error,
                        }),
                    );
                    return combine(Err(format!("action failed: {error}")), log).map(|_| true);
                }
                self.action_count += 1;
                if self.execute {
                    emit(
                        self.output,
                        json!({"type":"action_executed", "event":event, "action":binding.action}),
                    )?;
                }
            }
        }
        Ok(true)
    }

    fn end(&mut self, reason: &str, error: Option<&str>) -> Result<(), String> {
        self.input.reset();
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

fn replay(engine: BindingEngine, file: PathBuf, output: &mut impl Write) -> Result<(), String> {
    let (mut reader, session) = CaptureReader::new(BufReader::new(
        File::open(file).map_err(|error| error.to_string())?,
    ))?;
    let device = device_context(
        session.identity.vendor_id,
        session.identity.product_id,
        session.identity.serial_number.clone(),
    )?;
    let mut dispatcher = Dispatcher::new(false);
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

#[derive(Clone, Copy)]
struct LiveOptions {
    software_mode: bool,
    duration: Option<Duration>,
    max_reports: Option<u64>,
    timeout_ms: i32,
}

fn live_session<T: ReportTransport, D: ActionDispatcher, W: Write>(
    reader: &mut InputReader<T>,
    runtime: &mut Runtime<'_, D, W>,
    options: LiveOptions,
    stopped: &AtomicBool,
) -> Result<(), String> {
    if !(1..=1000).contains(&options.timeout_ms) {
        return Err("read timeout must be between 1 and 1000 ms".into());
    }
    if options.software_mode && runtime.device.module != ServiceModule::M131E {
        return Err("software mode requires an M131E session".into());
    }
    let started = Instant::now();
    let mut mode_attempted = false;
    let result: Result<&str, String> = (|| {
        if stopped.load(Ordering::SeqCst) {
            return Ok("interrupted");
        }
        if options.software_mode {
            mode_attempted = true;
            dial::set_software_mode(reader.transport(), true).map_err(|error| error.to_string())?;
        }
        loop {
            if stopped.load(Ordering::SeqCst) {
                return Ok("interrupted");
            }
            let timeout_ms = if let Some(duration) = options.duration {
                let remaining = duration.saturating_sub(started.elapsed());
                if remaining.as_millis() == 0 {
                    return Ok("duration_elapsed");
                }
                remaining.as_millis().min(options.timeout_ms as u128) as i32
            } else {
                options.timeout_ms
            };
            if let Some(raw) = reader
                .read_input(timeout_ms)
                .map_err(|error| format!("input read failed: {error}"))?
            {
                if stopped.load(Ordering::SeqCst) {
                    return Ok("interrupted");
                }
                if !runtime.process(
                    raw,
                    u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX),
                    Some(stopped),
                )? {
                    return Ok("interrupted");
                }
                if options
                    .max_reports
                    .is_some_and(|maximum| runtime.report_count >= maximum)
                {
                    return Ok("max_reports");
                }
            }
        }
    })();
    runtime.input.reset();
    let cleanup = if mode_attempted {
        dial::set_software_mode(reader.transport(), false)
            .map_err(|error| format!("restoring dial hardware mode failed: {error}"))
    } else {
        Ok(())
    };
    let reason = result.as_ref().copied().unwrap_or("error");
    let result = combine(result.map(|_| ()), cleanup);
    let footer = runtime.end(
        if result.is_err() { "error" } else { reason },
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
    use cooltek_sdk_masterhub::hidapi::{HidError, HidResult};
    use cooltek_sdk_service::Action;
    use std::{cell::RefCell, collections::VecDeque, rc::Rc};

    #[derive(Default)]
    struct Actions {
        received: Vec<Action>,
        fail: bool,
        stop_on_first: Option<Arc<AtomicBool>>,
    }
    impl ActionDispatcher for Actions {
        fn dispatch(&mut self, action: &Action) -> Result<(), String> {
            self.received.push(action.clone());
            if self.received.len() == 1
                && let Some(stopped) = &self.stop_on_first
            {
                stopped.store(true, Ordering::SeqCst);
            }
            if self.fail {
                Err("synthetic action failure".into())
            } else {
                Ok(())
            }
        }
    }

    enum ReadResult {
        Data(Vec<u8>),
        Disconnected,
        Idle,
    }
    #[derive(Default)]
    struct Transport {
        reads: RefCell<VecDeque<ReadResult>>,
        writes: Rc<RefCell<Vec<Vec<u8>>>>,
        timeouts: Rc<RefCell<Vec<i32>>>,
        fail_enable: bool,
        fail_cleanup: bool,
        interrupt_on_read: Option<Arc<AtomicBool>>,
    }
    fn hid_error(message: &str) -> HidError {
        HidError::HidApiError {
            message: message.into(),
        }
    }
    impl ReportTransport for Transport {
        fn read_timeout(&self, buffer: &mut [u8], timeout_ms: i32) -> HidResult<usize> {
            self.timeouts.borrow_mut().push(timeout_ms);
            if let Some(stopped) = &self.interrupt_on_read {
                stopped.store(true, Ordering::SeqCst);
            }
            match self
                .reads
                .borrow_mut()
                .pop_front()
                .unwrap_or(ReadResult::Idle)
            {
                ReadResult::Data(raw) => {
                    buffer[..raw.len()].copy_from_slice(&raw);
                    Ok(raw.len())
                }
                ReadResult::Disconnected => Err(hid_error("synthetic unplug")),
                ReadResult::Idle => {
                    std::thread::sleep(Duration::from_millis(timeout_ms as u64));
                    Ok(0)
                }
            }
        }
        fn get_feature_report(&self, _: &mut [u8]) -> HidResult<usize> {
            panic!("service must not query features")
        }
        fn send_feature_report(&self, data: &[u8]) -> HidResult<()> {
            self.writes.borrow_mut().push(data.to_vec());
            if self.fail_enable && data[2] == 1 {
                Err(hid_error("enable failed"))
            } else if self.fail_cleanup && data[2] == 0 {
                Err(hid_error("cleanup failed"))
            } else {
                Ok(())
            }
        }
    }

    fn engine() -> BindingEngine {
        BindingEngine::new(
            Profile::from_json(include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../examples/profiles/macos.json"
            )))
            .unwrap(),
        )
        .unwrap()
    }
    fn context(module: ServiceModule) -> DeviceContext {
        DeviceContext {
            module,
            serial_number: None,
        }
    }
    fn options(software_mode: bool) -> LiveOptions {
        LiveOptions {
            software_mode,
            duration: Some(Duration::from_millis(20)),
            max_reports: Some(1),
            timeout_ms: 10,
        }
    }
    fn dial_rotation() -> ReadResult {
        ReadResult::Data(vec![1, 0x44, 3, 0, 3, 0, 0])
    }
    fn expected_modes() -> Vec<Vec<u8>> {
        vec![
            dial::software_mode_report(true).to_vec(),
            dial::software_mode_report(false).to_vec(),
        ]
    }

    #[test]
    fn default_live_session_is_read_only_and_dispatches_known_binding() {
        let transport = Transport {
            reads: RefCell::new(VecDeque::from([dial_rotation()])),
            ..Transport::default()
        };
        let writes = Rc::clone(&transport.writes);
        let mut reader = InputReader::new(transport);
        let mut actions = Actions::default();
        let mut output = Vec::new();
        let mut runtime = Runtime::new(
            engine(),
            context(ServiceModule::M131E),
            &mut actions,
            &mut output,
            false,
        );
        live_session(
            &mut reader,
            &mut runtime,
            options(false),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(writes.borrow().is_empty());
        assert_eq!(actions.received, vec![Action::Volume { delta: 5 }]);
        assert!(String::from_utf8(output).unwrap().contains("max_reports"));
    }

    #[test]
    fn software_mode_is_restored_after_success_unplug_and_failed_activation() {
        for (reads, fail_enable, expected_error) in [
            (VecDeque::from([dial_rotation()]), false, None),
            (
                VecDeque::from([ReadResult::Disconnected]),
                false,
                Some("synthetic unplug"),
            ),
            (VecDeque::new(), true, Some("enable failed")),
        ] {
            let transport = Transport {
                reads: RefCell::new(reads),
                fail_enable,
                ..Transport::default()
            };
            let writes = Rc::clone(&transport.writes);
            let mut reader = InputReader::new(transport);
            let mut actions = Actions::default();
            let mut output = Vec::new();
            let mut runtime = Runtime::new(
                engine(),
                context(ServiceModule::M131E),
                &mut actions,
                &mut output,
                false,
            );
            let result = live_session(
                &mut reader,
                &mut runtime,
                options(true),
                &AtomicBool::new(false),
            );
            match expected_error {
                Some(error) => assert!(result.unwrap_err().contains(error)),
                None => result.unwrap(),
            }
            assert_eq!(*writes.borrow(), expected_modes());
        }
    }

    #[test]
    fn interruption_after_enable_cleans_up_once_and_drops_pending_input() {
        let stopped = Arc::new(AtomicBool::new(false));
        let transport = Transport {
            reads: RefCell::new(VecDeque::from([dial_rotation()])),
            interrupt_on_read: Some(Arc::clone(&stopped)),
            ..Transport::default()
        };
        let writes = Rc::clone(&transport.writes);
        let mut reader = InputReader::new(transport);
        let mut actions = Actions::default();
        let mut output = Vec::new();
        let mut runtime = Runtime::new(
            engine(),
            context(ServiceModule::M131E),
            &mut actions,
            &mut output,
            false,
        );
        live_session(&mut reader, &mut runtime, options(true), &stopped).unwrap();
        assert_eq!(*writes.borrow(), expected_modes());
        assert!(actions.received.is_empty());
        assert!(String::from_utf8(output).unwrap().contains("interrupted"));
    }

    #[test]
    fn idle_deadline_bounds_reads_and_restores_mode() {
        let transport = Transport::default();
        let writes = Rc::clone(&transport.writes);
        let timeouts = Rc::clone(&transport.timeouts);
        let mut reader = InputReader::new(transport);
        let mut actions = Actions::default();
        let mut output = Vec::new();
        let mut runtime = Runtime::new(
            engine(),
            context(ServiceModule::M131E),
            &mut actions,
            &mut output,
            false,
        );
        let mut configuration = options(true);
        configuration.duration = Some(Duration::from_millis(5));
        configuration.timeout_ms = 100;
        live_session(
            &mut reader,
            &mut runtime,
            configuration,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(*writes.borrow(), expected_modes());
        assert!(
            timeouts
                .borrow()
                .iter()
                .all(|timeout| (1..=5).contains(timeout))
        );
        assert!(
            String::from_utf8(output)
                .unwrap()
                .contains("duration_elapsed")
        );
    }

    #[test]
    fn action_and_cleanup_failures_are_both_reported_without_retries() {
        let transport = Transport {
            reads: RefCell::new(VecDeque::from([dial_rotation()])),
            fail_cleanup: true,
            ..Transport::default()
        };
        let writes = Rc::clone(&transport.writes);
        let mut reader = InputReader::new(transport);
        let mut actions = Actions {
            fail: true,
            ..Actions::default()
        };
        let mut output = Vec::new();
        let mut runtime = Runtime::new(
            engine(),
            context(ServiceModule::M131E),
            &mut actions,
            &mut output,
            true,
        );
        let error = live_session(
            &mut reader,
            &mut runtime,
            options(true),
            &AtomicBool::new(false),
        )
        .unwrap_err();
        assert!(error.contains("synthetic action failure") && error.contains("cleanup failed"));
        assert_eq!(actions.received.len(), 1);
        assert_eq!(*writes.borrow(), expected_modes());
    }

    #[test]
    fn output_failure_prevents_actions_and_still_restores_mode() {
        struct BrokenOutput;
        impl Write for BrokenOutput {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::Error::other("synthetic full disk"))
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let transport = Transport {
            reads: RefCell::new(VecDeque::from([dial_rotation()])),
            ..Transport::default()
        };
        let writes = Rc::clone(&transport.writes);
        let mut reader = InputReader::new(transport);
        let mut actions = Actions::default();
        let mut output = BrokenOutput;
        let mut runtime = Runtime::new(
            engine(),
            context(ServiceModule::M131E),
            &mut actions,
            &mut output,
            true,
        );
        let error = live_session(
            &mut reader,
            &mut runtime,
            options(true),
            &AtomicBool::new(false),
        )
        .unwrap_err();
        assert!(error.contains("synthetic full disk"));
        assert!(actions.received.is_empty());
        assert_eq!(*writes.borrow(), expected_modes());
    }

    #[test]
    fn failed_action_stops_other_knobs_in_same_report() {
        let profile = Profile::from_json(br#"{"schema_version":1,"name":"test","bindings":[
            {"module":"m003k","control":0,"gesture":"clockwise","action":{"type":"volume","delta":5}},
            {"module":"m003k","control":1,"gesture":"clockwise","action":{"type":"volume","delta":5}}
        ]}"#).unwrap();
        let mut actions = Actions {
            fail: true,
            ..Actions::default()
        };
        let mut output = Vec::new();
        let mut runtime = Runtime::new(
            BindingEngine::new(profile).unwrap(),
            context(ServiceModule::M003K),
            &mut actions,
            &mut output,
            true,
        );
        assert!(
            runtime
                .process(&[1, 0x2a, 5, 0, 0, 3, 1, 1, 0xff], 1, None)
                .is_err()
        );
        assert_eq!(actions.received.len(), 1);
    }

    #[test]
    fn invalid_live_configuration_has_no_transport_effects() {
        let transport = Transport::default();
        let writes = Rc::clone(&transport.writes);
        let mut reader = InputReader::new(transport);
        let mut actions = Actions::default();
        let mut output = Vec::new();
        let mut runtime = Runtime::new(
            engine(),
            context(ServiceModule::M003K),
            &mut actions,
            &mut output,
            false,
        );
        assert!(
            live_session(
                &mut reader,
                &mut runtime,
                options(true),
                &AtomicBool::new(false)
            )
            .is_err()
        );
        let mut invalid = options(false);
        invalid.timeout_ms = 0;
        assert!(live_session(&mut reader, &mut runtime, invalid, &AtomicBool::new(false)).is_err());
        assert!(writes.borrow().is_empty());
    }

    #[test]
    fn interruption_during_an_action_prevents_the_next_action_in_that_report() {
        let profile = Profile::from_json(br#"{"schema_version":1,"name":"test","bindings":[
            {"module":"m003k","control":0,"gesture":"clockwise","action":{"type":"volume","delta":5}},
            {"module":"m003k","control":1,"gesture":"clockwise","action":{"type":"volume","delta":5}}
        ]}"#).unwrap();
        let stopped = Arc::new(AtomicBool::new(false));
        let transport = Transport {
            reads: RefCell::new(VecDeque::from([ReadResult::Data(vec![
                1, 0x2a, 5, 0, 0, 3, 1, 1, 0xff,
            ])])),
            ..Transport::default()
        };
        let mut reader = InputReader::new(transport);
        let mut actions = Actions {
            stop_on_first: Some(Arc::clone(&stopped)),
            ..Actions::default()
        };
        let mut output = Vec::new();
        let mut runtime = Runtime::new(
            BindingEngine::new(profile).unwrap(),
            context(ServiceModule::M003K),
            &mut actions,
            &mut output,
            true,
        );
        live_session(&mut reader, &mut runtime, options(false), &stopped).unwrap();
        assert_eq!(actions.received.len(), 1);
        assert!(String::from_utf8(output).unwrap().contains("interrupted"));
    }
}
