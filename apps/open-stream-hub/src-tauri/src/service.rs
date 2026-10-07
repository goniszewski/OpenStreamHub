//! Supervises `masterhub-service run` child processes, the only path that may
//! execute profile actions. The app never runs actions in-process: it writes a
//! private profile snapshot, spawns the service for one exact interface, relays
//! its JSONL stdout as `service://record` events without interpreting them, and
//! reports the process lifecycle as `service://status`.
//!
//! Stopping sends one SIGINT (the service then restores hardware mode; a
//! second Ctrl-C would make it exit immediately, so it is never repeated),
//! waits up to `STOP_GRACE`, then SIGKILLs the child's process group. The
//! child runs in its own process group so a terminal Ctrl-C aimed at the app
//! (e.g. during `tauri dev`) does not reach it a second time.
//!
//! The binary is bundled as a Tauri sidecar (`bundle.externalBin`), which the
//! bundler installs next to the app executable without the target-triple
//! suffix. It is found next to the app executable or, in debug builds only,
//! via `OPEN_STREAM_HUB_SERVICE` or in a workspace `target/` directory.

use crate::{feed, replay};
use cooltek_sdk_cli::{capture::encode_hex, device::parse_path};
use cooltek_sdk_service::{Action, Profile};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashMap,
    ffi::{OsStr, OsString},
    fs,
    io::{self, BufRead, BufReader, Read},
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

pub const RECORD_EVENT: &str = "service://record";
pub const STATUS_EVENT: &str = "service://status";
/// Overrides the service binary location (debug builds only).
pub const BINARY_ENV: &str = "OPEN_STREAM_HUB_SERVICE";

/// Longest stdout line relayed as a record; matches the service's own limit.
pub const MAX_LINE_BYTES: usize = 65_536;
/// Characters kept from a line that is relayed as a `raw` record.
pub const RAW_LINE_CHARS: usize = 512;
/// Bytes of stderr kept for the `exited` status.
pub const STDERR_TAIL_BYTES: usize = 4096;
/// How long a stop waits after SIGINT before killing the process group.
pub const STOP_GRACE: Duration = Duration::from_secs(3);
/// How long the supervisor waits for the pipes to drain after the child is
/// reaped; a grandchild holding a pipe open cannot delay `exited` beyond it.
const DRAIN_GRACE: Duration = Duration::from_millis(500);
const POLL: Duration = Duration::from_millis(20);
const PROFILE_DIR: &str = "service-profiles";
pub const SHUTTING_DOWN: &str = "the app is shutting down";

fn binary_name() -> &'static str {
    if cfg!(windows) {
        "masterhub-service.exe"
    } else {
        "masterhub-service"
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartRequest {
    pub profile: Profile,
    pub path_hex: String,
    pub execute: bool,
    pub software_mode: bool,
    /// The saved profile `profile` was started from, when the editor holds
    /// it unchanged; null for an unsaved draft. Only used to find the current
    /// position for a profile key (next/previous).
    #[serde(default)]
    pub profile_id: Option<String>,
}

/// True when `profile` binds a `profile` action anywhere: flat bindings,
/// keypad action tiles on any page, or dial function bindings. Only then is
/// `--profile-switch-requests` passed, so a profile without the feature
/// runs with exactly the arguments it had before (an older sidecar keeps
/// working).
pub fn uses_profile_action(profile: &Profile) -> bool {
    let is_profile = |action: &Action| matches!(action, Action::Profile { .. });
    profile
        .bindings
        .iter()
        .any(|binding| is_profile(&binding.action))
        || profile.keypad.iter().any(|keypad| {
            keypad
                .pages
                .iter()
                .flat_map(|page| page.slots.iter().flatten())
                .any(|tile| match tile {
                    cooltek_sdk_service::KeypadTile::Action { bindings, .. } => {
                        bindings.iter().any(|binding| is_profile(&binding.action))
                    }
                    _ => false,
                })
        })
        || profile.dial_functions.iter().flatten().any(|function| {
            function
                .bindings
                .iter()
                .any(|binding| is_profile(&binding.action))
        })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceState {
    Starting,
    Running,
    Stopping,
    Exited,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct StatusPayload {
    pub service_id: String,
    pub state: ServiceState,
    pub execute: bool,
    /// Only on `exited`: the exit code, or null when ended by a signal.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<Option<i32>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stderr_tail: Option<String>,
}

impl StatusPayload {
    pub fn new(service_id: &str, state: ServiceState, execute: bool) -> Self {
        Self {
            service_id: service_id.into(),
            state,
            execute,
            exit_code: None,
            error: None,
            stderr_tail: None,
        }
    }

    pub fn failed(service_id: &str, execute: bool, error: String) -> Self {
        Self {
            error: Some(error),
            ..Self::new(service_id, ServiceState::Failed, execute)
        }
    }

    pub fn exited(service_id: &str, execute: bool, code: Option<i32>, stderr: String) -> Self {
        Self {
            exit_code: Some(code),
            stderr_tail: Some(stderr),
            ..Self::new(service_id, ServiceState::Exited, execute)
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RecordPayload {
    pub service_id: String,
    pub record: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ServiceMessage {
    Record(RecordPayload),
    Status(StatusPayload),
}

pub type Emit = Arc<dyn Fn(ServiceMessage) + Send + Sync>;

/// Returns the stable id `service:<lowercase path hex>` for a valid path.
pub fn service_id(path_hex: &str) -> Result<String, String> {
    Ok(format!("service:{}", encode_hex(&parse_path(path_hex)?)))
}

/// Action execution exists only on macOS; refuse it elsewhere before spawning.
pub fn check_execute_supported(execute: bool) -> Result<(), String> {
    if execute && !cfg!(target_os = "macos") {
        return Err("executing actions is supported only on macOS".into());
    }
    Ok(())
}

/// Arguments for `masterhub-service`; passed directly, never through a shell.
/// `profile_switch_requests` is set for session-managed runs: the service then
/// reports `profile` key actions as `profile_switch_requested` records, which
/// the session driver resolves against the saved profiles.
pub fn service_args(
    profile: &Path,
    path_hex: &str,
    execute: bool,
    software_mode: bool,
    profile_switch_requests: bool,
) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec![
        "run".into(),
        "--profile".into(),
        profile.into(),
        "--path-hex".into(),
        path_hex.into(),
    ];
    if execute {
        args.push("--execute".into());
    }
    if software_mode {
        args.push("--software-mode".into());
    }
    if profile_switch_requests {
        args.push("--profile-switch-requests".into());
    }
    args
}

fn is_workspace_root(directory: &Path) -> bool {
    fs::read_to_string(directory.join("Cargo.toml")).is_ok_and(|manifest| {
        manifest.lines().any(|line| {
            let line = line.trim();
            line == "[workspace]" || line.starts_with("[workspace.")
        })
    })
}

/// Finds the service binary: (with `dev_fallback`) `env_override` when it
/// names a file, then next to the app executable, then (with `dev_fallback`)
/// `target/debug` and `target/release` of every ancestor workspace root of
/// `exe_dir`. Release builds never honor the override, so an environment
/// variable cannot redirect a shipped app to another binary.
pub fn resolve_service_binary(
    env_override: Option<&OsStr>,
    exe_dir: Option<&Path>,
    dev_fallback: bool,
) -> Result<PathBuf, String> {
    let mut checked = Vec::new();
    let mut candidates = Vec::new();
    if let Some(path) = env_override.filter(|value| dev_fallback && !value.is_empty()) {
        candidates.push(PathBuf::from(path));
    }
    if let Some(exe_dir) = exe_dir {
        candidates.push(exe_dir.join(binary_name()));
        if dev_fallback {
            for root in exe_dir.ancestors().filter(|dir| is_workspace_root(dir)) {
                for profile in ["debug", "release"] {
                    candidates.push(root.join("target").join(profile).join(binary_name()));
                }
            }
        }
    }
    for candidate in candidates {
        if candidate.is_file() {
            return Ok(candidate);
        }
        checked.push(candidate.display().to_string());
    }
    let checked = if checked.is_empty() {
        "no locations".to_string()
    } else {
        checked.join(", ")
    };
    let hint = if dev_fallback {
        format!(
            "Build it with `cargo build -p cooltek-sdk-cli --bin masterhub-service` or set \
             {BINARY_ENV} to its path."
        )
    } else {
        "Reinstall the app; the service is bundled next to it.".to_string()
    };
    Err(format!(
        "could not find {name}. Checked: {checked}. {hint}",
        name = binary_name()
    ))
}

/// A private profile snapshot handed to one child; removed when dropped.
#[derive(Debug)]
pub struct TempProfile {
    path: PathBuf,
}

impl TempProfile {
    /// Writes `profile` to a new, uniquely named file (0600) in `directory`
    /// (created 0700).
    pub fn write(directory: &Path, profile: &Profile) -> Result<Self, String> {
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
            builder.mode(0o700);
            builder
                .create(directory)
                .map_err(|error| format!("could not create the service folder: {error}"))?;
            fs::set_permissions(directory, fs::Permissions::from_mode(0o700))
                .map_err(|error| format!("could not secure the service folder: {error}"))?;
        }
        #[cfg(not(unix))]
        builder
            .create(directory)
            .map_err(|error| format!("could not create the service folder: {error}"))?;

        let bytes = serde_json::to_vec(profile)
            .map_err(|error| format!("could not encode profile: {error}"))?;
        let path = directory.join(format!("profile-{}.json", uuid::Uuid::new_v4()));
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let mut file = options
            .open(&path)
            .map_err(|error| format!("could not write the service profile: {error}"))?;
        // Owns the path from here, so a failed write still removes the file.
        let snapshot = Self { path };
        io::Write::write_all(&mut file, &bytes)
            .and_then(|()| file.sync_all())
            .map_err(|error| format!("could not write the service profile: {error}"))?;
        Ok(snapshot)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempProfile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// Where profile snapshots live under the app data folder.
pub fn profile_dir(app_data: &Path) -> PathBuf {
    app_data.join(PROFILE_DIR)
}

/// Removes snapshots left behind by a crashed previous run.
pub fn clean_stale_profiles(directory: &Path) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("profile-") && name.ends_with(".json") {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// Reads one line of at most `max` bytes, newline excluded. Returns `None` at
/// EOF, otherwise whether the line was longer than `max`; an over-long line
/// is consumed through its newline and only its first `max` bytes are kept.
pub fn read_line_bounded(
    reader: &mut impl BufRead,
    max: usize,
    line: &mut Vec<u8>,
) -> io::Result<Option<bool>> {
    line.clear();
    let (mut any, mut overflow) = (false, false);
    loop {
        let available = match reader.fill_buf() {
            Ok(available) => available,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        if available.is_empty() {
            return Ok(any.then_some(overflow));
        }
        any = true;
        let newline = available.iter().position(|&byte| byte == b'\n');
        let chunk = &available[..newline.unwrap_or(available.len())];
        let room = max.saturating_sub(line.len());
        overflow |= chunk.len() > room;
        line.extend_from_slice(&chunk[..chunk.len().min(room)]);
        let used = newline.map_or(available.len(), |index| index + 1);
        reader.consume(used);
        if newline.is_some() {
            return Ok(Some(overflow));
        }
    }
}

/// Maps one stdout line to a record: parsed JSON as-is, or `{type: raw}` with
/// at most `RAW_LINE_CHARS` characters for over-long or non-JSON lines. Blank
/// lines produce nothing.
pub fn record_payload(service_id: &str, line: &[u8], truncated: bool) -> Option<RecordPayload> {
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    if !truncated && line.iter().all(u8::is_ascii_whitespace) {
        return None;
    }
    let record = (!truncated)
        .then(|| serde_json::from_slice::<Value>(line).ok())
        .flatten()
        .unwrap_or_else(|| {
            let text: String = String::from_utf8_lossy(line)
                .chars()
                .take(RAW_LINE_CHARS)
                .collect();
            serde_json::json!({ "type": "raw", "line": text })
        });
    Some(RecordPayload {
        service_id: service_id.into(),
        record,
    })
}

fn lock_child(child: &Mutex<Child>) -> MutexGuard<'_, Child> {
    child
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// True while the child has not been reaped, so its pid cannot be reused.
fn unreaped(child: &mut Child) -> bool {
    matches!(child.try_wait(), Ok(None))
}

/// Sends SIGINT to the child only; the service cancels its own subprocesses.
fn interrupt(child: &mut Child) {
    #[cfg(unix)]
    if let Ok(pid) = libc::pid_t::try_from(child.id()) {
        // SAFETY: plain syscall; the caller holds the child lock and checked it
        // is unreaped, so `pid` still names this child.
        unsafe { libc::kill(pid, libc::SIGINT) };
    }
    #[cfg(not(unix))]
    let _ = child.kill();
}

/// SIGKILLs the child's whole process group (it is the group leader), so
/// in-flight system tools do not outlive it.
fn force_kill(child: &mut Child) {
    #[cfg(unix)]
    if let Ok(pid) = libc::pid_t::try_from(child.id()) {
        // SAFETY: as in `interrupt`; the group id equals the unreaped leader's pid.
        unsafe { libc::kill(-pid, libc::SIGKILL) };
    }
    let _ = child.kill();
}

/// SIGKILLs what is left of the process group led by the reaped child `pid`,
/// so grandchildren (e.g. in-flight system tools) do not outlive the service.
/// The group has no leader any more, so ESRCH (no members) is expected.
fn kill_group(pid: u32) {
    #[cfg(unix)]
    if let Ok(pid) = libc::pid_t::try_from(pid) {
        // SAFETY: plain syscall on the child's own process group.
        unsafe { libc::killpg(pid, libc::SIGKILL) };
    }
    #[cfg(not(unix))]
    let _ = pid;
}

struct ServiceHandle {
    child: Arc<Mutex<Child>>,
    execute: bool,
    stopping: Arc<AtomicBool>,
    /// Set after `exited` is emitted, so the id can be reused.
    done: Arc<AtomicBool>,
    profile_path: Option<PathBuf>,
    emit: Emit,
    join: Option<JoinHandle<()>>,
}

impl ServiceHandle {
    fn finished(&self) -> bool {
        self.done.load(Ordering::SeqCst) || self.join.as_ref().is_some_and(JoinHandle::is_finished)
    }

    /// Emits `stopping` once and sends SIGINT, unless the child already exited.
    fn request_stop(&self, service_id: &str) {
        let mut child = lock_child(&self.child);
        if unreaped(&mut child) && !self.stopping.swap(true, Ordering::SeqCst) {
            (self.emit)(ServiceMessage::Status(StatusPayload::new(
                service_id,
                ServiceState::Stopping,
                self.execute,
            )));
            interrupt(&mut child);
        }
    }

    fn kill_if_running(&self) {
        let mut child = lock_child(&self.child);
        if unreaped(&mut child) {
            force_kill(&mut child);
        }
    }
}

fn wait_until(deadline: Instant, mut ready: impl FnMut() -> bool) -> bool {
    loop {
        if ready() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(POLL);
    }
}

/// Running service children by id.
#[derive(Default)]
pub struct Services {
    services: Mutex<HashMap<String, ServiceHandle>>,
    /// Set by `stop_all` (app exit) under the lock; no child starts after it.
    closed: AtomicBool,
}

impl Services {
    fn lock(&self) -> MutexGuard<'_, HashMap<String, ServiceHandle>> {
        self.services
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// True once app exit began; every later start is refused.
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    pub fn is_running(&self, service_id: &str) -> bool {
        self.lock()
            .get(service_id)
            .is_some_and(|handle| !handle.finished())
    }

    /// Ids of services whose child has not exited, sorted.
    pub fn running_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self
            .lock()
            .iter()
            .filter(|(_, handle)| !handle.finished())
            .map(|(id, _)| id.clone())
            .collect();
        ids.sort();
        ids
    }

    /// Spawns `program args` as `service_id`, emitting `starting`, then
    /// `running` or `failed`. A duplicate running id is rejected without any
    /// event. `profile` is removed once the child is reaped. Refused without
    /// any event once `stop_all` began.
    pub fn start(
        &self,
        service_id: String,
        execute: bool,
        program: &Path,
        args: Vec<OsString>,
        profile: Option<TempProfile>,
        emit: Emit,
    ) -> Result<String, String> {
        let mut services = self.lock();
        if self.is_closed() {
            return Err(SHUTTING_DOWN.into());
        }
        if let Some(handle) = services.get(&service_id) {
            if !handle.finished() {
                return Err(format!("service {service_id} is already running"));
            }
            if let Some(join) = services
                .remove(&service_id)
                .and_then(|mut old| old.join.take())
            {
                let _ = join.join();
            }
        }
        let status =
            |state| ServiceMessage::Status(StatusPayload::new(&service_id, state, execute));
        emit(status(ServiceState::Starting));
        let mut command = Command::new(program);
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(unix)]
        std::os::unix::process::CommandExt::process_group(&mut command, 0);
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                let error = format!("could not start {}: {error}", program.display());
                emit(ServiceMessage::Status(StatusPayload::failed(
                    &service_id,
                    execute,
                    error.clone(),
                )));
                return Err(error);
            }
        };
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let child = Arc::new(Mutex::new(child));
        let done = Arc::new(AtomicBool::new(false));
        emit(status(ServiceState::Running));

        let profile_path = profile.as_ref().map(|profile| profile.path().to_path_buf());
        let supervisor = Supervisor {
            service_id: service_id.clone(),
            execute,
            child: Arc::clone(&child),
            done: Arc::clone(&done),
            emit: Arc::clone(&emit),
        };
        let join = thread::Builder::new()
            .name(format!("supervise {service_id}"))
            .spawn(move || supervisor.run(stdout, stderr, profile));
        let join = match join {
            Ok(join) => join,
            Err(error) => {
                let mut child = lock_child(&child);
                force_kill(&mut child);
                let _ = child.wait();
                if let Some(path) = &profile_path {
                    let _ = fs::remove_file(path);
                }
                let error = format!("could not supervise the service: {error}");
                emit(ServiceMessage::Status(StatusPayload::failed(
                    &service_id,
                    execute,
                    error.clone(),
                )));
                return Err(error);
            }
        };
        services.insert(
            service_id.clone(),
            ServiceHandle {
                child,
                execute,
                stopping: Arc::new(AtomicBool::new(false)),
                done,
                profile_path,
                emit,
                join: Some(join),
            },
        );
        Ok(service_id)
    }

    /// SIGINT, up to `grace` for a clean exit, then SIGKILL. Returns once the
    /// child is reaped and `exited` emitted. Unknown ids are ignored.
    pub fn stop(&self, service_id: &str, grace: Duration) -> Result<(), String> {
        let (join, done) = {
            let mut services = self.lock();
            let Some(handle) = services.get_mut(service_id) else {
                return Ok(());
            };
            handle.request_stop(service_id);
            (handle.join.take(), Arc::clone(&handle.done))
        };
        let reaped = wait_until(Instant::now() + grace, || done.load(Ordering::SeqCst));
        if !reaped && let Some(handle) = self.lock().get(service_id) {
            handle.kill_if_running();
        }
        match join {
            Some(join) => {
                let _ = join.join();
            }
            // Another stop owns the join; wait for it to finish the child.
            None => {
                wait_until(Instant::now() + grace + Duration::from_secs(2), || {
                    done.load(Ordering::SeqCst)
                });
            }
        }
        let mut services = self.lock();
        if services
            .get(service_id)
            .is_some_and(|handle| handle.done.load(Ordering::SeqCst))
        {
            services.remove(service_id);
        }
        Ok(())
    }

    /// Stops several services in parallel: every SIGINT goes out at once, so
    /// the whole call is bounded by one `stop`, not by their sum.
    pub fn stop_many(&self, service_ids: &[String], grace: Duration) {
        thread::scope(|scope| {
            for service_id in service_ids {
                scope.spawn(move || {
                    if let Err(error) = self.stop(service_id, grace) {
                        eprintln!("could not stop {service_id}: {error}");
                    }
                });
            }
        });
    }

    /// App exit: SIGINT every child, wait up to `grace`, SIGKILL the rest, and
    /// wait briefly for reaping without joining threads, so a stuck pipe never
    /// blocks exit. Profile snapshots are removed either way. Later starts are
    /// refused.
    pub fn stop_all(&self, grace: Duration) {
        let services = self.lock();
        self.closed.store(true, Ordering::SeqCst);
        for (service_id, handle) in services.iter() {
            handle.request_stop(service_id);
        }
        let all_done = || services.values().all(ServiceHandle::finished);
        if !wait_until(Instant::now() + grace, all_done) {
            for handle in services.values() {
                handle.kill_if_running();
            }
            wait_until(Instant::now() + Duration::from_millis(500), all_done);
        }
        for path in services
            .values()
            .filter_map(|handle| handle.profile_path.as_ref())
        {
            let _ = fs::remove_file(path);
        }
    }
}

struct Supervisor {
    service_id: String,
    execute: bool,
    child: Arc<Mutex<Child>>,
    done: Arc<AtomicBool>,
    emit: Emit,
}

impl Supervisor {
    fn run(
        self,
        stdout: Option<impl Read + Send + 'static>,
        stderr: Option<impl Read + Send + 'static>,
        profile: Option<TempProfile>,
    ) {
        // Set once `exited` is about to be emitted; late lines are dropped.
        let closed = Arc::new(AtomicBool::new(false));
        let tail = Arc::new(Mutex::new(Vec::new()));
        let (drained_tx, drained_rx) = mpsc::channel::<()>();
        let mut readers = 0;
        if let Some(stdout) = stdout {
            let (id, emit, closed, drained) = (
                self.service_id.clone(),
                Arc::clone(&self.emit),
                Arc::clone(&closed),
                drained_tx.clone(),
            );
            let spawned = thread::Builder::new()
                .name(format!("stdout {}", self.service_id))
                .spawn(move || {
                    relay_records(&id, stdout, &closed, &*emit);
                    let _ = drained.send(());
                });
            readers += usize::from(spawned.is_ok());
        }
        if let Some(stderr) = stderr {
            let (tail, drained) = (Arc::clone(&tail), drained_tx.clone());
            let spawned = thread::Builder::new()
                .name(format!("stderr {}", self.service_id))
                .spawn(move || {
                    keep_tail(stderr, &tail);
                    let _ = drained.send(());
                });
            readers += usize::from(spawned.is_ok());
        }
        drop(drained_tx);

        let pid = lock_child(&self.child).id();
        let status = self.reap();
        kill_group(pid);
        let deadline = Instant::now() + DRAIN_GRACE;
        for _ in 0..readers {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if drained_rx.recv_timeout(remaining).is_err() {
                break;
            }
        }
        closed.store(true, Ordering::SeqCst);
        drop(profile);
        let stderr_tail =
            String::from_utf8_lossy(&tail.lock().unwrap_or_else(|poisoned| poisoned.into_inner()))
                .into_owned();
        (self.emit)(ServiceMessage::Status(StatusPayload::exited(
            &self.service_id,
            self.execute,
            status.and_then(|status| status.code()),
            stderr_tail,
        )));
        self.done.store(true, Ordering::SeqCst);
    }

    /// Polls rather than blocking in `wait`, so stop can signal under the lock
    /// while the pid is known to be unreaped.
    fn reap(&self) -> Option<ExitStatus> {
        loop {
            {
                let mut child = lock_child(&self.child);
                match child.try_wait() {
                    Ok(Some(status)) => return Some(status),
                    Ok(None) => {}
                    Err(_) => {
                        force_kill(&mut child);
                        return child.wait().ok();
                    }
                }
            }
            thread::sleep(POLL);
        }
    }
}

fn relay_records(
    service_id: &str,
    stdout: impl Read,
    closed: &AtomicBool,
    emit: &(dyn Fn(ServiceMessage) + Send + Sync),
) {
    let mut reader = BufReader::new(stdout);
    let mut line = Vec::new();
    while let Ok(Some(truncated)) = read_line_bounded(&mut reader, MAX_LINE_BYTES, &mut line) {
        if closed.load(Ordering::SeqCst) {
            return;
        }
        if let Some(record) = record_payload(service_id, &line, truncated) {
            emit(ServiceMessage::Record(record));
        }
    }
}

fn keep_tail(mut stderr: impl Read, tail: &Mutex<Vec<u8>>) {
    let mut buffer = [0; 4096];
    loop {
        match stderr.read(&mut buffer) {
            Ok(0) => return,
            Ok(read) => {
                let mut tail = tail.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                tail.extend_from_slice(&buffer[..read]);
                let excess = tail.len().saturating_sub(STDERR_TAIL_BYTES);
                tail.drain(..excess);
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => return,
        }
    }
}

/// The request checks `start_service` runs before spawning: a valid profile
/// and an execute mode this platform supports. The session driver runs the
/// same checks for a start while asleep, which spawns nothing yet.
pub fn check_request(request: &StartRequest) -> Result<(), String> {
    replay::validate_profile(&request.profile)?;
    if let Some(id) = &request.profile_id {
        crate::store::validate_id(id)?;
    }
    check_execute_supported(request.execute)
}

/// The `start_service` command without Tauri: validates, stops a read-only
/// feed on the same interface, writes the snapshot, and spawns `program`.
/// Failures after the id is known (except a duplicate) emit `failed`.
pub fn start_service(
    services: &Services,
    request: StartRequest,
    profile_switch_requests: bool,
    app_data: Result<&Path, String>,
    program: Result<PathBuf, String>,
    stop_feed: impl FnOnce(&str) -> Result<(), String>,
    emit: Emit,
) -> Result<String, String> {
    let service_id = service_id(&request.path_hex)?;
    if services.is_closed() {
        return Err(SHUTTING_DOWN.into());
    }
    if services.is_running(&service_id) {
        return Err(format!("service {service_id} is already running"));
    }
    let path_hex = encode_hex(&parse_path(&request.path_hex)?);
    let prepared = (|| {
        check_request(&request)?;
        let program = program?;
        let directory = profile_dir(app_data?);
        stop_feed(&feed::feed_id(&feed::FeedSource::Live {
            path_hex: path_hex.clone(),
        })?)?;
        let profile = TempProfile::write(&directory, &request.profile)?;
        Ok::<_, String>((program, profile))
    })();
    let (program, profile) = prepared.inspect_err(|error| {
        emit(ServiceMessage::Status(StatusPayload::failed(
            &service_id,
            request.execute,
            error.clone(),
        )))
    })?;
    let args = service_args(
        profile.path(),
        &path_hex,
        request.execute,
        request.software_mode,
        profile_switch_requests,
    );
    services.start(
        service_id,
        request.execute,
        &program,
        args,
        Some(profile),
        emit,
    )
}

/// A read-only live feed may not open an interface a service owns.
pub fn check_feed_allowed(services: &Services, source: &feed::FeedSource) -> Result<(), String> {
    if let feed::FeedSource::Live { path_hex } = source {
        let service_id = service_id(path_hex)?;
        if services.is_running(&service_id) {
            return Err(format!(
                "service {service_id} is using this device; stop it before starting a feed"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_profiles;
    use std::io::Cursor;

    #[test]
    fn bounded_lines_truncate_without_losing_the_next_line() {
        let mut input = vec![b'a'; 20];
        input.extend_from_slice(b"\nshort\r\nlast");
        let mut reader = BufReader::with_capacity(4, Cursor::new(input));
        let mut line = Vec::new();
        assert_eq!(
            read_line_bounded(&mut reader, 8, &mut line).unwrap(),
            Some(true)
        );
        assert_eq!(line, b"aaaaaaaa");
        assert_eq!(
            read_line_bounded(&mut reader, 8, &mut line).unwrap(),
            Some(false)
        );
        assert_eq!(line, b"short\r");
        assert_eq!(
            read_line_bounded(&mut reader, 8, &mut line).unwrap(),
            Some(false)
        );
        assert_eq!(line, b"last");
        assert_eq!(read_line_bounded(&mut reader, 8, &mut line).unwrap(), None);
    }

    #[test]
    fn records_pass_json_through_and_wrap_everything_else_as_raw() {
        let record = record_payload("service:41", br#"{"type":"event","n":[1]}"#, false);
        assert_eq!(
            record.unwrap().record,
            serde_json::json!({"type":"event","n":[1]})
        );
        let raw = record_payload("service:41", b"not json\r", false).unwrap();
        assert_eq!(
            raw.record,
            serde_json::json!({"type":"raw","line":"not json"})
        );
        let long = "é".repeat(600);
        let raw = record_payload("service:41", long.as_bytes(), true).unwrap();
        assert_eq!(raw.record["line"].as_str().unwrap().chars().count(), 512);
        // Even valid JSON is raw once truncated.
        let raw = record_payload("service:41", b"{}", true).unwrap();
        assert_eq!(raw.record["type"], "raw");
        assert!(record_payload("service:41", b"  \r", false).is_none());
    }

    #[test]
    fn only_profiles_with_a_profile_action_ask_for_switch_requests() {
        use serde_json::json;
        let parse = |value: serde_json::Value| {
            Profile::from_json(&serde_json::to_vec(&value).unwrap()).unwrap()
        };
        let key = json!({"type":"profile","direction":"next"});
        assert!(!uses_profile_action(&test_profiles::folders()));
        let mut flat = test_profiles::folders_json();
        flat["bindings"][0]["action"] = key.clone();
        assert!(uses_profile_action(&parse(flat)));
        let mut tile = test_profiles::folders_json();
        // A tile inside a folder page.
        tile["keypad"]["pages"][2]["slots"][1]["bindings"][0]["action"] = key.clone();
        assert!(uses_profile_action(&parse(tile)));
        let dial = parse(
            json!({"schema_version":1,"name":"Dial","bindings":[],"dial_functions":[
                {"name":"Volume","bindings":[{"gesture":"right","action":{"type":"volume","delta":5}}]},
                {"name":"Profiles","bindings":[{"gesture":"long_press","action":key}]}
            ]}),
        );
        assert!(uses_profile_action(&dial));
    }

    #[test]
    fn a_start_request_names_its_saved_profile_only_by_canonical_uuid() {
        let mut request: StartRequest = serde_json::from_value(serde_json::json!({
            "profile": test_profiles::folders_json(), "path_hex": "41",
            "execute": false, "software_mode": false,
            "profile_id": "00000000-0000-4000-8000-0000000000a1",
        }))
        .unwrap();
        assert!(check_request(&request).is_ok());
        for bad in ["../x", "00000000-0000-4000-8000-0000000000A1", ""] {
            request.profile_id = Some(bad.into());
            assert!(check_request(&request).is_err(), "{bad}");
        }
    }

    #[test]
    fn ids_args_and_execute_policy() {
        assert_eq!(service_id("4A42").unwrap(), "service:4a42");
        assert!(service_id("00").is_err());
        let args = service_args(Path::new("/p.json"), "4a", true, false, false);
        assert_eq!(
            args,
            [
                "run",
                "--profile",
                "/p.json",
                "--path-hex",
                "4a",
                "--execute"
            ]
            .map(OsString::from)
        );
        let args = service_args(Path::new("/p.json"), "4a", false, true, false);
        assert_eq!(args.last().unwrap(), "--software-mode");
        assert!(!args.iter().any(|arg| arg == "--execute"));
        assert!(!args.iter().any(|arg| arg == "--profile-switch-requests"));
        let args = service_args(Path::new("/p.json"), "4a", false, false, true);
        assert_eq!(args.last().unwrap(), "--profile-switch-requests");
        assert!(check_execute_supported(false).is_ok());
        assert_eq!(
            check_execute_supported(true).is_ok(),
            cfg!(target_os = "macos")
        );
        let request: StartRequest = serde_json::from_value(serde_json::json!({
            "profile": test_profiles::folders_json(), "path_hex": "41",
            "execute": false, "software_mode": false,
        }))
        .unwrap();
        assert!(!request.execute);
        // `execute` must be stated explicitly and unknown fields are refused.
        assert!(
            serde_json::from_value::<StartRequest>(serde_json::json!({
                "profile": test_profiles::folders_json(), "path_hex": "41", "software_mode": false,
            }))
            .is_err()
        );
    }

    #[test]
    fn binary_resolution_order_and_error() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path();
        let exe_dir = root.join("app/src-tauri/target/debug");
        fs::create_dir_all(&exe_dir).unwrap();
        fs::write(root.join("Cargo.toml"), "[workspace]\nmembers = []\n").unwrap();
        fs::write(root.join("app/Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();

        let error = resolve_service_binary(None, Some(&exe_dir), true).unwrap_err();
        assert!(error.contains("cargo build -p cooltek-sdk-cli --bin masterhub-service"));
        assert!(error.contains(&exe_dir.join(binary_name()).display().to_string()));
        assert!(error.contains(&root.join("target/release").display().to_string()));

        let dev = root.join("target/release").join(binary_name());
        fs::create_dir_all(dev.parent().unwrap()).unwrap();
        fs::write(&dev, b"").unwrap();
        assert_eq!(
            resolve_service_binary(None, Some(&exe_dir), true),
            Ok(dev.clone())
        );
        // Release builds never search workspaces.
        assert!(resolve_service_binary(None, Some(&exe_dir), false).is_err());

        let beside = exe_dir.join(binary_name());
        fs::write(&beside, b"").unwrap();
        assert_eq!(
            resolve_service_binary(None, Some(&exe_dir), true),
            Ok(beside.clone())
        );

        let custom = root.join("custom-service");
        fs::write(&custom, b"").unwrap();
        assert_eq!(
            resolve_service_binary(Some(custom.as_os_str()), Some(&exe_dir), true),
            Ok(custom.clone())
        );
        // Release builds ignore the override and do not suggest it.
        assert_eq!(
            resolve_service_binary(Some(custom.as_os_str()), Some(&exe_dir), false),
            Ok(beside.clone())
        );
        let error = resolve_service_binary(Some(custom.as_os_str()), None, false).unwrap_err();
        assert!(!error.contains("custom-service") && !error.contains(BINARY_ENV));
        // A missing override falls through to the next location.
        let missing = root.join("missing");
        assert_eq!(
            resolve_service_binary(Some(missing.as_os_str()), Some(&exe_dir), true),
            Ok(beside)
        );
        let error = resolve_service_binary(Some(missing.as_os_str()), None, true).unwrap_err();
        assert!(error.contains("missing") && error.contains(BINARY_ENV));
    }

    #[cfg(unix)]
    #[test]
    fn temp_profiles_are_private_and_removed_on_drop() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let directory = profile_dir(root.path());
        let snapshot = TempProfile::write(&directory, &test_profiles::folders()).unwrap();
        let other = TempProfile::write(&directory, &test_profiles::folders()).unwrap();
        assert_ne!(snapshot.path(), other.path());
        let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&directory), 0o700);
        assert_eq!(mode(snapshot.path()), 0o600);
        let written = Profile::from_json(&fs::read(snapshot.path()).unwrap()).unwrap();
        assert_eq!(written, test_profiles::folders());
        let path = snapshot.path().to_path_buf();
        drop(snapshot);
        assert!(!path.exists());
        let stale = other.path().to_path_buf();
        std::mem::forget(other);
        clean_stale_profiles(&directory);
        assert!(!stale.exists());
    }

    /// Fake child harness: runs `sh -c script` through the real supervisor.
    #[cfg(unix)]
    mod process {
        use super::*;

        pub fn collector() -> (Emit, Arc<Mutex<Vec<ServiceMessage>>>) {
            let messages = Arc::new(Mutex::new(Vec::new()));
            let sink = Arc::clone(&messages);
            let emit: Emit = Arc::new(move |message| sink.lock().unwrap().push(message));
            (emit, messages)
        }

        pub fn start_script(
            services: &Services,
            id: &str,
            script: &str,
            profile: Option<TempProfile>,
            emit: Emit,
        ) -> Result<String, String> {
            services.start(
                id.into(),
                false,
                Path::new("/bin/sh"),
                vec!["-c".into(), script.into()],
                profile,
                emit,
            )
        }

        pub fn statuses(messages: &Mutex<Vec<ServiceMessage>>) -> Vec<StatusPayload> {
            messages
                .lock()
                .unwrap()
                .iter()
                .filter_map(|message| match message {
                    ServiceMessage::Status(status) => Some(status.clone()),
                    ServiceMessage::Record(_) => None,
                })
                .collect()
        }

        pub fn records(messages: &Mutex<Vec<ServiceMessage>>) -> Vec<Value> {
            messages
                .lock()
                .unwrap()
                .iter()
                .filter_map(|message| match message {
                    ServiceMessage::Record(record) => Some(record.record.clone()),
                    ServiceMessage::Status(_) => None,
                })
                .collect()
        }

        pub fn states(messages: &Mutex<Vec<ServiceMessage>>) -> Vec<ServiceState> {
            statuses(messages)
                .iter()
                .map(|status| status.state)
                .collect()
        }

        pub fn wait_done(services: &Services, id: &str) {
            assert!(
                wait_until(Instant::now() + Duration::from_secs(5), || !services
                    .is_running(id)),
                "service never exited"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn child_records_are_relayed_then_exited_with_code_and_stderr_tail() {
        use process::*;
        let services = Services::default();
        let (emit, messages) = collector();
        let root = tempfile::tempdir().unwrap();
        let profile = TempProfile::write(root.path(), &test_profiles::folders()).unwrap();
        let profile_path = profile.path().to_path_buf();
        let script = r#"
            printf '{"type":"service_start"}\n'
            printf 'garbage\n'
            head -c 70000 /dev/zero | tr '\0' 'a'; printf '\n'
            printf '\n{"type":"service_end","n":2}\n'
            head -c 6000 /dev/zero | tr '\0' 'e' >&2
            printf 'fatal: boom' >&2
            exit 3
        "#;
        start_script(&services, "service:41", script, Some(profile), emit).unwrap();
        wait_done(&services, "service:41");
        let records = records(&messages);
        assert_eq!(records.len(), 4);
        assert_eq!(records[0], serde_json::json!({"type":"service_start"}));
        assert_eq!(
            records[1],
            serde_json::json!({"type":"raw","line":"garbage"})
        );
        assert_eq!(records[2]["type"], "raw");
        assert_eq!(records[2]["line"].as_str().unwrap().len(), RAW_LINE_CHARS);
        assert_eq!(records[3]["n"], 2);
        let statuses = statuses(&messages);
        assert_eq!(
            statuses.iter().map(|s| s.state).collect::<Vec<_>>(),
            [
                ServiceState::Starting,
                ServiceState::Running,
                ServiceState::Exited
            ]
        );
        let exited = statuses.last().unwrap();
        assert_eq!(exited.exit_code, Some(Some(3)));
        let tail = exited.stderr_tail.as_deref().unwrap();
        assert_eq!(tail.len(), STDERR_TAIL_BYTES);
        assert!(tail.ends_with("fatal: boom"));
        assert!(!profile_path.exists());
        // The finished id can be started again.
        let (emit, _) = collector();
        start_script(&services, "service:41", "exit 0", None, emit).unwrap();
        services.stop("service:41", STOP_GRACE).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn duplicate_start_is_rejected_without_events_and_stop_uses_sigint() {
        use process::*;
        let services = Services::default();
        let (emit, messages) = collector();
        // Like the real service: SIGINT triggers cleanup and a clean exit.
        let script = r#"trap 'printf "{\"type\":\"service_end\"}\n"; exit 0' INT
            printf '{"type":"service_start"}\n'
            while :; do sleep 0.02; done"#;
        start_script(&services, "service:42", script, None, emit).unwrap();
        let (other, other_messages) = collector();
        let error = start_script(&services, "service:42", "exit 0", None, other).unwrap_err();
        assert!(error.contains("already running"));
        assert!(other_messages.lock().unwrap().is_empty());

        thread::sleep(Duration::from_millis(100));
        let started = Instant::now();
        services.stop("service:42", STOP_GRACE).unwrap();
        assert!(started.elapsed() < Duration::from_secs(2));
        assert_eq!(
            states(&messages),
            [
                ServiceState::Starting,
                ServiceState::Running,
                ServiceState::Stopping,
                ServiceState::Exited
            ]
        );
        assert_eq!(statuses(&messages)[3].exit_code, Some(Some(0)));
        assert_eq!(
            records(&messages).last().unwrap(),
            &serde_json::json!({"type":"service_end"})
        );
        assert!(!services.is_running("service:42"));
        services.stop("service:42", STOP_GRACE).unwrap();
        services.stop("service:unknown", STOP_GRACE).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_child_ignoring_sigint_is_killed_with_its_process_group() {
        use process::*;
        let services = Services::default();
        let (emit, messages) = collector();
        // The background sleeper models an in-flight system tool.
        let script = "trap '' INT; sleep 30 & exec sleep 30";
        start_script(&services, "service:43", script, None, emit).unwrap();
        thread::sleep(Duration::from_millis(100));
        let started = Instant::now();
        services
            .stop("service:43", Duration::from_millis(300))
            .unwrap();
        let elapsed = started.elapsed();
        assert!(elapsed >= Duration::from_millis(300));
        assert!(elapsed < Duration::from_secs(2), "stop took {elapsed:?}");
        let exited = statuses(&messages).pop().unwrap();
        assert_eq!(exited.state, ServiceState::Exited);
        // Ended by SIGKILL, so there is no exit code.
        assert_eq!(exited.exit_code, Some(None));
    }

    #[cfg(unix)]
    #[test]
    fn stop_all_interrupts_kills_and_removes_snapshots_without_blocking() {
        use process::*;
        let services = Services::default();
        let root = tempfile::tempdir().unwrap();
        let (emit, messages) = collector();
        let polite = TempProfile::write(root.path(), &test_profiles::folders()).unwrap();
        let polite_path = polite.path().to_path_buf();
        start_script(
            &services,
            "service:44",
            "trap 'exit 0' INT; while :; do sleep 0.02; done",
            Some(polite),
            Arc::clone(&emit),
        )
        .unwrap();
        let stubborn = TempProfile::write(root.path(), &test_profiles::folders()).unwrap();
        let stubborn_path = stubborn.path().to_path_buf();
        start_script(
            &services,
            "service:45",
            "trap '' INT; exec sleep 30",
            Some(stubborn),
            emit,
        )
        .unwrap();
        thread::sleep(Duration::from_millis(100));
        let started = Instant::now();
        services.stop_all(Duration::from_millis(300));
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(!polite_path.exists() && !stubborn_path.exists());
        wait_done(&services, "service:44");
        wait_done(&services, "service:45");
        let exits: Vec<_> = statuses(&messages)
            .into_iter()
            .filter(|status| status.state == ServiceState::Exited)
            .map(|status| (status.service_id, status.exit_code))
            .collect();
        assert!(exits.contains(&("service:44".into(), Some(Some(0)))));
        assert!(exits.contains(&("service:45".into(), Some(None))));
    }

    /// Smoke against the real binary (no device needed): `OPEN_STREAM_HUB_SERVICE=
    /// <repo>/target/debug/masterhub-service cargo test -- --ignored real_service`.
    /// An absent interface must fail after the profile snapshot was accepted.
    #[cfg(unix)]
    #[test]
    #[ignore = "needs a built masterhub-service binary"]
    fn real_service_rejects_an_absent_interface_after_loading_the_snapshot() {
        use process::*;
        let program =
            resolve_service_binary(std::env::var_os(BINARY_ENV).as_deref(), None, true).unwrap();
        let services = Services::default();
        let root = tempfile::tempdir().unwrap();
        let (emit, messages) = collector();
        let request = StartRequest {
            profile: test_profiles::folders(),
            path_hex: encode_hex(b"/no/such/opensteamhub/interface"),
            execute: false,
            software_mode: false,
            profile_id: None,
        };
        let id = start_service(
            &services,
            request,
            false,
            Ok(root.path()),
            Ok(program),
            |_| Ok(()),
            emit,
        )
        .unwrap();
        wait_done(&services, &id);
        let exited = statuses(&messages).pop().unwrap();
        assert_eq!(exited.state, ServiceState::Exited);
        assert_eq!(exited.exit_code, Some(Some(1)));
        let stderr = exited.stderr_tail.unwrap();
        assert!(stderr.starts_with("masterhub-service: "), "{stderr}");
        assert!(!stderr.contains("profile"), "{stderr}");
        assert_eq!(fs::read_dir(profile_dir(root.path())).unwrap().count(), 0);
        eprintln!("real service stderr: {stderr}");
    }

    #[cfg(unix)]
    #[test]
    fn start_service_validates_stops_the_feed_and_reports_failures() {
        use process::*;
        let services = Services::default();
        let root = tempfile::tempdir().unwrap();
        let request = |execute| StartRequest {
            profile: test_profiles::folders(),
            path_hex: "4A".into(),
            execute,
            software_mode: true,
            profile_id: None,
        };

        // A missing binary: one `failed` status, no snapshot left behind.
        let (emit, messages) = collector();
        let error = start_service(
            &services,
            request(false),
            false,
            Ok(root.path()),
            Err("could not find masterhub-service".into()),
            |_| panic!("the feed must not be stopped before the binary is found"),
            emit,
        )
        .unwrap_err();
        assert!(error.contains("could not find"));
        assert_eq!(
            statuses(&messages),
            [StatusPayload::failed("service:4a", false, error)]
        );

        // Spawn failure: starting, then failed.
        let (emit, messages) = collector();
        start_service(
            &services,
            request(false),
            false,
            Ok(root.path()),
            Ok(root.path().join("no-such-binary")),
            |_| Ok(()),
            emit,
        )
        .unwrap_err();
        assert_eq!(
            states(&messages),
            [ServiceState::Starting, ServiceState::Failed]
        );
        let directory = profile_dir(root.path());
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 0);

        // A fake service that echoes its arguments as a JSON record.
        let fake = root.path().join("fake-service");
        fs::write(
            &fake,
            "#!/bin/sh\nprintf '{\"argc\":%d,\"args\":\"%s\"}\\n' \"$#\" \"$*\"\n\
             test -f \"$3\" || exit 9\n",
        )
        .unwrap();
        fs::set_permissions(&fake, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        let (emit, messages) = collector();
        let mut stopped_feed = None;
        let id = start_service(
            &services,
            request(false),
            false,
            Ok(root.path()),
            Ok(fake.clone()),
            |feed_id| {
                stopped_feed = Some(feed_id.to_string());
                Ok(())
            },
            emit,
        )
        .unwrap();
        assert_eq!(id, "service:4a");
        assert_eq!(stopped_feed.as_deref(), Some("live:4a"));
        wait_done(&services, &id);
        let record = &records(&messages)[0];
        assert_eq!(record["argc"], 6);
        let args = record["args"].as_str().unwrap();
        assert!(
            args.starts_with("run --profile ") && args.ends_with(" --path-hex 4a --software-mode")
        );
        assert_eq!(statuses(&messages).last().unwrap().exit_code, Some(Some(0)));
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 0);

        // Execute off macOS and invalid profiles fail before spawning.
        if !cfg!(target_os = "macos") {
            let (emit, messages) = collector();
            assert!(
                start_service(
                    &services,
                    request(true),
                    false,
                    Ok(root.path()),
                    Ok(fake.clone()),
                    |_| Ok(()),
                    emit
                )
                .is_err()
            );
            assert_eq!(states(&messages), [ServiceState::Failed]);
        }
        let mut invalid = request(false);
        invalid.profile.name.clear();
        let (emit, messages) = collector();
        assert!(
            start_service(
                &services,
                invalid,
                false,
                Ok(root.path()),
                Ok(fake),
                |_| Ok(()),
                emit
            )
            .is_err()
        );
        assert_eq!(states(&messages), [ServiceState::Failed]);
    }
    #[cfg(unix)]
    #[test]
    fn grandchildren_are_killed_once_the_service_exits() {
        use process::*;
        let services = Services::default();
        let (emit, messages) = collector();
        // The service exits on its own and leaves a background tool behind.
        let script = "sleep 30 >/dev/null 2>&1 & printf '{\"pid\":%d}\\n' $!; exit 0";
        start_script(&services, "service:46", script, None, emit).unwrap();
        wait_done(&services, "service:46");
        let pid = records(&messages)[0]["pid"].as_i64().unwrap();
        let pid = libc::pid_t::try_from(pid).unwrap();
        // SAFETY: signal 0 only checks whether the process still exists.
        let alive = || unsafe { libc::kill(pid, 0) } == 0;
        assert!(
            wait_until(Instant::now() + Duration::from_secs(3), || !alive()),
            "the grandchild outlived the service"
        );
    }

    #[cfg(unix)]
    #[test]
    fn stop_many_stops_services_in_parallel() {
        use process::*;
        let services = Services::default();
        let (emit, messages) = collector();
        let ids: Vec<String> = (47..50).map(|n| format!("service:{n}")).collect();
        for id in &ids {
            start_script(
                &services,
                id,
                "trap '' INT; exec sleep 30",
                None,
                Arc::clone(&emit),
            )
            .unwrap();
        }
        thread::sleep(Duration::from_millis(100));
        let started = Instant::now();
        services.stop_many(&ids, Duration::from_millis(400));
        let elapsed = started.elapsed();
        // Serially this would take at least 1.2 s.
        assert!(elapsed < Duration::from_millis(1_100), "took {elapsed:?}");
        assert!(services.running_ids().is_empty());
        let stopping = states(&messages)
            .into_iter()
            .filter(|state| *state == ServiceState::Stopping)
            .count();
        assert_eq!(stopping, 3);
    }

    #[cfg(unix)]
    #[test]
    fn nothing_starts_once_stop_all_began() {
        use process::*;
        let services = Services::default();
        services.stop_all(Duration::from_millis(10));
        assert!(services.is_closed());
        let (emit, messages) = collector();
        let error = start_script(&services, "service:50", "exit 0", None, emit).unwrap_err();
        assert_eq!(error, SHUTTING_DOWN);
        assert!(messages.lock().unwrap().is_empty());
        let root = tempfile::tempdir().unwrap();
        let (emit, messages) = collector();
        let request = StartRequest {
            profile: test_profiles::folders(),
            path_hex: "4A".into(),
            execute: false,
            software_mode: false,
            profile_id: None,
        };
        let error = start_service(
            &services,
            request,
            false,
            Ok(root.path()),
            Ok(PathBuf::from("/bin/sh")),
            |_| panic!("no feed is stopped during shutdown"),
            emit,
        )
        .unwrap_err();
        assert_eq!(error, SHUTTING_DOWN);
        assert!(messages.lock().unwrap().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn a_live_feed_is_refused_while_a_service_owns_the_interface() {
        use process::*;
        let services = Services::default();
        let live = |path_hex: &str| feed::FeedSource::Live {
            path_hex: path_hex.into(),
        };
        assert!(check_feed_allowed(&services, &live("4A")).is_ok());
        let (emit, _) = collector();
        start_script(&services, "service:4a", "exec sleep 30", None, emit).unwrap();
        let error = check_feed_allowed(&services, &live("4A")).unwrap_err();
        assert!(error.contains("service:4a"), "{error}");
        assert!(check_feed_allowed(&services, &live("4B")).is_ok());
        assert!(
            check_feed_allowed(
                &services,
                &feed::FeedSource::Demo {
                    demo: replay::DemoKind::Dial
                }
            )
            .is_ok()
        );
        assert!(check_feed_allowed(&services, &live("00")).is_err());
        services
            .stop("service:4a", Duration::from_millis(200))
            .unwrap();
        assert!(check_feed_allowed(&services, &live("4A")).is_ok());
    }
}
