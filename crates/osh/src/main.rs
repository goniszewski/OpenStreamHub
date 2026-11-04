//! `osh`: sets and reads OpenStreamHub profile variables from scripts and agent hooks.
//!
//! It sends one JSON line to the running app's local socket, with the token the app writes at each launch, and
//! prints the answer. It never waits long, so a hook calling it cannot hang its agent.
//!
//! ```text
//! osh set agent_state waiting      # text
//! osh set busy true                # bool
//! osh set count 3                  # number
//! osh set --kind text label 42     # force a kind
//! osh get [name]
//! osh ping
//! osh agent state working          # for agent hooks: always exits 0, prints nothing
//! osh agent ask [--timeout 55]     # for a PermissionRequest hook: waits for Accept or Deny on the deck
//! osh agent setup claude|codex     # prints the hook configuration for this osh
//! ```
//!
//! `OSH_SOCKET` and `OSH_TOKEN_FILE` override where the socket and the token are found.

mod registry;

use osh::{
    AGENT_DECISION as DECISION, AGENT_STATE as STATE, ASK_DEFAULT_SECS, Agent, CLAUDE_HEADER,
    DECISION_CLEARED, TRANSLOCATED, is_translocated, setup,
};
use serde_json::{Value, json};
use std::{
    env,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::ExitCode,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

/// The app's bundle identifier (tauri.conf.json), which names its data directory.
const APP_IDENTIFIER: &str = "dev.openstreamhub";
/// How long `osh` waits for the app, in total per step.
const TIMEOUT: Duration = Duration::from_secs(3);
/// The longest answer read back.
const MAX_ANSWER: u64 = 64 * 1024;

const USAGE: &str = "usage: osh set [--kind bool|number|text] <name> <value>\n       osh get [name]\n       osh ping\n       osh agent state <working|waiting|done|idle|end>\n       osh agent ask [--timeout SECS]\n       osh agent setup <claude|codex>";

/// The states `agent state` takes (the terminal starter's `agent_state`).
const AGENT_STATES: [&str; 5] = ["working", "waiting", "done", "idle", "end"];

/// The number variable counting the sessions that wait for the deck (optional in a profile).
const WAITING: &str = "agent_waiting";
/// The registry of sessions and its lock, next to the socket.
const REGISTRY: &str = "agents.json";
const REGISTRY_LOCK: &str = "agents.lock";
/// How long a hook waits for the registry lock.
const REGISTRY_WAIT: Duration = Duration::from_secs(2);
/// After one prompt is answered the next waits this long, showing Working, before it asks: a repeated press for the
/// first prompt must not answer the second, which was never shown.
const SETTLE: Duration = Duration::from_millis(1500);
/// How long `agent ask` waits for the deck at most (the default is `osh::ASK_DEFAULT_SECS`).
const ASK_MAX_SECS: u64 = 600;
/// How often `agent ask` reads the decision.
const POLL: Duration = Duration::from_millis(200);
/// How long a hook waits for its JSON on stdin, and the most read.
const STDIN_WAIT: Duration = Duration::from_millis(500);
const MAX_STDIN: u64 = 1024 * 1024;
/// The lock file, next to the socket, that `agent ask` holds while it waits.
const ASK_LOCK: &str = "ask.lock";

#[derive(Debug, PartialEq)]
enum Command {
    Set { name: String, value: Value },
    Get { name: Option<String> },
    Ping,
    AgentState(String),
    AgentAsk { timeout: Duration },
    AgentSetup(Agent),
}

/// An error with its exit code: 2 for usage, 3 when the app is not reachable, 1 for a refused request.
#[derive(Debug, PartialEq)]
struct Failure(u8, String);

fn usage(message: &str) -> Failure {
    Failure(2, format!("osh: {message}\n{USAGE}"))
}

/// Reads a value as bool (`true`/`false`), whole number, or else text; `kind` forces one.
fn infer_value(text: &str, kind: Option<&str>) -> Result<Value, Failure> {
    let number = || {
        text.parse::<i64>()
            .ok()
            .map(|number| json!(number.clamp(i32::MIN.into(), i32::MAX.into())))
    };
    let bool = || match text {
        "true" => Some(json!(true)),
        "false" => Some(json!(false)),
        _ => None,
    };
    match kind {
        None => Ok(bool().or_else(number).unwrap_or_else(|| json!(text))),
        Some("text") => Ok(json!(text)),
        Some("bool") => bool().ok_or_else(|| usage("a bool value is true or false")),
        Some("number") => number().ok_or_else(|| usage("a number value is a whole number")),
        Some(other) => Err(usage(&format!("unknown kind {other:?}"))),
    }
}

fn parse(args: &[String]) -> Result<Command, Failure> {
    let mut rest = args.iter().map(String::as_str);
    match rest.next() {
        Some("set") => {
            let mut kind = None;
            let mut positional = Vec::new();
            while let Some(arg) = rest.next() {
                match arg {
                    "--kind" => {
                        kind = Some(rest.next().ok_or_else(|| usage("--kind needs a value"))?)
                    }
                    _ => positional.push(arg),
                }
            }
            let [name, value] = positional[..] else {
                return Err(usage("set takes a name and a value"));
            };
            Ok(Command::Set {
                name: name.to_string(),
                value: infer_value(value, kind)?,
            })
        }
        Some("get") => match (rest.next(), rest.next()) {
            (name, None) => Ok(Command::Get {
                name: name.map(str::to_string),
            }),
            _ => Err(usage("get takes at most one name")),
        },
        Some("ping") if rest.next().is_none() => Ok(Command::Ping),
        Some("agent") => parse_agent(&rest.collect::<Vec<_>>()),
        Some("-h" | "--help" | "help") => Err(Failure(0, USAGE.to_string())),
        _ => Err(usage("unknown command")),
    }
}

fn parse_agent(args: &[&str]) -> Result<Command, Failure> {
    match args {
        ["state", state] if AGENT_STATES.contains(state) => {
            Ok(Command::AgentState(state.to_string()))
        }
        ["ask"] => Ok(Command::AgentAsk {
            timeout: Duration::from_secs(ASK_DEFAULT_SECS),
        }),
        ["ask", "--timeout", secs] => match secs.parse::<u64>() {
            Ok(secs @ 1..=ASK_MAX_SECS) => Ok(Command::AgentAsk {
                timeout: Duration::from_secs(secs),
            }),
            _ => Err(usage(&format!(
                "--timeout takes 1 to {ASK_MAX_SECS} seconds"
            ))),
        },
        ["setup", "claude"] => Ok(Command::AgentSetup(Agent::Claude)),
        ["setup", "codex"] => Ok(Command::AgentSetup(Agent::Codex)),
        _ => Err(usage(
            "agent takes state <working|waiting|done|idle|end>, ask [--timeout SECS] or setup <claude|codex>",
        )),
    }
}

/// `agent state` and `agent ask` run as agent hooks, where a failing exit code can block the agent (exit 2 blocks
/// a prompt in Claude Code), so they always exit 0.
fn is_hook(args: &[String]) -> bool {
    args.first().is_some_and(|arg| arg == "agent")
        && args
            .get(1)
            .is_some_and(|arg| arg == "state" || arg == "ask")
}

fn data_dir() -> Option<PathBuf> {
    let home = env::var_os("HOME").map(PathBuf::from);
    let base = if cfg!(target_os = "macos") {
        home?.join("Library/Application Support")
    } else {
        env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| home.map(|home| home.join(".local/share")))?
    };
    Some(base.join(APP_IDENTIFIER).join("ingress"))
}

fn locate() -> Result<Paths, Failure> {
    let dir = data_dir();
    let pick = |variable: &str, file: &str| {
        env::var_os(variable)
            .map(PathBuf::from)
            .or_else(|| dir.as_ref().map(|dir| dir.join(file)))
            .ok_or_else(|| Failure(3, format!("osh: set {variable}; HOME is not set")))
    };
    Ok((
        pick("OSH_SOCKET", "ingress.sock")?,
        pick("OSH_TOKEN_FILE", "ingress.token")?,
    ))
}

type Paths = (PathBuf, PathBuf);

fn not_running() -> Failure {
    Failure(3, "osh: OpenStreamHub is not running".to_string())
}

/// Sends one request line with the token and returns the answer's `result`.
#[cfg(unix)]
fn request(socket: &Path, token_file: &Path, mut body: Value) -> Result<Value, Failure> {
    use std::io::Read;
    let token = std::fs::read_to_string(token_file).map_err(|_| not_running())?;
    body["token"] = json!(token.trim());
    let stream = std::os::unix::net::UnixStream::connect(socket).map_err(|_| not_running())?;
    let io = |error: std::io::Error| Failure(3, format!("osh: the app did not answer: {error}"));
    stream.set_read_timeout(Some(TIMEOUT)).map_err(io)?;
    stream.set_write_timeout(Some(TIMEOUT)).map_err(io)?;
    (&stream)
        .write_all(format!("{body}\n").as_bytes())
        .map_err(io)?;
    let mut line = String::new();
    BufReader::new((&stream).take(MAX_ANSWER))
        .read_line(&mut line)
        .map_err(io)?;
    let answer: Value = serde_json::from_str(&line)
        .map_err(|_| Failure(3, "osh: the app's answer was not understood".to_string()))?;
    if answer["ok"] == true {
        Ok(answer.get("result").cloned().unwrap_or(Value::Null))
    } else {
        let error = answer["error"]
            .as_str()
            .unwrap_or("the request was refused");
        Err(Failure(1, format!("osh: {error}")))
    }
}

#[cfg(not(unix))]
fn request(_: &Path, _: &Path, _: Value) -> Result<Value, Failure> {
    Err(Failure(
        3,
        "osh: only macOS and Linux are supported".to_string(),
    ))
}

/// Text as it is, everything else as JSON.
fn plain(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| value.to_string())
}

/// The variables of every running module: `[{"service":..,"name":..,"value":..}]`.
fn variables(socket: &Path, token: &Path) -> Result<Vec<Value>, Failure> {
    let result = request(socket, token, json!({"type":"get_variables"}))?;
    Ok(result["variables"].as_array().cloned().unwrap_or_default())
}

fn set_text(socket: &Path, token: &Path, name: &str, value: &str) -> Result<(), Failure> {
    request(
        socket,
        token,
        json!({"type":"set_variable","name":name,"value":value}),
    )
    .map(|_| ())
}

/// Clears `agent_decision` in strict mode: an error unless every running module that declares it cleared it.
fn clear_decision(socket: &Path, token: &Path) -> Result<(), Failure> {
    request(
        socket,
        token,
        json!({"type":"set_variable","name":DECISION,"value":DECISION_CLEARED,"strict":true}),
    )
    .map(|_| ())
}

/// The lock `agent ask` holds while it waits, next to the socket.
fn ask_lock_path(socket: &Path) -> PathBuf {
    socket
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(ASK_LOCK)
}

/// Takes an exclusive advisory lock on `path` without waiting. `Ok(None)` when another process (or another open
/// of the file) holds it. The lock lasts as long as the returned file is open, and the system releases it when the
/// process dies. The file is created 0600 and never opened through a symbolic link.
#[cfg(unix)]
fn try_lock(path: &Path) -> std::io::Result<Option<std::fs::File>> {
    use std::os::unix::{fs::OpenOptionsExt, io::AsRawFd};
    #[cfg(any(target_os = "macos", not(target_os = "linux")))]
    const O_NOFOLLOW: i32 = 0x0100;
    #[cfg(all(
        target_os = "linux",
        any(
            target_arch = "aarch64",
            target_arch = "arm",
            target_arch = "powerpc64"
        )
    ))]
    const O_NOFOLLOW: i32 = 0o100000;
    #[cfg(all(
        target_os = "linux",
        not(any(
            target_arch = "aarch64",
            target_arch = "arm",
            target_arch = "powerpc64"
        ))
    ))]
    const O_NOFOLLOW: i32 = 0o400000;
    // The same values on macOS and Linux.
    const LOCK_EX: i32 = 2;
    const LOCK_NB: i32 = 4;
    unsafe extern "C" {
        fn flock(fd: i32, operation: i32) -> i32;
    }
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(O_NOFOLLOW)
        .open(path)?;
    // The descriptor stays open for the call: `file` owns it.
    if unsafe { flock(file.as_raw_fd(), LOCK_EX | LOCK_NB) } == 0 {
        return Ok(Some(file));
    }
    let error = std::io::Error::last_os_error();
    if error.kind() == std::io::ErrorKind::WouldBlock {
        Ok(None)
    } else {
        Err(error)
    }
}

#[cfg(not(unix))]
fn try_lock(_: &Path) -> std::io::Result<Option<std::fs::File>> {
    Err(std::io::Error::from(std::io::ErrorKind::Unsupported))
}

/// Set by SIGINT or SIGTERM while `agent ask` waits.
static INTERRUPTED: AtomicBool = AtomicBool::new(false);

#[cfg(unix)]
fn catch_interrupts() {
    extern "C" fn on_signal(_: i32) {
        INTERRUPTED.store(true, Ordering::SeqCst);
    }
    unsafe extern "C" {
        fn signal(signum: i32, handler: extern "C" fn(i32)) -> usize;
    }
    // SIGINT and SIGTERM are 2 and 15 on macOS and Linux. The handler only stores to an atomic.
    unsafe {
        signal(2, on_signal);
        signal(15, on_signal);
    }
}

#[cfg(not(unix))]
fn catch_interrupts() {}

/// The hook's JSON from stdin, or empty: never read from a terminal, and never waited on for long, so a hook
/// runner that keeps stdin open cannot stall the hook.
fn read_stdin() -> String {
    use std::io::{IsTerminal, Read};
    if std::io::stdin().is_terminal() {
        return String::new();
    }
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut text = String::new();
        let _ = std::io::stdin().take(MAX_STDIN).read_to_string(&mut text);
        let _ = sender.send(text);
    });
    receiver.recv_timeout(STDIN_WAIT).unwrap_or_default()
}

/// What a PermissionRequest hook prints for the deck's answer.
fn hook_answer(behavior: &str) -> String {
    let mut decision = json!({"behavior": behavior});
    if behavior == "deny" {
        decision["message"] = json!("Denied on the deck");
    }
    json!({"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":decision}})
        .to_string()
}

/// Takes the registry lock, waiting a little for another hook that holds it.
fn registry_lock(socket: &Path) -> Option<std::fs::File> {
    let path = dir_of(socket).join(REGISTRY_LOCK);
    let deadline = Instant::now() + REGISTRY_WAIT;
    loop {
        match try_lock(&path) {
            Ok(Some(file)) => return Some(file),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            _ => return None,
        }
    }
}

fn dir_of(socket: &Path) -> &Path {
    socket.parent().unwrap_or_else(|| Path::new("."))
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64)
}

/// Records one entry's state and shows the summary of all entries on the deck.
fn publish(socket: &Path, token: &Path, changes: &[(&str, &str)]) {
    // Without the lock nothing is written: a raw state could hide another session's prompt.
    let Some(_lock) = registry_lock(socket) else {
        return;
    };
    let path = dir_of(socket).join(REGISTRY);
    let now = now_ms();
    let mut sessions = registry::load(&path);
    registry::prune(&mut sessions, now, &registry::alive);
    for (id, state) in changes {
        registry::record(&mut sessions, id, state, now, std::process::id());
    }
    let _ = registry::save(&path, &sessions);
    let summary = registry::summary(&sessions);
    let _ = set_text(socket, token, STATE, summary.state);
    // Optional: a profile that does not declare it refuses, which is fine.
    let _ = request(
        socket,
        token,
        json!({"type":"set_variable","name":WAITING,"value":summary.waiting}),
    );
}

/// Whether session `id` is the one whose ask goes next (the registry says nothing against it).
fn is_next(socket: &Path, id: &str) -> bool {
    let path = dir_of(socket).join(REGISTRY);
    let mut sessions = registry::load(&path);
    registry::prune(&mut sessions, now_ms(), &registry::alive);
    registry::front(&sessions).is_none_or(|front| front == id)
}

/// Shows the request on the deck and waits for Accept or Deny. Returns what the hook prints: the answer, or
/// nothing, which leaves the agent's own permission prompt in charge. Only an explicit `allow` on the deck allows;
/// any error, the timeout or an interrupt prints nothing.
///
/// Sessions share the deck: this one is recorded as waiting (the deck shows Needs you, and the count of those
/// waiting), then it queues for the ask lock, oldest first, so Accept and Deny answer one prompt at a time. The
/// timeout covers the queue too. The decision is cleared in strict mode before the wait; when that does not reach
/// every module that declares it, the ask prints nothing.
fn ask(socket: &Path, token: &Path, hook: &Value, timeout: Duration) -> String {
    if hook
        .get("hook_event_name")
        .is_some_and(|event| event != "PermissionRequest")
    {
        return String::new();
    }
    let deadline = Instant::now() + timeout;
    let Ok(declared) = variables(socket, token) else {
        return String::new();
    };
    if ![STATE, DECISION]
        .iter()
        .all(|name| declared.iter().any(|variable| variable["name"] == *name))
    {
        return String::new();
    }
    let session = registry::session_id(hook);
    let key = registry::ask_key(&session, std::process::id());
    publish(socket, token, &[(&key, "waiting")]);
    // The lock is held until the decision is cleared again, so the next ask in line starts from a clean slate.
    let turn = queue_for_turn(socket, &key, deadline);
    let decision = turn.as_ref().and_then(|(_, queued)| {
        if *queued {
            // The press that answered the previous prompt may be repeated: show Working for a moment, clear what
            // that left behind, and only then ask, so a late press never reaches this prompt.
            publish(socket, token, &[(&key, "settling")]);
            sleep_until(Instant::now() + SETTLE, deadline);
            clear_decision(socket, token).ok()?;
            publish(socket, token, &[(&key, "waiting")]);
        }
        clear_decision(socket, token).ok()?;
        await_decision(socket, token, deadline)
    });
    // Only the ask whose turn it was may clear: another's answer could be waiting.
    if turn.is_some() {
        let _ = clear_decision(socket, token);
    }
    drop(turn);
    match decision {
        Some(behavior) => {
            // The session carries on with the tool: it works until its own hooks say more.
            publish(socket, token, &[(&key, "idle"), (&session, "working")]);
            hook_answer(behavior)
        }
        None => {
            publish(socket, token, &[(&key, "idle")]);
            String::new()
        }
    }
}

/// Sleeps until `until` or the deadline, and stops early on an interrupt.
fn sleep_until(until: Instant, deadline: Instant) {
    let end = until.min(deadline);
    while Instant::now() < end && !INTERRUPTED.load(Ordering::SeqCst) {
        std::thread::sleep(POLL.min(end.saturating_duration_since(Instant::now())));
    }
}

/// Waits until this ask is first in line and takes the ask lock; the lock is the returned file, with whether the
/// ask had to queue. None on the deadline, an interrupt, or when no lock can be had.
fn queue_for_turn(socket: &Path, key: &str, deadline: Instant) -> Option<(std::fs::File, bool)> {
    let lock = ask_lock_path(socket);
    let mut queued = false;
    loop {
        if INTERRUPTED.load(Ordering::SeqCst) || Instant::now() >= deadline {
            return None;
        }
        if is_next(socket, key) {
            match try_lock(&lock) {
                Ok(Some(file)) => return Some((file, queued)),
                Ok(None) => {}
                Err(_) => return None,
            }
        }
        queued = true;
        std::thread::sleep(POLL);
    }
}

/// Polls `agent_decision` until the deck says `allow` or `deny` (a `deny` from any module wins). None on the
/// deadline, an interrupt, or when the app or the variable goes away.
fn await_decision(socket: &Path, token: &Path, deadline: Instant) -> Option<&'static str> {
    loop {
        if INTERRUPTED.load(Ordering::SeqCst) {
            return None;
        }
        let decisions: Vec<Value> = variables(socket, token)
            .ok()?
            .into_iter()
            .filter(|variable| variable["name"] == DECISION)
            .map(|variable| variable["value"].clone())
            .collect();
        if decisions.is_empty() {
            return None;
        }
        if decisions.iter().any(|value| value == "deny") {
            return Some("deny");
        }
        if decisions.iter().any(|value| value == "allow") {
            return Some("allow");
        }
        let left = deadline.checked_duration_since(Instant::now())?;
        if left.is_zero() {
            return None;
        }
        std::thread::sleep(POLL.min(left));
    }
}

/// The hook configuration for the `osh` at `osh` (resolved), refused when macOS runs it from a translocated copy:
/// hooks calling that path would stop working at the next launch.
fn setup_for(agent: Agent, osh: &Path) -> Result<String, Failure> {
    let osh = osh.to_string_lossy();
    if is_translocated(&osh) {
        return Err(Failure(
            1,
            format!("osh: this copy runs from a temporary location. {TRANSLOCATED}."),
        ));
    }
    if agent == Agent::Claude {
        eprintln!("{CLAUDE_HEADER}");
    }
    Ok(setup(agent, &osh))
}

/// Records the state of the session a hook belongs to; the deck shows the summary of all sessions.
fn set_state(socket: &Path, token: &Path, hook: &Value, state: &str) {
    publish(socket, token, &[(&registry::session_id(hook), state)]);
}

/// Runs one command and returns what to print on stdout. `locate` finds the socket and the token file.
fn run(args: &[String], locate: &dyn Fn() -> Result<Paths, Failure>) -> Result<String, Failure> {
    let command = parse(args)?;
    match command {
        Command::AgentSetup(agent) => {
            let osh = env::current_exe()
                .and_then(|path| path.canonicalize())
                .map_err(|error| Failure(1, format!("osh: cannot find its own path: {error}")))?;
            return setup_for(agent, &osh);
        }
        Command::AgentState(state) => {
            let hook = serde_json::from_str(&read_stdin()).unwrap_or(Value::Null);
            if let Ok((socket, token)) = locate() {
                set_state(&socket, &token, &hook, &state);
            }
            return Ok(String::new());
        }
        Command::AgentAsk { timeout } => {
            catch_interrupts();
            let hook = serde_json::from_str(&read_stdin()).unwrap_or(Value::Null);
            return Ok(locate()
                .map(|(socket, token)| ask(&socket, &token, &hook, timeout))
                .unwrap_or_default());
        }
        _ => {}
    }
    let (socket, token) = locate()?;
    match command {
        Command::Ping => request(&socket, &token, json!({"type":"ping"})).map(|_| "ok".into()),
        Command::Set { name, value } => request(
            &socket,
            &token,
            json!({"type":"set_variable","name":name,"value":value}),
        )
        .map(|_| String::new()),
        Command::Get { name } => {
            let variables = variables(&socket, &token)?;
            let lines: Vec<String> = match &name {
                Some(name) => variables
                    .iter()
                    .filter(|variable| variable["name"] == name.as_str())
                    .map(|variable| plain(&variable["value"]))
                    .collect(),
                None => variables
                    .iter()
                    .map(|variable| {
                        format!("{}={}", plain(&variable["name"]), plain(&variable["value"]))
                    })
                    .collect(),
            };
            match name {
                Some(name) if lines.is_empty() => Err(Failure(
                    1,
                    format!("osh: no running profile declares variable {name}"),
                )),
                _ => Ok(lines.join("\n")),
            }
        }
        Command::AgentState(_) | Command::AgentAsk { .. } | Command::AgentSetup(_) => {
            unreachable!("handled above")
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    match run(&args, &locate) {
        Ok(output) => {
            if !output.is_empty() {
                println!("{output}");
            }
            ExitCode::SUCCESS
        }
        Err(Failure(code, message)) => {
            if code == 0 {
                println!("{message}");
            } else {
                eprintln!("{message}");
            }
            ExitCode::from(if is_hook(&args) { 0 } else { code })
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;

    fn args(line: &str) -> Vec<String> {
        line.split_whitespace().map(str::to_string).collect()
    }

    #[test]
    fn values_are_inferred_and_kinds_can_be_forced() {
        assert_eq!(infer_value("true", None).unwrap(), json!(true));
        assert_eq!(infer_value("-3", None).unwrap(), json!(-3));
        assert_eq!(infer_value("waiting", None).unwrap(), json!("waiting"));
        assert_eq!(infer_value("42", Some("text")).unwrap(), json!("42"));
        assert_eq!(infer_value("99999999999", None).unwrap(), json!(i32::MAX));
        assert_eq!(infer_value("yes", Some("bool")).unwrap_err().0, 2);
        assert_eq!(infer_value("1.5", Some("number")).unwrap_err().0, 2);
        assert_eq!(infer_value("x", Some("list")).unwrap_err().0, 2);
    }

    #[test]
    fn commands_are_parsed_strictly() {
        assert_eq!(
            parse(&args("set --kind text label 42")).unwrap(),
            Command::Set {
                name: "label".into(),
                value: json!("42")
            }
        );
        assert_eq!(
            parse(&args("get busy")).unwrap(),
            Command::Get {
                name: Some("busy".into())
            }
        );
        assert_eq!(parse(&args("ping")).unwrap(), Command::Ping);
        for bad in ["", "set name", "set a b c", "get a b", "ping now", "nope"] {
            assert_eq!(parse(&args(bad)).unwrap_err().0, 2, "{bad}");
        }
    }

    /// A one-shot fake app: answers the first line with `answer`, and returns the request it read.
    fn fake_app(dir: &std::path::Path, answer: Value) -> std::thread::JoinHandle<Value> {
        let listener = UnixListener::bind(dir.join("s")).unwrap();
        std::fs::write(dir.join("t"), "abc\n").unwrap();
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(&stream).read_line(&mut line).unwrap();
            (&stream)
                .write_all(format!("{answer}\n").as_bytes())
                .unwrap();
            serde_json::from_str(&line).unwrap()
        })
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = env::temp_dir().join(format!("osh-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_set_sends_the_token_and_a_refusal_is_an_error() {
        let dir = temp_dir("set");
        let app = fake_app(&dir, json!({"ok":true,"result":{"applied":1}}));
        let result = request(
            &dir.join("s"),
            &dir.join("t"),
            json!({"type":"set_variable","name":"busy","value":true}),
        );
        assert!(result.is_ok(), "{result:?}");
        let sent = app.join().unwrap();
        assert_eq!(sent["token"], "abc");
        assert_eq!(sent["value"], true);

        std::fs::remove_file(dir.join("s")).unwrap();
        let app = fake_app(
            &dir,
            json!({"ok":false,"error":"variable nope is not declared"}),
        );
        let refused = request(&dir.join("s"), &dir.join("t"), json!({"type":"ping"})).unwrap_err();
        assert_eq!(
            refused,
            Failure(1, "osh: variable nope is not declared".into())
        );
        app.join().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_app_is_reported_as_not_running() {
        let dir = temp_dir("none");
        let missing = request(&dir.join("s"), &dir.join("t"), json!({"type":"ping"})).unwrap_err();
        assert_eq!(missing.0, 3);
        // A stale token without a listener is the same.
        std::fs::write(dir.join("t"), "abc").unwrap();
        let stale = request(&dir.join("s"), &dir.join("t"), json!({"type":"ping"})).unwrap_err();
        assert_eq!(stale, not_running());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn agent_commands_are_parsed_strictly() {
        assert_eq!(
            parse(&args("agent state waiting")).unwrap(),
            Command::AgentState("waiting".into())
        );
        assert_eq!(
            parse(&args("agent ask")).unwrap(),
            Command::AgentAsk {
                timeout: Duration::from_secs(55)
            }
        );
        assert_eq!(
            parse(&args("agent ask --timeout 600")).unwrap(),
            Command::AgentAsk {
                timeout: Duration::from_secs(600)
            }
        );
        assert_eq!(
            parse(&args("agent setup codex")).unwrap(),
            Command::AgentSetup(Agent::Codex)
        );
        for bad in [
            "agent",
            "agent state",
            "agent state busy",
            "agent state done now",
            "agent ask --timeout",
            "agent ask --timeout 0",
            "agent ask --timeout 601",
            "agent ask --timeout -1",
            "agent ask 5",
            "agent setup",
            "agent setup gemini",
        ] {
            assert_eq!(parse(&args(bad)).unwrap_err().0, 2, "{bad}");
        }
        assert!(is_hook(&args("agent state nope")));
        assert!(is_hook(&args("agent ask --timeout 0")));
        assert!(!is_hook(&args("agent setup claude")));
        assert!(!is_hook(&args("set agent_state done")));
    }

    #[test]
    fn agent_state_succeeds_silently_whether_or_not_the_app_answers() {
        let dir = temp_dir("state");
        let paths = || Ok((dir.join("s"), dir.join("t")));
        // Not running.
        assert_eq!(run(&args("agent state working"), &paths), Ok(String::new()));
        // Running, but the variable is not declared: the refusal is swallowed.
        let app = fake_app(
            &dir,
            json!({"ok":false,"error":"variable agent_state is not declared"}),
        );
        assert_eq!(run(&args("agent state done"), &paths), Ok(String::new()));
        let sent = app.join().unwrap();
        assert_eq!(sent["name"], "agent_state");
        assert_eq!(sent["value"], "done");
        // No HOME or override: still nothing.
        let nowhere = || Err(not_running());
        assert_eq!(run(&args("agent state idle"), &nowhere), Ok(String::new()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// What the fake deck does once `agent_state` is set to waiting.
    #[derive(Clone, Copy)]
    enum Deck {
        /// Sets `agent_decision` to this after the second read.
        Press(&'static str),
        /// Never answers.
        Silent,
        /// The app quits at the first read.
        Vanish,
        /// Never answers, and refuses every strict set (a module that did not apply it).
        RefuseStrict,
    }

    /// A fake app serving many connections: it keeps the declared text variables, logs every request, and plays
    /// `deck` once `agent_state` is set to waiting.
    fn fake_service(
        dir: &Path,
        declared: &[(&str, &str)],
        deck: Deck,
    ) -> std::sync::Arc<std::sync::Mutex<Vec<Value>>> {
        let listener = UnixListener::bind(dir.join("s")).unwrap();
        std::fs::write(dir.join("t"), "abc\n").unwrap();
        let log = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = log.clone();
        let mut variables: Vec<(String, Value)> = declared
            .iter()
            .map(|(name, value)| (name.to_string(), json!(value)))
            .collect();
        std::thread::spawn(move || {
            let mut reads_while_waiting = 0;
            for stream in listener.incoming() {
                let Ok(stream) = stream else { break };
                let mut line = String::new();
                BufReader::new(&stream).read_line(&mut line).unwrap();
                let request: Value = serde_json::from_str(&line).unwrap();
                let waiting = variables
                    .iter()
                    .any(|(name, value)| name == STATE && value == "waiting");
                let answer = match request["type"].as_str() {
                    Some("get_variables") => {
                        if waiting {
                            reads_while_waiting += 1;
                            match deck {
                                Deck::Vanish => break,
                                Deck::Press(value) if reads_while_waiting == 2 => {
                                    for (name, slot) in &mut variables {
                                        if name == DECISION {
                                            *slot = json!(value);
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                        let listed: Vec<Value> = variables
                            .iter()
                            .map(|(name, value)| json!({"service":"m4315","name":name,"value":value}))
                            .collect();
                        json!({"ok":true,"result":{"variables":listed}})
                    }
                    Some("set_variable")
                        if matches!(deck, Deck::RefuseStrict) && request["strict"] == true =>
                    {
                        json!({"ok":false,"error":"applied by 1 of 2 modules"})
                    }
                    Some("set_variable") => {
                        match variables
                            .iter_mut()
                            .find(|(name, _)| request["name"] == *name)
                        {
                            Some((_, slot)) => {
                                *slot = request["value"].clone();
                                json!({"ok":true,"result":{"applied":1}})
                            }
                            None => json!({"ok":false,"error":"not declared"}),
                        }
                    }
                    _ => json!({"ok":true,"result":{}}),
                };
                seen.lock().unwrap().push(request);
                (&stream)
                    .write_all(format!("{answer}\n").as_bytes())
                    .unwrap();
            }
        });
        log
    }

    /// Every `set_variable` request the fake saw, as `name=value`.
    fn all_sets(log: &std::sync::Mutex<Vec<Value>>) -> Vec<String> {
        log.lock()
            .unwrap()
            .iter()
            .filter(|request| request["type"] == "set_variable")
            .map(|request| format!("{}={}", plain(&request["name"]), plain(&request["value"])))
            .collect()
    }

    /// The sets without the optional waiting counter.
    fn sets(log: &std::sync::Mutex<Vec<Value>>) -> Vec<String> {
        all_sets(log)
            .into_iter()
            .filter(|set| !set.starts_with("agent_waiting="))
            .collect()
    }

    const BOTH: [(&str, &str); 2] = [(STATE, "idle"), (DECISION, "none")];
    const HOOK: &str = r#"{"hook_event_name":"PermissionRequest","tool_name":"Bash","tool_input":{"command":"ls"}}"#;

    fn ask_in(dir: &Path, timeout_ms: u64) -> String {
        let hook: Value = serde_json::from_str(HOOK).unwrap();
        ask(
            &dir.join("s"),
            &dir.join("t"),
            &hook,
            Duration::from_millis(timeout_ms),
        )
    }

    #[test]
    fn ask_allows_only_on_an_allow_from_the_deck() {
        let dir = temp_dir("allow");
        let log = fake_service(&dir, &BOTH, Deck::Press("allow"));
        let output: Value = serde_json::from_str(&ask_in(&dir, 5_000)).unwrap();
        assert_eq!(
            output,
            json!({"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow"}}})
        );
        assert_eq!(
            all_sets(&log),
            [
                "agent_state=waiting",
                "agent_waiting=1",
                "agent_decision=none",
                "agent_decision=none",
                "agent_state=working",
                "agent_waiting=0"
            ]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ask_denies_with_a_message() {
        let dir = temp_dir("deny");
        let log = fake_service(&dir, &BOTH, Deck::Press("deny"));
        let output: Value = serde_json::from_str(&ask_in(&dir, 5_000)).unwrap();
        assert_eq!(
            output["hookSpecificOutput"]["decision"],
            json!({"behavior":"deny","message":"Denied on the deck"})
        );
        assert_eq!(sets(&log).last().unwrap(), "agent_state=working");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ask_times_out_to_the_normal_prompt_and_ignores_a_stale_allow() {
        let dir = temp_dir("timeout");
        // An allow left over from before the request must not count: ask clears it first.
        let log = fake_service(&dir, &[(STATE, "idle"), (DECISION, "allow")], Deck::Silent);
        let started = Instant::now();
        assert_eq!(ask_in(&dir, 600), "");
        assert!(started.elapsed() < Duration::from_secs(3));
        assert_eq!(
            sets(&log),
            [
                "agent_state=waiting",
                "agent_decision=none",
                "agent_decision=none",
                "agent_state=idle"
            ]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ask_prints_nothing_when_the_app_is_missing_quits_or_lacks_the_variables() {
        let dir = temp_dir("ask-none");
        let started = Instant::now();
        assert_eq!(ask_in(&dir, 5_000), "");
        assert!(started.elapsed() < Duration::from_secs(1));

        let log = fake_service(&dir, &[(STATE, "idle")], Deck::Press("allow"));
        assert_eq!(ask_in(&dir, 5_000), "");
        assert_eq!(sets(&log), Vec::<String>::new());

        let other = temp_dir("ask-vanish");
        fake_service(&other, &BOTH, Deck::Vanish);
        assert_eq!(ask_in(&other, 5_000), "");

        // Wired to another event, ask does nothing at all.
        let third = temp_dir("ask-event");
        let log = fake_service(&third, &BOTH, Deck::Press("allow"));
        let hook = json!({"hook_event_name":"PreToolUse"});
        assert_eq!(
            ask(
                &third.join("s"),
                &third.join("t"),
                &hook,
                Duration::from_secs(5)
            ),
            ""
        );
        assert!(log.lock().unwrap().is_empty());
        for dir in [dir, other, third] {
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn an_ask_that_cannot_clear_the_decision_everywhere_prints_nothing() {
        let dir = temp_dir("ask-strict");
        let log = fake_service(&dir, &BOTH, Deck::RefuseStrict);
        assert_eq!(ask_in(&dir, 5_000), "");
        // The refused clear ends the ask: the deck goes back to idle and nothing is answered.
        assert_eq!(
            sets(&log),
            [
                "agent_state=waiting",
                "agent_decision=none",
                "agent_decision=none",
                "agent_state=idle"
            ]
        );
        let strict: Vec<Value> = log
            .lock()
            .unwrap()
            .iter()
            .filter(|request| request["type"] == "set_variable" && request["name"] == DECISION)
            .map(|request| request["strict"].clone())
            .collect();
        assert!(!strict.is_empty() && strict.iter().all(|strict| *strict == json!(true)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_ask_lock_is_exclusive_and_released_when_closed() {
        let dir = temp_dir("lock");
        let path = dir.join(ASK_LOCK);
        let held = try_lock(&path).unwrap().expect("free");
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        // A second open of the same file, as another process would make, cannot take it.
        assert!(try_lock(&path).unwrap().is_none());
        drop(held);
        assert!(try_lock(&path).unwrap().is_some());
        // A symbolic link in its place is never followed.
        std::fs::remove_file(&path).unwrap();
        let target = dir.join("elsewhere");
        std::os::unix::fs::symlink(&target, &path).unwrap();
        assert!(try_lock(&path).is_err());
        assert!(!target.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn ask_as(dir: &Path, session: &str, timeout_ms: u64) -> String {
        let hook = json!({"hook_event_name":"PermissionRequest","session_id":session});
        ask(
            &dir.join("s"),
            &dir.join("t"),
            &hook,
            Duration::from_millis(timeout_ms),
        )
    }

    /// Waits until the fake has seen `set` (as `name=value`).
    fn wait_for(log: &std::sync::Mutex<Vec<Value>>, set: &str) {
        let started = Instant::now();
        while !all_sets(log).iter().any(|seen| seen == set) {
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "waiting for {set}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn sessions_share_the_deck_one_prompt_at_a_time_and_the_count_shows() {
        let dir = temp_dir("ask-two");
        // The deck presses allow on the second read while waiting: that answers the first ask only.
        let log = fake_service(
            &dir,
            &[(STATE, "idle"), (DECISION, "none"), (WAITING, "0")],
            Deck::Press("allow"),
        );
        let first = {
            let dir = dir.clone();
            std::thread::spawn(move || ask_as(&dir, "one", 4_000))
        };
        wait_for(&log, "agent_waiting=1");
        let second = {
            let dir = dir.clone();
            std::thread::spawn(move || ask_as(&dir, "two", 1_500))
        };
        wait_for(&log, "agent_waiting=2");
        let answer: Value = serde_json::from_str(&first.join().unwrap()).unwrap();
        assert_eq!(
            answer["hookSpecificOutput"]["decision"]["behavior"],
            "allow"
        );
        // The other session queued behind it and was never answered, so it falls back to its own prompt.
        assert_eq!(second.join().unwrap(), "");
        let sets = sets(&log);
        // The first's answer did not end Needs you while the second still waited; the deck ends on the first
        // session's work once the second gave up.
        let working = sets
            .iter()
            .position(|set| set == "agent_state=working")
            .unwrap();
        assert!(
            sets[..working]
                .iter()
                .filter(|set| *set == "agent_state=waiting")
                .count()
                >= 2,
            "{sets:?}"
        );
        assert_eq!(sets.last().unwrap(), "agent_state=working", "{sets:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_queued_ask_shows_working_before_it_asks_so_a_repeated_press_misses_it() {
        let dir = temp_dir("ask-settle");
        let log = fake_service(
            &dir,
            &[(STATE, "idle"), (DECISION, "none"), (WAITING, "0")],
            Deck::Press("allow"),
        );
        let first = {
            let dir = dir.clone();
            std::thread::spawn(move || ask_as(&dir, "one", 6_000))
        };
        wait_for(&log, "agent_waiting=1");
        let second = {
            let dir = dir.clone();
            std::thread::spawn(move || ask_as(&dir, "two", 3_000))
        };
        assert!(first.join().unwrap().contains("allow"));
        assert_eq!(second.join().unwrap(), "");
        let states: Vec<String> = sets(&log)
            .into_iter()
            .filter(|set| set.starts_with("agent_state="))
            .collect();
        // The second ask went waiting -> working (settling) -> waiting, and every decision clear of its turn came
        // after the settle (the first working of the second's turn).
        let text = states.join(",");
        assert!(
            text.contains("agent_state=working,agent_state=waiting"),
            "{text}"
        );
        let sets = sets(&log);
        let settle = sets
            .iter()
            .position(|set| set == "agent_state=working")
            .unwrap();
        let clears_after = sets[settle..]
            .iter()
            .filter(|set| *set == "agent_decision=none")
            .count();
        assert!(clears_after >= 2, "{sets:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_state_hook_of_another_session_does_not_hide_a_prompt() {
        let dir = temp_dir("ask-state");
        let log = fake_service(&dir, &BOTH, Deck::Silent);
        let asking = {
            let dir = dir.clone();
            std::thread::spawn(move || ask_as(&dir, "asker", 1_200))
        };
        wait_for(&log, "agent_state=waiting");
        let paths = || Ok((dir.join("s"), dir.join("t")));
        // Another session finishes (and one hand-typed hook has no session at all): the deck still says waiting.
        for state in ["agent state done", "agent state working"] {
            assert_eq!(run(&args(state), &paths), Ok(String::new()));
        }
        assert_eq!(
            sets(&log)
                .iter()
                .filter(|s| s.starts_with("agent_state="))
                .collect::<Vec<_>>(),
            ["agent_state=waiting"; 3]
        );
        assert_eq!(asking.join().unwrap(), "");
        // Once nothing waits, the other session's work shows.
        assert_eq!(sets(&log).last().unwrap(), "agent_state=working");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_session_that_ends_leaves_the_summary() {
        let dir = temp_dir("ask-end");
        let log = fake_service(&dir, &BOTH, Deck::Silent);
        let paths = || Ok((dir.join("s"), dir.join("t")));
        assert_eq!(run(&args("agent state working"), &paths), Ok(String::new()));
        assert_eq!(run(&args("agent state end"), &paths), Ok(String::new()));
        assert_eq!(sets(&log), ["agent_state=working", "agent_state=idle"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn setup_refuses_a_translocated_copy() {
        let failure = setup_for(
            Agent::Codex,
            Path::new("/private/var/folders/a/T/AppTranslocation/1/d/OpenStreamHub.app/Contents/MacOS/osh"),
        )
        .unwrap_err();
        assert_eq!(failure.0, 1);
        assert!(failure.1.contains(TRANSLOCATED), "{}", failure.1);
        let config = setup_for(
            Agent::Codex,
            Path::new("/Applications/OpenStreamHub.app/Contents/MacOS/osh"),
        )
        .unwrap();
        assert!(config.contains("/Applications/OpenStreamHub.app/Contents/MacOS/osh agent ask"));
    }
}
