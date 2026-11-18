//! Bounded subprocesses and pauses for host actions, shared by every platform's host.
//!
//! Every system tool a host action starts runs through [`Process`]: within a
//! timeout and an output limit, with stdin bounded, and stopped (its whole
//! process group when isolated) on timeout or cancellation. Pauses between
//! steps go through the same context, so tests inject a fake runner and clock
//! and never start a process or sleep.

use std::{
    collections::BTreeSet,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

pub(crate) const PROCESS_TIMEOUT: Duration = Duration::from_secs(5);
pub(crate) const OUTPUT_LIMIT: usize = 16 * 1024;
/// Largest stdin payload. It is below the 16 KiB initial macOS pipe buffer, so
/// writing it before monitoring the child cannot block on a child that never reads.
pub(crate) const STDIN_LIMIT: usize = 8 * 1024;

/// Process groups of the `run_program` children running now. Each leads a
/// group of its own, out of reach of a kill aimed at the service's group, so
/// an exit that skips the normal stop (a forced exit, a panic) kills these
/// with [`kill_running_programs`].
static PROGRAM_GROUPS: Mutex<BTreeSet<i32>> = Mutex::new(BTreeSet::new());

pub(crate) fn program_groups() -> std::sync::MutexGuard<'static, BTreeSet<i32>> {
    PROGRAM_GROUPS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// SIGKILLs the process group of every `run_program` still running, for exit
/// paths that skip the normal stop. A service killed with SIGKILL cannot run
/// this, so a program running at that moment keeps running.
pub fn kill_running_programs() {
    for &group in program_groups().iter() {
        // SAFETY: killpg only sends a signal to a group this process started.
        #[cfg(unix)]
        unsafe {
            libc::killpg(group, libc::SIGKILL);
        }
        #[cfg(not(unix))]
        let _ = group;
    }
}

/// Waits between steps and reads the time; injected so tests never sleep.
pub(crate) trait Sleeper: Send {
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

pub(crate) struct SystemSleeper;

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
pub(crate) struct ProcessRequest {
    pub(crate) program: &'static str,
    pub(crate) args: Vec<String>,
    /// Bytes written to stdin before monitoring; `None` connects /dev/null.
    pub(crate) stdin: Option<Vec<u8>>,
    /// Fixed variables added to the inherited environment.
    pub(crate) env: &'static [(&'static str, &'static str)],
    pub(crate) timeout: Duration,
    pub(crate) output_limit: usize,
    /// The file to execute when it is not the fixed `program` (a `run_program`
    /// path); `program` then only names the request in errors.
    pub(crate) path: Option<String>,
    /// Directory the process starts in; inherited when `None`.
    pub(crate) current_dir: Option<String>,
    /// Run in a process group of its own with stdout and stderr on /dev/null,
    /// waiting only for the process to exit: stopping it (timeout or
    /// cancellation) kills the whole group, and anything it leaves running
    /// after a normal exit keeps running.
    pub(crate) isolated: bool,
}

pub(crate) struct ProcessOutput {
    pub(crate) success: bool,
    pub(crate) status: String,
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: Vec<u8>,
}

pub(crate) trait ProcessRunner: Send {
    fn run(&mut self, request: &ProcessRequest) -> Result<ProcessOutput, String>;

    fn cancellation(&self) -> Option<&AtomicBool> {
        None
    }

    /// Whether the dispatcher checks paths on the local file system before a
    /// request; false for the test runner, which starts nothing.
    fn checks_paths(&self) -> bool {
        true
    }

    /// An independent system runner cancelled by `cancellation`, if supported.
    fn fork(&self, cancellation: Arc<AtomicBool>) -> Option<Box<dyn ProcessRunner>> {
        let _ = cancellation;
        None
    }
}

pub(crate) struct SystemProcessRunner {
    pub(crate) cancellation: Option<Arc<AtomicBool>>,
}

impl SystemProcessRunner {
    fn fork_runner(cancellation: Arc<AtomicBool>) -> Option<Box<dyn ProcessRunner>> {
        Some(Box::new(Self {
            cancellation: Some(cancellation),
        }))
    }
}

pub(crate) fn check_cancellation(
    cancellation: Option<&AtomicBool>,
    program: &str,
) -> Result<(), String> {
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

    fn fork(&self, cancellation: Arc<AtomicBool>) -> Option<Box<dyn ProcessRunner>> {
        Self::fork_runner(cancellation)
    }
}

#[cfg(target_os = "macos")]
impl ProcessRunner for SystemProcessRunner {
    fn run(&mut self, request: &ProcessRequest) -> Result<ProcessOutput, String> {
        use std::os::unix::process::CommandExt;
        use std::process::{Command, Stdio};
        check_cancellation(self.cancellation(), request.program)?;
        let output = || {
            if request.isolated {
                Stdio::null()
            } else {
                Stdio::piped()
            }
        };
        let mut command = Command::new(request.path.as_deref().unwrap_or(request.program));
        command
            .args(&request.args)
            .envs(request.env.iter().copied())
            .stdin(if request.stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(output())
            .stderr(output());
        if let Some(directory) = &request.current_dir {
            command.current_dir(directory);
        }
        if request.isolated {
            command.process_group(0);
        }
        let mut child = command
            .spawn()
            .map_err(|error| format!("{}: could not start process: {error}", request.program))?;
        let group = request
            .isolated
            .then(|| i32::try_from(child.id()).ok())
            .flatten();
        if let Some(group) = group {
            program_groups().insert(group);
        }
        let result = write_child_stdin(&mut child, request)
            .and_then(|()| collect_child_output(&mut child, request, self.cancellation()));
        // Stop the process (its whole group when isolated) on all monitoring
        // failures. No retry is safe: a partially completed action could
        // otherwise execute twice.
        let result = match result {
            Err(error) => match stop_child(&mut child, request.isolated) {
                Ok(()) => Err(error),
                Err(cleanup_error) => Err(format!(
                    "{error}; subprocess cleanup failed: {cleanup_error}"
                )),
            },
            ok => ok,
        };
        if let Some(group) = group {
            program_groups().remove(&group);
        }
        result
    }

    fn cancellation(&self) -> Option<&AtomicBool> {
        self.cancellation.as_deref()
    }

    fn fork(&self, cancellation: Arc<AtomicBool>) -> Option<Box<dyn ProcessRunner>> {
        Self::fork_runner(cancellation)
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn write_child_stdin(
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
pub(crate) fn stop_child(child: &mut std::process::Child, group: bool) -> Result<(), String> {
    // The child leads its group (process_group(0)), so its pid is the group
    // id; this reaches what it started even after it exits. A failure is
    // reported after the leader itself is killed and reaped.
    let group_error = if group {
        match libc::pid_t::try_from(child.id()) {
            // SAFETY: killpg only sends a signal; ESRCH (group already gone) is fine.
            Ok(pgid) if unsafe { libc::killpg(pgid, libc::SIGKILL) } == -1 => {
                let error = std::io::Error::last_os_error();
                (error.raw_os_error() != Some(libc::ESRCH))
                    .then(|| format!("could not kill process group: {error}"))
            }
            Ok(_) => None,
            Err(error) => Some(format!("could not name process group: {error}")),
        }
    } else {
        None
    };
    let stopped = stop_leader(child);
    match (group_error, stopped) {
        (None, result) => result,
        (Some(group), Ok(())) => Err(group),
        (Some(group), Err(leader)) => Err(format!("{group}; {leader}")),
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn stop_leader(child: &mut std::process::Child) -> Result<(), String> {
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
pub(crate) fn collect_child_output(
    child: &mut std::process::Child,
    request: &ProcessRequest,
    cancellation: Option<&AtomicBool>,
) -> Result<ProcessOutput, String> {
    use std::time::Instant;
    let pipes = (child.stdout.take(), child.stderr.take());
    let start = Instant::now();
    let poll_status = || {
        child
            .try_wait()
            .map(|status| status.map(|status| (status.success(), status.to_string())))
            .map_err(|error| error.to_string())
    };
    let elapsed = || start.elapsed();
    let pause = || std::thread::sleep(Duration::from_millis(10));
    let cancelled = || cancellation.is_some_and(|flag| flag.load(Ordering::Acquire));
    if request.isolated {
        // Output goes to /dev/null: there are no pipes, only the exit to wait for.
        return collect_output(
            &mut std::io::empty(),
            &mut std::io::empty(),
            request,
            poll_status,
            elapsed,
            pause,
            cancelled,
        );
    }
    let (Some(mut stdout_pipe), Some(mut stderr_pipe)) = pipes else {
        return Err("missing subprocess stdout or stderr pipe".into());
    };
    make_nonblocking(&stdout_pipe)?;
    make_nonblocking(&stderr_pipe)?;
    collect_output(
        &mut stdout_pipe,
        &mut stderr_pipe,
        request,
        poll_status,
        elapsed,
        pause,
        cancelled,
    )
}

#[cfg(any(target_os = "macos", test))]
pub(crate) fn collect_output(
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
            // One last look, so a process that exited right at the deadline is
            // not reported (or, isolated, its group killed) as timed out.
            if let Some((success, status)) = poll_status().map_err(|error| {
                format!("{}: could not inspect process: {error}", request.program)
            })? && stdout_closed
                && stderr_closed
            {
                return Ok(ProcessOutput {
                    success,
                    status,
                    stdout,
                    stderr,
                });
            }
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
pub(crate) fn make_nonblocking(pipe: &impl std::os::fd::AsRawFd) -> Result<(), String> {
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
pub(crate) fn drain_pipe(
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

/// The subprocess runner and clock of one dispatcher, handed to its host for
/// each operation. Opaque outside this crate: hosts are built here.
pub struct Process {
    pub(crate) runner: Box<dyn ProcessRunner>,
    pub(crate) sleeper: Box<dyn Sleeper>,
}

impl Process {
    pub(crate) fn new(runner: Box<dyn ProcessRunner>, sleeper: Box<dyn Sleeper>) -> Self {
        Self { runner, sleeper }
    }

    pub(crate) fn run(
        &mut self,
        program: &'static str,
        args: Vec<String>,
    ) -> Result<Vec<u8>, String> {
        self.run_request(ProcessRequest {
            program,
            args,
            ..ProcessRequest::default()
        })
    }

    pub(crate) fn run_request(&mut self, request: ProcessRequest) -> Result<Vec<u8>, String> {
        self.run_checked(request).map(|output| output.stdout)
    }

    /// [`Self::run_checked`] whose failure message gives only the size of
    /// stderr, never its text, for tools that receive user text in argv.
    pub(crate) fn run_quiet(&mut self, request: ProcessRequest) -> Result<ProcessOutput, String> {
        self.run_checked_with(request, false)
    }

    /// Runs one bounded request and fails on a nonzero exit; the successful
    /// output keeps stderr for callers that report diagnostics.
    pub(crate) fn run_checked(&mut self, request: ProcessRequest) -> Result<ProcessOutput, String> {
        self.run_checked_with(request, true)
    }

    pub(crate) fn run_checked_with(
        &mut self,
        request: ProcessRequest,
        stderr_in_errors: bool,
    ) -> Result<ProcessOutput, String> {
        let program = request.program;
        let output = self.run_bounded(request)?;
        if !output.success {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let details = stderr.trim();
            return Err(if details.is_empty() {
                format!("{program}: {}", output.status)
            } else if !stderr_in_errors {
                format!(
                    "{program}: {} (stderr: {} bytes, not shown)",
                    output.status,
                    output.stderr.len()
                )
            } else {
                format!("{program}: {}: {details}", output.status)
            });
        }
        Ok(output)
    }

    /// Runs one request within the timeout and output limits; the output is
    /// returned whatever the exit status.
    pub(crate) fn run_bounded(&mut self, request: ProcessRequest) -> Result<ProcessOutput, String> {
        let program = request.program;
        check_cancellation(self.runner.cancellation(), program)?;
        if request
            .stdin
            .as_ref()
            .is_some_and(|input| input.len() > STDIN_LIMIT)
        {
            return Err(format!("{program}: stdin exceeds {STDIN_LIMIT} bytes"));
        }
        // Requests that set their own bound (timer alerts, terminal windows,
        // macro replay, run_program) keep it; all others get PROCESS_TIMEOUT.
        let timeout = if request.timeout.is_zero() {
            PROCESS_TIMEOUT
        } else {
            request.timeout
        };
        let output = self.runner.run(&ProcessRequest {
            timeout,
            output_limit: OUTPUT_LIMIT,
            ..request
        })?;
        if output.stdout.len() > OUTPUT_LIMIT || output.stderr.len() > OUTPUT_LIMIT {
            return Err(format!(
                "{program}: subprocess output exceeded {OUTPUT_LIMIT} bytes"
            ));
        }
        Ok(output)
    }

    /// Waits `duration`, ending early when the run is cancelled.
    pub(crate) fn pause(&mut self, duration: Duration) -> Result<(), String> {
        self.sleeper.sleep(duration, self.runner.cancellation())
    }

    /// The clock that sequence deadlines are measured on.
    pub(crate) fn now(&self) -> std::time::Instant {
        self.sleeper.now()
    }

    /// The run's cancellation flag, when it has one.
    pub(crate) fn cancellation(&self) -> Option<&AtomicBool> {
        self.runner.cancellation()
    }

    /// Whether paths are checked on the file system before a request; false
    /// for test runners, which start nothing.
    pub(crate) fn checks_paths(&self) -> bool {
        self.runner.checks_paths()
    }
}
