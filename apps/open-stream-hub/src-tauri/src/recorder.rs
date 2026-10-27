//! Keyboard recording for the macro editor: a listen-only `CGEventTap` that
//! collects key down and up events with their modifiers and timing while the
//! user records, plus the macOS permission checks the flow needs.
//!
//! Listening needs the Input Monitoring permission; posting the recorded keys
//! later needs Accessibility. Both are preflighted here (through the same
//! public `CGPreflight*EventAccess` functions the service's scripts use) and
//! never requested implicitly: the app explains them and links to System
//! Settings instead. Nothing here posts events or changes the system.

use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// One recorded keyboard event, in recording order. `key` is the
/// OpenStreamHub key name (or literal character) of the virtual key code, or
/// `None` for a key no macro can hold; the editor folds these into steps.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RecordedKey {
    pub key: Option<String>,
    pub key_code: u32,
    pub down: bool,
    pub shift: bool,
    pub control: bool,
    pub option: bool,
    pub command: bool,
    /// Milliseconds since the recording started.
    pub at_ms: u32,
}

/// The current recording, as the editor polls it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RecordingView {
    pub events: Vec<RecordedKey>,
    pub elapsed_ms: u32,
    /// True once the recording hit its event or time cap and later input is
    /// dropped; the editor tells the user to stop and use what was captured.
    pub capped: bool,
}

/// Which privacy pane of System Settings a flow links to.
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionTarget {
    /// Privacy & Security > Accessibility (posting keyboard events).
    PostEvents,
    /// Privacy & Security > Input Monitoring (listening to keyboard events).
    ListenEvents,
    /// Privacy & Security > Automation (controlling System Events and players).
    Automation,
}

/// The two permission preflights the macro flow needs.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct InputPermissions {
    /// Whether keyboard events can be posted (Accessibility).
    pub post_events: bool,
    /// Whether keyboard events can be listened to (Input Monitoring).
    pub listen_events: bool,
}

/// A recording stops collecting after this long; the editor stops it.
const RECORDING_MAX_MS: u32 = 60_000;
/// A recording keeps at most this many events (keys plus their releases).
const EVENT_CAP: usize = 4096;
/// How long starting waits for the event tap to come up before giving up.
const TAP_START_WAIT: Duration = Duration::from_secs(2);
/// How long stopping waits for the tap thread's last push before reading the
/// events anyway.
const TAP_STOP_WAIT: Duration = Duration::from_secs(1);

struct TapState {
    events: Mutex<Vec<RecordedKey>>,
    started: Instant,
    /// Set as soon as stopping begins, so an event between stop and the run
    /// loop exit is dropped by the callback.
    stopping: std::sync::atomic::AtomicBool,
    /// Set by the tap thread once its run loop has stopped, so no more pushes
    /// can race the final read.
    stopped: Mutex<bool>,
}

struct Recorder {
    state: Arc<TapState>,
    /// The tap thread's run loop, as a `usize` so the guard stays `Send`.
    runloop: Arc<Mutex<Option<usize>>>,
    /// Why the tap could not start, set by the tap thread.
    failure: Arc<Mutex<Option<String>>>,
}

static RECORDER: Mutex<Option<Recorder>> = Mutex::new(None);

#[cfg(target_os = "macos")]
mod tap {
    use super::{EVENT_CAP, RECORDING_MAX_MS, RecordedKey, TapState};
    use std::ffi::c_void;
    use std::sync::atomic::Ordering;
    use std::sync::{Arc, Mutex};

    pub(super) type CFMachPortRef = *mut c_void;
    pub(super) type CFRunLoopRef = *mut c_void;
    pub(super) type CFRunLoopSourceRef = *mut c_void;
    pub(super) type CGEventRef = *mut c_void;
    pub(super) type CGEventTapProxy = *mut c_void;
    pub(super) type CGEventTapCallBack =
        unsafe extern "C" fn(CGEventTapProxy, u32, CGEventRef, *mut c_void) -> CGEventRef;

    // kCGSessionEventTap, kCGHeadInsertEventTap, kCGEventTapOptionListenOnly.
    const SESSION_TAP: u32 = 1;
    const HEAD_INSERT: u32 = 0;
    const LISTEN_ONLY: u32 = 1;
    // kCGEventKeyDown and kCGEventKeyUp.
    pub(super) const KEY_DOWN: u32 = 10;
    pub(super) const KEY_UP: u32 = 11;
    // kCGKeyboardEventKeycode.
    const FIELD_KEY_CODE: u32 = 9;
    // kCGEventFlagMaskShift, Control, Alternate and Command.
    const FLAG_SHIFT: u64 = 1 << 17;
    const FLAG_CONTROL: u64 = 1 << 18;
    const FLAG_OPTION: u64 = 1 << 19;
    const FLAG_COMMAND: u64 = 1 << 20;

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        // A `CFStringRef` constant: the symbol holds the pointer, so pass its value, never its address.
        static kCFRunLoopCommonModes: *const c_void;

        fn CFMachPortCreateRunLoopSource(
            allocator: *const c_void,
            port: CFMachPortRef,
            order: isize,
        ) -> CFRunLoopSourceRef;
        fn CFRunLoopAddSource(
            runloop: CFRunLoopRef,
            source: CFRunLoopSourceRef,
            mode: *const c_void,
        );
        fn CFRunLoopGetCurrent() -> CFRunLoopRef;
        fn CFRunLoopRun();
        fn CFRunLoopStop(runloop: CFRunLoopRef);
        fn CFRelease(memory: *const c_void);
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGEventTapCreate(
            tap: u32,
            place: u32,
            options: u32,
            events_of_interest: u64,
            callback: CGEventTapCallBack,
            user_info: *mut c_void,
        ) -> CFMachPortRef;
        fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
        fn CGEventGetIntegerValueField(event: CGEventRef, field: u32) -> i64;
        fn CGEventGetFlags(event: CGEventRef) -> u64;
    }

    /// The tap callback: one recorded event per key down or up, never
    /// modifying or swallowing the event. `user_info` is a `Box<Arc<TapState>>`
    /// owned by the tap thread, which keeps it alive until its run loop
    /// returns, so the pointer is valid for every callback in between.
    unsafe extern "C" fn record(
        _proxy: CGEventTapProxy,
        event_type: u32,
        event: CGEventRef,
        user_info: *mut c_void,
    ) -> CGEventRef {
        if event_type != KEY_DOWN && event_type != KEY_UP {
            return event;
        }
        // SAFETY: `user_info` is the boxed state the tap thread owns for the
        // whole life of the tap, and `event` is the live CoreGraphics event;
        // every call follows the framework's contract.
        unsafe {
            let state = &*user_info.cast::<Arc<TapState>>();
            // Stopping wins over a push that was already in flight.
            if state.stopping.load(Ordering::SeqCst) {
                return event;
            }
            let elapsed = state.started.elapsed();
            let Ok(mut events) = state.events.try_lock() else {
                return event;
            };
            if events.len() >= EVENT_CAP || elapsed.as_millis() >= u128::from(RECORDING_MAX_MS) {
                return event;
            }
            let code = CGEventGetIntegerValueField(event, FIELD_KEY_CODE);
            let flags = CGEventGetFlags(event);
            events.push(RecordedKey {
                key: cooltek_sdk_service::key_for_virtual_code(code.clamp(0, 255) as u8),
                key_code: code.clamp(0, i64::from(u8::MAX)) as u32,
                down: event_type == KEY_DOWN,
                shift: flags & FLAG_SHIFT != 0,
                control: flags & FLAG_CONTROL != 0,
                option: flags & FLAG_OPTION != 0,
                command: flags & FLAG_COMMAND != 0,
                at_ms: u32::try_from(elapsed.as_millis()).unwrap_or(u32::MAX),
            });
        }
        event
    }

    /// Creates the tap and runs its run loop until `CFRunLoopStop`. The boxed
    /// `Arc` handed to the tap as `user_info` is recovered on the way out.
    pub(super) fn run_tap(
        state: Arc<TapState>,
        runloop_slot: Arc<Mutex<Option<usize>>>,
        failure: Arc<Mutex<Option<String>>>,
    ) {
        // SAFETY: the raw tap callback reads only CoreGraphics-owned data and
        // the boxed state this thread owns for the whole run; every Core
        // Foundation call follows its contract (null allocator, one release
        // per create, stop from another thread).
        unsafe {
            let user_info = Box::into_raw(Box::new(state)) as *mut c_void;
            let port = CGEventTapCreate(
                SESSION_TAP,
                HEAD_INSERT,
                LISTEN_ONLY,
                (1 << KEY_DOWN) | (1 << KEY_UP),
                record,
                user_info,
            );
            if port.is_null() {
                *failure.lock().unwrap() = Some(
                    "macOS refused the keyboard listener. Allow OpenStreamHub in System Settings > Privacy & Security > Input Monitoring, then record again.".into(),
                );
                drop(Box::from_raw(user_info.cast::<Arc<TapState>>()));
                return;
            }
            CGEventTapEnable(port, true);
            let source = CFMachPortCreateRunLoopSource(std::ptr::null(), port, 0);
            if source.is_null() {
                *failure.lock().unwrap() =
                    Some("could not create the keyboard listener's run-loop source".into());
                CFRelease(port);
                drop(Box::from_raw(user_info.cast::<Arc<TapState>>()));
                return;
            }
            let runloop = CFRunLoopGetCurrent();
            *runloop_slot.lock().unwrap() = Some(runloop as usize);
            CFRunLoopAddSource(runloop, source, kCFRunLoopCommonModes);
            CFRunLoopRun();
            CFRelease(source);
            CFRelease(port);
            drop(Box::from_raw(user_info.cast::<Arc<TapState>>()));
        }
    }

    /// Stops a running tap's run loop; the thread finishes on its own.
    ///
    /// # Safety
    /// `runloop` must be the run loop the tap thread reported.
    pub(super) unsafe fn stop_runloop(runloop: usize) {
        // SAFETY: see the function contract.
        unsafe { CFRunLoopStop(runloop as CFRunLoopRef) }
    }
}

/// What `spawn_tap` hands back: the shared state, the tap thread's run loop
/// and the reason it could not start, if any.
type TapParts = (
    Arc<TapState>,
    Arc<Mutex<Option<usize>>>,
    Arc<Mutex<Option<String>>>,
);

/// Creates the tap thread. Shared by `start` so both the macOS and the
/// fallback path agree on the shared-slot protocol.
fn spawn_tap() -> TapParts {
    let state = Arc::new(TapState {
        events: Mutex::new(Vec::new()),
        started: Instant::now(),
        stopping: std::sync::atomic::AtomicBool::new(false),
        stopped: Mutex::new(false),
    });
    let runloop = Arc::new(Mutex::new(None));
    let failure = Arc::new(Mutex::new(None));
    #[cfg(target_os = "macos")]
    {
        let thread_state = state.clone();
        let thread_runloop = runloop.clone();
        let thread_failure = failure.clone();
        // Detached: after `CFRunLoopStop` the run loop returns and the thread
        // releases the tap on its own; stopping never joins it.
        std::thread::spawn(move || tap::run_tap(thread_state, thread_runloop, thread_failure));
    }
    #[cfg(not(target_os = "macos"))]
    {
        *failure.lock().unwrap() = Some("Recording keyboard input works only on macOS.".into());
    }
    (state, runloop, failure)
}

/// Starts recording keyboard input, refusing without the Input Monitoring
/// permission or while a recording is already running.
pub fn start() -> Result<(), String> {
    let mut guard = RECORDER
        .lock()
        .map_err(|error| format!("the recorder is unavailable: {error}"))?;
    if guard.is_some() {
        return Err("a macro recording is already running".into());
    }
    if !cfg!(target_os = "macos") {
        return Err("Recording keyboard input works only on macOS.".into());
    }
    if !permissions()?.listen_events {
        return Err(
            "Recording keys needs the Input Monitoring permission. Allow OpenStreamHub in System Settings > Privacy & Security > Input Monitoring, then record again."
                .into(),
        );
    }
    let (state, runloop, failure) = spawn_tap();
    // Wait for the tap to come up so an immediate refusal (denied permission,
    // a failed create) is reported to the editor as the start's error.
    let deadline = Instant::now() + TAP_START_WAIT;
    loop {
        if let Some(error) = failure.lock().unwrap().clone() {
            return Err(error);
        }
        if runloop.lock().unwrap().is_some() {
            break;
        }
        if Instant::now() >= deadline {
            return Err("the keyboard listener did not start within 2 s".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    *guard = Some(Recorder {
        state,
        runloop,
        failure,
    });
    Ok(())
}

/// The current recording, or an error when nothing is recording.
pub fn view() -> Result<RecordingView, String> {
    let guard = RECORDER
        .lock()
        .map_err(|error| format!("the recorder is unavailable: {error}"))?;
    let recorder = guard.as_ref().ok_or("no macro recording is running")?;
    Ok(view_of(recorder))
}

fn view_of(recorder: &Recorder) -> RecordingView {
    let elapsed = recorder.state.started.elapsed();
    let events = recorder.state.events.lock().unwrap().clone();
    let capped = events.len() >= EVENT_CAP || elapsed.as_millis() >= u128::from(RECORDING_MAX_MS);
    RecordingView {
        events,
        elapsed_ms: u32::try_from(elapsed.as_millis()).unwrap_or(u32::MAX),
        capped,
    }
}

/// Stops the running recording and returns what it captured. The recorded
/// events stay available for nothing afterwards: the editor keeps its copy.
pub fn stop() -> Result<RecordingView, String> {
    let recorder = {
        let mut guard = RECORDER
            .lock()
            .map_err(|error| format!("the recorder is unavailable: {error}"))?;
        guard
            .take()
            .ok_or_else(|| "no macro recording is running".to_string())?
    };
    recorder
        .state
        .stopping
        .store(true, std::sync::atomic::Ordering::SeqCst);
    if let Some(error) = recorder.failure.lock().unwrap().clone() {
        return Err(error);
    }
    // The tap may still be starting; wait for its run loop, then stop it.
    let deadline = Instant::now() + TAP_START_WAIT;
    loop {
        if let Some(runloop) = recorder.runloop.lock().unwrap().as_ref().copied() {
            #[cfg(target_os = "macos")]
            // SAFETY: the pointer is the run loop the tap thread reported,
            // still live or already stopped; `CFRunLoopStop` handles both.
            unsafe {
                tap::stop_runloop(runloop)
            }
            break;
        }
        if Instant::now() >= deadline {
            return Err("the keyboard listener did not start within 2 s".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    // Wait for the tap thread's last push before reading the events.
    let deadline = Instant::now() + TAP_STOP_WAIT;
    while !*recorder.state.stopped.lock().unwrap() {
        if Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    *recorder.state.stopped.lock().unwrap() = true;
    Ok(view_of(&recorder))
}

/// Fixed JavaScript for Automation that preflights both permissions and
/// prints them. No argument is passed and nothing is requested or posted.
const PREFLIGHT_SCRIPT: &[&str] = &[
    "ObjC.import('CoreGraphics');",
    "ObjC.bindFunction('CGPreflightPostEventAccess', ['bool', []]);",
    "ObjC.bindFunction('CGPreflightListenEventAccess', ['bool', []]);",
    "function run() { return $.CGPreflightPostEventAccess() + ' ' + $.CGPreflightListenEventAccess(); }",
];

const OSASCRIPT: &str = "/usr/bin/osascript";
const OPEN: &str = "/usr/bin/open";
const ACCESSIBILITY_PANE: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility";
const LISTEN_PANE: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent";
const AUTOMATION_PANE: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_Automation";
/// Bounded subprocesses: the checks are one-shot tools, so 5 s is generous.
const TOOL_TIMEOUT: Duration = Duration::from_secs(5);

struct ToolOutput {
    success: bool,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

/// Runs one short-lived tool with a poll deadline. Unlike the service crate's
/// runner this has no stdin, no cancellation and no output cap needs: the
/// preflight prints two words and `open` prints nothing.
fn run_tool(program: &str, args: &[&str]) -> Result<ToolOutput, String> {
    if !cfg!(target_os = "macos") {
        return Err("this works only on macOS".into());
    }
    let mut child = std::process::Command::new(program)
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|error| format!("{program}: could not start process: {error}"))?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut stdout = Vec::new();
                let mut stderr = Vec::new();
                if let Some(mut pipe) = child.stdout.take() {
                    std::io::Read::read_to_end(&mut pipe, &mut stdout)
                        .map_err(|error| error.to_string())?;
                }
                if let Some(mut pipe) = child.stderr.take() {
                    std::io::Read::read_to_end(&mut pipe, &mut stderr)
                        .map_err(|error| error.to_string())?;
                }
                return Ok(ToolOutput {
                    success: status.success(),
                    stdout,
                    stderr,
                });
            }
            Ok(None) => {
                if started.elapsed() >= TOOL_TIMEOUT {
                    let _ = child.kill();
                    return Err(format!("{program}: timed out after 5 s"));
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => return Err(format!("{program}: could not inspect process: {error}")),
        }
    }
}

/// Preflights both permissions through the fixed script. A failed tool is an
/// error, not "denied": the flow must not send the user to System Settings
/// because `osascript` itself was broken.
pub fn permissions() -> Result<InputPermissions, String> {
    let mut args = vec!["-l", "JavaScript"];
    for line in PREFLIGHT_SCRIPT {
        args.extend(["-e", line]);
    }
    let output = run_tool(OSASCRIPT, &args)?;
    if !output.success {
        return Err(format!(
            "{OSASCRIPT}: the permission check failed (stderr: {} bytes, not shown)",
            output.stderr.len()
        ));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut parts = text.split_whitespace();
    let (post_events, listen_events) = match (parts.next(), parts.next()) {
        (Some(post), Some(listen)) => (post == "true", listen == "true"),
        _ => {
            return Err(format!("{OSASCRIPT}: unexpected permission check output"));
        }
    };
    Ok(InputPermissions {
        post_events,
        listen_events,
    })
}

/// Opens the System Settings pane of one permission. Deep links are best
/// effort: `open` succeeds even when System Settings ignores the anchor, so
/// the flow says which pane to look at either way.
pub fn open_permission_settings(target: PermissionTarget) -> Result<(), String> {
    let pane = match target {
        PermissionTarget::PostEvents => ACCESSIBILITY_PANE,
        PermissionTarget::ListenEvents => LISTEN_PANE,
        PermissionTarget::Automation => AUTOMATION_PANE,
    };
    let output = run_tool(OPEN, &["-u", pane])?;
    if output.success {
        Ok(())
    } else {
        Err(format!(
            "{OPEN}: could not open System Settings (stderr: {} bytes, not shown)",
            output.stderr.len()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recording_views_report_cap_and_elapsed() {
        let state = Arc::new(TapState {
            events: Mutex::new(Vec::new()),
            started: Instant::now(),
            stopping: std::sync::atomic::AtomicBool::new(false),
            stopped: Mutex::new(false),
        });
        let recorder = Recorder {
            state: state.clone(),
            runloop: Arc::new(Mutex::new(None)),
            failure: Arc::new(Mutex::new(None)),
        };
        assert_eq!(view_of(&recorder).elapsed_ms, 0);
        *state.events.lock().unwrap() = vec![RecordedKey {
            key: Some("a".into()),
            key_code: 0,
            down: true,
            shift: false,
            control: false,
            option: false,
            command: true,
            at_ms: 5,
        }];
        let view = view_of(&recorder);
        assert_eq!(view.events.len(), 1);
        assert!(!view.capped);
        state
            .events
            .lock()
            .unwrap()
            .resize(EVENT_CAP, view.events[0].clone());
        assert!(view_of(&recorder).capped);
    }

    #[test]
    fn permission_commands_are_strict_off_macos() {
        if cfg!(target_os = "macos") {
            // The preflight runs for real; both answers are accepted.
            permissions().unwrap();
        } else {
            assert!(permissions().is_err());
            assert!(open_permission_settings(PermissionTarget::ListenEvents).is_err());
            assert!(start().is_err());
        }
        assert_eq!(stop().unwrap_err(), "no macro recording is running");
    }
}
