//! Explicit macOS actions through system tools. Constructing a dispatcher performs no actions.

use crate::{Action, ActionDispatcher, Modifier};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

const OSASCRIPT: &str = "/usr/bin/osascript";
const OPEN: &str = "/usr/bin/open";
const PROCESS_TIMEOUT: Duration = Duration::from_secs(5);
const OUTPUT_LIMIT: usize = 16 * 1024;

/// The system output's volume and mute state, suitable for exact restoration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AudioSnapshot {
    /// Output volume in percentage points, between 0 and 100 inclusive.
    pub output_volume: u8,
    /// Whether the system output is muted.
    pub output_muted: bool,
}

impl AudioSnapshot {
    /// Checks the volume before a caller-provided snapshot reaches system tools.
    pub fn validate(&self) -> Result<(), String> {
        if self.output_volume > 100 {
            Err("audio snapshot output volume must be between 0 and 100".into())
        } else {
            Ok(())
        }
    }
}

/// Executes validated actions on macOS. Shortcut execution may require the user's
/// Accessibility and Automation permissions for the invoking application.
///
/// Failed or timed-out actions are returned to the caller and are never retried;
/// an action may already have taken effect before its process reports a failure.
pub struct MacOsDispatcher {
    runner: Box<dyn ProcessRunner>,
}

impl MacOsDispatcher {
    /// Creates a dispatcher without launching subprocesses or requesting permissions.
    pub fn new() -> Self {
        Self {
            runner: Box::new(SystemProcessRunner { cancellation: None }),
        }
    }

    /// Creates a dispatcher that stops starting actions when the shared flag is set.
    ///
    /// Cancellation also stops monitoring an in-flight system-tool process and
    /// attempts bounded kill/reap cleanup. An action may already have taken effect.
    pub fn with_cancellation(cancellation: Arc<AtomicBool>) -> Self {
        Self {
            runner: Box::new(SystemProcessRunner {
                cancellation: Some(cancellation),
            }),
        }
    }

    /// Queries current output volume and mute state through bounded subprocesses.
    ///
    /// This performs no setter operations. A fixed-volume output can return an
    /// unsupported value; malformed or unavailable state is reported as an error.
    pub fn audio_snapshot(&mut self) -> Result<AudioSnapshot, String> {
        Ok(AudioSnapshot {
            output_volume: self.output_volume()?,
            output_muted: self.output_muted()?,
        })
    }

    /// Attempts to restore both fields of a validated snapshot independently.
    ///
    /// A failed volume setter does not prevent the mute setter from being
    /// attempted. Errors are combined; neither setter is retried. Existing
    /// cancellation remains effective, including during restoration. A successful
    /// return reports system-tool completion, not a fresh state verification.
    pub fn restore_audio_snapshot(&mut self, snapshot: &AudioSnapshot) -> Result<(), String> {
        snapshot.validate()?;
        let volume = self
            .apple_script(format!(
                "set volume output volume {}",
                snapshot.output_volume
            ))
            .map_err(|error| format!("restoring output volume failed: {error}"));
        let muted = self
            .apple_script(format!("set volume output muted {}", snapshot.output_muted))
            .map_err(|error| format!("restoring output mute failed: {error}"));
        match (volume, muted) {
            (Ok(_), Ok(_)) => Ok(()),
            (Err(error), Ok(_)) | (Ok(_), Err(error)) => Err(error),
            (Err(volume), Err(muted)) => Err(format!("{volume}; {muted}")),
        }
    }

    fn run(&mut self, program: &'static str, args: Vec<String>) -> Result<Vec<u8>, String> {
        check_cancellation(self.runner.cancellation(), program)?;
        let output = self.runner.run(&ProcessRequest {
            program,
            args,
            timeout: PROCESS_TIMEOUT,
            output_limit: OUTPUT_LIMIT,
        })?;
        if output.stdout.len() > OUTPUT_LIMIT || output.stderr.len() > OUTPUT_LIMIT {
            return Err(format!(
                "{program}: subprocess output exceeded {OUTPUT_LIMIT} bytes"
            ));
        }
        if !output.success {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let details = stderr.trim();
            return Err(if details.is_empty() {
                format!("{program}: {}", output.status)
            } else {
                format!("{program}: {}: {details}", output.status)
            });
        }
        Ok(output.stdout)
    }

    fn apple_script(&mut self, script: String) -> Result<Vec<u8>, String> {
        self.run(OSASCRIPT, vec!["-e".into(), script])
    }

    fn query(&mut self, property: &str) -> Result<String, String> {
        let output = self.apple_script(format!("{property} of (get volume settings)"))?;
        let text = String::from_utf8(output)
            .map_err(|_| format!("{OSASCRIPT}: invalid UTF-8 in {property} response"))?;
        Ok(text.trim().to_owned())
    }

    fn output_volume(&mut self) -> Result<u8, String> {
        let response = self.query("output volume")?;
        response
            .parse::<u8>()
            .ok()
            .filter(|value| *value <= 100)
            .ok_or_else(|| format!("{OSASCRIPT}: invalid output volume response {response:?}"))
    }

    fn output_muted(&mut self) -> Result<bool, String> {
        let response = self.query("output muted")?;
        match response.as_str() {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err(format!(
                "{OSASCRIPT}: invalid output muted response {response:?}"
            )),
        }
    }
}

impl Default for MacOsDispatcher {
    fn default() -> Self {
        Self::new()
    }
}

impl ActionDispatcher for MacOsDispatcher {
    fn dispatch(&mut self, action: &Action) -> Result<(), String> {
        // Validate even when the caller did not use the profile loader.
        action.validate()?;
        match action {
            Action::Volume { delta } => {
                let current = self.output_volume()?;
                let target = (i32::from(current) + i32::from(*delta)).clamp(0, 100);
                self.apple_script(format!("set volume output volume {target}"))?;
            }
            Action::ToggleMute => {
                let target = !self.output_muted()?;
                self.apple_script(format!("set volume output muted {target}"))?;
            }
            Action::Shortcut { key, modifiers } => {
                // The validated key contains one ASCII alphanumeric character or space,
                // so it cannot introduce AppleScript syntax or additional commands.
                let mut script =
                    format!("tell application \"System Events\" to keystroke \"{key}\"");
                if !modifiers.is_empty() {
                    let names = modifiers
                        .iter()
                        .map(|modifier| match modifier {
                            Modifier::Command => "command down",
                            Modifier::Control => "control down",
                            Modifier::Option => "option down",
                            Modifier::Shift => "shift down",
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    script.push_str(&format!(" using {{{names}}}"));
                }
                self.apple_script(script)?;
            }
            Action::LaunchApp { bundle_id } => {
                self.run(OPEN, vec!["-b".into(), bundle_id.clone()])?;
            }
        }
        Ok(())
    }
}

#[derive(Debug, PartialEq, Eq)]
struct ProcessRequest {
    program: &'static str,
    args: Vec<String>,
    timeout: Duration,
    output_limit: usize,
}

struct ProcessOutput {
    success: bool,
    status: String,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

trait ProcessRunner: Send {
    fn run(&mut self, request: &ProcessRequest) -> Result<ProcessOutput, String>;

    fn cancellation(&self) -> Option<&AtomicBool> {
        None
    }
}

struct SystemProcessRunner {
    cancellation: Option<Arc<AtomicBool>>,
}

fn check_cancellation(cancellation: Option<&AtomicBool>, program: &str) -> Result<(), String> {
    if cancellation.is_some_and(|flag| flag.load(Ordering::Acquire)) {
        Err(format!("{program}: action execution cancelled"))
    } else {
        Ok(())
    }
}

#[cfg(not(target_os = "macos"))]
impl ProcessRunner for SystemProcessRunner {
    fn run(&mut self, request: &ProcessRequest) -> Result<ProcessOutput, String> {
        check_cancellation(self.cancellation(), request.program)?;
        Err("macOS action execution is unsupported on this platform".into())
    }

    fn cancellation(&self) -> Option<&AtomicBool> {
        self.cancellation.as_deref()
    }
}

#[cfg(target_os = "macos")]
impl ProcessRunner for SystemProcessRunner {
    fn run(&mut self, request: &ProcessRequest) -> Result<ProcessOutput, String> {
        use std::process::{Command, Stdio};
        check_cancellation(self.cancellation(), request.program)?;
        let mut child = Command::new(request.program)
            .args(&request.args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("{}: could not start process: {error}", request.program))?;
        let result = collect_child_output(&mut child, request, self.cancellation());
        if let Err(error) = result {
            // Stop the direct system-tool process on all monitoring failures. No retry
            // is safe: a partially completed action could otherwise execute twice.
            return match stop_child(&mut child) {
                Ok(()) => Err(error),
                Err(cleanup_error) => Err(format!(
                    "{error}; subprocess cleanup failed: {cleanup_error}"
                )),
            };
        }
        result
    }

    fn cancellation(&self) -> Option<&AtomicBool> {
        self.cancellation.as_deref()
    }
}

#[cfg(target_os = "macos")]
fn stop_child(child: &mut std::process::Child) -> Result<(), String> {
    use std::time::Instant;
    if child
        .try_wait()
        .map_err(|error| error.to_string())?
        .is_some()
    {
        return Ok(());
    }
    if let Err(error) = child.kill() {
        // The process may have exited between the status check and kill.
        if child
            .try_wait()
            .map_err(|error| error.to_string())?
            .is_none()
        {
            return Err(format!("could not kill process: {error}"));
        }
        return Ok(());
    }
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return Ok(()),
            Err(error) => return Err(format!("could not reap killed process: {error}")),
            Ok(None) => {
                if started.elapsed() >= Duration::from_secs(1) {
                    return Err("killed process did not exit within 1000 ms".into());
                }
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(target_os = "macos")]
fn collect_child_output(
    child: &mut std::process::Child,
    request: &ProcessRequest,
    cancellation: Option<&AtomicBool>,
) -> Result<ProcessOutput, String> {
    use std::time::Instant;
    let mut stdout_pipe = child
        .stdout
        .take()
        .ok_or("missing subprocess stdout pipe")?;
    let mut stderr_pipe = child
        .stderr
        .take()
        .ok_or("missing subprocess stderr pipe")?;
    make_nonblocking(&stdout_pipe)?;
    make_nonblocking(&stderr_pipe)?;
    let start = Instant::now();
    collect_output(
        &mut stdout_pipe,
        &mut stderr_pipe,
        request,
        || {
            child
                .try_wait()
                .map(|status| status.map(|status| (status.success(), status.to_string())))
                .map_err(|error| error.to_string())
        },
        || start.elapsed(),
        || std::thread::sleep(Duration::from_millis(10)),
        || cancellation.is_some_and(|flag| flag.load(Ordering::Acquire)),
    )
}

#[cfg(any(target_os = "macos", test))]
fn collect_output(
    stdout_pipe: &mut impl std::io::Read,
    stderr_pipe: &mut impl std::io::Read,
    request: &ProcessRequest,
    mut poll_status: impl FnMut() -> Result<Option<(bool, String)>, String>,
    mut elapsed: impl FnMut() -> Duration,
    mut pause: impl FnMut(),
    mut cancelled: impl FnMut() -> bool,
) -> Result<ProcessOutput, String> {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut stdout_closed = false;
    let mut stderr_closed = false;
    loop {
        if cancelled() {
            return Err(format!(
                "{}: action execution cancelled; action may already have taken effect",
                request.program
            ));
        }
        if !stdout_closed {
            stdout_closed = drain_pipe(stdout_pipe, &mut stdout, request.output_limit)
                .map_err(|error| format!("{}: stdout: {error}", request.program))?;
        }
        if !stderr_closed {
            stderr_closed = drain_pipe(stderr_pipe, &mut stderr, request.output_limit)
                .map_err(|error| format!("{}: stderr: {error}", request.program))?;
        }
        if cancelled() {
            return Err(format!(
                "{}: action execution cancelled; action may already have taken effect",
                request.program
            ));
        }
        if let Some((success, status)) = poll_status()
            .map_err(|error| format!("{}: could not inspect process: {error}", request.program))?
            && stdout_closed
            && stderr_closed
        {
            return Ok(ProcessOutput {
                success,
                status,
                stdout,
                stderr,
            });
        }
        if elapsed() >= request.timeout {
            return Err(format!(
                "{}: subprocess timed out after {} ms; action may already have taken effect",
                request.program,
                request.timeout.as_millis()
            ));
        }
        pause();
    }
}

#[cfg(target_os = "macos")]
fn make_nonblocking(pipe: &impl std::os::fd::AsRawFd) -> Result<(), String> {
    let fd = pipe.as_raw_fd();
    // SAFETY: fd belongs to a live child pipe. These fcntl operations read and
    // modify only its status flags and do not take ownership of the descriptor.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags == -1 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
        return Err(format!(
            "could not make subprocess pipe nonblocking: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

#[cfg(any(target_os = "macos", test))]
fn drain_pipe(
    pipe: &mut impl std::io::Read,
    collected: &mut Vec<u8>,
    limit: usize,
) -> Result<bool, String> {
    use std::io::ErrorKind;
    let mut buffer = [0; 1024];
    // A read budget also bounds repeated Interrupted responses so the caller
    // regains control to check its deadline even under continuous signals.
    for _ in 0..32 {
        match pipe.read(&mut buffer) {
            Ok(0) => return Ok(true),
            Ok(count) => {
                if count > limit.saturating_sub(collected.len()) {
                    return Err(format!("subprocess output exceeded {limit} bytes"));
                }
                collected.extend_from_slice(&buffer[..count]);
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => return Ok(false),
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::VecDeque, sync::Mutex};

    #[derive(Default)]
    struct FakeState {
        requests: Vec<ProcessRequest>,
        results: VecDeque<Result<ProcessOutput, String>>,
    }
    struct FakeRunner(Arc<Mutex<FakeState>>);
    impl ProcessRunner for FakeRunner {
        fn run(&mut self, request: &ProcessRequest) -> Result<ProcessOutput, String> {
            let mut state = self.0.lock().unwrap();
            state.requests.push(ProcessRequest {
                program: request.program,
                args: request.args.clone(),
                timeout: request.timeout,
                output_limit: request.output_limit,
            });
            state.results.pop_front().expect("unexpected subprocess")
        }
    }

    fn success(stdout: &str) -> Result<ProcessOutput, String> {
        Ok(ProcessOutput {
            success: true,
            status: "exit status: 0".into(),
            stdout: stdout.as_bytes().to_vec(),
            stderr: Vec::new(),
        })
    }

    fn fake(
        results: Vec<Result<ProcessOutput, String>>,
    ) -> (MacOsDispatcher, Arc<Mutex<FakeState>>) {
        let state = Arc::new(Mutex::new(FakeState {
            requests: Vec::new(),
            results: results.into(),
        }));
        let dispatcher = MacOsDispatcher {
            runner: Box::new(FakeRunner(state.clone())),
        };
        (dispatcher, state)
    }

    #[test]
    fn volume_queries_before_setting_and_clamps_both_edges() {
        for (initial, delta, target) in [("98\n", 5, 100), ("2\n", -5, 0), ("40\n", 4, 44)] {
            let (mut dispatcher, state) = fake(vec![success(initial), success("")]);
            dispatcher.dispatch(&Action::Volume { delta }).unwrap();
            let state = state.lock().unwrap();
            assert_eq!(state.requests.len(), 2);
            assert_eq!(state.requests[0].program, OSASCRIPT);
            assert_eq!(
                state.requests[0].args,
                ["-e", "output volume of (get volume settings)"]
            );
            assert_eq!(
                state.requests[1].args,
                [
                    "-e".to_string(),
                    format!("set volume output volume {target}")
                ]
            );
            assert!(
                state
                    .requests
                    .iter()
                    .all(|request| request.timeout == PROCESS_TIMEOUT
                        && request.output_limit == OUTPUT_LIMIT)
            );
        }
    }

    #[test]
    fn invalid_volume_response_stops_before_setting() {
        for response in ["missing value", "101", "-1", "50.1", "0\n100"] {
            let (mut dispatcher, state) = fake(vec![success(response)]);
            assert!(
                dispatcher
                    .dispatch(&Action::Volume { delta: 1 })
                    .unwrap_err()
                    .contains("invalid output volume")
            );
            assert_eq!(state.lock().unwrap().requests.len(), 1);
        }
    }

    #[test]
    fn toggle_mute_reads_and_inverts_only_boolean_responses() {
        for (current, target) in [("true\n", "false"), ("false\n", "true")] {
            let (mut dispatcher, state) = fake(vec![success(current), success("")]);
            dispatcher.dispatch(&Action::ToggleMute).unwrap();
            let state = state.lock().unwrap();
            assert_eq!(
                state.requests[0].args[1],
                "output muted of (get volume settings)"
            );
            assert_eq!(
                state.requests[1].args[1],
                format!("set volume output muted {target}")
            );
        }
        let (mut dispatcher, state) = fake(vec![success("missing value")]);
        assert!(dispatcher.dispatch(&Action::ToggleMute).is_err());
        assert_eq!(state.lock().unwrap().requests.len(), 1);
    }

    #[test]
    fn shortcuts_use_literal_validated_keys_and_modifier_names() {
        let (mut dispatcher, state) = fake(vec![success(""), success("")]);
        dispatcher
            .dispatch(&Action::Shortcut {
                key: "s".into(),
                modifiers: vec![
                    Modifier::Command,
                    Modifier::Control,
                    Modifier::Option,
                    Modifier::Shift,
                ],
            })
            .unwrap();
        dispatcher
            .dispatch(&Action::Shortcut {
                key: " ".into(),
                modifiers: vec![],
            })
            .unwrap();
        let state = state.lock().unwrap();
        assert_eq!(
            state.requests[0].args[1],
            "tell application \"System Events\" to keystroke \"s\" using {command down, control down, option down, shift down}"
        );
        assert_eq!(
            state.requests[1].args[1],
            "tell application \"System Events\" to keystroke \" \""
        );
    }

    #[test]
    fn launch_app_uses_bundle_id_argument_without_shell() {
        let (mut dispatcher, state) = fake(vec![success("")]);
        dispatcher
            .dispatch(&Action::LaunchApp {
                bundle_id: "com.apple.TextEdit".into(),
            })
            .unwrap();
        let state = state.lock().unwrap();
        assert_eq!(state.requests[0].program, OPEN);
        assert_eq!(state.requests[0].args, ["-b", "com.apple.TextEdit"]);
    }

    #[test]
    fn invalid_actions_never_start_subprocesses() {
        let actions = [
            Action::Volume { delta: 0 },
            Action::Volume { delta: 101 },
            Action::Shortcut {
                key: "\" & do shell script \"bad".into(),
                modifiers: vec![],
            },
            Action::Shortcut {
                key: "s".into(),
                modifiers: vec![Modifier::Command, Modifier::Command],
            },
            Action::LaunchApp {
                bundle_id: "-a TextEdit".into(),
            },
        ];
        let (mut dispatcher, state) = fake(vec![]);
        for action in actions {
            assert!(dispatcher.dispatch(&action).is_err());
        }
        assert!(state.lock().unwrap().requests.is_empty());
    }

    #[test]
    fn exit_stderr_and_timeout_are_propagated_without_retry() {
        let (mut dispatcher, state) = fake(vec![Ok(ProcessOutput {
            success: false,
            status: "exit status: 1".into(),
            stdout: Vec::new(),
            stderr: b"System Events is not allowed assistive access (-1719)\n".to_vec(),
        })]);
        let action = Action::Shortcut {
            key: "s".into(),
            modifiers: vec![Modifier::Command],
        };
        let error = dispatcher.dispatch(&action).unwrap_err();
        assert!(error.contains("exit status: 1"));
        assert!(error.contains("not allowed assistive access (-1719)"));
        assert_eq!(state.lock().unwrap().requests.len(), 1);
        let (mut dispatcher, state) = fake(vec![Err("subprocess timed out after 5000 ms".into())]);
        assert_eq!(
            dispatcher.dispatch(&action).unwrap_err(),
            "subprocess timed out after 5000 ms"
        );
        assert_eq!(state.lock().unwrap().requests.len(), 1);
    }

    #[test]
    fn excessive_output_fails_before_any_followup_command() {
        let (mut dispatcher, state) = fake(vec![success(&"9".repeat(OUTPUT_LIMIT + 1))]);
        assert!(
            dispatcher
                .dispatch(&Action::Volume { delta: 1 })
                .unwrap_err()
                .contains("output exceeded")
        );
        assert_eq!(state.lock().unwrap().requests.len(), 1);
    }

    #[test]
    fn drain_pipe_is_bounded_and_preserves_exact_bytes() {
        let mut data = std::io::Cursor::new(b"hello".to_vec());
        let mut collected = Vec::new();
        assert!(drain_pipe(&mut data, &mut collected, 5).unwrap());
        assert_eq!(collected, b"hello");
        let mut data = std::io::Cursor::new(vec![0; 4096]);
        let mut collected = Vec::new();
        assert!(
            drain_pipe(&mut data, &mut collected, 1100)
                .unwrap_err()
                .contains("output exceeded")
        );
        assert!(collected.len() <= 1100);
    }

    #[test]
    fn drain_pipe_handles_would_block_and_read_failure() {
        struct Reader(std::io::ErrorKind);
        impl std::io::Read for Reader {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::new(self.0, "fake read result"))
            }
        }
        assert!(
            !drain_pipe(
                &mut Reader(std::io::ErrorKind::WouldBlock),
                &mut Vec::new(),
                8
            )
            .unwrap()
        );
        assert!(
            drain_pipe(
                &mut Reader(std::io::ErrorKind::BrokenPipe),
                &mut Vec::new(),
                8
            )
            .unwrap_err()
            .contains("fake read result")
        );
    }

    #[test]
    fn monitoring_times_out_for_a_running_process_or_unclosed_pipe() {
        use std::cell::Cell;
        struct OpenPipe;
        impl std::io::Read for OpenPipe {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::ErrorKind::WouldBlock.into())
            }
        }
        for exited in [false, true] {
            let clock = Cell::new(Duration::ZERO);
            let request = ProcessRequest {
                program: OSASCRIPT,
                args: vec![],
                timeout: PROCESS_TIMEOUT,
                output_limit: OUTPUT_LIMIT,
            };
            let error = collect_output(
                &mut OpenPipe,
                &mut std::io::empty(),
                &request,
                || Ok(exited.then(|| (true, "exit status: 0".into()))),
                || clock.get(),
                || clock.set(clock.get() + Duration::from_secs(1)),
                || false,
            )
            .err()
            .unwrap();
            assert!(error.contains("timed out after 5000 ms"));
            assert_eq!(clock.get(), PROCESS_TIMEOUT);
        }
    }

    #[test]
    fn monitoring_collects_both_pipes_and_returns_exit_status() {
        let request = ProcessRequest {
            program: OSASCRIPT,
            args: vec![],
            timeout: PROCESS_TIMEOUT,
            output_limit: OUTPUT_LIMIT,
        };
        let result = collect_output(
            &mut std::io::Cursor::new(b"result"),
            &mut std::io::Cursor::new(b"permission denied"),
            &request,
            || Ok(Some((false, "exit status: 1".into()))),
            || Duration::ZERO,
            || panic!("completed process should not pause"),
            || false,
        )
        .unwrap();
        assert!(!result.success);
        assert_eq!(result.status, "exit status: 1");
        assert_eq!(result.stdout, b"result");
        assert_eq!(result.stderr, b"permission denied");
    }

    #[test]
    fn repeated_interrupted_reads_return_control_to_deadline_checker() {
        struct Interrupted;
        impl std::io::Read for Interrupted {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::ErrorKind::Interrupted.into())
            }
        }
        assert!(!drain_pipe(&mut Interrupted, &mut Vec::new(), OUTPUT_LIMIT).unwrap());
    }

    #[test]
    fn dispatcher_can_move_to_an_action_worker() {
        fn assert_send<T: Send>() {}
        assert_send::<MacOsDispatcher>();
    }

    #[test]
    fn cancellation_prevents_process_start_without_real_actions() {
        let cancellation = Arc::new(AtomicBool::new(true));
        let mut dispatcher = MacOsDispatcher::with_cancellation(cancellation.clone());
        let error = dispatcher.run("/not/an/executable", vec![]).unwrap_err();
        assert!(error.contains("action execution cancelled"));
        let mut runner = SystemProcessRunner {
            cancellation: Some(cancellation),
        };
        let error = runner
            .run(&ProcessRequest {
                program: "/not/an/executable",
                args: vec![],
                timeout: PROCESS_TIMEOUT,
                output_limit: OUTPUT_LIMIT,
            })
            .err()
            .unwrap();
        assert!(error.contains("action execution cancelled"));
    }

    #[test]
    fn cancellation_between_query_and_setter_prevents_the_setter() {
        struct CancellingRunner {
            cancellation: Arc<AtomicBool>,
            requests: Arc<Mutex<Vec<ProcessRequest>>>,
        }
        impl ProcessRunner for CancellingRunner {
            fn run(&mut self, request: &ProcessRequest) -> Result<ProcessOutput, String> {
                self.requests.lock().unwrap().push(ProcessRequest {
                    program: request.program,
                    args: request.args.clone(),
                    timeout: request.timeout,
                    output_limit: request.output_limit,
                });
                self.cancellation.store(true, Ordering::Release);
                success("40")
            }

            fn cancellation(&self) -> Option<&AtomicBool> {
                Some(&self.cancellation)
            }
        }
        let requests = Arc::new(Mutex::new(Vec::new()));
        let mut dispatcher = MacOsDispatcher {
            runner: Box::new(CancellingRunner {
                cancellation: Arc::new(AtomicBool::new(false)),
                requests: requests.clone(),
            }),
        };
        assert!(
            dispatcher
                .dispatch(&Action::Volume { delta: 5 })
                .unwrap_err()
                .contains("action execution cancelled")
        );
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].args[1],
            "output volume of (get volume settings)"
        );
    }

    #[test]
    fn audio_snapshot_queries_both_fields_without_setters() {
        let (mut dispatcher, requests) = fake(vec![success("40\n"), success("true\n")]);
        assert_eq!(
            dispatcher.audio_snapshot().unwrap(),
            AudioSnapshot {
                output_volume: 40,
                output_muted: true,
            }
        );
        let requests = requests.lock().unwrap();
        assert_eq!(requests.requests.len(), 2);
        assert_eq!(
            requests.requests[0].args[1],
            "output volume of (get volume settings)"
        );
        assert_eq!(
            requests.requests[1].args[1],
            "output muted of (get volume settings)"
        );
    }

    #[test]
    fn restore_snapshot_validates_before_any_process_request() {
        let (mut dispatcher, requests) = fake(vec![]);
        for volume in [101, 255] {
            assert!(
                dispatcher
                    .restore_audio_snapshot(&AudioSnapshot {
                        output_volume: volume,
                        output_muted: false,
                    })
                    .is_err()
            );
        }
        assert!(requests.lock().unwrap().requests.is_empty());
    }

    #[test]
    fn restore_snapshot_sets_exact_volume_and_mute_independently() {
        let (mut dispatcher, requests) = fake(vec![
            Err("synthetic volume failure".into()),
            Err("synthetic mute failure".into()),
        ]);
        let error = dispatcher
            .restore_audio_snapshot(&AudioSnapshot {
                output_volume: 0,
                output_muted: true,
            })
            .unwrap_err();
        assert!(error.contains("restoring output volume failed: synthetic volume failure"));
        assert!(error.contains("restoring output mute failed: synthetic mute failure"));
        let requests = requests.lock().unwrap();
        assert_eq!(requests.requests.len(), 2);
        assert_eq!(requests.requests[0].args[1], "set volume output volume 0");
        assert_eq!(requests.requests[1].args[1], "set volume output muted true");
    }

    #[test]
    fn restore_snapshot_does_not_bypass_cancellation() {
        struct CancelledRunner(AtomicBool);
        impl ProcessRunner for CancelledRunner {
            fn run(&mut self, _: &ProcessRequest) -> Result<ProcessOutput, String> {
                panic!("cancelled restoration must not reach subprocess runner");
            }

            fn cancellation(&self) -> Option<&AtomicBool> {
                Some(&self.0)
            }
        }
        let mut dispatcher = MacOsDispatcher {
            runner: Box::new(CancelledRunner(AtomicBool::new(true))),
        };
        let error = dispatcher
            .restore_audio_snapshot(&AudioSnapshot {
                output_volume: 50,
                output_muted: false,
            })
            .unwrap_err();
        assert!(error.contains("restoring output volume failed"));
        assert!(error.contains("restoring output mute failed"));
        assert!(error.contains("action execution cancelled"));
    }

    #[test]
    fn monitoring_cancels_a_running_process_before_timeout() {
        use std::cell::Cell;
        struct OpenPipe;
        impl std::io::Read for OpenPipe {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::ErrorKind::WouldBlock.into())
            }
        }
        let cancellation = AtomicBool::new(false);
        let polls = Cell::new(0);
        let elapsed = Cell::new(Duration::ZERO);
        let error = collect_output(
            &mut OpenPipe,
            &mut std::io::empty(),
            &ProcessRequest {
                program: OSASCRIPT,
                args: vec![],
                timeout: PROCESS_TIMEOUT,
                output_limit: OUTPUT_LIMIT,
            },
            || {
                polls.set(polls.get() + 1);
                Ok(None)
            },
            || elapsed.get(),
            || {
                elapsed.set(Duration::from_millis(10));
                cancellation.store(true, Ordering::Release);
            },
            || cancellation.load(Ordering::Acquire),
        )
        .err()
        .unwrap();
        assert!(error.contains("action execution cancelled"));
        assert!(error.contains("action may already have taken effect"));
        assert_eq!(polls.get(), 1);
        assert_eq!(elapsed.get(), Duration::from_millis(10));
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn native_backend_reports_unsupported_platform() {
        let error = MacOsDispatcher::new()
            .dispatch(&Action::ToggleMute)
            .unwrap_err();
        assert!(error.contains("unsupported on this platform"));
    }
}
