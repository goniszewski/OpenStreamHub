use std::{
    error::Error,
    fs::File,
    io::{self, BufReader, Read, Write},
    path::PathBuf,
    process::ExitCode,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use clap::{Parser, Subcommand};
use cooltek_sdk_cli::capture::{
    CaptureReader, EndReason, MAX_REPORT_BYTES, decode_hex, encode_hex,
};
use cooltek_sdk_cli::{
    control::SharedRecords,
    device::{DevicePath, TIMEOUT_MS_RANGE, parse_path, select_device},
    dial_artwork::{DialArtwork, DialContent, SystemClock},
    display_artwork::PreparedKeyArtwork,
    display_sink::{SharedTransport, live_dial_display, live_display},
    interrupt::WatchedStdin,
    live_output::TimedStdout,
    live_runtime::{LiveOptions, LiveSummary, run_live, run_live_with_display},
    service_events::{
        TimerCancelReason, cancel_timers, dial_function_details, emit, end_record, inline_alert,
        plan_report, process_due_timers, process_work, start_record,
    },
};
use cooltek_sdk_masterhub::{DeviceRegistry, InputReader, ModuleKind, lighting::LedModule};
use cooltek_sdk_service::{
    ActionDispatcher, DeviceContext, InputSession, Profile, ProfileSession, ServiceModule,
    TimerAlert, TimerAlerter, TimerTable,
    macos::MacOsDispatcher,
    obs::{OBS_UNAVAILABLE, ObsClient, ObsEndpoint, ObsPassword},
};
use serde_json::json;

/// Where the control handler is installed once the device is open.
type ControlSlot = Arc<Mutex<Option<Box<dyn FnMut(&str) + Send>>>>;

#[derive(Parser)]
#[command(
    name = "masterhub-service",
    version,
    about = "Bind MasterHUB gestures to macOS actions",
    long_about = "Bind MasterHUB gestures to macOS actions. Replay always plans actions without HID or macOS access. Live runs default to dry-run; --execute enables actions, while --software-mode independently enables software mode for the IPS dial or keys. Each process owns one explicitly selected interface. Ctrl-C or SIGTERM (or, with --exit-on-stdin-eof, end of stdin) ends the session and attempts requested mode cleanup; a second Ctrl-C exits immediately."
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
        /// Experimental, hardware-unverified: draw M4315 key images for a schema-2
        /// keypad profile and redraw after folder navigation, or, with
        /// --dial-content, draw the M131E dial screen. Requires --software-mode.
        /// Off by default; display failures are logged and never end the session.
        #[arg(long)]
        display_sync: bool,
        /// Experimental, hardware-unverified: with --display-sync on an M4315, draw a
        /// key dimmed while it is held down (one extra key write on press and one on
        /// release). Dropped for the run if a press write takes over 150 ms.
        #[arg(long, requires = "display_sync")]
        press_feedback: bool,
        /// Experimental, hardware-unverified: what the M131E dial screen shows
        /// with --display-sync: the active dial function's name (`function`), a
        /// digital clock (`clock`), an analog clock (`analog-clock`), the
        /// machine's CPU usage (`cpu`; macOS only, `--` elsewhere), or the
        /// picture in the profile's `dial_display` setting (`image`). Redrawn
        /// only when it changes, at most once a second. `off` (the default)
        /// draws nothing; any other value requires --display-sync and an M131E.
        #[arg(long, value_enum, default_value_t = DialContent::Off)]
        dial_content: DialContent,
        /// Report `profile` (next/previous) actions as `profile_switch_requested`
        /// records for a supervising session manager, such as the OpenStreamHub
        /// desktop app, which owns the saved profiles. Without it a `profile`
        /// action is logged as `action_unavailable` and does nothing.
        #[arg(long)]
        profile_switch_requests: bool,
        /// Loopback OBS WebSocket (obs-websocket v5, OBS 28+) endpoint for
        /// OBS actions (every `obs_*` action type):
        /// exactly 127.0.0.1:PORT, [::1]:PORT or localhost:PORT. Without it
        /// such an action is logged as
        /// `action_unavailable` and does nothing. Each action opens one
        /// connection, bounded to 3 s, and closes it.
        #[arg(long, value_parser = ObsEndpoint::parse)]
        obs_endpoint: Option<ObsEndpoint>,
        /// Read the OBS WebSocket password from the first line of stdin. The
        /// password is never accepted as an argument or environment variable,
        /// is kept only in memory, and is never logged. Requires --obs-endpoint.
        #[arg(long, requires = "obs_endpoint")]
        obs_password_stdin: bool,
        /// End the session as Ctrl-C or SIGTERM would (hardware mode restored)
        /// when stdin reaches end of file, so a supervising process that dies,
        /// closing the stdin pipe it holds open, never leaves the device in
        /// software mode. The footer reason is then `stdin_closed`. stdin must
        /// be a pipe or socket that is still open when the run starts, checked
        /// before the device is opened; the OBS password line, if requested,
        /// is read first, and any later stdin data is ignored.
        #[arg(long)]
        exit_on_stdin_eof: bool,
        /// Accept control requests as JSON lines on stdin after the OBS
        /// password line, answering each on stdout with a `control_result`
        /// record: per-LED lighting for the knobs and faders written through
        /// the handle this run already holds, and a read of the module's own
        /// LED report. See `cooltek_sdk_cli::control`. Requires
        /// --exit-on-stdin-eof, whose end-of-file stop is unchanged.
        #[arg(long, requires = "exit_on_stdin_eof")]
        control: bool,
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
            display_sync,
            press_feedback,
            dial_content,
            profile_switch_requests,
            obs_endpoint,
            obs_password_stdin,
            exit_on_stdin_eof,
            control,
        } => {
            let profile = load_profile(profile)?;
            // Before the password read: a terminal, file or /dev/null stdin is
            // refused without prompting for (and echoing) a password.
            if exit_on_stdin_eof {
                cooltek_sdk_cli::interrupt::check_stdin_pipe(io::stdin())?;
            }
            // Read before any HID access, so a missing password never leaves
            // the device in software mode.
            let obs = obs_client(obs_endpoint, obs_password_stdin, &mut io::stdin().lock())?;
            // After the password line, before any HID access: a pipe whose
            // writer is gone would stop the run as soon as it started.
            if exit_on_stdin_eof {
                cooltek_sdk_cli::interrupt::check_stdin_open(io::stdin())?;
            }
            let profile_name = profile.name.clone();
            check_display_sync(display_sync, software_mode, dial_content, &profile)?;
            if execute && !cfg!(target_os = "macos") {
                return Err("--execute is supported only on macOS".into());
            }
            // Prepared before any HID access, so nothing is encoded per input
            // event (clock faces are drawn once per minute, and CPU usage is
            // sampled at most once a second, on the display thread).
            let artwork = match (display_sync, dial_content) {
                (false, _) => None,
                (true, DialContent::Off) => Some(Artwork::Keys(
                    PreparedKeyArtwork::prepare_profile(&profile)?,
                )),
                (true, content) => Some(Artwork::Dial(DialArtwork::prepare(
                    content,
                    &profile,
                    Box::new(SystemClock),
                )?)),
            };
            // Reject unsupported stdout (such as a regular file) before any HID access.
            let stdout = SharedRecords::new(TimedStdout::new(Duration::from_millis(500))?);
            let stopped = cooltek_sdk_cli::interrupt::install()?;
            // `io::stdin()` shares the buffer the password was read through,
            // so bytes after the password line are neither lost nor reordered.
            let control_slot: ControlSlot = Arc::new(Mutex::new(None));
            // Where a new profile engine waits for the input loop; one per process, so it lives as long as it.
            let profile_slot: Option<&'static Mutex<Option<ProfileSession>>> =
                control.then(|| &*Box::leak(Box::new(Mutex::new(None))));
            let mut options = live_options(
                software_mode,
                duration_secs,
                max_reports,
                timeout_ms,
                stop_on_action_error,
                profile_switch_requests,
                exit_on_stdin_eof,
                io::stdin(),
                &stopped,
                &cooltek_sdk_cli::interrupt::STDIN_CLOSED,
                control.then(|| Arc::clone(&control_slot)),
            )?;
            options.profile_updates = profile_slot;
            let registry = DeviceRegistry::new()?;
            let devices = registry.devices();
            let selected = select_device(&devices, &path_hex, software_mode)?;
            let device = device_context(
                selected.vendor_id,
                selected.product_id,
                selected.serial_number.clone(),
            )?;
            let engine = live_bindings(profile, device.module, display_sync, dial_content)?;
            if let Some(functions) = dial_function_details(&engine, &device) {
                // stdout carries JSONL only; the same facts are in `service_start`.
                eprintln!(
                    "masterhub-service: dial functions active ({} configured, starting at the first); a short click switches functions and never runs an action; the current function resets to the first whenever this process restarts",
                    functions["count"]
                );
            }
            let opened = registry.open(selected)?;
            let mut details = json!({
                "source":"hardware", "profile":profile_name, "path_hex":encode_hex(&path_hex),
                "vendor_id":selected.vendor_id, "product_id":selected.product_id,
                "interface_number":selected.interface_number, "usage_page":selected.usage_page,
                "usage":selected.usage, "software_mode_requested":software_mode,
                "queue_capacity":64, "log_timeout_ms":500,
                "drain_timeout_ms":10000, "cancellation_timeout_ms":2000,
                "stop_on_action_error":stop_on_action_error,
                "display_sync_requested":display_sync,
                "profile_switch_requests":profile_switch_requests,
            });
            // Only when set, so default runs log exactly what they did before.
            if dial_content != DialContent::Off {
                details["dial_content"] = json!(dial_content.as_str());
            }
            if let Some(obs) = &obs {
                details["obs"] = obs_details(obs);
            }
            let control_out = stdout.clone();
            // The run's mode lives in one flag the dispatcher reads for every action, so a supervisor can switch it
            // in place; without `--control` the dispatcher is fixed at start, as before.
            let mode = control.then(|| Arc::new(AtomicBool::new(execute)));
            let switchable = mode.clone();
            let factory =
                move |abort| Ok((Dispatcher::new(execute, switchable, abort, obs), stdout));
            // The handle is shared only when something else writes through it: the display, or control requests.
            // A plain run keeps reading it directly, as before.
            let (direct, shared) = if control || artwork.is_some() {
                (None, Some(SharedTransport::new(opened)))
            } else {
                (Some(opened), None)
            };
            if control && let Some(shared) = &shared {
                let led = match device.module {
                    ServiceModule::M003K => Some(LedModule::Knob),
                    ServiceModule::M005S => Some(LedModule::Slider),
                    _ => None,
                };
                let module = device.module;
                let hooks = cooltek_sdk_cli::control::Hooks {
                    mode: mode.map(|execute| cooltek_sdk_cli::control::RunMode {
                        execute,
                        can_execute: cfg!(target_os = "macos"),
                    }),
                    profile: profile_slot.map(|slot| {
                        Box::new(move |value: &serde_json::Value| {
                            // A run that draws on the module holds the engine and the artwork on its display thread, so
                            // it takes a new profile by restarting.
                            if display_sync {
                                return Err(
                                    "this run draws on the module, so it takes a new profile by restarting"
                                        .to_string(),
                                );
                            }
                            let profile = Profile::from_json(value.to_string().as_bytes())
                                .map_err(|error| error.to_string())?;
                            let engine = live_bindings(profile, module, false, DialContent::Off)?;
                            *slot.lock().map_err(|_| "the profile slot failed".to_string())? =
                                Some(engine);
                            Ok(())
                        }) as cooltek_sdk_cli::control::ProfileSwap
                    }),
                };
                let handler =
                    cooltek_sdk_cli::control::serve(shared.clone(), led, hooks, control_out);
                *control_slot
                    .lock()
                    .map_err(|_| "control handler lock failed")? = Some(Box::new(handler));
            }
            match artwork {
                None => {
                    match (direct, shared) {
                        (Some(opened), _) => run_live(
                            InputReader::new(opened),
                            engine,
                            device,
                            execute,
                            details,
                            options,
                            stopped,
                            factory,
                        )?,
                        (None, Some(shared)) => run_live(
                            InputReader::new(shared),
                            engine,
                            device,
                            execute,
                            details,
                            options,
                            stopped,
                            factory,
                        )?,
                        (None, None) => unreachable!("a run always has a handle"),
                    };
                }
                // Hardware only: one opened handle shared by the input reader and
                // the display thread. Never constructed in tests.
                Some(artwork) => {
                    let shared = shared.expect("display runs share the handle");
                    // Fail closed before software mode: folder navigation must
                    // not run live without its display.
                    let display = Some(match artwork {
                        Artwork::Keys(artwork) => live_display(&shared, selected.module, artwork)?
                            .with_press_feedback(press_feedback),
                        Artwork::Dial(artwork) => {
                            live_dial_display(&shared, selected.module, artwork)?
                        }
                    });
                    run_live_with_display(
                        InputReader::new(shared),
                        engine,
                        device,
                        execute,
                        details,
                        options,
                        stopped,
                        display,
                        factory,
                    )?;
                }
            }
        }
    }
    Ok(())
}

/// The OBS client for `--obs-endpoint`, with the password read from `stdin`
/// when `--obs-password-stdin` is given. No I/O happens beyond that read.
fn obs_client(
    endpoint: Option<ObsEndpoint>,
    password_stdin: bool,
    stdin: &mut impl io::BufRead,
) -> Result<Option<ObsClient>, String> {
    let Some(endpoint) = endpoint else {
        return Ok(None);
    };
    let password = password_stdin
        .then(|| ObsPassword::read_line(stdin))
        .transpose()?;
    Ok(Some(ObsClient::new(endpoint, password)))
}

/// The live options for `run`. With `--exit-on-stdin-eof` it also starts the
/// stdin watchdog on what is left of `stdin` after the OBS password line, and
/// `stdin_closed` names the flag the watchdog sets before requesting a stop on
/// `interrupted`. Without it nothing is started and stdin is not read again.
#[allow(clippy::too_many_arguments)]
fn live_options(
    software_mode: bool,
    duration_secs: Option<u64>,
    max_reports: Option<u64>,
    timeout_ms: i32,
    stop_on_action_error: bool,
    profile_switch_requests: bool,
    exit_on_stdin_eof: bool,
    stdin: impl WatchedStdin,
    interrupted: &Arc<AtomicBool>,
    closed: &'static AtomicBool,
    control: Option<ControlSlot>,
) -> Result<LiveOptions, String> {
    let stdin_closed = if exit_on_stdin_eof {
        // Control lines reach the handler once the device is open; one that arrives sooner is dropped (the
        // supervisor sends none before `service_start`).
        if let Some(control) = control {
            cooltek_sdk_cli::interrupt::watch_stdin_lines(
                stdin,
                Arc::clone(interrupted),
                closed,
                move |line| {
                    if let Ok(mut handler) = control.lock()
                        && let Some(handler) = handler.as_mut()
                    {
                        handler(line);
                    }
                },
            )?;
        } else {
            cooltek_sdk_cli::interrupt::watch_stdin(stdin, Arc::clone(interrupted), closed)?;
        }
        Some(closed)
    } else {
        None
    };
    Ok(LiveOptions {
        software_mode,
        duration: duration_secs.map(Duration::from_secs),
        max_reports,
        timeout_ms,
        stop_on_action_error,
        profile_switch_requests,
        stdin_closed,
        ..LiveOptions::default()
    })
}

/// `service_start` details for OBS: the endpoint and whether a password is
/// set, never the password.
fn obs_details(obs: &ObsClient) -> serde_json::Value {
    json!({"endpoint":obs.endpoint().to_string(), "password":obs.has_password()})
}

/// Prepared display artwork, chosen before any HID access.
enum Artwork {
    Keys(PreparedKeyArtwork),
    Dial(DialArtwork),
}

/// Rejects invalid `--display-sync` and `--dial-content` combinations before
/// any HID access. The module is checked later by [`live_bindings`], still
/// before the device is opened.
fn check_display_sync(
    display_sync: bool,
    software_mode: bool,
    dial_content: DialContent,
    profile: &Profile,
) -> Result<(), String> {
    if dial_content != DialContent::Off && !display_sync {
        return Err("--dial-content requires --display-sync".into());
    }
    if !display_sync {
        return Ok(());
    }
    if !software_mode {
        return Err("--display-sync requires --software-mode".into());
    }
    if dial_content == DialContent::Off && (profile.schema_version != 2 || profile.keypad.is_none())
    {
        return Err(
            "--display-sync requires a schema-2 keypad profile, or --dial-content for an M131E dial"
                .into(),
        );
    }
    Ok(())
}

// Folder pages need display updates before their mappings can be used live, so
// a schema-2 M4315 run is refused unless --display-sync opts in to the
// display path (OpenStreamHub passes it with Module control set to App).
// Other modules can retain their existing flat bindings from the same profile.
// Dial functions are accepted live; the dial screen shows the current function
// only with the experimental --display-sync --dial-content function. A short
// click only switches state and never dispatches, and `service_start` records
// that the function resets on restart.
fn live_bindings(
    mut profile: Profile,
    module: ServiceModule,
    display_sync: bool,
    dial_content: DialContent,
) -> Result<ProfileSession, String> {
    profile.validate()?;
    if dial_content != DialContent::Off {
        if !display_sync {
            return Err("--dial-content requires --display-sync".into());
        }
        if module != ServiceModule::M131E {
            return Err("--dial-content requires an M131E dial interface".into());
        }
    } else if display_sync && module != ServiceModule::M4315 {
        return Err("--display-sync requires an M4315 keypad interface".into());
    }
    if profile.schema_version == 2 {
        if module == ServiceModule::M4315 {
            if !display_sync {
                return Err(cooltek_sdk_cli::service_events::NESTED_KEYPAD_REFUSAL.into());
            }
            return ProfileSession::new(profile);
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
    /// A run whose supervisor may switch it between Live and Test while it runs (`--control`): each action checks the
    /// shared flag. Only built on macOS, where actions can execute at all.
    Switchable {
        execute: Arc<AtomicBool>,
        /// Whether the run has an OBS endpoint, so a Test action is reported as an executing one would be.
        obs: bool,
        macos: MacOsDispatcher,
    },
    /// Dry run and replay: validates only. `obs` records whether the run has
    /// an OBS endpoint, so a dry run reports OBS actions exactly as an
    /// executing run would (planned, or unavailable); it never connects.
    Preview {
        obs: bool,
    },
    MacOs(MacOsDispatcher),
}

impl Dispatcher {
    fn new(
        execute: bool,
        switchable: Option<Arc<AtomicBool>>,
        abort: Arc<AtomicBool>,
        obs: Option<ObsClient>,
    ) -> Self {
        if let Some(flag) = switchable.filter(|_| cfg!(target_os = "macos")) {
            let has_obs = obs.is_some();
            let obs = obs.map(|obs| obs.with_cancellation(Arc::clone(&abort)));
            return Self::Switchable {
                execute: flag,
                obs: has_obs,
                macos: MacOsDispatcher::with_cancellation(abort).with_obs(obs),
            };
        }
        if execute {
            let obs = obs.map(|obs| obs.with_cancellation(Arc::clone(&abort)));
            Self::MacOs(MacOsDispatcher::with_cancellation(abort).with_obs(obs))
        } else {
            Self::Preview { obs: obs.is_some() }
        }
    }
}

impl ActionDispatcher for Dispatcher {
    fn dispatch_with_execution(
        &mut self,
        action: &cooltek_sdk_service::Action,
        execute: bool,
    ) -> Result<bool, String> {
        if let Self::Switchable { execute, macos, .. } = self {
            let executing = execute.load(Ordering::SeqCst);
            if executing {
                macos.dispatch(action)?;
            } else {
                action.validate_executable()?;
            }
            Ok(executing)
        } else {
            self.dispatch(action).map(|()| execute)
        }
    }

    fn dispatch(&mut self, action: &cooltek_sdk_service::Action) -> Result<(), String> {
        match self {
            Self::Switchable { execute, macos, .. } => {
                if execute.load(Ordering::SeqCst) {
                    macos.dispatch(action)
                } else {
                    action.validate_executable()
                }
            }
            Self::Preview { .. } => action.validate_executable(),
            Self::MacOs(dispatcher) => dispatcher.dispatch(action),
        }
    }

    fn unavailable(&self, action: &cooltek_sdk_service::Action) -> Option<&'static str> {
        match self {
            Self::Switchable {
                execute,
                obs,
                macos,
            } => {
                if execute.load(Ordering::SeqCst) {
                    macos.unavailable(action)
                } else {
                    (!obs && action.uses_obs()).then_some(OBS_UNAVAILABLE)
                }
            }
            Self::Preview { obs } => (!obs && action.uses_obs()).then_some(OBS_UNAVAILABLE),
            Self::MacOs(dispatcher) => dispatcher.unavailable(action),
        }
    }

    fn timer_alert(&mut self, alert: &TimerAlert) -> Result<Option<String>, String> {
        match self {
            Self::Switchable { execute, macos, .. } => {
                if execute.load(Ordering::SeqCst) {
                    macos.timer_alert(alert)
                } else {
                    alert.validate().map(|()| None)
                }
            }
            // Dry runs and replay log the expiry and show or play nothing.
            Self::Preview { .. } => alert.validate().map(|()| None),
            Self::MacOs(dispatcher) => dispatcher.timer_alert(alert),
        }
    }

    fn timer_alerter(&mut self, cancel: Arc<AtomicBool>) -> Option<Box<dyn TimerAlerter>> {
        match self {
            Self::Switchable { execute, macos, .. } => macos.timer_alerter(cancel).map(|inner| {
                Box::new(ModeAlerter {
                    execute: Arc::clone(execute),
                    inner,
                }) as Box<dyn TimerAlerter>
            }),
            // Validation only is instant, so preview alerts stay inline.
            Self::Preview { .. } => None,
            Self::MacOs(dispatcher) => dispatcher.timer_alerter(cancel),
        }
    }
}

/// Always stays on the alert thread, including when created in Test mode.
/// Each expiry uses the current mode rather than the first expiry's mode.
struct ModeAlerter {
    execute: Arc<AtomicBool>,
    inner: Box<dyn TimerAlerter>,
}

impl TimerAlerter for ModeAlerter {
    fn alert(&mut self, alert: &TimerAlert) -> Result<Option<String>, String> {
        if self.execute.load(Ordering::SeqCst) {
            self.inner.alert(alert)
        } else {
            alert.validate().map(|()| None)
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
    // Replay never has an OBS endpoint: OBS actions are reported as unavailable.
    let mut dispatcher = Dispatcher::Preview { obs: false };
    let never = AtomicBool::new(false);
    let mut summary = LiveSummary::default();
    let mut matched_actions = 0u64;
    let mut details = json!({"source":session.source, "identity":session.identity});
    if let Some(functions) = dial_function_details(&engine, &device) {
        details["dial_functions"] = functions;
    }
    emit(output, start_record(&device, false, details))?;
    // Replay runs countdowns on the capture's own timestamps: a timer expires
    // before the first report at or after its deadline; the rest are cancelled
    // when the capture ends. Expiry is logged only (preview dispatcher).
    let mut timers = TimerTable::default();
    let result = (|| {
        while let Some(report) = reader.next_report()? {
            summary.report_count += 1;
            process_due_timers(
                &mut timers,
                report.timestamp_micros,
                output,
                &never,
                &mut summary,
                inline_alert(&mut dispatcher),
            )?;
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
                false,
                &mut timers,
                report.timestamp_micros,
                &mut summary,
            )?;
        }
        cancel_timers(
            &mut timers,
            TimerCancelReason::SessionEnd,
            output,
            &mut summary,
        )?;
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
    summary.incomplete_action_count = matched_actions.saturating_sub(
        summary.action_count + summary.profile_request_count + summary.unavailable_action_count,
    );
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
        let error = live_bindings(
            nested_profile(),
            ServiceModule::M4315,
            false,
            DialContent::Off,
        )
        .unwrap_err();
        assert_eq!(
            error,
            "nested keypad profiles run live only with --display-sync (which requires --software-mode); in OpenStreamHub, set Module control to App in Settings"
        );
    }

    #[test]
    fn display_sync_opt_in_keeps_the_keypad_tree_for_an_m4315_only() {
        let mut engine = live_bindings(
            nested_profile(),
            ServiceModule::M4315,
            true,
            DialContent::Off,
        )
        .unwrap();
        assert!(engine.profile().keypad.is_some());
        let device = DeviceContext {
            module: ServiceModule::M4315,
            serial_number: None,
        };
        let press = InputEvent {
            timestamp_micros: 0,
            control: 1,
            gesture: Gesture::Pressed,
            value: None,
        };
        assert!(matches!(
            engine.resolve(&device, &press).unwrap(),
            KeypadResolution::Navigation { .. }
        ));
        assert_eq!(engine.path(), ["root", "development"]);
        for module in [ServiceModule::M131E, ServiceModule::M003K] {
            let error =
                live_bindings(nested_profile(), module, true, DialContent::Off).unwrap_err();
            assert!(error.contains("requires an M4315"), "{error}");
        }
    }

    #[test]
    fn dial_content_needs_display_sync_and_an_m131e_before_the_device_is_opened() {
        let dial = Profile::from_json(include_bytes!(
            "../../../../examples/profiles/dial-functions.json"
        ))
        .unwrap();
        for content in [
            DialContent::Function,
            DialContent::Clock,
            DialContent::AnalogClock,
            DialContent::Cpu,
        ] {
            // A flat profile is enough for the dial; no keypad tree is needed.
            assert!(check_display_sync(true, true, content, &dial).is_ok());
            assert_eq!(
                check_display_sync(false, true, content, &dial).unwrap_err(),
                "--dial-content requires --display-sync"
            );
            assert_eq!(
                check_display_sync(true, false, content, &dial).unwrap_err(),
                "--display-sync requires --software-mode"
            );
            let engine = live_bindings(dial.clone(), ServiceModule::M131E, true, content).unwrap();
            assert_eq!(engine.dial_function_index(), Some(0));
            for module in [
                ServiceModule::M4315,
                ServiceModule::M003K,
                ServiceModule::M005S,
            ] {
                let error = live_bindings(dial.clone(), module, true, content).unwrap_err();
                assert_eq!(error, "--dial-content requires an M131E dial interface");
            }
            let error =
                live_bindings(dial.clone(), ServiceModule::M131E, false, content).unwrap_err();
            assert_eq!(error, "--dial-content requires --display-sync");
            // Keypad folders on an M131E are flattened as before.
            let engine =
                live_bindings(nested_profile(), ServiceModule::M131E, true, content).unwrap();
            assert!(engine.profile().keypad.is_none());
        }
        // Off is the default and changes nothing.
        assert!(check_display_sync(false, false, DialContent::Off, &dial).is_ok());
        assert!(live_bindings(dial, ServiceModule::M131E, false, DialContent::Off).is_ok());
    }

    #[test]
    fn dial_content_values_are_parsed_strictly() {
        let parse = |extra: &[&str]| {
            let mut args = vec![
                "masterhub-service",
                "run",
                "--profile",
                "p.json",
                "--path-hex",
                "41",
            ];
            args.extend_from_slice(extra);
            Cli::try_parse_from(args).map(|cli| match cli.command {
                Command::Run { dial_content, .. } => dial_content,
                _ => unreachable!(),
            })
        };
        assert_eq!(parse(&[]).unwrap(), DialContent::Off);
        for (value, content) in [
            ("off", DialContent::Off),
            ("function", DialContent::Function),
            ("clock", DialContent::Clock),
            ("analog-clock", DialContent::AnalogClock),
            ("cpu", DialContent::Cpu),
            ("image", DialContent::Image),
        ] {
            assert_eq!(parse(&["--dial-content", value]).unwrap(), content);
        }
        for bad in [
            "Clock",
            "analog",
            "",
            "clock,function",
            "stats",
            "CPU",
            "gpu",
            "clock+cpu",
            "cpu-usage",
        ] {
            assert!(parse(&["--dial-content", bad]).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn display_sync_combinations_are_checked_before_any_hid_access() {
        let flat =
            Profile::from_json(include_bytes!("../../../../examples/profiles/macos.json")).unwrap();
        let off = DialContent::Off;
        assert!(check_display_sync(false, false, off, &flat).is_ok());
        assert!(check_display_sync(true, true, off, &nested_profile()).is_ok());
        assert_eq!(
            check_display_sync(true, false, off, &nested_profile()).unwrap_err(),
            "--display-sync requires --software-mode"
        );
        assert!(
            check_display_sync(true, true, off, &flat)
                .unwrap_err()
                .contains("schema-2 keypad profile")
        );
    }

    #[test]
    fn other_modules_keep_their_bindings_in_a_folder_profile() {
        let mut engine = live_bindings(
            nested_profile(),
            ServiceModule::M003K,
            false,
            DialContent::Off,
        )
        .unwrap();
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
    fn live_dial_functions_are_accepted_and_a_click_only_switches() {
        let profile = Profile::from_json(include_bytes!(
            "../../../../examples/profiles/dial-functions.json"
        ))
        .unwrap();
        let mut engine =
            live_bindings(profile, ServiceModule::M131E, false, DialContent::Off).unwrap();
        let device = DeviceContext {
            module: ServiceModule::M131E,
            serial_number: None,
        };
        let details = dial_function_details(&engine, &device).unwrap();
        assert_eq!(details["resets_on_restart"], true);
        assert_eq!(details["initial_index"], 0);
        let event = |gesture| InputEvent {
            timestamp_micros: 0,
            control: 0,
            gesture,
            value: None,
        };
        for gesture in [Gesture::Pressed, Gesture::Released, Gesture::ShortPress] {
            assert!(!matches!(
                engine.route(&device, &event(gesture)).unwrap(),
                cooltek_sdk_service::ProfileResolution::Action { .. }
            ));
        }
        assert_eq!(engine.dial_function_index(), Some(1));
    }

    fn run_args(extra: &[&str]) -> Result<(Option<ObsEndpoint>, bool), clap::Error> {
        let mut args = vec![
            "masterhub-service",
            "run",
            "--profile",
            "p.json",
            "--path-hex",
            "41",
        ];
        args.extend_from_slice(extra);
        Cli::try_parse_from(args).map(|cli| match cli.command {
            Command::Run {
                obs_endpoint,
                obs_password_stdin,
                ..
            } => (obs_endpoint, obs_password_stdin),
            _ => unreachable!(),
        })
    }

    #[test]
    fn control_needs_the_stdin_watchdog_that_keeps_its_end_of_file_stop() {
        assert!(run_args(&["--control"]).is_err());
        assert!(run_args(&["--control", "--exit-on-stdin-eof"]).is_ok());
    }

    #[test]
    fn obs_flags_accept_only_loopback_endpoints_and_no_password_value() {
        assert_eq!(run_args(&[]).unwrap(), (None, false));
        for (value, shown) in [
            ("127.0.0.1:4455", "127.0.0.1:4455"),
            ("[::1]:4455", "[::1]:4455"),
            ("localhost:4456", "localhost:4456"),
        ] {
            let (endpoint, stdin) = run_args(&["--obs-endpoint", value]).unwrap();
            assert_eq!(endpoint.unwrap().to_string(), shown);
            assert!(!stdin);
        }
        for bad in [
            "192.168.1.5:4455",
            "0.0.0.0:4455",
            "obs.example.com:4455",
            "ws://127.0.0.1:4455",
            "127.0.0.1:0",
            "127.0.0.1",
            "",
        ] {
            assert!(run_args(&["--obs-endpoint", bad]).is_err(), "{bad:?}");
        }
        // The password channel is stdin only: it needs an endpoint and takes no value.
        assert!(run_args(&["--obs-password-stdin"]).is_err());
        assert!(
            run_args(&["--obs-endpoint", "127.0.0.1:4455", "--obs-password-stdin"])
                .unwrap()
                .1
        );
        for smuggled in [
            &["--obs-endpoint", "127.0.0.1:4455", "--obs-password", "x"][..],
            &[
                "--obs-endpoint",
                "127.0.0.1:4455",
                "--obs-password-stdin=hunter2",
            ][..],
        ] {
            assert!(run_args(smuggled).is_err(), "{smuggled:?}");
        }
    }

    #[test]
    fn the_obs_password_is_read_from_stdin_only_and_never_logged() {
        let endpoint = ObsEndpoint::parse("127.0.0.1:4455").unwrap();
        // Without the flag stdin is never read.
        let mut untouched = io::Cursor::new(b"hunter2-secret\n".to_vec());
        let obs = obs_client(Some(endpoint), false, &mut untouched)
            .unwrap()
            .unwrap();
        assert!(!obs.has_password());
        assert_eq!(untouched.position(), 0);
        assert!(
            obs_client(None, false, &mut io::Cursor::new(Vec::new()))
                .unwrap()
                .is_none()
        );
        let mut stdin = io::Cursor::new(b"hunter2-secret\n".to_vec());
        let obs = obs_client(Some(endpoint), true, &mut stdin)
            .unwrap()
            .unwrap();
        assert!(obs.has_password());
        let details = obs_details(&obs);
        assert_eq!(
            details,
            json!({"endpoint":"127.0.0.1:4455","password":true})
        );
        for rendered in [details.to_string(), format!("{obs:?}")] {
            assert!(!rendered.contains("hunter2"), "{rendered}");
        }
        let error =
            obs_client(Some(endpoint), true, &mut io::Cursor::new(b"\n".to_vec())).unwrap_err();
        assert!(error.contains("OBS password"), "{error}");
        let error = obs_client(
            Some(endpoint),
            true,
            &mut io::Cursor::new(b"hunter2\x07secret\n".to_vec()),
        )
        .unwrap_err();
        assert!(!error.contains("hunter2"), "{error}");
    }

    fn wait_for(flag: &AtomicBool) -> bool {
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        while std::time::Instant::now() < deadline {
            if flag.load(std::sync::atomic::Ordering::SeqCst) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        false
    }

    #[cfg(unix)]
    #[test]
    fn exit_on_stdin_eof_is_off_by_default_and_then_never_reads_stdin() {
        let parse = |extra: &[&str]| {
            let mut args = vec![
                "masterhub-service",
                "run",
                "--profile",
                "p.json",
                "--path-hex",
                "41",
            ];
            args.extend_from_slice(extra);
            Cli::try_parse_from(args).map(|cli| match cli.command {
                Command::Run {
                    exit_on_stdin_eof, ..
                } => exit_on_stdin_eof,
                _ => unreachable!(),
            })
        };
        assert!(!parse(&[]).unwrap());
        assert!(parse(&["--exit-on-stdin-eof"]).unwrap());
        assert!(parse(&["--exit-on-stdin-eof=true"]).is_err());
        // Off: no watchdog, so stdin (here at EOF at once) is never read and
        // nothing is stopped; the other run flags still reach the options.
        let read = Arc::new(AtomicBool::new(false));
        let interrupted = Arc::new(AtomicBool::new(false));
        let closed: &'static AtomicBool = Box::leak(Box::new(AtomicBool::new(false)));
        let options = options_for(false, Untouched::new(&read), &interrupted, closed);
        assert!(options.stdin_closed.is_none());
        assert!(options.software_mode && options.stop_on_action_error);
        assert!(options.profile_switch_requests);
        assert_eq!(options.duration, Some(Duration::from_secs(30)));
        assert_eq!((options.max_reports, options.timeout_ms), (Some(7), 250));
        std::thread::sleep(Duration::from_millis(50));
        assert!(!read.load(std::sync::atomic::Ordering::SeqCst));
        assert!(!interrupted.load(std::sync::atomic::Ordering::SeqCst));
        // On: the options carry the watchdog's flag (so the footer can say
        // `stdin_closed`), the same stdin is read, and its EOF requests the stop.
        let options = options_for(true, Untouched::new(&read), &interrupted, closed);
        assert!(std::ptr::eq(options.stdin_closed.unwrap(), closed));
        assert!(wait_for(&interrupted));
        assert!(read.load(std::sync::atomic::Ordering::SeqCst));
        assert!(closed.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[cfg(unix)]
    /// `run`'s options for fixed sample flags, with or without the watchdog.
    fn options_for(
        exit_on_stdin_eof: bool,
        stdin: impl WatchedStdin,
        interrupted: &Arc<AtomicBool>,
        closed: &'static AtomicBool,
    ) -> LiveOptions {
        live_options(
            true,
            Some(30),
            Some(7),
            250,
            true,
            true,
            exit_on_stdin_eof,
            stdin,
            interrupted,
            closed,
            None,
        )
        .unwrap()
    }

    #[cfg(unix)]
    /// Records any read and reads as EOF; its descriptor is an unused pipe.
    struct Untouched(Arc<AtomicBool>, io::PipeReader);
    #[cfg(unix)]
    impl Untouched {
        fn new(read: &Arc<AtomicBool>) -> Self {
            let (reader, _) = io::pipe().unwrap();
            Self(Arc::clone(read), reader)
        }
    }
    #[cfg(unix)]
    impl Read for Untouched {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            self.0.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(0)
        }
    }
    #[cfg(unix)]
    impl std::os::fd::AsFd for Untouched {
        fn as_fd(&self) -> std::os::fd::BorrowedFd<'_> {
            self.1.as_fd()
        }
    }

    #[cfg(unix)]
    /// A buffered pipe, like the process-wide stdin buffer `run` reads
    /// the password through before the watchdog takes over.
    struct Buffered(BufReader<io::PipeReader>);
    #[cfg(unix)]
    impl Read for Buffered {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            self.0.read(buffer)
        }
    }
    #[cfg(unix)]
    impl std::os::fd::AsFd for Buffered {
        fn as_fd(&self) -> std::os::fd::BorrowedFd<'_> {
            self.0.get_ref().as_fd()
        }
    }

    #[cfg(unix)]
    #[test]
    fn the_password_line_is_read_intact_before_the_watchdog_takes_the_rest() {
        let (reader, mut writer) = io::pipe().unwrap();
        // The password and later data arrive together, as one buffered read.
        writer
            .write_all(b"hunter2-secret\nlater bytes, ignored\n")
            .unwrap();
        let mut stdin = BufReader::new(reader);
        let interrupted = Arc::new(AtomicBool::new(false));
        let closed: &'static AtomicBool = Box::leak(Box::new(AtomicBool::new(false)));
        // The order of `run`: the password line first (the read `obs_client`
        // makes), then the watchdog on the same buffered stdin.
        assert_eq!(
            ObsPassword::read_line(&mut stdin).unwrap(),
            ObsPassword::new("hunter2-secret".into()).unwrap()
        );
        assert!(!interrupted.load(std::sync::atomic::Ordering::SeqCst));
        options_for(true, Buffered(stdin), &interrupted, closed);
        // Data after the password, more of it, and time: no stop.
        writer.write_all(b"more\n").unwrap();
        std::thread::sleep(Duration::from_millis(100));
        assert!(!interrupted.load(std::sync::atomic::Ordering::SeqCst));
        // The parent goes away: the pipe closes and the stop is requested.
        drop(writer);
        assert!(wait_for(&interrupted));
        assert!(closed.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_switchable_run_in_test_mode_plans_without_executing_and_reports_like_a_dry_run() {
        use cooltek_sdk_service::Action;
        let scene = Action::ObsScene {
            scene: "Live".into(),
        };
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint =
            ObsEndpoint::new("127.0.0.1", listener.local_addr().unwrap().port()).unwrap();
        let mode = Arc::new(AtomicBool::new(false));
        let mut run = Dispatcher::new(
            false,
            Some(Arc::clone(&mode)),
            Arc::new(AtomicBool::new(false)),
            Some(ObsClient::new(endpoint, None)),
        );
        assert_eq!(run.unavailable(&scene), None);
        run.dispatch(&scene).unwrap();
        assert!(
            listener.accept().is_err(),
            "Test mode never connects to OBS, whatever the dispatcher can do"
        );
        // Without OBS a Test action is reported as unavailable, as in a dry run.
        let mut bare = Dispatcher::new(false, Some(mode), Arc::new(AtomicBool::new(false)), None);
        assert_eq!(bare.unavailable(&scene), Some(OBS_UNAVAILABLE));
        assert!(
            bare.timer_alerter(Arc::new(AtomicBool::new(false)))
                .is_some()
        );
    }

    #[test]
    fn dry_runs_report_obs_scene_like_executing_runs_without_connecting() {
        use cooltek_sdk_service::Action;
        let scene = Action::ObsScene {
            scene: "Live".into(),
        };
        let abort = Arc::new(AtomicBool::new(false));
        let mut without = Dispatcher::new(false, None, Arc::clone(&abort), None);
        assert_eq!(without.unavailable(&scene), Some(OBS_UNAVAILABLE));
        assert_eq!(without.unavailable(&Action::ToggleMute), None);
        // A closed port: a dry run must plan without any connection attempt.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint =
            ObsEndpoint::new("127.0.0.1", listener.local_addr().unwrap().port()).unwrap();
        let mut with = Dispatcher::new(false, None, abort, Some(ObsClient::new(endpoint, None)));
        assert_eq!(with.unavailable(&scene), None);
        with.dispatch(&scene).unwrap();
        assert!(
            listener.accept().is_err(),
            "a dry run never connects to OBS"
        );
        assert!(
            without
                .dispatch(&Action::ObsScene {
                    scene: String::new()
                })
                .is_err()
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

#[cfg(all(test, target_os = "macos"))]
mod control_regressions {
    use super::*;
    use cooltek_sdk_cli::live_runtime::LiveSummary;
    use cooltek_sdk_cli::service_events::{PlannedEvent, Work, process_work};
    use cooltek_sdk_service::{Action, Gesture, InputEvent, ProfileResolution};
    #[test]
    fn a_switch_to_test_does_not_log_execution() {
        // The run starts in Live and is switched to Test. The action is an OBS scene aimed at a local listener, so a
        // regression that still executed would only open a connection to this port, never touch the machine.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint =
            ObsEndpoint::new("127.0.0.1", listener.local_addr().unwrap().port()).unwrap();
        let mode = Arc::new(AtomicBool::new(true));
        let mut d = Dispatcher::new(
            true,
            Some(mode.clone()),
            Arc::new(AtomicBool::new(false)),
            Some(ObsClient::new(endpoint, None)),
        );
        mode.store(false, Ordering::SeqCst);
        let work = Work {
            raw: vec![1],
            timestamp_micros: 0,
            status: "decoded",
            error: None,
            events: vec![PlannedEvent {
                event: InputEvent {
                    timestamp_micros: 0,
                    control: 0,
                    gesture: Gesture::Pressed,
                    value: None,
                },
                binding: None,
                resolution: ProfileResolution::Action {
                    action: Action::ObsScene {
                        scene: "Live".into(),
                    },
                    page_id: None,
                    dial_function: None,
                },
                dial_function: None,
                coalesced: 1,
            }],
            report_count: 1,
            coalesced_parse_error_count: 0,
            dropped_event_count: 0,
            dropped_dial_function_count: 0,
            merge_barrier: false,
        };
        let mut output = Vec::new();
        process_work(
            work,
            &mut d,
            &mut output,
            &AtomicBool::new(false),
            true,
            false,
            false,
            &mut TimerTable::default(),
            0,
            &mut LiveSummary::default(),
        )
        .unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(
            !output.contains("action_executed"),
            "Test action was reported as executed: {output}"
        );
        assert!(
            listener.accept().is_err(),
            "a switch to Test must not connect to OBS"
        );
    }
    #[test]
    fn every_timer_expiry_uses_the_current_mode() {
        struct Recording(std::sync::Arc<std::sync::atomic::AtomicUsize>);
        impl TimerAlerter for Recording {
            fn alert(&mut self, _: &TimerAlert) -> Result<Option<String>, String> {
                self.0.fetch_add(1, Ordering::SeqCst);
                Ok(None)
            }
        }
        let mode = Arc::new(AtomicBool::new(false));
        let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut alerter = ModeAlerter {
            execute: mode.clone(),
            inner: Box::new(Recording(count.clone())),
        };
        let alert = TimerAlert {
            seconds: 1,
            label: None,
            sound: cooltek_sdk_service::TimerSound::Ping,
        };
        for live in [false, true, false, true, false] {
            mode.store(live, Ordering::SeqCst);
            alerter.alert(&alert).unwrap();
        }
        assert_eq!(count.load(Ordering::SeqCst), 2);
    }
}
