//! Runs plugin processes and carries presses to them (see `docs/plugins-design.md`).
//!
//! The app starts one instance of each plugin that an executing (Live) module run uses, and stops it when no
//! executing run uses it, and on quit. Test runs never start a plugin: they only log what would run. A plugin gets
//! the socket path, its id and a token that is new at every start in its environment (`OSH_PLUGIN_SOCKET`,
//! `OSH_PLUGIN_ID`, `OSH_PLUGIN_TOKEN`), connects to `<app data>/plugins/plugins.sock` (0700 directory) and sends
//! `hello`. It runs in its own process group, which is killed when it stops, so helpers it started do not outlive it.
//!
//! One JSON object per line, in both directions, each line at most [`MAX_LINE`] bytes:
//!
//! - plugin → app: `{"type":"hello","id":"com.example.x","token":"…"}` first, within [`HELLO_TIMEOUT`]; then
//!   `{"type":"result","request":7,"ok":true}` or `…"ok":false,"error":"…"}`, `{"type":"set_variable","id":1,
//!   "name":"room","value":"desk"}` (only variables its manifest lists, at most 10 a second), `{"type":"log",
//!   "message":"…"}` (at most 1 KiB and 20 a second; more are counted and dropped), `{"type":"pong"}`.
//! - app → plugin: `{"type":"welcome"}`, `{"type":"invoke","request":7,"action":"set","params":{…},"timeout_ms":
//!   15000}`, `{"type":"ping"}`, and `{"type":"reply","id":1,"ok":…}` for each `set_variable`.
//!
//! An invoke is answered with the plugin's `result`, or fails when the plugin is not running, has 8 invokes in
//! flight, stops before answering, or (on the module's side) answers too late. After [`IDLE_PING`] without a line the
//! app sends `ping`, and a plugin silent for [`PONG_TIMEOUT`] more is killed and restarted, as is one that exits or
//! closes its connection, or one that has not said hello [`START_TIMEOUT`] after it started: after 1, 2, 4, 8, 16,
//! then 30 s. After [`MAX_FAILURES`] failures within
//! [`FAILURE_WINDOW`] it stays stopped until a profile that uses it is executed again.
//!
//! A plugin starts only when the user has enabled exactly the `plugin.json` it reads at that start (its fingerprint is
//! the approved one, see `plugins.rs`). One that is not enabled, or whose manifest changed since, is `not_enabled`:
//! not a failure, so nothing is counted and it is not restarted; its presses fail at once with the reason. Enabling
//! it while a Live run uses it starts it at once, and disabling it stops it.
//!
//! Secret parameters are filled in here, from the Keychain ([`PluginSecrets`]), just before an invoke is checked and
//! sent; a profile never holds them. A press whose secrets are all held in memory (or that has none) is sent at once;
//! one that needs a Keychain read is handed to that plugin's own invoke thread, so a read that waits for the user holds up
//! neither the module run that relays it nor other plugins' presses. A press still waiting when its deadline passes is
//! answered with an error and never sent. Known secret values are replaced with `[secret]` in what
//! the plugin sends back (its log, stderr and error texts) before it is kept or passed on.

use crate::{
    plugin_secrets::PluginSecrets,
    plugins::{self, Approval, Approved, Manifest},
};
use cooltek_sdk_service::PluginValue;
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, VecDeque},
    fs,
    io::{self, BufRead, BufReader, Read, Write},
    os::unix::{fs::PermissionsExt, net::UnixListener, net::UnixStream},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Condvar, Mutex, MutexGuard, Weak,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

pub const SOCKET_FILE: &str = "plugins.sock";
/// The longest line either way; a longer line from a plugin closes its connection.
pub const MAX_LINE: usize = 64 * 1024;
/// Connections that have not said `hello` yet; more are refused.
const MAX_UNNAMED: usize = 8;
/// How long a new connection has to say `hello`.
pub const HELLO_TIMEOUT: Duration = Duration::from_secs(2);
/// Invokes one plugin may have in flight.
const MAX_IN_FLIGHT: usize = 8;
/// The longest an invoke waits, whatever the module asks for.
const MAX_INVOKE: Duration = Duration::from_secs(15);
/// After this long without a line from a plugin, the app sends `ping`.
pub const IDLE_PING: Duration = Duration::from_millis(if cfg!(test) { 300 } else { 10_000 });
/// How long a pinged plugin has to send any line.
pub const PONG_TIMEOUT: Duration = Duration::from_millis(if cfg!(test) { 300 } else { 5_000 });
/// How long a started plugin has to connect and say hello before it is killed and restarted.
pub const START_TIMEOUT: Duration = Duration::from_millis(if cfg!(test) { 1_000 } else { 10_000 });
/// Waits before each restart, the last one repeating.
const BACKOFF: [u64; 6] = [1, 2, 4, 8, 16, 30];
/// Failures within [`FAILURE_WINDOW`] after which a plugin stays stopped.
const MAX_FAILURES: usize = 5;
const FAILURE_WINDOW: Duration = Duration::from_secs(300);
/// `set_variable` writes a plugin may make per second.
const MAX_SETS_PER_SECOND: usize = 10;
/// Log lines a plugin may send per second, and the longest kept.
const MAX_LOGS_PER_SECOND: usize = 20;
/// Input events one plugin is sent a second; more are dropped and counted in its log.
const MAX_EVENTS_PER_SECOND: usize = 50;
/// Input events waiting for a plugin that is not reading its socket; more are dropped and counted.
const MAX_QUEUED_EVENTS: usize = 64;
/// Bytes waiting to be written to one plugin. Past this an input event is dropped, and any other line ends the
/// connection: a plugin that reads nothing while it is sent this much is not working.
const MAX_OUTBOX_BYTES: usize = 1024 * 1024;
const MAX_LOG_BYTES: usize = 1024;
/// Log lines kept per plugin for Settings.
const KEPT_LOGS: usize = 50;
/// The longest error text from a plugin passed on to a module.
const MAX_ERROR_BYTES: usize = 512;
/// How long a write to a plugin may block: a refused hello's error, or one wait of a connection's writer, which
/// then checks the connection still stands and goes on.
const WRITE_TIMEOUT: Duration = Duration::from_secs(1);
/// How long a stopping plugin has after SIGTERM before SIGKILL.
const STOP_GRACE: Duration = Duration::from_secs(1);
const TICK: Duration = Duration::from_millis(100);
/// How long a plugin keeps running after the last run that used it stopped executing, so a module run that restarts
/// (a large profile change, a replug) does not restart its plugins.
pub const LINGER: Duration = Duration::from_millis(if cfg!(test) { 300 } else { 5_000 });

/// Answers one invoke; called once, on the host's answer thread.
pub type Reply = Box<dyn FnOnce(Result<(), String>) + Send>;

/// An invoke handed to an invoke thread: the run it came from, the record, its answer and when it arrived.
type Queued = (String, Value, Reply, Instant);

/// The parameters the profile gave and those with the secrets filled in.
type Filled = (BTreeMap<String, PluginValue>, BTreeMap<String, PluginValue>);

struct Pending {
    deadline: Instant,
    reply: Reply,
}

/// One line waiting for a connection's writer.
struct Line {
    message: Value,
    /// Its length as queued, counted in [`Outbox::bytes`].
    len: usize,
    event: bool,
    /// For an invoke, when the module gives up on it: it is not written after that, and its `timeout_ms` is what is
    /// left when it is.
    deadline: Option<Instant>,
}

/// Lines waiting for a connection's writer, in the order they were sent.
#[derive(Default)]
struct Outbox {
    lines: VecDeque<Line>,
    bytes: usize,
    events: usize,
}

/// A connected plugin's socket and when it last said anything. Every line to the plugin goes through its outbox and
/// is written, in order, by the connection's writer ([`PluginHost::write_lines`]): no sender waits for the plugin to
/// read, and an event is never written between the halves of another line.
struct Conn {
    /// Only shut down from here, which ends the writer and the reader.
    stream: UnixStream,
    /// Why this side shut the connection down, if it did, for the reader to report.
    closed: Mutex<Option<&'static str>>,
    outbox: Mutex<Outbox>,
    queued: Condvar,
    last_seen: Mutex<Instant>,
    pinged: Mutex<Option<Instant>>,
}

impl Conn {
    fn new(stream: UnixStream) -> Self {
        Self {
            stream,
            closed: Mutex::new(None),
            outbox: Mutex::new(Outbox::default()),
            queued: Condvar::new(),
            last_seen: Mutex::new(Instant::now()),
            pinged: Mutex::new(None),
        }
    }

    /// Queues one line. Fails, and ends the connection, when the plugin has left too much unread.
    fn send(&self, message: &Value) -> io::Result<()> {
        self.queue_or_close(message, None)
    }

    /// Queues one invoke, which is dropped instead of written once `deadline` passes.
    fn send_invoke(&self, message: &Value, deadline: Instant) -> io::Result<()> {
        self.queue_or_close(message, Some(deadline))
    }

    fn queue_or_close(&self, message: &Value, deadline: Option<Instant>) -> io::Result<()> {
        if self.queue(message, false, deadline) {
            return Ok(());
        }
        const WHY: &str = "the plugin was sent more than it read";
        self.close(WHY);
        Err(io::Error::other(WHY))
    }

    /// Queues one input event, or says it was dropped: [`MAX_QUEUED_EVENTS`] are waiting already, or the outbox is
    /// full.
    fn queue_event(&self, message: &Value) -> bool {
        self.queue(message, true, None)
    }

    fn queue(&self, message: &Value, event: bool, deadline: Option<Instant>) -> bool {
        let len = message.to_string().len() + 1;
        let mut outbox = self
            .outbox
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if outbox.bytes + len > MAX_OUTBOX_BYTES || (event && outbox.events >= MAX_QUEUED_EVENTS) {
            return false;
        }
        outbox.bytes += len;
        outbox.events += usize::from(event);
        outbox.lines.push_back(Line {
            message: message.clone(),
            len,
            event,
            deadline,
        });
        self.queued.notify_one();
        true
    }

    /// The next line to write, waiting at most `wait` for one. An invoke whose deadline passed while it waited is
    /// left out: the module has given up on it, and its pending entry has expired.
    fn next_line(&self, wait: Duration) -> Option<Vec<u8>> {
        let mut outbox = self
            .outbox
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if outbox.lines.is_empty() {
            outbox = self
                .queued
                .wait_timeout(outbox, wait)
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .0;
        }
        while let Some(mut line) = outbox.lines.pop_front() {
            outbox.bytes -= line.len;
            outbox.events -= usize::from(line.event);
            if let Some(deadline) = line.deadline {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    continue;
                }
                line.message["timeout_ms"] = json!(left.as_millis().max(1) as u64);
            }
            let mut bytes = line.message.to_string().into_bytes();
            bytes.push(b'\n');
            return Some(bytes);
        }
        None
    }

    fn close(&self, why: &'static str) {
        self.closed
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get_or_insert(why);
        let _ = self.stream.shutdown(std::net::Shutdown::Both);
    }

    /// Why this side closed the connection, if it did.
    fn closed_why(&self) -> Option<&'static str> {
        *self
            .closed
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn seen(&self) {
        *self
            .last_seen
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Instant::now();
        *self
            .pinged
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    }
}

/// A sliding one-second window for a rate limit.
#[derive(Default)]
struct Window(VecDeque<Instant>);

impl Window {
    fn allow(&mut self, per_second: usize) -> bool {
        let now = Instant::now();
        while self
            .0
            .front()
            .is_some_and(|at| now.duration_since(*at) >= Duration::from_secs(1))
        {
            self.0.pop_front();
        }
        if self.0.len() >= per_second {
            return false;
        }
        self.0.push_back(now);
        true
    }
}

struct Instance {
    manifest: Option<Manifest>,
    /// The fingerprint of the manifest the running process was started from.
    fingerprint: Option<String>,
    /// Not started because the user has not enabled this `plugin.json`; `last_error` says why.
    not_enabled: bool,
    folder: PathBuf,
    token: String,
    /// Grows at every start, so a connection or a reader of an older process can tell it is stale.
    generation: u64,
    child: Option<Child>,
    /// When the running process was started; it must say hello within [`START_TIMEOUT`].
    started_at: Instant,
    conn: Option<Arc<Conn>>,
    pending: HashMap<u64, Pending>,
    failures: VecDeque<Instant>,
    restart_at: Option<Instant>,
    gave_up: bool,
    /// Since when no executing run uses it; it stops after [`LINGER`].
    unused_since: Option<Instant>,
    last_error: Option<String>,
    logs: VecDeque<String>,
    dropped_logs: u64,
    sets: Window,
    log_rate: Window,
    event_rate: Window,
    /// Events dropped by the rate limit since the last one sent, told in the log with the next one sent.
    dropped_events: u64,
}

impl Instance {
    fn new(folder: PathBuf) -> Self {
        Self {
            manifest: None,
            fingerprint: None,
            not_enabled: false,
            folder,
            token: String::new(),
            generation: 0,
            child: None,
            started_at: Instant::now(),
            conn: None,
            pending: HashMap::new(),
            failures: VecDeque::new(),
            restart_at: None,
            gave_up: false,
            unused_since: None,
            last_error: None,
            logs: VecDeque::new(),
            dropped_logs: 0,
            sets: Window::default(),
            log_rate: Window::default(),
            event_rate: Window::default(),
            dropped_events: 0,
        }
    }

    fn keep_log(&mut self, line: String) {
        if self.logs.len() == KEPT_LOGS {
            self.logs.pop_front();
        }
        self.logs.push_back(line);
    }

    fn state(&self) -> &'static str {
        match (&self.child, &self.conn) {
            (Some(_), Some(_)) => "running",
            (Some(_), None) => "starting",
            (None, _) if self.not_enabled => "not_enabled",
            (None, _) if self.gave_up => "stopped",
            (None, _) if self.restart_at.is_some() => "restarting",
            (None, _) => "unusable",
        }
    }
}

#[derive(Default)]
struct State {
    /// The plugins each executing module run uses.
    usage: HashMap<String, BTreeSet<String>>,
    instances: HashMap<String, Instance>,
    /// The fingerprint the user enabled for each plugin.
    approved: Approved,
    next_request: u64,
}

impl State {
    fn needed(&self) -> BTreeSet<String> {
        self.usage.values().flatten().cloned().collect()
    }
}

/// Sends a module's variable write on to every running module, as ingress does.
pub type Forward = crate::ingress::Forward;

struct Inner {
    dir: PathBuf,
    socket: PathBuf,
    forward: Forward,
    secrets: PluginSecrets,
    state: Mutex<State>,
    stopped: AtomicBool,
    answers: Mutex<mpsc::Sender<(Reply, Result<(), String>)>>,
    /// One invoke thread per plugin that needed a Keychain read, fed the presses waiting for one, in order. Bounded by
    /// the plugins that run with a manifest, since only their presses can need a read.
    workers: Mutex<HashMap<String, mpsc::Sender<Queued>>>,
    unnamed: AtomicUsize,
}

/// What Settings shows about one running or wanted plugin.
#[derive(Clone, Debug, Serialize)]
pub struct Status {
    pub id: String,
    pub state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub logs: Vec<String>,
    pub dropped_logs: u64,
}

/// The plugin host; clones share it.
#[derive(Clone)]
pub struct PluginHost(Arc<Inner>);

/// The `input` of a module's `plugin_invoke` or `plugin_event` record, rebuilt from its known fields so nothing else
/// passes to the plugin: `{"gesture":…,"control":{"module":…,"index":…}}`, with `"position":{"raw":…,"maximum":…}`
/// for a fader. `None` when it is missing or malformed.
fn input_of(record: &Value) -> Option<Value> {
    let input = &record["input"];
    let name = |value: &Value| {
        value
            .as_str()
            .filter(|text| {
                !text.is_empty()
                    && text.len() <= 32
                    && text
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
            })
            .map(str::to_string)
    };
    let gesture = name(&input["gesture"])?;
    let module = name(&input["control"]["module"])?;
    let index = input["control"]["index"]
        .as_u64()
        .filter(|index| *index <= 255)?;
    let mut rebuilt = json!({"gesture": gesture, "control": {"module": module, "index": index}});
    if !input["position"].is_null() {
        let raw = input["position"]["raw"]
            .as_u64()
            .filter(|raw| *raw <= 65_535)?;
        let maximum = input["position"]["maximum"]
            .as_u64()
            .filter(|max| *max <= 65_535)?;
        rebuilt["position"] = json!({"raw": raw, "maximum": maximum});
    }
    Some(rebuilt)
}

fn truncate(text: &str, bytes: usize) -> String {
    if text.len() <= bytes {
        return text.to_string();
    }
    let mut end = bytes;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

/// Reads one line of at most `max` bytes. `Ok(None)` is end of file; a longer line is an error.
fn read_line(reader: &mut impl BufRead, max: usize, line: &mut Vec<u8>) -> io::Result<Option<()>> {
    loop {
        let available = match reader.fill_buf() {
            Ok(available) => available,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        if available.is_empty() {
            return Ok(None);
        }
        let (taken, done) = match available.iter().position(|&byte| byte == b'\n') {
            Some(at) => (at + 1, true),
            None => (available.len(), false),
        };
        line.extend_from_slice(&available[..taken]);
        reader.consume(taken);
        if line.len() > max + 1 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "line too long"));
        }
        if done {
            line.pop();
            return Ok(Some(()));
        }
    }
}

fn kill_group(child: &mut Child) {
    let pid = child.id() as i32;
    // SAFETY: signals the plugin's own process group, which it leads (started with process_group(0)).
    unsafe {
        libc::killpg(pid, libc::SIGTERM);
    }
    let deadline = Instant::now() + STOP_GRACE;
    while Instant::now() < deadline {
        if matches!(child.try_wait(), Ok(Some(_))) {
            // The leader is gone; helpers in its group may not be.
            unsafe {
                libc::killpg(pid, libc::SIGKILL);
            }
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }
    unsafe {
        libc::killpg(pid, libc::SIGKILL);
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// Stops a process in the background, so no caller waits for its grace period.
fn stop_child(mut child: Child) {
    let _ = thread::Builder::new()
        .name("plugin-stop".into())
        .spawn(move || kill_group(&mut child));
}

impl PluginHost {
    /// Binds the socket in `plugins_dir` (made 0700, replacing a stale socket) and starts serving. `approved` is what
    /// the user has enabled, `secrets` fills secret parameters in, and `forward` carries a plugin's `set_variable` to
    /// the running modules.
    pub fn start(
        plugins_dir: &Path,
        approved: Approved,
        secrets: PluginSecrets,
        forward: Forward,
    ) -> Result<Self, String> {
        crate::ingress::prepare_private_dir(plugins_dir, "plugins", "plugins cannot run")?;
        let socket = plugins_dir.join(SOCKET_FILE);
        match fs::remove_file(&socket) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => {
                return Err(format!("could not remove the old plugin socket: {error}"));
            }
            _ => {}
        }
        let listener = UnixListener::bind(&socket)
            .map_err(|error| format!("could not bind the plugin socket: {error}"))?;
        fs::set_permissions(&socket, fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("could not protect the plugin socket: {error}"))?;
        let (answers, answered) = mpsc::channel::<(Reply, Result<(), String>)>();
        let host = Self(Arc::new(Inner {
            dir: plugins_dir.to_path_buf(),
            socket,
            forward,
            secrets,
            state: Mutex::new(State {
                approved,
                ..State::default()
            }),
            stopped: AtomicBool::new(false),
            answers: Mutex::new(answers),
            workers: Mutex::new(HashMap::new()),
            unnamed: AtomicUsize::new(0),
        }));
        let spawn = |name: &str, run: Box<dyn FnOnce() + Send>| {
            thread::Builder::new()
                .name(name.into())
                .spawn(run)
                .map(drop)
                .map_err(|error| format!("could not start the plugin host: {error}"))
        };
        spawn(
            "plugin-answers",
            Box::new(move || {
                for (reply, result) in answered {
                    reply(result);
                }
            }),
        )?;
        let accepting = host.clone();
        spawn(
            "plugin-accept",
            Box::new(move || accepting.accept(listener)),
        )?;
        let ticking = host.clone();
        spawn(
            "plugin-tick",
            Box::new(move || {
                while !ticking.0.stopped.load(Ordering::SeqCst) {
                    ticking.tick();
                    thread::sleep(TICK);
                }
            }),
        )?;
        Ok(host)
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.0
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn answer(&self, reply: Reply, result: Result<(), String>) {
        let _ = self
            .0
            .answers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .send((reply, result));
    }

    /// Records which plugins the module run `service_id` uses while it executes its actions (`None` once it stops
    /// executing or ends), then starts the plugins now needed and stops the ones no longer needed. A plugin this run
    /// starts using starts fresh, even after it gave up and even while another run still uses it.
    pub fn set_usage(&self, service_id: &str, plugins: Option<BTreeSet<String>>) {
        if self.0.stopped.load(Ordering::SeqCst) {
            return;
        }
        let mut guard = self.lock();
        let state = &mut *guard;
        let before = state.needed();
        let previous = state.usage.get(service_id).cloned().unwrap_or_default();
        match plugins.filter(|plugins| !plugins.is_empty()) {
            Some(plugins) => state.usage.insert(service_id.to_string(), plugins),
            None => state.usage.remove(service_id),
        };
        let after = state.needed();
        let current = state.usage.get(service_id).cloned().unwrap_or_default();
        let now = Instant::now();
        for gone in before.difference(&after) {
            if let Some(instance) = state.instances.get_mut(gone) {
                instance.unused_since = Some(now);
            }
        }
        // Every plugin new to the whole set is new to this run, but a plugin new to this run may already be needed by
        // another one: it is still reset when it gave up, as its error promises.
        for new in current.difference(&previous) {
            match state.instances.get_mut(new) {
                // Running or lingering: keep it, unless it is stopped for good, which a new execution resets.
                Some(instance) => {
                    instance.unused_since = None;
                    if instance.gave_up
                        || (instance.child.is_none() && instance.restart_at.is_none())
                    {
                        instance.gave_up = false;
                        instance.failures.clear();
                        self.launch(new, instance, &state.approved);
                    }
                }
                None => {
                    let mut instance = Instance::new(self.0.dir.join(new));
                    self.launch(new, &mut instance, &state.approved);
                    state.instances.insert(new.clone(), instance);
                }
            }
        }
    }

    /// Records that the user enabled `fingerprint` for `id`, or with `None` that they disabled it. A plugin a Live run
    /// uses starts at once when enabled (and restarts when it runs a manifest other than the one enabled now), and
    /// stops at once when disabled, failing the presses it still owed.
    pub fn set_approval(&self, id: &str, fingerprint: Option<String>) {
        if self.0.stopped.load(Ordering::SeqCst) {
            return;
        }
        let mut guard = self.lock();
        let state = &mut *guard;
        match &fingerprint {
            Some(fingerprint) => state.approved.insert(id.to_string(), fingerprint.clone()),
            None => state.approved.remove(id),
        };
        let Some(instance) = state.instances.get_mut(id) else {
            return;
        };
        match fingerprint {
            None => {
                self.halt(instance, "it was disabled in Settings > Plugins");
                instance.manifest = None;
                instance.fingerprint = None;
                instance.restart_at = None;
                instance.gave_up = false;
                instance.not_enabled = true;
                instance.last_error = Approval::NotEnabled.refusal().map(str::to_string);
            }
            Some(fingerprint) => {
                if instance.child.is_some()
                    && instance.fingerprint.as_deref() == Some(fingerprint.as_str())
                {
                    return;
                }
                self.halt(instance, "its plugin.json was enabled again");
                instance.gave_up = false;
                instance.failures.clear();
                instance.restart_at = None;
                if instance.unused_since.is_none() {
                    self.launch(id, instance, &state.approved);
                }
            }
        }
    }

    /// Stops an instance's process and connection and fails what it still owed.
    fn halt(&self, instance: &mut Instance, why: &str) {
        if let Some(child) = instance.child.take() {
            stop_child(child);
        }
        if let Some(conn) = instance.conn.take() {
            conn.close("the plugin was stopped");
        }
        for (_, pending) in instance.pending.drain() {
            self.answer(pending.reply, Err(format!("the plugin was stopped: {why}")));
        }
    }

    /// Stops an instance that is no longer wanted and fails what it still owed.
    fn retire(&self, mut instance: Instance, why: &str) {
        self.halt(&mut instance, why);
    }

    /// Starts the plugin's process with a fresh token, reading its manifest again first. A manifest whose
    /// fingerprint is not the one in `approved` is not started, and that is not counted as a failure.
    fn launch(&self, id: &str, instance: &mut Instance, approved: &Approved) {
        instance.restart_at = None;
        instance.not_enabled = false;
        let (manifest, fingerprint) = match plugins::load(&instance.folder) {
            Ok(loaded) => loaded,
            Err(error) => {
                instance.manifest = None;
                instance.fingerprint = None;
                instance.last_error = Some(if instance.folder.exists() {
                    error
                } else {
                    format!("{id} is not installed (no folder {id} in the plugins folder)")
                });
                return;
            }
        };
        if let Some(refusal) = Approval::of(approved, id, &fingerprint).refusal() {
            instance.manifest = None;
            instance.fingerprint = None;
            instance.not_enabled = true;
            instance.last_error = Some(refusal.to_string());
            return;
        }
        let token = match crate::ingress::new_token() {
            Ok(token) => token,
            Err(error) => {
                instance.last_error = Some(error);
                return;
            }
        };
        let (program, args) = manifest.command_in(&instance.folder);
        let mut command = Command::new(&program);
        command
            .args(args)
            .current_dir(&instance.folder)
            .env("OSH_PLUGIN_SOCKET", &self.0.socket)
            .env("OSH_PLUGIN_ID", id)
            .env("OSH_PLUGIN_TOKEN", &token)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        std::os::unix::process::CommandExt::process_group(&mut command, 0);
        instance.generation += 1;
        instance.manifest = Some(manifest);
        instance.fingerprint = Some(fingerprint);
        instance.token = token;
        instance.conn = None;
        match command.spawn() {
            Ok(mut child) => {
                if let Some(stderr) = child.stderr.take() {
                    let host = self.clone();
                    let (id, generation) = (id.to_string(), instance.generation);
                    let _ = thread::Builder::new()
                        .name(format!("plugin-stderr {id}"))
                        .spawn(move || host.keep_stderr(&id, generation, stderr));
                }
                instance.child = Some(child);
                instance.started_at = Instant::now();
                instance.last_error = None;
            }
            Err(error) => {
                let error = format!("could not start {}: {error}", program.display());
                self.failed(id, instance, error);
            }
        }
    }

    /// Keeps the plugin's stderr lines with its log, so Settings can show why it fails.
    fn keep_stderr(&self, id: &str, generation: u64, stderr: impl Read) {
        let mut reader = BufReader::new(stderr);
        let mut line = Vec::new();
        loop {
            line.clear();
            match read_line(&mut reader, MAX_LINE, &mut line) {
                Ok(Some(())) => {}
                Ok(None) | Err(_) => return,
            }
            let text = self.0.secrets.redact(id, &String::from_utf8_lossy(&line));
            let mut state = self.lock();
            let Some(instance) = state.instances.get_mut(id) else {
                return;
            };
            if instance.generation != generation {
                return;
            }
            if instance.log_rate.allow(MAX_LOGS_PER_SECOND) {
                let text = truncate(&text, MAX_LOG_BYTES);
                instance.keep_log(format!("stderr: {text}"));
            } else {
                instance.dropped_logs += 1;
            }
        }
    }

    /// Notes a failure of a wanted plugin and plans its restart, or gives up after too many.
    fn failed(&self, id: &str, instance: &mut Instance, error: String) {
        if let Some(child) = instance.child.take() {
            stop_child(child);
        }
        instance.conn = None;
        for (_, pending) in instance.pending.drain() {
            self.answer(
                pending.reply,
                Err(format!("{id} stopped before it answered: {error}")),
            );
        }
        let now = Instant::now();
        while instance
            .failures
            .front()
            .is_some_and(|at| now.duration_since(*at) > FAILURE_WINDOW)
        {
            instance.failures.pop_front();
        }
        instance.failures.push_back(now);
        if instance.failures.len() >= MAX_FAILURES {
            instance.gave_up = true;
            instance.restart_at = None;
            instance.last_error = Some(format!(
                "{error}; it failed {MAX_FAILURES} times within 5 minutes and stays stopped until a profile that uses it is executed again"
            ));
        } else {
            let wait = BACKOFF[(instance.failures.len() - 1).min(BACKOFF.len() - 1)];
            instance.restart_at = Some(now + Duration::from_secs(wait));
            instance.last_error = Some(error);
        }
    }

    /// Reaps exited plugins, restarts them when due, pings idle ones and kills silent ones, and forgets invokes past
    /// their deadline (the module has given up on them).
    fn tick(&self) {
        let mut pings = Vec::new();
        {
            let mut guard = self.lock();
            let state = &mut *guard;
            let now = Instant::now();
            let ids: Vec<String> = state.instances.keys().cloned().collect();
            for id in ids {
                if state.instances[&id]
                    .unused_since
                    .is_some_and(|since| now.duration_since(since) >= LINGER)
                {
                    if let Some(instance) = state.instances.remove(&id) {
                        self.retire(instance, "no executing profile uses it any more");
                    }
                    continue;
                }
                let Some(instance) = state.instances.get_mut(&id) else {
                    continue;
                };
                instance.pending.retain(|_, pending| pending.deadline > now);
                let exited = instance
                    .child
                    .as_mut()
                    .and_then(|child| child.try_wait().ok().flatten());
                if let Some(status) = exited {
                    // `failed` takes the child and signals its process group, so helpers the plugin started do not
                    // outlive it and pile up with each restart.
                    self.failed(&id, instance, format!("the plugin exited ({status})"));
                    continue;
                }
                if instance.child.is_none() {
                    if instance.restart_at.is_some_and(|at| at <= now) {
                        self.launch(&id, instance, &state.approved);
                    }
                    continue;
                }
                if let Some(conn) = &instance.conn {
                    let idle = now
                        .duration_since(*conn.last_seen.lock().unwrap_or_else(|p| p.into_inner()));
                    let pinged = *conn.pinged.lock().unwrap_or_else(|p| p.into_inner());
                    match pinged {
                        Some(at) if now.duration_since(at) >= PONG_TIMEOUT => {
                            self.failed(&id, instance, "the plugin stopped answering pings".into());
                        }
                        None if idle >= IDLE_PING => {
                            *conn.pinged.lock().unwrap_or_else(|p| p.into_inner()) = Some(now);
                            pings.push((id.clone(), Arc::clone(conn)));
                        }
                        _ => {}
                    }
                } else if now.duration_since(instance.started_at) >= START_TIMEOUT {
                    // A plugin that hangs before connecting, or connects but never says a valid hello, would
                    // otherwise stay "starting" for good.
                    let error = format!(
                        "the plugin did not connect and say hello within {} s of starting",
                        START_TIMEOUT.as_secs()
                    );
                    self.failed(&id, instance, error);
                }
            }
        }
        for (id, conn) in pings {
            if conn.send(&json!({"type":"ping"})).is_err() {
                self.connection_lost(&id, &conn, "the plugin's connection broke");
            }
        }
    }

    /// Ends an instance's current connection as a failure, unless a newer one replaced it already.
    /// `why` gives way to the reason this side closed the connection, if it did.
    fn connection_lost(&self, id: &str, conn: &Arc<Conn>, why: &str) {
        let why = conn.closed_why().unwrap_or(why);
        let mut state = self.lock();
        if let Some(instance) = state.instances.get_mut(id)
            && instance
                .conn
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, conn))
        {
            self.failed(id, instance, why.to_string());
        }
    }

    fn accept(&self, listener: UnixListener) {
        for stream in listener.incoming() {
            if self.0.stopped.load(Ordering::SeqCst) {
                return;
            }
            let Ok(stream) = stream else { continue };
            if self.0.unnamed.fetch_add(1, Ordering::SeqCst) >= MAX_UNNAMED {
                self.0.unnamed.fetch_sub(1, Ordering::SeqCst);
                let _ = stream.set_write_timeout(Some(WRITE_TIMEOUT));
                let _ = (&stream)
                    .write_all(b"{\"type\":\"error\",\"error\":\"too many connections\"}\n");
                continue;
            }
            let host = self.clone();
            let spawned = thread::Builder::new()
                .name("plugin-connection".into())
                .spawn(move || host.serve(stream));
            if spawned.is_err() {
                self.0.unnamed.fetch_sub(1, Ordering::SeqCst);
            }
        }
    }

    /// One plugin connection: `hello` first, then messages until it closes.
    fn serve(&self, stream: UnixStream) {
        let hello = self.hello(&stream);
        self.0.unnamed.fetch_sub(1, Ordering::SeqCst);
        let Some((id, conn, reader)) = hello else {
            return;
        };
        let mut reader = reader;
        let mut line = Vec::new();
        let _ = reader
            .get_ref()
            .set_read_timeout(Some(Duration::from_millis(500)));
        loop {
            if self.0.stopped.load(Ordering::SeqCst) {
                return;
            }
            // A connection that is no longer the instance's current one (it was replaced or retired) just ends.
            let current = self
                .lock()
                .instances
                .get(&id)
                .and_then(|instance| {
                    instance
                        .conn
                        .as_ref()
                        .map(|current| Arc::ptr_eq(current, &conn))
                })
                .unwrap_or(false);
            if !current {
                return;
            }
            match read_line(&mut reader, MAX_LINE, &mut line) {
                Ok(Some(())) => {
                    conn.seen();
                    let message = serde_json::from_slice::<Value>(&line);
                    line.clear();
                    match message {
                        Ok(message) => self.message(&id, &conn, &message),
                        Err(_) => {
                            let _ = conn.send(
                                &json!({"type":"error","error":"the line was not a JSON object"}),
                            );
                        }
                    }
                }
                Ok(None) => {
                    self.connection_lost(&id, &conn, "the plugin closed its connection");
                    return;
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    // A partial line stays in `line` and is completed by the next read.
                }
                Err(error) => {
                    let why = if error.kind() == io::ErrorKind::InvalidData {
                        format!("the plugin sent a line longer than {} KiB", MAX_LINE / 1024)
                    } else {
                        format!("the plugin's connection broke: {error}")
                    };
                    self.connection_lost(&id, &conn, &why);
                    return;
                }
            }
        }
    }

    /// Reads `hello` within [`HELLO_TIMEOUT`] and attaches the connection to its instance.
    fn hello(&self, stream: &UnixStream) -> Option<(String, Arc<Conn>, BufReader<UnixStream>)> {
        let refuse = |error: &str| {
            let _ = stream.set_write_timeout(Some(WRITE_TIMEOUT));
            let mut writer = stream;
            let _ =
                writer.write_all(format!("{}\n", json!({"type":"error","error":error})).as_bytes());
        };
        let _ = stream.set_read_timeout(Some(HELLO_TIMEOUT));
        let reader_stream = stream.try_clone().ok()?;
        let mut reader = BufReader::new(reader_stream);
        let mut line = Vec::new();
        let deadline = Instant::now() + HELLO_TIMEOUT;
        loop {
            match read_line(&mut reader, MAX_LINE, &mut line) {
                Ok(Some(())) => break,
                Ok(None) => return None,
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) && Instant::now() < deadline => {}
                Err(_) => {
                    refuse("say hello within 2 s, on one line of at most 64 KiB");
                    return None;
                }
            }
        }
        let hello: Value = serde_json::from_slice(&line).unwrap_or(Value::Null);
        let (Some("hello"), Some(id), Some(token)) = (
            hello["type"].as_str(),
            hello["id"].as_str(),
            hello["token"].as_str(),
        ) else {
            refuse("the first line must be {\"type\":\"hello\",\"id\":…,\"token\":…}");
            return None;
        };
        let (Ok(writer), Ok(handle)) = (stream.try_clone(), stream.try_clone()) else {
            return None;
        };
        let _ = writer.set_write_timeout(Some(WRITE_TIMEOUT));
        let conn = Arc::new(Conn::new(handle));
        // Queued before anything else can be: the connection is reachable once it is the instance's below.
        let _ = conn.send(&json!({"type":"welcome"}));
        {
            let mut state = self.lock();
            let Some(instance) = state
                .instances
                .get_mut(id)
                .filter(|instance| instance.child.is_some() && !instance.token.is_empty())
            else {
                drop(state);
                refuse("the token is missing or wrong");
                return None;
            };
            if !crate::ingress::token_matches(&instance.token, token) {
                drop(state);
                refuse("the token is missing or wrong");
                return None;
            }
            if instance.conn.is_some() {
                drop(state);
                refuse("this plugin is already connected");
                return None;
            }
            instance.conn = Some(Arc::clone(&conn));
            // A new process starts with a fresh limit and no count from the last one.
            instance.event_rate = Window::default();
            instance.dropped_events = 0;
        }
        let (host, writer_id, weak) = (self.clone(), id.to_string(), Arc::downgrade(&conn));
        if thread::Builder::new()
            .name(format!("plugin-writer {id}"))
            .spawn(move || host.write_lines(&writer_id, &weak, writer))
            .is_err()
        {
            self.connection_lost(id, &conn, "could not start the plugin's writer");
            return None;
        }
        Some((id.to_string(), conn, reader))
    }

    fn message(&self, id: &str, conn: &Arc<Conn>, message: &Value) {
        let request_id = message.get("id").cloned().unwrap_or(Value::Null);
        let reply = |result: Result<Value, String>| {
            let answer = match result {
                Ok(_) => json!({"type":"reply","id":request_id,"ok":true}),
                Err(error) => json!({"type":"reply","id":request_id,"ok":false,"error":error}),
            };
            let _ = conn.send(&answer);
        };
        match message["type"].as_str() {
            Some("result") => {
                let Some(request) = message["request"].as_u64() else {
                    return;
                };
                let result = if message["ok"] == true {
                    Ok(())
                } else {
                    let error = message["error"]
                        .as_str()
                        .unwrap_or("the plugin reported a failure");
                    let error = self.0.secrets.redact(id, error);
                    Err(truncate(&error, MAX_ERROR_BYTES))
                };
                let pending = self
                    .lock()
                    .instances
                    .get_mut(id)
                    .and_then(|instance| instance.pending.remove(&request));
                if let Some(pending) = pending {
                    self.answer(pending.reply, result);
                }
            }
            Some("set_variable") => {
                let allowed = {
                    let mut state = self.lock();
                    match state.instances.get_mut(id) {
                        Some(instance) => {
                            let name = message["name"].as_str().unwrap_or("");
                            let listed = instance.manifest.as_ref().is_some_and(|manifest| {
                                manifest
                                    .permissions
                                    .variables
                                    .iter()
                                    .any(|listed| listed == name)
                            });
                            if !listed {
                                Err(format!(
                                    "{name} is not among the variables this plugin's manifest says it sets"
                                ))
                            } else if !instance.sets.allow(MAX_SETS_PER_SECOND) {
                                Err(format!(
                                    "at most {MAX_SETS_PER_SECOND} variable writes a second"
                                ))
                            } else {
                                Ok(())
                            }
                        }
                        None => Err("this plugin is not running".into()),
                    }
                };
                reply(
                    allowed.and_then(|()| crate::ingress::set_variable(message, &*self.0.forward)),
                );
            }
            Some("log") => {
                let text = self
                    .0
                    .secrets
                    .redact(id, message["message"].as_str().unwrap_or(""));
                let mut state = self.lock();
                if let Some(instance) = state.instances.get_mut(id) {
                    if instance.log_rate.allow(MAX_LOGS_PER_SECOND) {
                        instance.keep_log(truncate(&text, MAX_LOG_BYTES));
                    } else {
                        instance.dropped_logs += 1;
                    }
                }
            }
            Some("pong") => {}
            Some("ping") => {
                let _ = conn.send(&json!({"type":"pong"}));
            }
            _ => reply(Err("unknown message type".into())),
        }
    }

    /// Carries one `plugin_invoke` record from a module run to its plugin; `reply` gets the plugin's answer, or the
    /// reason it was not asked. Never waits: a press whose secrets are not held in memory yet is handed to the invoke
    /// thread, since reading one may wait for the Keychain.
    ///
    /// `service_id` is the module run the press came from: it is sent only while that run still executes with this
    /// plugin, so a press that waited for the Keychain never runs after its run stopped or switched to Test.
    pub fn invoke(&self, service_id: &str, record: &Value, reply: Reply) {
        if self.0.stopped.load(Ordering::SeqCst) {
            return self.answer(reply, Err(crate::service::SHUTTING_DOWN.into()));
        }
        let arrived = Instant::now();
        match self.with_secrets(record, false) {
            Ok(Some(filled)) => {
                return self.send_invoke(service_id, record, filled, reply, arrived);
            }
            Err(error) => return self.answer(reply, Err(error)),
            Ok(None) => {}
        }
        // Only a press naming a plugin with a manifest needs a read, so the id is there.
        let id = record["plugin"].as_str().unwrap_or_default().to_string();
        let queued = {
            let mut workers = self
                .0
                .workers
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let worker = match workers.get(&id) {
                Some(worker) => Ok(worker.clone()),
                None => {
                    let (worker, queued) = mpsc::channel::<Queued>();
                    let host = self.clone();
                    thread::Builder::new()
                        .name(format!("plugin-invokes {id}"))
                        .spawn(move || host.handle_invokes(queued))
                        .map(|_| {
                            workers.insert(id.clone(), worker.clone());
                            worker
                        })
                }
            };
            match worker {
                Ok(worker) => worker
                    .send((service_id.to_string(), record.clone(), reply, arrived))
                    .map_err(|mpsc::SendError((_, _, reply, _))| {
                        (reply, crate::service::SHUTTING_DOWN.to_string())
                    }),
                Err(error) => Err((
                    reply,
                    format!("could not start reading the secret: {error}"),
                )),
            }
        };
        if let Err((reply, error)) = queued {
            self.answer(reply, Err(error));
        }
    }

    /// Carries one `plugin_event` record from a module run to its plugin, or drops it without a word: when the host
    /// stops, the run no longer executes with the plugin, the plugin is not running and connected, its manifest does
    /// not let the action take events or does not accept the record, or more than [`MAX_EVENTS_PER_SECOND`] came this
    /// second (those are counted in its log). Never waits for the plugin, and nothing is answered.
    pub fn event(&self, service_id: &str, record: &Value) {
        if self.0.stopped.load(Ordering::SeqCst) {
            return;
        }
        let sending = (|| {
            let id = record["plugin"].as_str()?;
            let action = record["action"].as_str()?;
            let input = input_of(record)?;
            let params: BTreeMap<String, PluginValue> =
                serde_json::from_value(record["params"].clone()).ok()?;
            let variables: Vec<String> = match &record["variables"] {
                Value::Null => Vec::new(),
                names => serde_json::from_value(names.clone()).ok()?,
            };
            let mut state = self.lock();
            if !state
                .usage
                .get(service_id)
                .is_some_and(|plugins| plugins.contains(id))
            {
                return None;
            }
            let instance = state.instances.get_mut(id)?;
            let manifest = instance.manifest.as_ref()?;
            if !manifest.satisfies(record["min_version"].as_str().unwrap_or("0.0.0"))
                || plugins::check_event(manifest, action, &params, &variables).is_err()
            {
                return None;
            }
            // The same text limits as an invoke's, checked again here as there.
            cooltek_sdk_service::plugin::validate_plugin_action(id, "0.0.0", action, &params, &[])
                .ok()?;
            let (Some(_), Some(conn)) = (&instance.child, &instance.conn) else {
                return None;
            };
            let mut message = json!({"type": "event", "action": action, "params": params});
            if let (Some(message), Value::Object(input)) = (message.as_object_mut(), input) {
                message.extend(input);
            }
            if message.to_string().len() > MAX_LINE {
                return None;
            }
            // Over the rate limit, or queued behind a plugin that is not reading: dropped and counted.
            if !instance.event_rate.allow(MAX_EVENTS_PER_SECOND) {
                instance.dropped_events += 1;
                return None;
            }
            if !conn.queue_event(&message) {
                // Not sent, so it does not count against the rate.
                instance.event_rate.0.pop_back();
                instance.dropped_events += 1;
                return None;
            }
            if instance.dropped_events > 0 {
                let dropped = std::mem::take(&mut instance.dropped_events);
                instance.keep_log(format!(
                    "{dropped} input events were not sent: more than {MAX_EVENTS_PER_SECOND} a second, or the plugin was not reading them"
                ));
            }
            Some(())
        })();
        let _ = sending;
    }

    /// A connection's writer: writes each queued line whole and in order, however long the plugin takes to read it,
    /// until the connection is retired (no longer held), shut down or broken. A plugin that reads nothing for long is
    /// found by its pings, which wait in the same line; its input events are dropped meanwhile.
    fn write_lines(&self, id: &str, conn: &Weak<Conn>, mut stream: UnixStream) {
        let gone = || conn.upgrade().is_none() || self.0.stopped.load(Ordering::SeqCst);
        loop {
            // The connection is held only while it is asked for a line, so a retired one is dropped and this ends.
            let Some(line) = (match conn.upgrade() {
                Some(conn) => conn.next_line(TICK),
                None => return,
            }) else {
                if gone() {
                    return;
                }
                continue;
            };
            let mut written = 0;
            while written < line.len() {
                match stream.write(&line[written..]) {
                    Ok(0) => return self.writer_lost(id, conn),
                    Ok(n) => written += n,
                    Err(error)
                        if matches!(
                            error.kind(),
                            io::ErrorKind::WouldBlock
                                | io::ErrorKind::TimedOut
                                | io::ErrorKind::Interrupted
                        ) =>
                    {
                        if gone() {
                            return;
                        }
                    }
                    Err(_) => return self.writer_lost(id, conn),
                }
            }
        }
    }

    fn writer_lost(&self, id: &str, conn: &Weak<Conn>) {
        if let Some(conn) = conn.upgrade() {
            self.connection_lost(id, &conn, "the plugin's connection broke");
        }
    }

    /// One plugin's invoke thread: reads secrets for its queued invokes in order until the host stops, then fails the
    /// ones left.
    fn handle_invokes(&self, queued: mpsc::Receiver<Queued>) {
        let stopped = || self.0.stopped.load(Ordering::SeqCst);
        loop {
            match queued.recv_timeout(TICK) {
                Ok((service_id, record, reply, arrived)) if !stopped() => {
                    // The read happens first: it is what may have kept this press waiting.
                    match self.with_secrets(&record, true) {
                        Ok(Some(filled)) => {
                            self.send_invoke(&service_id, &record, filled, reply, arrived)
                        }
                        Ok(None) => self.answer(reply, Err("the secret could not be read".into())),
                        Err(error) => self.answer(reply, Err(error)),
                    }
                }
                Ok((_, _, reply, _)) => {
                    self.answer(reply, Err(crate::service::SHUTTING_DOWN.into()))
                }
                Err(mpsc::RecvTimeoutError::Timeout) if !stopped() => {}
                Err(_) => {
                    while let Ok((_, _, reply, _)) = queued.try_recv() {
                        self.answer(reply, Err(crate::service::SHUTTING_DOWN.into()));
                    }
                    return;
                }
            }
        }
    }

    /// How long a press may take overall, as the module asked (at most [`MAX_INVOKE`]).
    fn timeout_of(record: &Value) -> Duration {
        record["timeout_ms"]
            .as_u64()
            .map(Duration::from_millis)
            .unwrap_or(MAX_INVOKE)
            .min(MAX_INVOKE)
    }

    /// Registers and sends one invoke whose secrets are filled in. A press whose deadline (from when it arrived)
    /// passed while it waited is answered with an error and not sent: the module has given up on it already.
    fn send_invoke(
        &self,
        service_id: &str,
        record: &Value,
        filled: Filled,
        reply: Reply,
        arrived: Instant,
    ) {
        let deadline = arrived + Self::timeout_of(record);
        if Instant::now() >= deadline {
            return self.answer(
                reply,
                Err("the press timed out before it could be sent to the plugin".into()),
            );
        }
        let (given, params) = filled;
        let registered = self.register(service_id, record, &given, params, reply, deadline);
        let (conn, message, id) = match registered {
            Ok(registered) => registered,
            Err((error, reply)) => return self.answer(reply, Err(error)),
        };
        // What was filled in beyond the profile's own parameters is a secret this plugin now knows.
        if let Some(sent) = message["params"].as_object() {
            for (name, value) in sent {
                if !given.contains_key(name)
                    && let Some(value) = value.as_str()
                {
                    self.0.secrets.disclosed(&id, value);
                }
            }
        }
        if conn.send_invoke(&message, deadline).is_err() {
            let request = message["request"].as_u64().unwrap_or(0);
            let pending = self
                .lock()
                .instances
                .get_mut(&id)
                .and_then(|instance| instance.pending.remove(&request));
            if let Some(pending) = pending {
                self.answer(pending.reply, Err(format!("{id} could not be reached")));
            }
            self.connection_lost(&id, &conn, "the plugin's connection broke");
        }
    }

    /// The parameters the profile gave and those with the plugin's stored secrets filled in. With `read` false only
    /// what [`PluginSecrets`] holds in memory is used, and `Ok(None)` says a Keychain read is needed first; with
    /// `read` true the Keychain is read, without the host's lock held, since a read may wait for the user. A plugin
    /// that is not running with a manifest is left to [`Self::register`] to refuse.
    fn with_secrets(&self, record: &Value, read: bool) -> Result<Option<Filled>, String> {
        let given: BTreeMap<String, PluginValue> = serde_json::from_value(record["params"].clone())
            .map_err(|_| "the invoke's parameters are not plugin values".to_string())?;
        let mut params = given.clone();
        let (Some(id), Some(action)) = (record["plugin"].as_str(), record["action"].as_str())
        else {
            return Ok(Some((given, params)));
        };
        let manifest = self
            .lock()
            .instances
            .get(id)
            .and_then(|instance| instance.manifest.clone());
        let Some(manifest) = manifest else {
            return Ok(Some((given, params)));
        };
        let mut needs_read = false;
        let filled = plugins::fill_secrets(&manifest, action, &mut params, |param| {
            if read {
                return self.0.secrets.get(id, &param.id);
            }
            self.0.secrets.cached(id, &param.id).unwrap_or_else(|| {
                needs_read = true;
                Ok(None)
            })
        });
        if needs_read {
            return Ok(None);
        }
        filled.map(|()| Some((given, params)))
    }

    /// Checks an invoke, whose `params` have their secrets filled in, against the plugin's state and manifest and
    /// registers `reply` for its answer, handing back the message to send. A refusal hands `reply` back with the
    /// reason.
    #[allow(clippy::type_complexity)]
    fn register(
        &self,
        service_id: &str,
        record: &Value,
        given: &BTreeMap<String, PluginValue>,
        params: BTreeMap<String, PluginValue>,
        reply: Reply,
        deadline: Instant,
    ) -> Result<(Arc<Conn>, Value, String), (String, Reply)> {
        let mut state = self.lock();
        state.next_request += 1;
        let request = state.next_request;
        let checked = (|| {
            let id = record["plugin"]
                .as_str()
                .ok_or("the invoke names no plugin")?
                .to_string();
            let action = record["action"]
                .as_str()
                .ok_or("the invoke names no action")?;
            let min_version = record["min_version"].as_str().unwrap_or("0.0.0");
            // Absent means no variables were filled in; anything else must be a list of names, so a
            // malformed record cannot skip the manifest's check.
            let variables: Vec<String> = match &record["variables"] {
                Value::Null => Vec::new(),
                names => serde_json::from_value(names.clone()).map_err(|_| {
                    "the invoke's variable parameters are not a list of names".to_string()
                })?,
            };
            // What is left of the press's time, counted from when it arrived.
            let timeout = deadline.saturating_duration_since(Instant::now());
            let instance = state.instances.get(&id).ok_or_else(|| {
                format!("{id} is not running: plugins run only while a profile that uses them is executed (Live)")
            })?;
            let Some(manifest) = instance.manifest.as_ref() else {
                return Err(match &instance.last_error {
                    Some(error) if instance.not_enabled => format!("{id}: {error}"),
                    Some(error) => error.clone(),
                    None => format!("{id} cannot be used"),
                });
            };
            if !manifest.satisfies(min_version) {
                return Err(format!(
                    "{} {} is older than {min_version}, which this key needs; update the plugin",
                    manifest.name, manifest.version
                ));
            }
            // The manifest may have changed since the secrets were filled in; what the profile gave is checked against
            // the current one.
            plugins::refuse_profile_secrets(manifest, action, given)?;
            plugins::check_invoke(manifest, action, &params, &variables)?;
            // The service checked the profile's own text; with the secrets filled in, the whole map must still fit the
            // same limits. The SDK's errors name parameters, never values.
            cooltek_sdk_service::plugin::validate_plugin_action(&id, "0.0.0", action, &params, &[])
                .map_err(|error| format!("{id}: with its secrets filled in, {error}"))?;
            let conn = match (&instance.child, &instance.conn) {
                (Some(_), Some(conn)) => Arc::clone(conn),
                (Some(_), None) => {
                    return Err(format!("{id} is still starting; press again in a moment"));
                }
                (None, _) => {
                    return Err(match &instance.last_error {
                        Some(error) => format!("{id} is not running: {error}"),
                        None => format!("{id} is not running"),
                    });
                }
            };
            // Only the run the press came from counts: another Live run using this plugin does not make a press from
            // a run that stopped executing (or switched to Test) one to send.
            if !state
                .usage
                .get(service_id)
                .is_some_and(|plugins| plugins.contains(&id))
            {
                return Err("the run stopped executing before the press could be sent".into());
            }
            if instance.pending.len() >= MAX_IN_FLIGHT {
                return Err(format!(
                    "{id} is busy: {MAX_IN_FLIGHT} presses are still waiting for it"
                ));
            }
            let mut message = json!({
                "type": "invoke",
                "request": request,
                "action": action,
                "params": params,
                "timeout_ms": u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX),
            });
            if let Some(input) = input_of(record) {
                message["input"] = input;
            }
            if message.to_string().len() > MAX_LINE {
                return Err(format!(
                    "{id}: the press would be longer than the {} KiB a plugin line may hold",
                    MAX_LINE / 1024
                ));
            }
            Ok((conn, message, id))
        })();
        match checked {
            Ok((conn, message, id)) => {
                if let Some(instance) = state.instances.get_mut(&id) {
                    instance
                        .pending
                        .insert(request, Pending { deadline, reply });
                }
                Ok((conn, message, id))
            }
            Err(error) => Err((error, reply)),
        }
    }

    /// The plugins Settings can show: every wanted one, with its state, last error and recent log.
    pub fn statuses(&self) -> Vec<Status> {
        let state = self.lock();
        let mut statuses: Vec<Status> = state
            .instances
            .iter()
            .map(|(id, instance)| Status {
                id: id.clone(),
                state: instance.state(),
                error: instance.last_error.clone(),
                logs: instance.logs.iter().cloned().collect(),
                dropped_logs: instance.dropped_logs,
            })
            .collect();
        statuses.sort_by(|a, b| a.id.cmp(&b.id));
        statuses
    }

    /// App exit: stops every plugin and removes the socket. Later usage changes are ignored.
    pub fn stop_all(&self) {
        self.0.stopped.store(true, Ordering::SeqCst);
        let instances: Vec<Instance> = {
            let mut state = self.lock();
            state.usage.clear();
            state
                .instances
                .drain()
                .map(|(_, instance)| instance)
                .collect()
        };
        for mut instance in instances {
            if let Some(mut child) = instance.child.take() {
                kill_group(&mut child);
            }
            for (_, pending) in instance.pending.drain() {
                self.answer(pending.reply, Err(crate::service::SHUTTING_DOWN.into()));
            }
        }
        let _ = fs::remove_file(&self.0.socket);
        // Wakes the accept loop so its thread ends.
        let _ = UnixStream::connect(&self.0.socket);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    const ID: &str = "com.example.lights";

    impl PluginHost {
        fn token(&self, id: &str) -> String {
            self.lock().instances[id].token.clone()
        }

        fn state_of(&self, id: &str) -> Option<&'static str> {
            self.lock().instances.get(id).map(Instance::state)
        }
    }

    struct Rig {
        _root: tempfile::TempDir,
        host: PluginHost,
        forwarded: Arc<Mutex<Vec<Value>>>,
        /// The fingerprint of the rig's plugin.json.
        fingerprint: String,
        /// The Keychain stand-in the host's secrets read.
        vault: crate::plugin_secrets::memory::Store,
    }

    impl Drop for Rig {
        fn drop(&mut self) {
            self.host.stop_all();
        }
    }

    /// Writes the test manifest for `id`, running `command`, into the plugins folder `dir`; hands back its
    /// fingerprint.
    fn install(dir: &Path, id: &str, command: &[&str]) -> String {
        let folder = dir.join(id);
        fs::create_dir_all(&folder).unwrap();
        fs::set_permissions(&folder, fs::Permissions::from_mode(0o755)).unwrap();
        let manifest = json!({
            "schema": 1, "id": id, "name": "Lights", "version": "1.2.0", "command": command,
            "actions": [
                { "id": "set", "label": "Set", "params": [
                    { "id": "entity", "label": "Light", "type": "text", "required": true }
                ] },
                { "id": "login", "label": "Log in", "params": [
                    { "id": "token", "label": "Token", "type": "text", "secret": true, "required": true },
                    { "id": "user", "label": "User", "type": "text" }
                ] },
                { "id": "talk", "label": "Talk", "events": true, "params": [
                    { "id": "channel", "label": "Channel", "type": "text", "variables": true },
                    { "id": "key", "label": "Key", "type": "text", "secret": true, "required": true }
                ] },
                { "id": "bulk", "label": "Bulk", "params": [
                    { "id": "s1", "label": "S1", "type": "text", "secret": true },
                    { "id": "s2", "label": "S2", "type": "text", "secret": true },
                    { "id": "s3", "label": "S3", "type": "text", "secret": true },
                    { "id": "s4", "label": "S4", "type": "text", "secret": true },
                    { "id": "s5", "label": "S5", "type": "text", "secret": true }
                ] }
            ],
            "permissions": { "variables": ["room"] }
        })
        .to_string();
        fs::write(folder.join(plugins::MANIFEST_FILE), &manifest).unwrap();
        plugins::fingerprint(manifest.as_bytes())
    }

    /// A host whose one plugin runs `command`, enabled, and a forward that records variable writes.
    fn rig(command: &[&str]) -> Rig {
        rig_with(command, |fingerprint| {
            [(ID.to_string(), fingerprint.to_string())].into()
        })
    }

    /// A rig whose host starts with the approvals `approve` makes from the manifest's fingerprint.
    fn rig_with(command: &[&str], approve: impl FnOnce(&str) -> Approved) -> Rig {
        let root = tempfile::Builder::new()
            .prefix("p")
            .tempdir_in("/tmp")
            .unwrap();
        let dir = root.path().join("plugins");
        let fingerprint = install(&dir, ID, command);
        let vault = crate::plugin_secrets::memory::Store::default();
        let forwarded = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&forwarded);
        let forward: Forward = Arc::new(move |body: &Value| {
            seen.lock().unwrap().push(body.clone());
            vec![("m1".to_string(), Ok(json!({})))]
        });
        let host = PluginHost::start(
            &dir,
            approve(&fingerprint),
            crate::plugin_secrets::memory::secrets(&vault),
            forward,
        )
        .unwrap();
        Rig {
            _root: root,
            host,
            forwarded,
            fingerprint,
            vault,
        }
    }

    fn used() -> Option<BTreeSet<String>> {
        Some([ID.to_string()].into())
    }

    fn wait_for(what: &str, mut ready: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !ready() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            thread::sleep(Duration::from_millis(10));
        }
    }

    /// The test's side of a plugin connection.
    struct Plugin {
        reader: BufReader<UnixStream>,
        writer: UnixStream,
    }

    impl Plugin {
        fn connect(host: &PluginHost) -> Self {
            let stream = UnixStream::connect(&host.0.socket).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            Self {
                reader: BufReader::new(stream.try_clone().unwrap()),
                writer: stream,
            }
        }

        fn send(&mut self, message: Value) {
            self.writer
                .write_all(format!("{message}\n").as_bytes())
                .unwrap();
        }

        fn receive(&mut self) -> Value {
            let mut line = String::new();
            self.reader.read_line(&mut line).unwrap();
            serde_json::from_str(&line).unwrap_or(Value::Null)
        }

        /// The next line that is not a ping or a pong.
        fn receive_message(&mut self) -> Value {
            loop {
                let message = self.receive();
                if !matches!(message["type"].as_str(), Some("ping" | "pong")) {
                    return message;
                }
            }
        }

        fn hello(host: &PluginHost, token: &str) -> (Self, Value) {
            Self::hello_as(host, ID, token)
        }

        fn hello_as(host: &PluginHost, id: &str, token: &str) -> (Self, Value) {
            let mut plugin = Self::connect(host);
            plugin.send(json!({"type":"hello","id":id,"token":token}));
            let answer = plugin.receive();
            (plugin, answer)
        }
    }

    fn invoke(host: &PluginHost, params: Value) -> mpsc::Receiver<Result<(), String>> {
        invoke_action(host, "set", params)
    }

    fn invoke_action(
        host: &PluginHost,
        action: &str,
        params: Value,
    ) -> mpsc::Receiver<Result<(), String>> {
        invoke_within(host, action, params, 5000)
    }

    fn invoke_within(
        host: &PluginHost,
        action: &str,
        params: Value,
        timeout_ms: u64,
    ) -> mpsc::Receiver<Result<(), String>> {
        let (sender, receiver) = mpsc::channel();
        host.invoke(
            "m1",
            &json!({"type":"plugin_invoke","request":1,"plugin":ID,"min_version":"1.0.0",
                "action":action,"params":params,"timeout_ms":timeout_ms}),
            Box::new(move |result| {
                let _ = sender.send(result);
            }),
        );
        receiver
    }

    fn answer(receiver: &mpsc::Receiver<Result<(), String>>) -> Result<(), String> {
        receiver.recv_timeout(Duration::from_secs(5)).unwrap()
    }

    #[test]
    fn a_press_reaches_the_plugin_and_its_answer_comes_back() {
        let rig = rig(&["/bin/sleep", "30"]);
        let host = &rig.host;
        assert!(
            answer(&invoke(host, json!({"entity":"a"})))
                .unwrap_err()
                .contains("is not running")
        );
        host.set_usage("m1", used());
        assert_eq!(host.state_of(ID), Some("starting"));
        assert!(
            answer(&invoke(host, json!({"entity":"a"})))
                .unwrap_err()
                .contains("still starting")
        );

        let (_, refused) = Plugin::hello(host, "wrong");
        assert_eq!(refused["type"], "error");
        let token = host.token(ID);
        let (mut plugin, welcome) = Plugin::hello(host, &token);
        assert_eq!(welcome, json!({"type":"welcome"}));
        assert_eq!(host.state_of(ID), Some("running"));
        let (_, second) = Plugin::hello(host, &token);
        assert_eq!(second["error"], "this plugin is already connected");

        let pressed = invoke(host, json!({"entity":"light.desk"}));
        let asked = plugin.receive();
        assert_eq!(asked["type"], "invoke");
        assert_eq!(asked["action"], "set");
        assert_eq!(asked["params"], json!({"entity":"light.desk"}));
        plugin.send(json!({"type":"result","request":asked["request"],"ok":true}));
        assert_eq!(answer(&pressed), Ok(()));

        let pressed = invoke(host, json!({"entity":"light.hall"}));
        let asked = plugin.receive();
        plugin.send(json!({"type":"result","request":asked["request"],"ok":false,"error":"the light is offline"}));
        assert_eq!(answer(&pressed), Err("the light is offline".into()));

        // Checked against the manifest before the plugin hears of it.
        assert!(
            answer(&invoke(host, json!({})))
                .unwrap_err()
                .contains("needs parameter entity")
        );
        assert!(
            answer(&invoke(host, json!({"entity":"a","x":1})))
                .unwrap_err()
                .contains("no parameter x")
        );
        let (sender, newer) = mpsc::channel();
        host.invoke(
            "m1",
            &json!({"plugin":ID,"min_version":"2.0.0","action":"set","params":{"entity":"a"}}),
            Box::new(move |result| {
                let _ = sender.send(result);
            }),
        );
        assert!(answer(&newer).unwrap_err().contains("older than 2.0.0"));
    }

    fn event(host: &PluginHost, service: &str, action: &str, input: Value) {
        host.event(
            service,
            &json!({"type":"plugin_event","plugin":ID,"min_version":"1.0.0","action":action,
                "params":{"channel":"team"},"variables":["channel"],"input":input}),
        );
    }

    #[test]
    fn a_plugin_hears_events_only_for_actions_that_take_them_from_runs_that_use_it() {
        let rig = rig(&["/bin/sleep", "30"]);
        let host = &rig.host;
        let released =
            json!({"gesture":"released","control":{"module":"m4315","index":3},"extra":1});
        // Not running yet: dropped, and nothing is waiting for it.
        event(host, "m1", "talk", released.clone());
        host.set_usage("m1", used());
        let (mut plugin, _) = Plugin::hello(host, &host.token(ID));
        // An action without "events", a run that does not use the plugin, a malformed input, a newer min_version:
        // all dropped without a word.
        event(host, "m1", "set", released.clone());
        event(host, "m2", "talk", released.clone());
        event(
            host,
            "m1",
            "talk",
            json!({"gesture":"Released","control":{"module":"m4315","index":3}}),
        );
        host.event(
            "m1",
            &json!({"plugin":ID,"min_version":"9.0.0","action":"talk","params":{},"input":released}),
        );
        event(
            host,
            "m1",
            "talk",
            json!({"gesture":"position_changed","control":{"module":"m005s","index":1},
                "position":{"raw":512,"maximum":1023}}),
        );
        // The first line the plugin hears is the one event that passed; unknown input fields are not passed on, and
        // the required secret is not needed.
        assert_eq!(
            plugin.receive_message(),
            json!({"type":"event","action":"talk","params":{"channel":"team"},"gesture":"position_changed",
                "control":{"module":"m005s","index":1},"position":{"raw":512,"maximum":1023}})
        );
        // An invoke passes its input on.
        let (sender, pressed) = mpsc::channel();
        host.invoke(
            "m1",
            &json!({"type":"plugin_invoke","request":1,"plugin":ID,"min_version":"1.0.0","action":"set",
                "params":{"entity":"a"},"timeout_ms":5000,"input":released}),
            Box::new(move |result| {
                let _ = sender.send(result);
            }),
        );
        let asked = plugin.receive_message();
        assert_eq!(asked["type"], "invoke");
        assert_eq!(
            asked["input"],
            json!({"gesture":"released","control":{"module":"m4315","index":3}})
        );
        plugin.send(json!({"type":"result","request":asked["request"],"ok":true}));
        assert_eq!(answer(&pressed), Ok(()));
    }

    #[test]
    fn a_plugin_that_does_not_read_its_events_loses_some_but_not_its_connection() {
        let rig = rig(&["/bin/sleep", "30"]);
        let host = &rig.host;
        host.set_usage("m1", used());
        let (mut plugin, _) = Plugin::hello(host, &host.token(ID));
        let long = "x".repeat(1500);
        let turn = json!({"gesture":"clockwise","control":{"module":"m003k","index":0}});
        let started = Instant::now();
        // Far more than the socket and the queue hold, in bursts within the rate limit, while the plugin reads
        // nothing but keeps answering pings.
        for _ in 0..8 {
            for _ in 0..MAX_EVENTS_PER_SECOND {
                host.event(
                    "m1",
                    &json!({"type":"plugin_event","plugin":ID,"min_version":"1.0.0","action":"talk",
                        "params":{"channel":long},"input":turn}),
                );
            }
            for _ in 0..11 {
                thread::sleep(Duration::from_millis(100));
                plugin.send(json!({"type":"pong"}));
            }
        }
        // Sending never waited for the plugin.
        assert!(started.elapsed() < Duration::from_secs(12));
        assert_eq!(host.state_of(ID), Some("running"));
        // It reads what was queued, then a new event is sent and the loss is logged.
        let mut heard = 0;
        plugin
            .reader
            .get_ref()
            .set_read_timeout(Some(Duration::from_millis(300)))
            .unwrap();
        loop {
            let mut line = String::new();
            match plugin.reader.read_line(&mut line) {
                Ok(n) if n > 0 => {
                    if serde_json::from_str::<Value>(&line).unwrap()["type"] == "event" {
                        heard += 1;
                    }
                }
                _ => break,
            }
        }
        assert!(heard > 0 && heard < 8 * MAX_EVENTS_PER_SECOND, "{heard}");
        plugin
            .reader
            .get_ref()
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        event(host, "m1", "talk", turn);
        assert_eq!(plugin.receive_message()["type"], "event");
        assert_eq!(host.state_of(ID), Some("running"));
        let logs = host.statuses()[0].logs.clone();
        assert!(
            logs.iter()
                .any(|line| line.contains("input events were not sent")),
            "{logs:?}"
        );
    }

    #[test]
    fn large_events_and_a_large_press_after_them_wait_for_a_busy_plugin_and_arrive_in_order() {
        let rig = rig(&["/bin/sleep", "30"]);
        let host = &rig.host;
        host.set_usage("m1", used());
        let (mut plugin, _) = Plugin::hello(host, &host.token(ID));
        let long = "x".repeat(2000);
        let turn = json!({"gesture":"clockwise","control":{"module":"m003k","index":0}});
        // More than the socket holds (macOS gives local sockets 8 KB each way), while the plugin reads nothing for
        // longer than a write may block.
        for _ in 0..20 {
            host.event(
                "m1",
                &json!({"type":"plugin_event","plugin":ID,"min_version":"1.0.0","action":"talk",
                    "params":{"channel":long},"input":turn}),
            );
        }
        let pressed = invoke(host, json!({"entity": "y".repeat(2000)}));
        // It keeps answering pings, as a plugin busy on another thread would.
        for _ in 0..25 {
            thread::sleep(Duration::from_millis(100));
            plugin.send(json!({"type":"pong"}));
        }
        assert_eq!(host.state_of(ID), Some("running"));
        for _ in 0..20 {
            assert_eq!(plugin.receive_message()["type"], "event");
        }
        let press = plugin.receive_message();
        assert_eq!(press["type"], "invoke");
        plugin.send(json!({"type":"result","request":press["request"],"ok":true}));
        assert_eq!(answer(&pressed), Ok(()));
        assert_eq!(host.state_of(ID), Some("running"));
    }

    #[test]
    fn a_press_that_waits_behind_a_busy_plugin_past_its_deadline_is_never_written() {
        let rig = rig(&["/bin/sleep", "30"]);
        let host = &rig.host;
        host.set_usage("m1", used());
        let (mut plugin, _) = Plugin::hello(host, &host.token(ID));
        let long = "x".repeat(2000);
        let turn = json!({"gesture":"clockwise","control":{"module":"m003k","index":0}});
        for _ in 0..20 {
            host.event(
                "m1",
                &json!({"type":"plugin_event","plugin":ID,"min_version":"1.0.0","action":"talk",
                    "params":{"channel":long},"input":turn}),
            );
        }
        let late = invoke_within(host, "set", json!({"entity":"late"}), 300);
        let kept = invoke(host, json!({"entity":"kept"}));
        for _ in 0..10 {
            thread::sleep(Duration::from_millis(100));
            plugin.send(json!({"type":"pong"}));
        }
        drop(late);
        for _ in 0..20 {
            assert_eq!(plugin.receive_message()["type"], "event");
        }
        let press = plugin.receive_message();
        assert_eq!(press["params"]["entity"], "kept");
        assert!(press["timeout_ms"].as_u64().unwrap() < 4500);
        plugin.send(json!({"type":"result","request":press["request"],"ok":true}));
        assert_eq!(answer(&kept), Ok(()));
    }

    #[test]
    fn events_beyond_the_rate_limit_are_dropped_and_counted_in_the_log() {
        let rig = rig(&["/bin/sleep", "30"]);
        let host = &rig.host;
        host.set_usage("m1", used());
        let (mut plugin, _) = Plugin::hello(host, &host.token(ID));
        let turn = json!({"gesture":"clockwise","control":{"module":"m003k","index":0}});
        for _ in 0..MAX_EVENTS_PER_SECOND + 7 {
            event(host, "m1", "talk", turn.clone());
        }
        for _ in 0..MAX_EVENTS_PER_SECOND {
            assert_eq!(plugin.receive_message()["type"], "event");
        }
        // A second later, still answering pings meanwhile.
        for _ in 0..11 {
            thread::sleep(Duration::from_millis(100));
            plugin.send(json!({"type":"pong"}));
        }
        event(host, "m1", "talk", turn.clone());
        assert_eq!(plugin.receive_message()["type"], "event");
        assert_eq!(
            host.statuses()[0].logs,
            [
                "7 input events were not sent: more than 50 a second, or the plugin was not reading them"
            ]
        );
    }

    #[test]
    fn a_plugin_sets_only_the_variables_its_manifest_lists() {
        let rig = rig(&["/bin/sleep", "30"]);
        rig.host.set_usage("m1", used());
        let (mut plugin, _) = Plugin::hello(&rig.host, &rig.host.token(ID));
        plugin.send(json!({"type":"set_variable","id":1,"name":"room","value":"desk"}));
        assert_eq!(plugin.receive(), json!({"type":"reply","id":1,"ok":true}));
        plugin.send(json!({"type":"set_variable","id":2,"name":"agent_state","value":"x"}));
        let refused = plugin.receive();
        assert_eq!(refused["ok"], false);
        assert!(refused["error"].as_str().unwrap().contains("manifest"));
        assert_eq!(
            *rig.forwarded.lock().unwrap(),
            [json!({"type":"set_variable","name":"room","value":"desk"})]
        );
        plugin.send(json!({"type":"log","message":"hello from the plugin"}));
        plugin.send(json!({"type":"ping"}));
        assert_eq!(plugin.receive(), json!({"type":"pong"}));
        assert_eq!(rig.host.statuses()[0].logs, ["hello from the plugin"]);
    }

    #[test]
    fn a_plugin_that_goes_away_fails_its_presses_and_is_restarted() {
        let rig = rig(&["/bin/sleep", "30"]);
        let host = &rig.host;
        host.set_usage("m1", used());
        let (mut plugin, _) = Plugin::hello(host, &host.token(ID));
        let pressed = invoke(host, json!({"entity":"a"}));
        plugin.receive();
        drop(plugin);
        let error = answer(&pressed).unwrap_err();
        assert!(error.contains("closed its connection"), "{error}");
        assert_eq!(host.state_of(ID), Some("restarting"));
        wait_for("the restart", || host.state_of(ID) == Some("starting"));
        // A plugin that stops answering pings is restarted too.
        let (mut plugin, _) = Plugin::hello(host, &host.token(ID));
        assert_eq!(plugin.receive(), json!({"type":"ping"}));
        wait_for("the missed pong", || {
            host.state_of(ID) == Some("restarting")
        });
        assert!(host.statuses()[0].error.as_ref().unwrap().contains("pings"));
    }

    #[test]
    fn a_plugin_that_gave_up_starts_again_when_another_run_starts_using_it() {
        let rig = rig(&["/bin/sleep", "30"]);
        let host = &rig.host;
        host.set_usage("m1", used());
        host.set_usage("m2", used());
        {
            let mut state = host.lock();
            let instance = state.instances.get_mut(ID).unwrap();
            for _ in 0..MAX_FAILURES {
                host.failed(ID, instance, "test failure".into());
            }
        }
        assert_eq!(host.state_of(ID), Some("stopped"));
        // m2 still uses it; m1 executing its profile again resets it all the same.
        host.set_usage("m1", None);
        host.set_usage("m1", used());
        assert_eq!(host.state_of(ID), Some("starting"));
        assert!(host.statuses()[0].error.is_none());
    }

    #[test]
    fn a_plugin_that_never_says_hello_is_restarted() {
        let rig = rig(&["/bin/sleep", "30"]);
        let host = &rig.host;
        host.set_usage("m1", used());
        assert_eq!(host.state_of(ID), Some("starting"));
        wait_for("the startup timeout", || {
            host.state_of(ID) == Some("restarting")
        });
        assert!(
            host.statuses()[0]
                .error
                .as_ref()
                .unwrap()
                .contains("say hello")
        );
        wait_for("the restart", || host.state_of(ID) == Some("starting"));
    }

    #[test]
    fn a_plugin_that_exits_takes_the_helpers_it_started_with_it() {
        let rig = rig(&["/bin/sh", "-c", "sleep 30 & echo $! > helper.pid; exit 3"]);
        let host = &rig.host;
        let pid_file = rig._root.path().join("plugins").join(ID).join("helper.pid");
        host.set_usage("m1", used());
        let mut helper = 0;
        wait_for("the helper's pid", || {
            helper = fs::read_to_string(&pid_file)
                .ok()
                .and_then(|text| text.trim().parse().ok())
                .unwrap_or(0);
            helper > 0
        });
        wait_for("the helper to be stopped", || {
            // SAFETY: signal 0 only asks whether the process exists.
            unsafe { libc::kill(helper, 0) != 0 }
        });
    }

    #[test]
    fn an_invoke_is_refused_variables_its_manifest_does_not_allow() {
        let rig = rig(&["/bin/sleep", "30"]);
        let host = &rig.host;
        host.set_usage("m1", used());
        let _plugin = Plugin::hello(host, &host.token(ID));
        let ask = |variables: Value| {
            let (sender, receiver) = mpsc::channel();
            host.invoke(
                "m1",
                &json!({"type":"plugin_invoke","request":1,"plugin":ID,"min_version":"1.0.0",
                    "action":"set","params":{"entity":"light.desk"},"variables":variables,
                    "timeout_ms":5000}),
                Box::new(move |result| {
                    let _ = sender.send(result);
                }),
            );
            answer(&receiver).unwrap_err()
        };
        // The rig's `entity` does not take variables.
        let error = ask(json!(["entity"]));
        assert!(error.contains("does not take variables"), "{error}");
        let error = ask(json!("entity"));
        assert!(error.contains("not a list of names"), "{error}");
    }

    #[test]
    fn a_plugin_that_keeps_failing_stays_stopped_until_it_is_needed_again() {
        let rig = rig(&["/usr/bin/false"]);
        let host = &rig.host;
        host.set_usage("m1", used());
        // 1 + 2 + 4 + 8 s of backoff would make this slow; the failures are counted directly instead.
        {
            let mut state = host.lock();
            let instance = state.instances.get_mut(ID).unwrap();
            for _ in 0..MAX_FAILURES - 1 {
                instance.failures.push_back(Instant::now());
            }
        }
        wait_for("giving up", || host.state_of(ID) == Some("stopped"));
        assert!(
            host.statuses()[0]
                .error
                .as_ref()
                .unwrap()
                .contains("stays stopped")
        );
        host.set_usage("m1", None);
        host.set_usage("m1", used());
        assert_ne!(host.state_of(ID), Some("stopped"));
    }

    #[test]
    fn a_plugin_lingers_after_its_last_run_and_then_stops() {
        let rig = rig(&["/bin/sleep", "30"]);
        let host = &rig.host;
        host.set_usage("m1", used());
        let generation = host.lock().instances[ID].generation;
        host.set_usage("m1", None);
        // A restart of the module run within the linger keeps the same process.
        host.set_usage("m1", used());
        assert_eq!(host.lock().instances[ID].generation, generation);
        host.set_usage("m1", None);
        wait_for("the stop", || host.state_of(ID).is_none());
        assert!(host.statuses().is_empty());
    }

    /// The example plugin in `examples/plugins`, run for real by the system Python.
    #[test]
    fn the_example_plugin_answers_a_press_and_sets_its_variable() {
        if !Path::new("/usr/bin/python3").is_file() {
            eprintln!("skipped: no /usr/bin/python3");
            return;
        }
        let example = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../examples/plugins/com.example.notify");
        let root = tempfile::Builder::new()
            .prefix("p")
            .tempdir_in("/tmp")
            .unwrap();
        let dir = root.path().join("plugins");
        let folder = dir.join("com.example.notify");
        fs::create_dir_all(&folder).unwrap();
        fs::set_permissions(&folder, fs::Permissions::from_mode(0o755)).unwrap();
        for file in ["plugin.json", "plugin.py"] {
            fs::copy(example.join(file), folder.join(file)).unwrap();
            fs::set_permissions(folder.join(file), fs::Permissions::from_mode(0o644)).unwrap();
        }
        let forwarded = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&forwarded);
        let forward: Forward = Arc::new(move |body: &Value| {
            seen.lock().unwrap().push(body.clone());
            vec![("m1".to_string(), Ok(json!({})))]
        });
        let fingerprint = plugins::fingerprint(&fs::read(folder.join("plugin.json")).unwrap());
        let host = PluginHost::start(
            &dir,
            [("com.example.notify".to_string(), fingerprint)].into(),
            crate::plugin_secrets::memory::secrets(&Default::default()),
            forward,
        )
        .unwrap();
        host.set_usage("m1", Some(["com.example.notify".to_string()].into()));
        wait_for("the example to connect", || {
            host.state_of("com.example.notify") == Some("running")
        });
        let (sender, pressed) = mpsc::channel();
        host.invoke(
            "m1",
            &json!({"plugin":"com.example.notify","min_version":"1.0.0","action":"count","params":{}}),
            Box::new(move |result| {
                let _ = sender.send(result);
            }),
        );
        assert_eq!(answer(&pressed), Ok(()));
        wait_for("the variable write", || {
            !forwarded.lock().unwrap().is_empty()
        });
        assert_eq!(
            forwarded.lock().unwrap()[0],
            json!({"type":"set_variable","name":"plugin_presses","value":1})
        );
        assert_eq!(host.statuses()[0].logs[0], "connected");
        // Hold: the press is an invoke, the release an event, and the plugin logs how long the key was held.
        let key = json!({"gesture":"pressed","control":{"module":"m4315","index":3}});
        let (sender, pressed) = mpsc::channel();
        host.invoke(
            "m1",
            &json!({"plugin":"com.example.notify","min_version":"1.0.0","action":"hold","params":{},"input":key}),
            Box::new(move |result| {
                let _ = sender.send(result);
            }),
        );
        assert_eq!(answer(&pressed), Ok(()));
        host.event(
            "m1",
            &json!({"plugin":"com.example.notify","min_version":"1.0.0","action":"hold","params":{},
                "input":{"gesture":"released","control":{"module":"m4315","index":3}}}),
        );
        wait_for("the hold to be logged", || {
            host.statuses()[0]
                .logs
                .iter()
                .any(|line| line.starts_with("held for ") && line.ends_with(" s"))
        });
        host.stop_all();
    }

    #[test]
    fn a_plugin_that_is_not_enabled_is_not_started_and_not_counted_as_failing() {
        let rig = rig_with(&["/bin/sleep", "30"], |_| Approved::new());
        let host = &rig.host;
        host.set_usage("m1", used());
        assert_eq!(host.state_of(ID), Some("not_enabled"));
        {
            let state = host.lock();
            let instance = &state.instances[ID];
            assert!(instance.child.is_none());
            assert!(instance.failures.is_empty());
            assert!(instance.restart_at.is_none());
        }
        let error = answer(&invoke(host, json!({"entity":"a"}))).unwrap_err();
        assert_eq!(
            error,
            "com.example.lights: not enabled: enable it in Settings > Plugins"
        );
        // The tick never restarts it.
        thread::sleep(TICK * 3);
        assert_eq!(host.state_of(ID), Some("not_enabled"));
        assert!(host.lock().instances[ID].failures.is_empty());
    }

    #[test]
    fn a_plugin_whose_manifest_changed_since_it_was_enabled_is_not_started() {
        let rig = rig_with(&["/bin/sleep", "30"], |_| {
            [(
                ID.to_string(),
                plugins::fingerprint(b"an older plugin.json"),
            )]
            .into()
        });
        let host = &rig.host;
        host.set_usage("m1", used());
        assert_eq!(host.state_of(ID), Some("not_enabled"));
        assert!(
            host.statuses()[0]
                .error
                .as_ref()
                .unwrap()
                .contains("plugin.json changed since it was enabled")
        );
        assert!(host.lock().instances[ID].failures.is_empty());
    }

    #[test]
    fn enabling_a_used_plugin_starts_it_and_disabling_stops_it() {
        let rig = rig_with(&["/bin/sleep", "30"], |_| Approved::new());
        let host = &rig.host;
        host.set_usage("m1", used());
        assert_eq!(host.state_of(ID), Some("not_enabled"));
        host.set_approval(ID, Some(rig.fingerprint.clone()));
        assert_eq!(host.state_of(ID), Some("starting"));
        let (mut plugin, _) = Plugin::hello(host, &host.token(ID));
        let pressed = invoke(host, json!({"entity":"a"}));
        plugin.receive();
        host.set_approval(ID, None);
        assert_eq!(host.state_of(ID), Some("not_enabled"));
        let error = answer(&pressed).unwrap_err();
        assert!(error.contains("disabled in Settings > Plugins"), "{error}");
        assert!(host.lock().instances[ID].child.is_none());
        assert!(host.lock().instances[ID].failures.is_empty());
        // Enabling a fingerprint other than the file's does not start it either.
        host.set_approval(ID, Some(plugins::fingerprint(b"something else")));
        assert_eq!(host.state_of(ID), Some("not_enabled"));
    }

    #[test]
    fn a_secret_is_filled_in_from_the_keychain_and_never_taken_from_the_profile() {
        let rig = rig(&["/bin/sleep", "30"]);
        let host = &rig.host;
        rig.vault
            .values
            .lock()
            .unwrap()
            .insert(format!("{ID}:token"), "s3cret".into());
        host.set_usage("m1", used());
        let (mut plugin, _) = Plugin::hello(host, &host.token(ID));
        let pressed = invoke_action(host, "login", json!({"user":"me"}));
        let asked = plugin.receive();
        assert_eq!(asked["params"], json!({"token":"s3cret","user":"me"}));
        plugin.send(json!({"type":"result","request":asked["request"],"ok":true}));
        assert_eq!(answer(&pressed), Ok(()));

        let error = answer(&invoke_action(host, "login", json!({"token":"old"}))).unwrap_err();
        assert_eq!(
            error,
            "Token is a secret and is kept in the Keychain, not the profile; open the key and save it again"
        );
        host.0.secrets.forget(ID, "token").unwrap();
        let error = answer(&invoke_action(host, "login", json!({}))).unwrap_err();
        assert_eq!(
            error,
            "Token for Lights is not set; set it in Settings > Plugins"
        );
        // Read once, then held.
        assert_eq!(*rig.vault.reads.lock().unwrap(), 1);
    }

    #[test]
    fn a_manifest_changed_while_the_plugin_runs_is_not_started_again_after_a_crash() {
        let rig = rig(&["/bin/sleep", "30"]);
        let host = &rig.host;
        host.set_usage("m1", used());
        assert_eq!(host.state_of(ID), Some("starting"));
        let file = rig
            ._root
            .path()
            .join("plugins")
            .join(ID)
            .join(plugins::MANIFEST_FILE);
        let changed = format!("{} ", fs::read_to_string(&file).unwrap());
        fs::write(&file, changed).unwrap();
        let _ = host
            .lock()
            .instances
            .get_mut(ID)
            .unwrap()
            .child
            .as_mut()
            .unwrap()
            .kill();
        wait_for("the restart to be refused", || {
            host.state_of(ID) == Some("not_enabled")
        });
        let state = host.lock();
        let instance = &state.instances[ID];
        assert!(instance.child.is_none() && instance.restart_at.is_none());
        // Only the crash counted, not the refusal.
        assert_eq!(instance.failures.len(), 1);
        assert!(
            instance
                .last_error
                .as_ref()
                .unwrap()
                .contains("plugin.json changed since it was enabled")
        );
    }

    #[test]
    fn a_keychain_read_that_waits_does_not_hold_up_the_caller() {
        let rig = rig(&["/bin/sleep", "30"]);
        let host = &rig.host;
        rig.vault
            .values
            .lock()
            .unwrap()
            .insert(format!("{ID}:token"), "s3cret".into());
        let (release, hold) = mpsc::channel();
        *rig.vault.hold.lock().unwrap() = Some(hold);
        host.set_usage("m1", used());
        let (mut plugin, _) = Plugin::hello(host, &host.token(ID));
        let started = Instant::now();
        let pressed = invoke_action(host, "login", json!({}));
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "invoke waited for the Keychain"
        );
        release.send(()).unwrap();
        let asked = plugin.receive();
        assert_eq!(asked["params"], json!({"token":"s3cret"}));
        plugin.send(json!({"type":"result","request":asked["request"],"ok":true}));
        assert_eq!(answer(&pressed), Ok(()));
    }

    #[test]
    fn a_press_that_needs_no_keychain_read_is_not_held_up_by_one_that_does() {
        let rig = rig(&["/bin/sleep", "30"]);
        let host = &rig.host;
        rig.vault
            .values
            .lock()
            .unwrap()
            .insert(format!("{ID}:token"), "s3cret".into());
        let (release, hold) = mpsc::channel();
        *rig.vault.hold.lock().unwrap() = Some(hold);
        host.set_usage("m1", used());
        let (mut plugin, _) = Plugin::hello(host, &host.token(ID));
        let waiting = invoke_action(host, "login", json!({}));
        // Let the invoke thread start the read that waits.
        wait_for("the Keychain read", || {
            *rig.vault.reads.lock().unwrap() == 1
        });
        let started = Instant::now();
        let pressed = invoke(host, json!({"entity":"a"}));
        let asked = plugin.receive();
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "the press waited for another press's Keychain read"
        );
        assert_eq!(asked["action"], "set");
        plugin.send(json!({"type":"result","request":asked["request"],"ok":true}));
        assert_eq!(answer(&pressed), Ok(()));
        release.send(()).unwrap();
        let asked = plugin.receive();
        assert_eq!(asked["action"], "login");
        plugin.send(json!({"type":"result","request":asked["request"],"ok":true}));
        assert_eq!(answer(&waiting), Ok(()));
        // Held now: the next one is sent without the invoke thread.
        let pressed = invoke_action(host, "login", json!({}));
        let asked = plugin.receive();
        plugin.send(json!({"type":"result","request":asked["request"],"ok":true}));
        assert_eq!(answer(&pressed), Ok(()));
        assert_eq!(*rig.vault.reads.lock().unwrap(), 1);
    }

    #[test]
    fn one_plugins_keychain_read_does_not_hold_up_anothers() {
        const OTHER: &str = "com.example.other";
        let rig = rig(&["/bin/sleep", "30"]);
        let host = &rig.host;
        let other = install(
            &rig._root.path().join("plugins"),
            OTHER,
            &["/bin/sleep", "30"],
        );
        host.set_approval(OTHER, Some(other));
        for id in [ID, OTHER] {
            rig.vault
                .values
                .lock()
                .unwrap()
                .insert(format!("{id}:token"), format!("{id}-token"));
        }
        let (release, hold) = mpsc::channel();
        *rig.vault.hold.lock().unwrap() = Some(hold);
        *rig.vault.held_plugin.lock().unwrap() = Some(ID.into());
        host.set_usage("m1", Some([ID.to_string(), OTHER.to_string()].into()));
        let (_first, _) = Plugin::hello(host, &host.token(ID));
        let (mut second, _) = Plugin::hello_as(host, OTHER, &host.token(OTHER));
        let waiting = invoke_action(host, "login", json!({}));
        wait_for("the held read", || *rig.vault.reads.lock().unwrap() == 1);
        let started = Instant::now();
        let (sender, pressed) = mpsc::channel();
        host.invoke(
            "m1",
            &json!({"plugin":OTHER,"min_version":"1.0.0","action":"login","params":{},"timeout_ms":5000}),
            Box::new(move |result| {
                let _ = sender.send(result);
            }),
        );
        let asked = second.receive_message();
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "the other plugin's read waited for this one's"
        );
        assert_eq!(asked["params"], json!({"token":"com.example.other-token"}));
        second.send(json!({"type":"result","request":asked["request"],"ok":true}));
        assert_eq!(answer(&pressed), Ok(()));
        release.send(()).unwrap();
        drop(waiting);
    }

    #[test]
    fn a_value_sent_to_the_plugin_is_taken_out_after_it_is_changed_or_forgotten() {
        let rig = rig(&["/bin/sleep", "30"]);
        let host = &rig.host;
        host.0.secrets.set(ID, "token", "first-token").unwrap();
        host.set_usage("m1", used());
        let (mut plugin, _) = Plugin::hello(host, &host.token(ID));
        let pressed = invoke_action(host, "login", json!({}));
        let asked = plugin.receive_message();
        assert_eq!(asked["params"]["token"], "first-token");
        plugin.send(json!({"type":"result","request":asked["request"],"ok":true}));
        assert_eq!(answer(&pressed), Ok(()));
        host.0.secrets.set(ID, "token", "second-token").unwrap();
        plugin.send(json!({"type":"log","message":"still using first-token"}));
        host.0.secrets.forget(ID, "token").unwrap();
        plugin.send(json!({"type":"log","message":"after forget: first-token"}));
        // An error that echoes it: a press on an action without secrets.
        let pressed = invoke(host, json!({"entity":"a"}));
        let asked = plugin.receive_message();
        plugin.send(json!({"type":"result","request":asked["request"],"ok":false,"error":"bad token first-token"}));
        assert_eq!(answer(&pressed), Err("bad token [secret]".into()));
        let logs = host.statuses()[0].logs.join("\n");
        assert_eq!(logs, "still using [secret]\nafter forget: [secret]");
    }

    #[test]
    fn a_press_whose_run_stopped_executing_while_it_waited_is_never_sent() {
        let rig = rig(&["/bin/sleep", "30"]);
        let host = &rig.host;
        rig.vault
            .values
            .lock()
            .unwrap()
            .insert(format!("{ID}:token"), "s3cret".into());
        let (release, hold) = mpsc::channel();
        *rig.vault.hold.lock().unwrap() = Some(hold);
        host.set_usage("m1", used());
        host.set_usage("m2", used());
        let (mut plugin, _) = Plugin::hello(host, &host.token(ID));
        let waiting = invoke_action(host, "login", json!({}));
        wait_for("the held read", || *rig.vault.reads.lock().unwrap() == 1);
        // m1 stops executing; m2 still uses the plugin, which does not rescue m1's press.
        host.set_usage("m1", None);
        release.send(()).unwrap();
        assert_eq!(
            answer(&waiting),
            Err("the run stopped executing before the press could be sent".into())
        );
        // The plugin's next message is m2's press, so m1's never reached it.
        let (sender, pressed) = mpsc::channel();
        host.invoke(
            "m2",
            &json!({"plugin":ID,"min_version":"1.0.0","action":"set","params":{"entity":"a"},"timeout_ms":5000}),
            Box::new(move |result| {
                let _ = sender.send(result);
            }),
        );
        let asked = plugin.receive_message();
        assert_eq!(asked["action"], "set");
        plugin.send(json!({"type":"result","request":asked["request"],"ok":true}));
        assert_eq!(answer(&pressed), Ok(()));
        // The fast path checks it too.
        let error = answer(&invoke(host, json!({"entity":"a"}))).unwrap_err();
        assert_eq!(
            error,
            "the run stopped executing before the press could be sent"
        );
    }

    #[test]
    fn secrets_that_together_break_the_text_limit_fail_the_press() {
        let rig = rig(&["/bin/sleep", "30"]);
        let host = &rig.host;
        // Each fits on its own (at most 2048 bytes); five of them pass the 8192 bytes all text may take.
        for (index, param) in ["s1", "s2", "s3", "s4", "s5"].iter().enumerate() {
            let value = format!("{index}{}", "x".repeat(1999));
            host.0.secrets.set(ID, param, &value).unwrap();
        }
        host.set_usage("m1", used());
        let (mut plugin, _) = Plugin::hello(host, &host.token(ID));
        let error = answer(&invoke_action(host, "bulk", json!({}))).unwrap_err();
        assert_eq!(
            error,
            "com.example.lights: with its secrets filled in, plugin text parameters must total at most 8192 bytes"
        );
        assert!(!error.contains("xxxx"));
        let pressed = invoke(host, json!({"entity":"a"}));
        let asked = plugin.receive_message();
        assert_eq!(asked["action"], "set");
        plugin.send(json!({"type":"result","request":asked["request"],"ok":true}));
        assert_eq!(answer(&pressed), Ok(()));
    }

    #[test]
    fn a_press_that_waited_past_its_deadline_is_never_sent() {
        let rig = rig(&["/bin/sleep", "30"]);
        let host = &rig.host;
        rig.vault
            .values
            .lock()
            .unwrap()
            .insert(format!("{ID}:token"), "s3cret".into());
        let (release, hold) = mpsc::channel();
        *rig.vault.hold.lock().unwrap() = Some(hold);
        host.set_usage("m1", used());
        let (mut plugin, _) = Plugin::hello(host, &host.token(ID));
        let first = invoke_within(host, "login", json!({}), 300);
        wait_for("the Keychain read", || {
            *rig.vault.reads.lock().unwrap() == 1
        });
        let second = invoke_within(host, "login", json!({"user":"me"}), 300);
        // Stays in touch meanwhile, so the host's ping does not count it as hung.
        for _ in 0..4 {
            plugin.send(json!({"type":"ping"}));
            thread::sleep(Duration::from_millis(100));
        }
        release.send(()).unwrap();
        for pressed in [first, second] {
            assert_eq!(
                answer(&pressed),
                Err("the press timed out before it could be sent to the plugin".into())
            );
        }
        // Neither reached the plugin: the next message it gets is the next press.
        let pressed = invoke(host, json!({"entity":"a"}));
        let asked = plugin.receive_message();
        assert_eq!(asked["action"], "set");
        // The deadline sent counts from arrival, so it is at most what the module asked for.
        assert!(asked["timeout_ms"].as_u64().unwrap() <= 5000);
        plugin.send(json!({"type":"result","request":asked["request"],"ok":true}));
        assert_eq!(answer(&pressed), Ok(()));
    }

    #[test]
    fn a_keychain_that_fails_fails_the_press_without_asking_again_each_time() {
        let rig = rig(&["/bin/sleep", "30"]);
        let host = &rig.host;
        rig.vault
            .failing
            .store(true, std::sync::atomic::Ordering::SeqCst);
        host.set_usage("m1", used());
        let _plugin = Plugin::hello(host, &host.token(ID));
        for _ in 0..2 {
            let error = answer(&invoke_action(host, "login", json!({}))).unwrap_err();
            assert_eq!(
                error,
                "Token for Lights could not be read: the Keychain is locked"
            );
        }
        assert_eq!(*rig.vault.reads.lock().unwrap(), 1);
    }

    #[test]
    fn known_secrets_are_taken_out_of_what_the_plugin_sends_back() {
        let rig = rig(&["/bin/sh", "-c", "echo token=s3cret >&2; sleep 30"]);
        let host = &rig.host;
        rig.vault
            .values
            .lock()
            .unwrap()
            .insert(format!("{ID}:token"), "s3cret".into());
        // A press reads the secret first, so it is known before the plugin echoes it.
        host.0.secrets.get(ID, "token").unwrap();
        host.set_usage("m1", used());
        let (mut plugin, _) = Plugin::hello(host, &host.token(ID));
        plugin.send(json!({"type":"log","message":"using s3cret"}));
        let pressed = invoke_action(host, "login", json!({}));
        let asked = plugin.receive();
        plugin.send(json!({"type":"result","request":asked["request"],"ok":false,"error":"s3cret was refused"}));
        assert_eq!(answer(&pressed), Err("[secret] was refused".into()));
        wait_for("the stderr line", || host.statuses()[0].logs.len() == 2);
        let logs = host.statuses()[0].logs.join("\n");
        assert!(!logs.contains("s3cret"), "{logs}");
        assert!(
            logs.contains("using [secret]") && logs.contains("token=[secret]"),
            "{logs}"
        );
    }

    #[test]
    fn a_missing_plugin_says_so() {
        let rig = rig(&["/bin/sleep", "30"]);
        rig.host
            .set_usage("m1", Some(["com.example.gone".to_string()].into()));
        let status = &rig.host.statuses()[0];
        assert_eq!(status.state, "unusable");
        assert!(status.error.as_ref().unwrap().contains("is not installed"));
    }
}
