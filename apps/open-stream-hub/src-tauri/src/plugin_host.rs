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
//! closes its connection: after 1, 2, 4, 8, 16, then 30 s. After [`MAX_FAILURES`] failures within
//! [`FAILURE_WINDOW`] it stays stopped until a profile that uses it is executed again.

use crate::plugins::{self, Manifest};
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
        Arc, Mutex, MutexGuard,
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
/// Waits before each restart, the last one repeating.
const BACKOFF: [u64; 6] = [1, 2, 4, 8, 16, 30];
/// Failures within [`FAILURE_WINDOW`] after which a plugin stays stopped.
const MAX_FAILURES: usize = 5;
const FAILURE_WINDOW: Duration = Duration::from_secs(300);
/// `set_variable` writes a plugin may make per second.
const MAX_SETS_PER_SECOND: usize = 10;
/// Log lines a plugin may send per second, and the longest kept.
const MAX_LOGS_PER_SECOND: usize = 20;
const MAX_LOG_BYTES: usize = 1024;
/// Log lines kept per plugin for Settings.
const KEPT_LOGS: usize = 50;
/// The longest error text from a plugin passed on to a module.
const MAX_ERROR_BYTES: usize = 512;
/// How long a write to a plugin may block.
const WRITE_TIMEOUT: Duration = Duration::from_secs(1);
/// How long a stopping plugin has after SIGTERM before SIGKILL.
const STOP_GRACE: Duration = Duration::from_secs(1);
const TICK: Duration = Duration::from_millis(100);
/// How long a plugin keeps running after the last run that used it stopped executing, so a module run that restarts
/// (a large profile change, a replug) does not restart its plugins.
pub const LINGER: Duration = Duration::from_millis(if cfg!(test) { 300 } else { 5_000 });

/// Answers one invoke; called once, on the host's answer thread.
pub type Reply = Box<dyn FnOnce(Result<(), String>) + Send>;

struct Pending {
    deadline: Instant,
    reply: Reply,
}

/// A connected plugin's socket: the write half and when it last said anything.
struct Conn {
    writer: Mutex<UnixStream>,
    last_seen: Mutex<Instant>,
    pinged: Mutex<Option<Instant>>,
}

impl Conn {
    fn send(&self, message: &Value) -> io::Result<()> {
        let mut line = message.to_string();
        line.push('\n');
        let mut writer = self
            .writer
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        writer.write_all(line.as_bytes())
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
    folder: PathBuf,
    token: String,
    /// Grows at every start, so a connection or a reader of an older process can tell it is stale.
    generation: u64,
    child: Option<Child>,
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
}

impl Instance {
    fn new(folder: PathBuf) -> Self {
        Self {
            manifest: None,
            folder,
            token: String::new(),
            generation: 0,
            child: None,
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
    state: Mutex<State>,
    stopped: AtomicBool,
    answers: Mutex<mpsc::Sender<(Reply, Result<(), String>)>>,
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
    /// Binds the socket in `plugins_dir` (made 0700, replacing a stale socket) and starts serving. `forward` carries
    /// a plugin's `set_variable` to the running modules.
    pub fn start(plugins_dir: &Path, forward: Forward) -> Result<Self, String> {
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
            state: Mutex::new(State::default()),
            stopped: AtomicBool::new(false),
            answers: Mutex::new(answers),
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
    /// executing or ends), then starts the plugins now needed and stops the ones no longer needed. A plugin that
    /// becomes needed again starts fresh, even after it gave up.
    pub fn set_usage(&self, service_id: &str, plugins: Option<BTreeSet<String>>) {
        if self.0.stopped.load(Ordering::SeqCst) {
            return;
        }
        let mut state = self.lock();
        let before = state.needed();
        match plugins.filter(|plugins| !plugins.is_empty()) {
            Some(plugins) => state.usage.insert(service_id.to_string(), plugins),
            None => state.usage.remove(service_id),
        };
        let after = state.needed();
        let now = Instant::now();
        for gone in before.difference(&after) {
            if let Some(instance) = state.instances.get_mut(gone) {
                instance.unused_since = Some(now);
            }
        }
        for new in after.difference(&before) {
            match state.instances.get_mut(new) {
                // Still lingering: keep it, unless it gave up, which a new execution resets.
                Some(instance) => {
                    instance.unused_since = None;
                    if instance.gave_up
                        || (instance.child.is_none() && instance.restart_at.is_none())
                    {
                        instance.gave_up = false;
                        instance.failures.clear();
                        self.launch(new, instance);
                    }
                }
                None => {
                    let mut instance = Instance::new(self.0.dir.join(new));
                    self.launch(new, &mut instance);
                    state.instances.insert(new.clone(), instance);
                }
            }
        }
    }

    /// Stops an instance that is no longer wanted and fails what it still owed.
    fn retire(&self, mut instance: Instance, why: &str) {
        if let Some(child) = instance.child.take() {
            stop_child(child);
        }
        if let Some(conn) = instance.conn.take() {
            let _ = conn
                .writer
                .lock()
                .map(|stream| stream.shutdown(std::net::Shutdown::Both));
        }
        for (_, pending) in instance.pending.drain() {
            self.answer(pending.reply, Err(format!("the plugin was stopped: {why}")));
        }
    }

    /// Starts the plugin's process with a fresh token, reading its manifest again first.
    fn launch(&self, id: &str, instance: &mut Instance) {
        instance.restart_at = None;
        let manifest = match plugins::load(&instance.folder) {
            Ok(manifest) => manifest,
            Err(error) => {
                instance.manifest = None;
                instance.last_error = Some(if instance.folder.exists() {
                    error
                } else {
                    format!("{id} is not installed (no folder {id} in the plugins folder)")
                });
                return;
            }
        };
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
            let mut state = self.lock();
            let Some(instance) = state.instances.get_mut(id) else {
                return;
            };
            if instance.generation != generation {
                return;
            }
            if instance.log_rate.allow(MAX_LOGS_PER_SECOND) {
                let text = truncate(&String::from_utf8_lossy(&line), MAX_LOG_BYTES);
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
            let mut state = self.lock();
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
                        self.launch(&id, instance);
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
    fn connection_lost(&self, id: &str, conn: &Arc<Conn>, why: &str) {
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
        let writer = stream.try_clone().ok()?;
        let _ = writer.set_write_timeout(Some(WRITE_TIMEOUT));
        let conn = Arc::new(Conn {
            writer: Mutex::new(writer),
            last_seen: Mutex::new(Instant::now()),
            pinged: Mutex::new(None),
        });
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
        }
        if conn.send(&json!({"type":"welcome"})).is_err() {
            self.connection_lost(id, &conn, "the plugin's connection broke");
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
                    Err(truncate(
                        message["error"]
                            .as_str()
                            .unwrap_or("the plugin reported a failure"),
                        MAX_ERROR_BYTES,
                    ))
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
                let mut state = self.lock();
                if let Some(instance) = state.instances.get_mut(id) {
                    if instance.log_rate.allow(MAX_LOGS_PER_SECOND) {
                        let text = message["message"].as_str().unwrap_or("");
                        instance.keep_log(truncate(text, MAX_LOG_BYTES));
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
    /// reason it was not asked. Never waits for the plugin.
    pub fn invoke(&self, record: &Value, reply: Reply) {
        let (conn, message, id) = match self.register(record, reply) {
            Ok(registered) => registered,
            Err((error, reply)) => return self.answer(reply, Err(error)),
        };
        if conn.send(&message).is_err() {
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

    /// Checks an invoke against the plugin's state and manifest and registers `reply` for its answer, handing back
    /// the message to send. A refusal hands `reply` back with the reason.
    #[allow(clippy::type_complexity)]
    fn register(
        &self,
        record: &Value,
        reply: Reply,
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
            let params: BTreeMap<String, PluginValue> =
                serde_json::from_value(record["params"].clone())
                    .map_err(|_| "the invoke's parameters are not plugin values".to_string())?;
            // Absent means no variables were filled in; anything else must be a list of names, so a
            // malformed record cannot skip the manifest's check.
            let variables: Vec<String> = match &record["variables"] {
                Value::Null => Vec::new(),
                names => serde_json::from_value(names.clone()).map_err(|_| {
                    "the invoke's variable parameters are not a list of names".to_string()
                })?,
            };
            let timeout = record["timeout_ms"]
                .as_u64()
                .map(Duration::from_millis)
                .unwrap_or(MAX_INVOKE)
                .min(MAX_INVOKE);
            let instance = state.instances.get(&id).ok_or_else(|| {
                format!("{id} is not running: plugins run only while a profile that uses them is executed (Live)")
            })?;
            let Some(manifest) = instance.manifest.as_ref() else {
                return Err(instance
                    .last_error
                    .clone()
                    .unwrap_or_else(|| format!("{id} cannot be used")));
            };
            if !manifest.satisfies(min_version) {
                return Err(format!(
                    "{} {} is older than {min_version}, which this key needs; update the plugin",
                    manifest.name, manifest.version
                ));
            }
            plugins::check_invoke(manifest, action, &params, &variables)?;
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
            if instance.pending.len() >= MAX_IN_FLIGHT {
                return Err(format!(
                    "{id} is busy: {MAX_IN_FLIGHT} presses are still waiting for it"
                ));
            }
            let message = json!({
                "type": "invoke",
                "request": request,
                "action": action,
                "params": params,
                "timeout_ms": u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX),
            });
            Ok((conn, message, id, timeout))
        })();
        match checked {
            Ok((conn, message, id, timeout)) => {
                if let Some(instance) = state.instances.get_mut(&id) {
                    instance.pending.insert(
                        request,
                        Pending {
                            deadline: Instant::now() + timeout,
                            reply,
                        },
                    );
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
    }

    impl Drop for Rig {
        fn drop(&mut self) {
            self.host.stop_all();
        }
    }

    /// A host whose one plugin runs `command`, and a forward that records variable writes.
    fn rig(command: &[&str]) -> Rig {
        let root = tempfile::Builder::new()
            .prefix("p")
            .tempdir_in("/tmp")
            .unwrap();
        let dir = root.path().join("plugins");
        let folder = dir.join(ID);
        fs::create_dir_all(&folder).unwrap();
        fs::set_permissions(&folder, fs::Permissions::from_mode(0o755)).unwrap();
        let manifest = json!({
            "id": ID, "name": "Lights", "version": "1.2.0", "command": command,
            "actions": [{ "id": "set", "label": "Set", "params": [
                { "id": "entity", "label": "Light", "type": "text", "required": true }
            ] }],
            "permissions": { "variables": ["room"] }
        });
        fs::write(folder.join(plugins::MANIFEST_FILE), manifest.to_string()).unwrap();
        let forwarded = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&forwarded);
        let forward: Forward = Arc::new(move |body: &Value| {
            seen.lock().unwrap().push(body.clone());
            vec![("m1".to_string(), Ok(json!({})))]
        });
        let host = PluginHost::start(&dir, forward).unwrap();
        Rig {
            _root: root,
            host,
            forwarded,
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

        fn hello(host: &PluginHost, token: &str) -> (Self, Value) {
            let mut plugin = Self::connect(host);
            plugin.send(json!({"type":"hello","id":ID,"token":token}));
            let answer = plugin.receive();
            (plugin, answer)
        }
    }

    fn invoke(host: &PluginHost, params: Value) -> mpsc::Receiver<Result<(), String>> {
        let (sender, receiver) = mpsc::channel();
        host.invoke(
            &json!({"type":"plugin_invoke","request":1,"plugin":ID,"min_version":"1.0.0",
                "action":"set","params":params,"timeout_ms":5000}),
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
            &json!({"plugin":ID,"min_version":"2.0.0","action":"set","params":{"entity":"a"}}),
            Box::new(move |result| {
                let _ = sender.send(result);
            }),
        );
        assert!(answer(&newer).unwrap_err().contains("older than 2.0.0"));
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
        let host = PluginHost::start(&dir, forward).unwrap();
        host.set_usage("m1", Some(["com.example.notify".to_string()].into()));
        wait_for("the example to connect", || {
            host.state_of("com.example.notify") == Some("running")
        });
        let (sender, pressed) = mpsc::channel();
        host.invoke(
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
        host.stop_all();
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
