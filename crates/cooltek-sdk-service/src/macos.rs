//! Explicit macOS actions through system tools. Constructing a dispatcher performs no actions.

use crate::{Action, ActionDispatcher, Modifier};
use std::time::Duration;

const OSASCRIPT: &str = "/usr/bin/osascript";
const OPEN: &str = "/usr/bin/open";
const PROCESS_TIMEOUT: Duration = Duration::from_secs(5);
const OUTPUT_LIMIT: usize = 16 * 1024;

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
            runner: Box::new(SystemProcessRunner),
        }
    }

    fn run(&mut self, program: &'static str, args: Vec<String>) -> Result<Vec<u8>, String> {
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
                let response = self.query("output volume")?;
                let current = response
                    .parse::<i16>()
                    .ok()
                    .filter(|value| (0..=100).contains(value))
                    .ok_or_else(|| {
                        format!("{OSASCRIPT}: invalid output volume response {response:?}")
                    })?;
                let target = (i32::from(current) + i32::from(*delta)).clamp(0, 100);
                self.apple_script(format!("set volume output volume {target}"))?;
            }
            Action::ToggleMute => {
                let response = self.query("output muted")?;
                let target = match response.as_str() {
                    "true" => "false",
                    "false" => "true",
                    _ => {
                        return Err(format!(
                            "{OSASCRIPT}: invalid output muted response {response:?}"
                        ));
                    }
                };
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

trait ProcessRunner {
    fn run(&mut self, request: &ProcessRequest) -> Result<ProcessOutput, String>;
}

struct SystemProcessRunner;

#[cfg(not(target_os = "macos"))]
impl ProcessRunner for SystemProcessRunner {
    fn run(&mut self, _request: &ProcessRequest) -> Result<ProcessOutput, String> {
        Err("macOS action execution is unsupported on this platform".into())
    }
}

#[cfg(target_os = "macos")]
impl ProcessRunner for SystemProcessRunner {
    fn run(&mut self, request: &ProcessRequest) -> Result<ProcessOutput, String> {
        use std::process::{Command, Stdio};
        let mut child = Command::new(request.program)
            .args(&request.args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("{}: could not start process: {error}", request.program))?;
        let result = collect_child_output(&mut child, request);
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
) -> Result<ProcessOutput, String> {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut stdout_closed = false;
    let mut stderr_closed = false;
    loop {
        if !stdout_closed {
            stdout_closed = drain_pipe(stdout_pipe, &mut stdout, request.output_limit)
                .map_err(|error| format!("{}: stdout: {error}", request.program))?;
        }
        if !stderr_closed {
            stderr_closed = drain_pipe(stderr_pipe, &mut stderr, request.output_limit)
                .map_err(|error| format!("{}: stderr: {error}", request.program))?;
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
    use std::{cell::RefCell, collections::VecDeque, rc::Rc};

    #[derive(Default)]
    struct FakeState {
        requests: Vec<ProcessRequest>,
        results: VecDeque<Result<ProcessOutput, String>>,
    }
    struct FakeRunner(Rc<RefCell<FakeState>>);
    impl ProcessRunner for FakeRunner {
        fn run(&mut self, request: &ProcessRequest) -> Result<ProcessOutput, String> {
            let mut state = self.0.borrow_mut();
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
    ) -> (MacOsDispatcher, Rc<RefCell<FakeState>>) {
        let state = Rc::new(RefCell::new(FakeState {
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
            let state = state.borrow();
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
            assert_eq!(state.borrow().requests.len(), 1);
        }
    }

    #[test]
    fn toggle_mute_reads_and_inverts_only_boolean_responses() {
        for (current, target) in [("true\n", "false"), ("false\n", "true")] {
            let (mut dispatcher, state) = fake(vec![success(current), success("")]);
            dispatcher.dispatch(&Action::ToggleMute).unwrap();
            let state = state.borrow();
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
        assert_eq!(state.borrow().requests.len(), 1);
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
        let state = state.borrow();
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
        let state = state.borrow();
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
        assert!(state.borrow().requests.is_empty());
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
        assert_eq!(state.borrow().requests.len(), 1);
        let (mut dispatcher, state) = fake(vec![Err("subprocess timed out after 5000 ms".into())]);
        assert_eq!(
            dispatcher.dispatch(&action).unwrap_err(),
            "subprocess timed out after 5000 ms"
        );
        assert_eq!(state.borrow().requests.len(), 1);
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
        assert_eq!(state.borrow().requests.len(), 1);
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

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn native_backend_reports_unsupported_platform() {
        let error = MacOsDispatcher::new()
            .dispatch(&Action::ToggleMute)
            .unwrap_err();
        assert!(error.contains("unsupported on this platform"));
    }
}
