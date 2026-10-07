//! Explicit macOS actions through system tools. Constructing a dispatcher performs no actions.

use crate::{Action, ActionDispatcher, MediaCommand, Modifier, SystemCommand};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

const OSASCRIPT: &str = "/usr/bin/osascript";
const OPEN: &str = "/usr/bin/open";
const PBCOPY: &str = "/usr/bin/pbcopy";
const PBPASTE: &str = "/usr/bin/pbpaste";
const SCREENCAPTURE: &str = "/usr/sbin/screencapture";
const PROCESS_TIMEOUT: Duration = Duration::from_secs(5);
const OUTPUT_LIMIT: usize = 16 * 1024;
/// Largest stdin payload. It is below the 16 KiB initial macOS pipe buffer, so
/// writing it before monitoring the child cannot block on a child that never reads.
const STDIN_LIMIT: usize = 8 * 1024;
/// Fixed delay between `multi_action` steps, matching the official app.
const STEP_SPACING: Duration = Duration::from_millis(280);
/// Delay before restoring the clipboard, so the target application can read
/// the pasted snippet first. A slower application can still paste the restored text.
const CLIPBOARD_RESTORE_DELAY: Duration = Duration::from_millis(500);
/// Pasteboard tools choose their text encoding from the locale; a bundled app
/// can start without one, so clipboard tools always run with UTF-8.
const UTF8_ENV: &[(&str, &str)] = &[("LANG", "en_US.UTF-8"), ("LC_ALL", "en_US.UTF-8")];
/// Upper bound on the wall-clock time of a whole `multi_action`. It is checked
/// before each step starts; a step that is already running is bounded only by
/// its own subprocess timeouts.
const MULTI_ACTION_DEADLINE: Duration = Duration::from_secs(15);
/// Clipboard type names that `clipboard info` reports for plain text only.
const PLAIN_TEXT_TYPES: &[&str] = &["«class utf8»", "«class ut16»", "string", "Unicode text"];

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
    sleeper: Box<dyn Sleeper>,
}

impl MacOsDispatcher {
    /// Creates a dispatcher without launching subprocesses or requesting permissions.
    pub fn new() -> Self {
        Self {
            runner: Box::new(SystemProcessRunner { cancellation: None }),
            sleeper: Box::new(SystemSleeper),
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
            sleeper: Box::new(SystemSleeper),
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
        self.run_request(ProcessRequest {
            program,
            args,
            ..ProcessRequest::default()
        })
    }

    fn run_request(&mut self, request: ProcessRequest) -> Result<Vec<u8>, String> {
        let program = request.program;
        check_cancellation(self.runner.cancellation(), program)?;
        if request
            .stdin
            .as_ref()
            .is_some_and(|input| input.len() > STDIN_LIMIT)
        {
            return Err(format!("{program}: stdin exceeds {STDIN_LIMIT} bytes"));
        }
        let output = self.runner.run(&ProcessRequest {
            timeout: PROCESS_TIMEOUT,
            output_limit: OUTPUT_LIMIT,
            ..request
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

    fn pause(&mut self, duration: Duration) -> Result<(), String> {
        self.sleeper.sleep(duration, self.runner.cancellation())
    }

    fn set_clipboard(&mut self, text: Vec<u8>) -> Result<(), String> {
        self.run_request(ProcessRequest {
            program: PBCOPY,
            stdin: Some(text),
            env: UTF8_ENV,
            ..ProcessRequest::default()
        })
        .map(drop)
    }

    /// Returns the current clipboard text when it is small, plain text only.
    ///
    /// Any failure other than cancellation means "do not restore": the snippet
    /// is still pasted, and the clipboard keeps it afterwards.
    fn plain_text_clipboard(&mut self) -> Result<Option<Vec<u8>>, String> {
        let info = match self.run_request(ProcessRequest {
            program: OSASCRIPT,
            args: vec!["-e".into(), "clipboard info".into()],
            env: UTF8_ENV,
            ..ProcessRequest::default()
        }) {
            Ok(info) => info,
            Err(error) => {
                check_cancellation(self.runner.cancellation(), OSASCRIPT).map_err(|_| error)?;
                return Ok(None);
            }
        };
        if has_sensitive_clipboard_marker(&info) || !is_plain_text_clipboard(&info) {
            return Ok(None);
        }
        match self.run_request(ProcessRequest {
            program: PBPASTE,
            args: vec!["-Prefer".into(), "txt".into()],
            env: UTF8_ENV,
            ..ProcessRequest::default()
        }) {
            Ok(text)
                if !text.is_empty()
                    && text.len() <= STDIN_LIMIT
                    && std::str::from_utf8(&text).is_ok() =>
            {
                Ok(Some(text))
            }
            Ok(_) => Ok(None),
            Err(error) => {
                check_cancellation(self.runner.cancellation(), PBPASTE).map_err(|_| error)?;
                Ok(None)
            }
        }
    }

    /// Restores `saved` only while the clipboard still holds exactly `snippet`,
    /// so anything copied after the snippet was placed is never overwritten.
    ///
    /// A failed re-read (other than cancellation) skips the restore: the
    /// clipboard contents cannot be confirmed, so they are left alone.
    fn restore_clipboard_if_unchanged(
        &mut self,
        snippet: &[u8],
        saved: Vec<u8>,
    ) -> Result<(), String> {
        match self.run_request(ProcessRequest {
            program: PBPASTE,
            args: vec!["-Prefer".into(), "txt".into()],
            env: UTF8_ENV,
            ..ProcessRequest::default()
        }) {
            Ok(current) if current == snippet => self.set_clipboard(saved),
            Ok(_) => Ok(()),
            Err(error) => {
                check_cancellation(self.runner.cancellation(), PBPASTE).map_err(|_| error)
            }
        }
    }

    fn paste_text(&mut self, text: &str, press_enter: bool) -> Result<(), String> {
        let saved = self.plain_text_clipboard()?;
        self.set_clipboard(text.as_bytes().to_vec())?;
        // Fixed script lines only; the snippet travels through pbcopy's stdin.
        let mut lines = vec![
            "tell application \"System Events\"",
            "keystroke \"v\" using {command down}",
        ];
        if press_enter {
            lines.push("key code 36");
        }
        lines.push("end tell");
        let pasted = self.run(
            OSASCRIPT,
            lines
                .into_iter()
                .flat_map(|line| ["-e".to_string(), line.to_string()])
                .collect(),
        );
        let Some(saved) = saved else {
            return pasted.map(drop);
        };
        if let Err(error) = pasted {
            // Nothing pasted, so restore immediately; report both outcomes.
            return Err(
                match self.restore_clipboard_if_unchanged(text.as_bytes(), saved) {
                    Ok(()) => error,
                    Err(restore) => {
                        format!("{error}; restoring the previous clipboard text failed: {restore}")
                    }
                },
            );
        }
        self.pause(CLIPBOARD_RESTORE_DELAY)
            .and_then(|()| self.restore_clipboard_if_unchanged(text.as_bytes(), saved))
            .map_err(|error| {
                format!(
                    "text was pasted, but restoring the previous clipboard text failed: {error}"
                )
            })
    }

    fn execute(&mut self, action: &Action) -> Result<(), String> {
        match action {
            Action::Volume { delta } => {
                let current = self.output_volume()?;
                let target = (i32::from(current) + i32::from(*delta)).clamp(0, 100);
                self.apple_script(format!("set volume output volume {target}"))?;
            }
            Action::SetVolume { percent } => {
                self.apple_script(format!("set volume output volume {percent}"))?;
            }
            Action::VolumeFromPosition => {
                unreachable!("unresolved templates are rejected before host operations")
            }
            Action::ToggleMute => {
                let target = !self.output_muted()?;
                self.apple_script(format!("set volume output muted {target}"))?;
            }
            Action::Shortcut { key, modifiers } => {
                self.apple_script(keystroke_script(key, modifiers))?;
            }
            Action::LaunchApp { bundle_id } => {
                self.run(OPEN, vec!["-b".into(), bundle_id.clone()])?;
            }
            Action::OpenUrl { url } => {
                // The validated URL starts with http:// or https://, so it cannot be
                // read as an option; -u forces URL handling even if it matches a path.
                self.run(OPEN, vec!["-u".into(), url.clone()])?;
            }
            Action::Media { command } => {
                let name = match command {
                    MediaCommand::PlayPause => "play_pause",
                    MediaCommand::NextTrack => "next_track",
                    MediaCommand::PreviousTrack => "previous_track",
                    MediaCommand::Stop => "stop",
                };
                return Err(format!(
                    "media {name} is unsupported on the macOS backend: System Events cannot post hardware media keys; no action was taken"
                ));
            }
            Action::System { command } => match command {
                SystemCommand::Undo => {
                    self.apple_script(keystroke_script("z", &[Modifier::Command]))?;
                }
                SystemCommand::Redo => {
                    self.apple_script(keystroke_script(
                        "z",
                        &[Modifier::Command, Modifier::Shift],
                    ))?;
                }
                SystemCommand::Screenshot => {
                    self.run(SCREENCAPTURE, vec!["-c".into()])?;
                }
            },
            Action::Text { text, press_enter } => self.paste_text(text, *press_enter)?,
            Action::MultiAction { actions } => {
                let count = actions.len();
                let deadline = self.sleeper.now() + MULTI_ACTION_DEADLINE;
                for (index, step) in actions.iter().enumerate() {
                    if index > 0 {
                        self.pause(STEP_SPACING).map_err(|error| {
                            format!(
                                "multi_action stopped before step {} of {count}: {error}",
                                index + 1
                            )
                        })?;
                    }
                    if self.sleeper.now() >= deadline {
                        return Err(format!(
                            "multi_action stopped before step {} of {count} ({}): the sequence exceeded its {} s overall deadline; earlier steps already took effect",
                            index + 1,
                            step.type_name(),
                            MULTI_ACTION_DEADLINE.as_secs()
                        ));
                    }
                    self.execute(step).map_err(|error| {
                        format!(
                            "multi_action step {} of {count} ({}) failed: {error}",
                            index + 1,
                            step.type_name()
                        )
                    })?;
                }
            }
        }
        Ok(())
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
        // Validate even when the caller did not use the profile loader. This
        // also validates every multi_action step before the first one runs.
        action.validate_executable()?;
        self.execute(action)
    }
}

/// Builds a System Events keystroke from a validated key. A literal key is one
/// ASCII alphanumeric character or space and cannot introduce AppleScript
/// syntax; a named key is sent as an integer from the closed key-code table.
fn keystroke_script(key: &str, modifiers: &[Modifier]) -> String {
    let mut script = match crate::profile::named_key_code(key) {
        Some(code) => format!("tell application \"System Events\" to key code {code}"),
        None => {
            debug_assert!(
                key.len() == 1 && (key.as_bytes()[0].is_ascii_alphanumeric() || key == " ")
            );
            format!("tell application \"System Events\" to keystroke \"{key}\"")
        }
    };
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
    script
}

/// Accepts `clipboard info` output listing only plain-text types, such as
/// `«class utf8», 5, «class ut16», 12, string, 5, Unicode text, 10`.
fn is_plain_text_clipboard(info: &[u8]) -> bool {
    let Ok(info) = std::str::from_utf8(info) else {
        return false;
    };
    let items: Vec<_> = info.trim().split(", ").collect();
    !info.trim().is_empty()
        && items.len() % 2 == 0
        && items.chunks(2).all(|pair| {
            PLAIN_TEXT_TYPES.contains(&pair[0])
                && pair[1]
                    .parse::<usize>()
                    .is_ok_and(|size| size <= 2 * STDIN_LIMIT)
        })
}

/// Rejects a clipboard whose `clipboard info` names any pasteboard-marker type,
/// such as `org.nspasteboard.ConcealedType` or `TransientType`, regardless of
/// whether every other listed type is plain text. Whether AppleScript lists
/// these markers at all is unverified, so this is a best-effort extra check.
fn has_sensitive_clipboard_marker(info: &[u8]) -> bool {
    let info = String::from_utf8_lossy(info).to_lowercase();
    ["nspasteboard", "concealed", "transient"]
        .iter()
        .any(|marker| info.contains(marker))
}

/// Waits between steps and reads the time; injected so tests never sleep.
trait Sleeper: Send {
    /// Monotonic time used for the `multi_action` deadline.
    fn now(&self) -> std::time::Instant {
        std::time::Instant::now()
    }

    fn sleep(
        &mut self,
        duration: Duration,
        cancellation: Option<&AtomicBool>,
    ) -> Result<(), String>;
}

struct SystemSleeper;

impl Sleeper for SystemSleeper {
    fn sleep(
        &mut self,
        duration: Duration,
        cancellation: Option<&AtomicBool>,
    ) -> Result<(), String> {
        let deadline = std::time::Instant::now() + duration;
        loop {
            check_cancellation(cancellation, "action pause")?;
            let now = std::time::Instant::now();
            if now >= deadline {
                return Ok(());
            }
            std::thread::sleep((deadline - now).min(Duration::from_millis(10)));
        }
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
struct ProcessRequest {
    program: &'static str,
    args: Vec<String>,
    /// Bytes written to stdin before monitoring; `None` connects /dev/null.
    stdin: Option<Vec<u8>>,
    /// Fixed variables added to the inherited environment.
    env: &'static [(&'static str, &'static str)],
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
            .envs(request.env.iter().copied())
            .stdin(if request.stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("{}: could not start process: {error}", request.program))?;
        let result = write_child_stdin(&mut child, request)
            .and_then(|()| collect_child_output(&mut child, request, self.cancellation()));
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
fn write_child_stdin(
    child: &mut std::process::Child,
    request: &ProcessRequest,
) -> Result<(), String> {
    use std::io::Write;
    let Some(input) = &request.stdin else {
        return Ok(());
    };
    // Callers bound input below the pipe buffer, so this write cannot block.
    // Dropping the pipe afterwards signals end of input.
    let mut pipe = child
        .stdin
        .take()
        .ok_or_else(|| format!("{}: missing subprocess stdin pipe", request.program))?;
    pipe.write_all(input)
        .map_err(|error| format!("{}: could not write stdin: {error}", request.program))
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
        /// Pauses, recorded after the number of requests already started.
        pauses: Vec<(usize, Duration)>,
        /// Virtual time since the dispatcher was built; pauses advance it.
        elapsed: Duration,
        /// Virtual time each subprocess takes.
        request_cost: Duration,
    }
    struct FakeSleeper(Arc<Mutex<FakeState>>, std::time::Instant);
    impl Sleeper for FakeSleeper {
        fn sleep(&mut self, duration: Duration, _: Option<&AtomicBool>) -> Result<(), String> {
            let mut state = self.0.lock().unwrap();
            let started = state.requests.len();
            state.pauses.push((started, duration));
            state.elapsed += duration;
            Ok(())
        }

        fn now(&self) -> std::time::Instant {
            self.1 + self.0.lock().unwrap().elapsed
        }
    }
    struct FakeRunner(Arc<Mutex<FakeState>>);
    impl ProcessRunner for FakeRunner {
        fn run(&mut self, request: &ProcessRequest) -> Result<ProcessOutput, String> {
            let mut state = self.0.lock().unwrap();
            state.requests.push(ProcessRequest {
                program: request.program,
                args: request.args.clone(),
                stdin: request.stdin.clone(),
                env: request.env,
                timeout: request.timeout,
                output_limit: request.output_limit,
            });
            let cost = state.request_cost;
            state.elapsed += cost;
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
            results: results.into(),
            ..FakeState::default()
        }));
        let dispatcher = MacOsDispatcher {
            runner: Box::new(FakeRunner(state.clone())),
            sleeper: Box::new(FakeSleeper(state.clone(), std::time::Instant::now())),
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
    fn absolute_volume_sets_the_resolved_value_without_query_or_delta() {
        for percent in [0, 1, 50, 100] {
            let (mut dispatcher, state) = fake(vec![success("")]);
            dispatcher.dispatch(&Action::SetVolume { percent }).unwrap();
            let state = state.lock().unwrap();
            assert_eq!(state.requests.len(), 1);
            assert_eq!(state.requests[0].program, OSASCRIPT);
            assert_eq!(
                state.requests[0].args,
                [
                    "-e".to_string(),
                    format!("set volume output volume {percent}")
                ]
            );
        }
    }

    #[test]
    fn unresolved_fader_template_or_invalid_absolute_volume_never_starts_host_tools() {
        let (mut dispatcher, state) = fake(vec![]);
        for action in [
            Action::VolumeFromPosition,
            Action::SetVolume { percent: 101 },
        ] {
            assert!(dispatcher.dispatch(&action).is_err());
        }
        assert!(state.lock().unwrap().requests.is_empty());
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
                ..ProcessRequest::default()
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
            ..ProcessRequest::default()
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
                ..ProcessRequest::default()
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
                    stdin: request.stdin.clone(),
                    env: request.env,
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
            sleeper: Box::new(SystemSleeper),
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
            sleeper: Box::new(SystemSleeper),
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
                ..ProcessRequest::default()
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

    fn failure(stderr: &str) -> Result<ProcessOutput, String> {
        Ok(ProcessOutput {
            success: false,
            status: "exit status: 1".into(),
            stdout: Vec::new(),
            stderr: stderr.as_bytes().to_vec(),
        })
    }

    fn args(request: &ProcessRequest) -> Vec<&str> {
        request.args.iter().map(String::as_str).collect()
    }

    const PLAIN_INFO: &str = "«class utf8», 3, «class ut16», 8, string, 3, Unicode text, 6\n";
    const PASTE_ARGS: [&str; 6] = [
        "-e",
        "tell application \"System Events\"",
        "-e",
        "keystroke \"v\" using {command down}",
        "-e",
        "end tell",
    ];

    #[test]
    fn open_url_passes_the_url_as_one_argument_to_open() {
        let url = "https://example.com/a?b=1&c=it's#frag";
        let (mut dispatcher, state) = fake(vec![success("")]);
        dispatcher
            .dispatch(&Action::OpenUrl { url: url.into() })
            .unwrap();
        let state = state.lock().unwrap();
        assert_eq!(state.requests.len(), 1);
        assert_eq!(state.requests[0].program, OPEN);
        assert_eq!(args(&state.requests[0]), ["-u", url]);
        assert_eq!(state.requests[0].stdin, None);
        assert_eq!(state.requests[0].timeout, PROCESS_TIMEOUT);
    }

    #[test]
    fn named_keys_use_closed_integer_key_codes() {
        let cases = [
            (
                "arrow_left",
                vec![Modifier::Command, Modifier::Shift],
                "tell application \"System Events\" to key code 123 using {command down, shift down}",
            ),
            (
                "f20",
                vec![],
                "tell application \"System Events\" to key code 90",
            ),
            (
                "return",
                vec![Modifier::Option],
                "tell application \"System Events\" to key code 36 using {option down}",
            ),
            (
                "space",
                vec![Modifier::Control],
                "tell application \"System Events\" to key code 49 using {control down}",
            ),
            (
                "page_down",
                vec![],
                "tell application \"System Events\" to key code 121",
            ),
        ];
        let (mut dispatcher, state) = fake(cases.iter().map(|_| success("")).collect());
        for (key, modifiers, _) in &cases {
            dispatcher
                .dispatch(&Action::Shortcut {
                    key: (*key).into(),
                    modifiers: modifiers.clone(),
                })
                .unwrap();
        }
        let state = state.lock().unwrap();
        for (request, (_, _, script)) in state.requests.iter().zip(&cases) {
            assert_eq!(request.program, OSASCRIPT);
            assert_eq!(args(request), ["-e", script]);
        }
    }

    #[test]
    fn media_commands_are_plan_only_and_never_start_processes() {
        let (mut dispatcher, state) = fake(vec![]);
        for command in [
            MediaCommand::PlayPause,
            MediaCommand::NextTrack,
            MediaCommand::PreviousTrack,
            MediaCommand::Stop,
        ] {
            let error = dispatcher.dispatch(&Action::Media { command }).unwrap_err();
            assert!(
                error.contains("unsupported on the macOS backend"),
                "{error}"
            );
            assert!(error.contains("no action was taken"));
        }
        assert!(state.lock().unwrap().requests.is_empty());
    }

    #[test]
    fn system_commands_use_fixed_keystrokes_and_screencapture() {
        let (mut dispatcher, state) = fake(vec![success(""), success(""), success("")]);
        for command in [
            SystemCommand::Undo,
            SystemCommand::Redo,
            SystemCommand::Screenshot,
        ] {
            dispatcher.dispatch(&Action::System { command }).unwrap();
        }
        let state = state.lock().unwrap();
        assert_eq!(
            args(&state.requests[0]),
            [
                "-e",
                "tell application \"System Events\" to keystroke \"z\" using {command down}"
            ]
        );
        assert_eq!(
            args(&state.requests[1]),
            [
                "-e",
                "tell application \"System Events\" to keystroke \"z\" using {command down, shift down}"
            ]
        );
        assert_eq!(state.requests[2].program, SCREENCAPTURE);
        assert_eq!(args(&state.requests[2]), ["-c"]);
    }

    #[test]
    fn text_pastes_through_stdin_and_restores_plain_clipboard_text() {
        for press_enter in [false, true] {
            let (mut dispatcher, state) = fake(vec![
                success(PLAIN_INFO),
                success("old"),
                success(""),
                success(""),
                success("zażółć\nnext"),
                success(""),
            ]);
            dispatcher
                .dispatch(&Action::Text {
                    text: "zażółć\nnext".into(),
                    press_enter,
                })
                .unwrap();
            let state = state.lock().unwrap();
            let requests = &state.requests;
            assert_eq!(requests.len(), 6);
            assert_eq!(requests[0].program, OSASCRIPT);
            assert_eq!(args(&requests[0]), ["-e", "clipboard info"]);
            assert_eq!(requests[1].program, PBPASTE);
            assert_eq!(args(&requests[1]), ["-Prefer", "txt"]);
            assert_eq!(requests[2].program, PBCOPY);
            assert!(requests[2].args.is_empty());
            assert_eq!(
                requests[2].stdin.as_deref(),
                Some("zażółć\nnext".as_bytes())
            );
            assert_eq!(requests[3].program, OSASCRIPT);
            let mut expected = PASTE_ARGS.to_vec();
            if press_enter {
                expected.splice(4..4, ["-e", "key code 36"]);
            }
            assert_eq!(args(&requests[3]), expected);
            assert_eq!(requests[3].stdin, None);
            // The clipboard is re-read after the wait and restored only while
            // it still holds the snippet.
            assert_eq!(requests[4].program, PBPASTE);
            assert_eq!(args(&requests[4]), ["-Prefer", "txt"]);
            assert_eq!(requests[5].program, PBCOPY);
            assert_eq!(requests[5].stdin.as_deref(), Some(b"old".as_slice()));
            for index in [0, 1, 2, 4, 5] {
                assert_eq!(requests[index].env, UTF8_ENV);
            }
            assert_eq!(state.pauses, [(4, CLIPBOARD_RESTORE_DELAY)]);
        }
    }

    #[test]
    fn text_with_script_syntax_never_reaches_osascript_arguments() {
        let hostile = "\" & (do shell script \"touch /tmp/pwned\") & \"\\\nend tell";
        let (mut dispatcher, state) = fake(vec![
            success("«class PNGf», 2048, «class utf8», 3\n"),
            success(""),
            success(""),
        ]);
        dispatcher
            .dispatch(&Action::Text {
                text: hostile.into(),
                press_enter: false,
            })
            .unwrap();
        let state = state.lock().unwrap();
        // Rich or non-text clipboard contents are neither read nor restored.
        assert_eq!(state.requests.len(), 3);
        assert_eq!(state.requests[1].program, PBCOPY);
        assert_eq!(state.requests[1].stdin.as_deref(), Some(hostile.as_bytes()));
        assert_eq!(args(&state.requests[2]), PASTE_ARGS);
        for request in &state.requests {
            assert!(request.args.iter().all(|arg| !arg.contains("pwned")));
        }
        assert!(state.pauses.is_empty());
    }

    #[test]
    fn text_skips_restore_for_unreadable_empty_or_large_clipboards() {
        let large = "x".repeat(STDIN_LIMIT + 1);
        for snapshot in [
            vec![Err("synthetic clipboard info failure".to_string())],
            vec![success("")],
            vec![success("«class utf8», 3, «class RTF », 90")],
            vec![success(PLAIN_INFO), success("")],
            vec![success(PLAIN_INFO), success(&large)],
            vec![success(PLAIN_INFO), failure("pbpaste failed")],
        ] {
            let snapshot_requests = snapshot.len();
            let mut results = snapshot;
            results.extend([success(""), success("")]);
            let (mut dispatcher, state) = fake(results);
            dispatcher
                .dispatch(&Action::Text {
                    text: "x".into(),
                    press_enter: false,
                })
                .unwrap();
            let state = state.lock().unwrap();
            assert_eq!(state.requests.len(), snapshot_requests + 2);
            assert_eq!(state.requests.last().unwrap().program, OSASCRIPT);
            assert!(state.pauses.is_empty());
        }
    }

    #[test]
    fn failed_paste_restores_immediately_and_reports_the_failure() {
        let (mut dispatcher, state) = fake(vec![
            success(PLAIN_INFO),
            success("old"),
            success(""),
            failure("not allowed assistive access (-1719)"),
            success("x"),
            success(""),
        ]);
        let error = dispatcher
            .dispatch(&Action::Text {
                text: "x".into(),
                press_enter: true,
            })
            .unwrap_err();
        assert!(error.contains("-1719"));
        let state = state.lock().unwrap();
        assert_eq!(state.requests.len(), 6);
        assert_eq!(state.requests[4].program, PBPASTE);
        assert_eq!(state.requests[5].stdin.as_deref(), Some(b"old".as_slice()));
        assert!(state.pauses.is_empty());

        let (mut dispatcher, _) = fake(vec![
            success(PLAIN_INFO),
            success("old"),
            success(""),
            success(""),
            success("x"),
            failure("pbcopy failed"),
        ]);
        let error = dispatcher
            .dispatch(&Action::Text {
                text: "x".into(),
                press_enter: false,
            })
            .unwrap_err();
        assert!(error.starts_with("text was pasted, but restoring"));
    }

    #[test]
    fn plain_text_clipboard_detection_is_strict() {
        assert!(is_plain_text_clipboard(PLAIN_INFO.as_bytes()));
        assert!(is_plain_text_clipboard(b"string, 4"));
        for info in [
            "".as_bytes(),
            b"\n",
            b"\xff\xfe",
            "«class utf8», 3, «class RTF », 80".as_bytes(),
            "«class utf8»".as_bytes(),
            "«class utf8», x".as_bytes(),
            "«class utf8», 99999999".as_bytes(),
            b"org.nspasteboard.ConcealedType, 0, string, 8",
            b"{string, 4}",
        ] {
            assert!(!is_plain_text_clipboard(info), "{info:?}");
        }
    }

    #[test]
    fn text_never_overwrites_a_clipboard_changed_during_the_wait() {
        for current in [
            success("copied meanwhile"),
            success(""),
            // A prefix or a trailing newline is not byte-for-byte equal.
            success("snippe"),
            success("snippet\n"),
            failure("pbpaste failed"),
        ] {
            let (mut dispatcher, state) = fake(vec![
                success(PLAIN_INFO),
                success("old"),
                success(""),
                success(""),
                current,
            ]);
            dispatcher
                .dispatch(&Action::Text {
                    text: "snippet".into(),
                    press_enter: false,
                })
                .unwrap();
            let state = state.lock().unwrap();
            assert_eq!(state.requests.len(), 5);
            assert_eq!(state.requests[4].program, PBPASTE);
            assert!(
                state
                    .requests
                    .iter()
                    .all(|request| request.stdin.as_deref() != Some(b"old".as_slice()))
            );
            assert_eq!(state.pauses, [(4, CLIPBOARD_RESTORE_DELAY)]);
        }

        // Same rule for the immediate restore after a failed paste.
        let (mut dispatcher, state) = fake(vec![
            success(PLAIN_INFO),
            success("old"),
            success(""),
            failure("paste failed"),
            success("copied meanwhile"),
        ]);
        let error = dispatcher
            .dispatch(&Action::Text {
                text: "snippet".into(),
                press_enter: false,
            })
            .unwrap_err();
        assert!(error.contains("paste failed"));
        assert_eq!(state.lock().unwrap().requests.len(), 5);
    }

    #[test]
    fn text_skips_saving_concealed_or_transient_clipboards() {
        for info in [
            "org.nspasteboard.ConcealedType, 0, «class utf8», 3",
            "«class utf8», 3, org.nspasteboard.TransientType, 0",
            "«class utf8», 3, ORG.NSPASTEBOARD.AutoGeneratedType, 0",
            "«class utf8», 3, «class Conc», 0, concealed, 0",
            "«class utf8», 3, Transient, 0",
        ] {
            assert!(has_sensitive_clipboard_marker(info.as_bytes()), "{info}");
            let (mut dispatcher, state) = fake(vec![success(info), success(""), success("")]);
            dispatcher
                .dispatch(&Action::Text {
                    text: "x".into(),
                    press_enter: false,
                })
                .unwrap();
            let state = state.lock().unwrap();
            // Never read with pbpaste, so nothing is restored and there is no wait.
            assert_eq!(state.requests.len(), 3);
            assert!(state.requests.iter().all(|r| r.program != PBPASTE));
            assert_eq!(state.requests[1].program, PBCOPY);
            assert!(state.pauses.is_empty());
        }
        assert!(!has_sensitive_clipboard_marker(PLAIN_INFO.as_bytes()));
    }

    #[test]
    fn multi_action_stops_when_the_overall_deadline_passes() {
        let (mut dispatcher, state) = fake(vec![success(""), success(""), success("")]);
        // Each subprocess "takes" 7 s of virtual time; no real time passes.
        state.lock().unwrap().request_cost = Duration::from_secs(7);
        let error = dispatcher
            .dispatch(&Action::MultiAction {
                actions: vec![
                    Action::SetVolume { percent: 10 },
                    Action::SetVolume { percent: 20 },
                    Action::OpenUrl {
                        url: "https://example.com".into(),
                    },
                    Action::SetVolume { percent: 40 },
                ],
            })
            .unwrap_err();
        // 7 s + 0.28 s + 7 s + 0.28 s = 14.56 s, so step 3 starts; at 21.84 s
        // step 4 is refused.
        assert!(
            error.starts_with("multi_action stopped before step 4 of 4 (set_volume)"),
            "{error}"
        );
        assert!(error.contains("15 s overall deadline"));
        let state = state.lock().unwrap();
        assert_eq!(state.requests.len(), 3);
        assert_eq!(state.requests[2].program, OPEN);
        assert_eq!(MULTI_ACTION_DEADLINE, Duration::from_secs(15));
    }

    #[test]
    fn multi_action_deadline_is_checked_after_the_last_pause_before_a_step() {
        let (mut dispatcher, state) = fake(vec![success("")]);
        // The first step ends just under the deadline; the pause crosses it.
        state.lock().unwrap().request_cost = MULTI_ACTION_DEADLINE - Duration::from_millis(1);
        let error = dispatcher
            .dispatch(&Action::MultiAction {
                actions: vec![
                    Action::SetVolume { percent: 10 },
                    Action::SetVolume { percent: 20 },
                ],
            })
            .unwrap_err();
        assert!(error.starts_with("multi_action stopped before step 2 of 2"));
        let state = state.lock().unwrap();
        assert_eq!(state.requests.len(), 1);
        assert_eq!(state.pauses, [(1, STEP_SPACING)]);
    }

    #[test]
    fn oversize_stdin_is_rejected_before_any_process() {
        let (mut dispatcher, state) = fake(vec![]);
        let error = dispatcher
            .set_clipboard(vec![b'x'; STDIN_LIMIT + 1])
            .unwrap_err();
        assert!(error.contains("stdin exceeds"));
        assert!(state.lock().unwrap().requests.is_empty());
        const { assert!(Action::MAX_TEXT_BYTES <= STDIN_LIMIT) };
    }

    #[test]
    fn invalid_new_actions_never_start_subprocesses() {
        let (mut dispatcher, state) = fake(vec![]);
        for action in [
            Action::OpenUrl {
                url: "file:///etc/passwd".into(),
            },
            Action::OpenUrl {
                url: "https://example.com/\"; do shell script \"x\"".into(),
            },
            Action::Shortcut {
                key: "key code 36".into(),
                modifiers: vec![],
            },
            Action::Text {
                text: "a\u{1b}b".into(),
                press_enter: false,
            },
            Action::Text {
                text: "x".repeat(Action::MAX_TEXT_BYTES + 1),
                press_enter: false,
            },
            Action::MultiAction {
                actions: vec![Action::ToggleMute],
            },
            Action::MultiAction {
                actions: vec![
                    Action::ToggleMute,
                    Action::MultiAction {
                        actions: vec![Action::ToggleMute, Action::ToggleMute],
                    },
                ],
            },
            // An invalid later step blocks the whole sequence before step 1 runs.
            Action::MultiAction {
                actions: vec![
                    Action::SetVolume { percent: 10 },
                    Action::OpenUrl {
                        url: "javascript:alert(1)".into(),
                    },
                ],
            },
        ] {
            assert!(dispatcher.dispatch(&action).is_err(), "{action:?}");
        }
        assert!(state.lock().unwrap().requests.is_empty());
        assert!(state.lock().unwrap().pauses.is_empty());
    }

    #[test]
    fn multi_action_runs_steps_in_order_with_fixed_spacing() {
        let (mut dispatcher, state) = fake(vec![
            success(""),
            success(""),
            success("true\n"),
            success(""),
        ]);
        dispatcher
            .dispatch(&Action::MultiAction {
                actions: vec![
                    Action::OpenUrl {
                        url: "https://example.com".into(),
                    },
                    Action::Shortcut {
                        key: "tab".into(),
                        modifiers: vec![Modifier::Command],
                    },
                    Action::ToggleMute,
                ],
            })
            .unwrap();
        let state = state.lock().unwrap();
        assert_eq!(state.requests.len(), 4);
        assert_eq!(state.requests[0].program, OPEN);
        assert_eq!(
            args(&state.requests[1]),
            [
                "-e",
                "tell application \"System Events\" to key code 48 using {command down}"
            ]
        );
        assert_eq!(
            args(&state.requests[3]),
            ["-e", "set volume output muted false"]
        );
        // Spacing runs between steps only, never before the first or after the last.
        assert_eq!(state.pauses, [(1, STEP_SPACING), (2, STEP_SPACING)]);
        assert_eq!(STEP_SPACING, Duration::from_millis(280));
    }

    #[test]
    fn multi_action_stops_at_the_first_failed_step() {
        let (mut dispatcher, state) = fake(vec![success(""), failure("denied")]);
        let error = dispatcher
            .dispatch(&Action::MultiAction {
                actions: vec![
                    Action::SetVolume { percent: 20 },
                    Action::System {
                        command: SystemCommand::Undo,
                    },
                    Action::SetVolume { percent: 30 },
                ],
            })
            .unwrap_err();
        assert!(error.starts_with("multi_action step 2 of 3 (system) failed:"));
        assert!(error.contains("denied"));
        let state = state.lock().unwrap();
        assert_eq!(state.requests.len(), 2);
        assert_eq!(state.pauses.len(), 1);

        let (mut dispatcher, state) = fake(vec![success("")]);
        let error = dispatcher
            .dispatch(&Action::MultiAction {
                actions: vec![
                    Action::SetVolume { percent: 20 },
                    Action::Media {
                        command: MediaCommand::PlayPause,
                    },
                    Action::SetVolume { percent: 30 },
                ],
            })
            .unwrap_err();
        assert!(error.starts_with("multi_action step 2 of 3 (media) failed:"));
        assert!(error.contains("unsupported on the macOS backend"));
        assert_eq!(state.lock().unwrap().requests.len(), 1);
    }

    #[test]
    fn multi_action_cancellation_during_spacing_stops_remaining_steps() {
        struct FlagRunner {
            cancellation: Arc<AtomicBool>,
            requests: Arc<Mutex<Vec<ProcessRequest>>>,
        }
        impl ProcessRunner for FlagRunner {
            fn run(&mut self, request: &ProcessRequest) -> Result<ProcessOutput, String> {
                self.requests.lock().unwrap().push(ProcessRequest {
                    program: request.program,
                    args: request.args.clone(),
                    ..ProcessRequest::default()
                });
                success("")
            }

            fn cancellation(&self) -> Option<&AtomicBool> {
                Some(&self.cancellation)
            }
        }
        struct CancellingSleeper(Arc<AtomicBool>);
        impl Sleeper for CancellingSleeper {
            fn sleep(
                &mut self,
                _: Duration,
                cancellation: Option<&AtomicBool>,
            ) -> Result<(), String> {
                self.0.store(true, Ordering::Release);
                check_cancellation(cancellation, "action pause")
            }
        }
        let cancellation = Arc::new(AtomicBool::new(false));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let mut dispatcher = MacOsDispatcher {
            runner: Box::new(FlagRunner {
                cancellation: cancellation.clone(),
                requests: requests.clone(),
            }),
            sleeper: Box::new(CancellingSleeper(cancellation)),
        };
        let error = dispatcher
            .dispatch(&Action::MultiAction {
                actions: vec![
                    Action::SetVolume { percent: 20 },
                    Action::SetVolume { percent: 30 },
                ],
            })
            .unwrap_err();
        assert!(error.starts_with("multi_action stopped before step 2 of 2"));
        assert!(error.contains("action execution cancelled"));
        assert_eq!(requests.lock().unwrap().len(), 1);
    }

    #[test]
    fn system_sleeper_observes_cancellation_without_waiting() {
        let mut sleeper = SystemSleeper;
        assert!(sleeper.sleep(Duration::ZERO, None).is_ok());
        let cancelled = AtomicBool::new(true);
        assert!(
            sleeper
                .sleep(Duration::from_secs(60), Some(&cancelled))
                .unwrap_err()
                .contains("action execution cancelled")
        );
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
