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

use osh::{ASK_DEFAULT_SECS, Agent, CLAUDE_HEADER, setup};
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

const USAGE: &str = "usage: osh set [--kind bool|number|text] <name> <value>\n       osh get [name]\n       osh ping\n       osh agent state <working|waiting|done|idle>\n       osh agent ask [--timeout SECS]\n       osh agent setup <claude|codex>";

/// The variables the terminal starter declares for agent hooks.
const STATE: &str = "agent_state";
const DECISION: &str = "agent_decision";
const AGENT_STATES: [&str; 4] = ["working", "waiting", "done", "idle"];
/// How long `agent ask` waits for the deck at most (the default is `osh::ASK_DEFAULT_SECS`).
const ASK_MAX_SECS: u64 = 600;
/// How often `agent ask` reads the decision.
const POLL: Duration = Duration::from_millis(200);
/// How long a hook waits for its JSON on stdin, and the most read.
const STDIN_WAIT: Duration = Duration::from_millis(500);
const MAX_STDIN: u64 = 1024 * 1024;

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
            "agent takes state <working|waiting|done|idle>, ask [--timeout SECS] or setup <claude|codex>",
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

/// Shows the request on the deck and waits for Accept or Deny. Returns what the hook prints: the answer, or
/// nothing, which leaves the agent's own permission prompt in charge. Only an explicit `allow` on the deck allows;
/// any error, the timeout or an interrupt prints nothing.
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
    let decision = set_text(socket, token, DECISION, "none")
        .and_then(|()| set_text(socket, token, STATE, "waiting"))
        .ok()
        .and_then(|()| await_decision(socket, token, deadline));
    let _ = set_text(socket, token, DECISION, "none");
    match decision {
        Some(behavior) => {
            let _ = set_text(socket, token, STATE, "working");
            hook_answer(behavior)
        }
        None => {
            let _ = set_text(socket, token, STATE, "idle");
            String::new()
        }
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

/// Runs one command and returns what to print on stdout. `locate` finds the socket and the token file.
fn run(args: &[String], locate: &dyn Fn() -> Result<Paths, Failure>) -> Result<String, Failure> {
    let command = parse(args)?;
    match command {
        Command::AgentSetup(agent) => {
            let osh = env::current_exe()
                .and_then(|path| path.canonicalize())
                .map_err(|error| Failure(1, format!("osh: cannot find its own path: {error}")))?;
            if agent == Agent::Claude {
                eprintln!("{CLAUDE_HEADER}");
            }
            return Ok(setup(agent, &osh.to_string_lossy()));
        }
        Command::AgentState(state) => {
            read_stdin();
            if let Ok((socket, token)) = locate() {
                let _ = set_text(&socket, &token, STATE, &state);
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

    /// The `set_variable` requests the fake saw, as `name=value`.
    fn sets(log: &std::sync::Mutex<Vec<Value>>) -> Vec<String> {
        log.lock()
            .unwrap()
            .iter()
            .filter(|request| request["type"] == "set_variable")
            .map(|request| format!("{}={}", plain(&request["name"]), plain(&request["value"])))
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
            sets(&log),
            [
                "agent_decision=none",
                "agent_state=waiting",
                "agent_decision=none",
                "agent_state=working"
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
                "agent_decision=none",
                "agent_state=waiting",
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
}
