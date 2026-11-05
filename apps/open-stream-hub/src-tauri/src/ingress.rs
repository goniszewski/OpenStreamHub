//! A local socket through which processes outside the app (agent hooks, the `osh` tool) read and set the
//! variables of the running profiles.
//!
//! The socket and a token live in `<app data>/ingress/` (a real directory owned by this user, 0700; files 0600).
//! The token is new at every launch; a request must carry it, and a connection that sends a wrong one is closed.
//! Protocol: one JSON object per line, one JSON line back; each line must arrive within `READ_TIMEOUT` (from the
//! connection, or from the previous answer).
//!
//! - `{"token":"…","type":"ping"}`
//! - `{"token":"…","type":"set_variable","name":"agent_state","value":"waiting"}`: the value as in a profile
//!   (`true`, `3`, `"text"`). Sent to every running module; it succeeds when at least one declares the variable.
//!   With `"strict":true` it succeeds only when every running module applied it, apart from the ones that do not
//!   declare it (a module that fails for any other reason, or does not answer, counts as not applied).
//!   `agent_decision` can only be set to `none` here (`osh::outside_write_allowed`): only a deck key answers an
//!   agent's prompt.
//! - `{"token":"…","type":"get_variables"}`: `result.variables` is `[{"service":..,"name":..,"value":..}]`.
//!
//! Answers are `{"ok":true,"result":{..}}` or `{"ok":false,"error":".."}`.

use crate::service::Services;
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    os::unix::{
        fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

/// The longest request line.
pub const MAX_LINE: usize = 8 * 1024;
/// Connections served at once; more are answered with an error and closed.
pub const MAX_CONNECTIONS: usize = 8;
/// How long one request line may take to arrive, in total: from the connection for the first line, from the
/// previous answer for the next. A client that drips bytes cannot hold a connection slot longer.
pub const READ_TIMEOUT: Duration = Duration::from_secs(2);
/// How long each running module has to answer a forwarded request.
const FORWARD_TIMEOUT: Duration = Duration::from_secs(1);

pub const SOCKET_FILE: &str = "ingress.sock";
pub const TOKEN_FILE: &str = "ingress.token";

/// Each running module's answer, with its service id.
pub type Answers = Vec<(String, Result<Value, String>)>;

/// Sends one control request to every running module.
pub type Forward = Arc<dyn Fn(&Value) -> Answers + Send + Sync>;

/// Where the socket and the token live.
pub fn dir(app_data: &Path) -> PathBuf {
    app_data.join("ingress")
}

/// The real forward: every running service's control channel, one after another.
pub fn forward_to(services: &Services, body: &Value) -> Answers {
    services
        .running_ids()
        .into_iter()
        .filter_map(|id| {
            let pipe = services.control(&id)?;
            Some((id, pipe.request(body.clone(), FORWARD_TIMEOUT)))
        })
        .collect()
}

/// Compares in time that depends only on the lengths.
fn token_matches(expected: &str, given: &str) -> bool {
    let (expected, given) = (expected.as_bytes(), given.as_bytes());
    expected.len() == given.len()
        && expected
            .iter()
            .zip(given)
            .fold(0u8, |difference, (a, b)| difference | (a ^ b))
            == 0
}

const NOT_RUNNING: &str = "no module is running in OpenStreamHub";
const BAD_TOKEN: &str = "the token is missing or wrong";

fn refusal(answers: Answers) -> String {
    let mut errors: Vec<String> = answers
        .into_iter()
        .filter_map(|(_, answer)| answer.err())
        .collect();
    errors.dedup();
    if errors.is_empty() {
        NOT_RUNNING.to_string()
    } else {
        errors.join("; ")
    }
}

/// Whether a module's refusal says only that its run does not declare the variable (a strict set skips those).
fn undeclared(error: &str) -> bool {
    error.contains("is not declared") || error == "this run has no variables"
}

fn set_variable(request: &Value, forward: &dyn Fn(&Value) -> Answers) -> Result<Value, String> {
    let name = request["name"]
        .as_str()
        .ok_or("set_variable needs a name")?;
    let value = request.get("value").ok_or("set_variable needs a value")?;
    if !osh::outside_write_allowed(name, value) {
        return Err(osh::DECISION_REFUSED.to_string());
    }
    let answers = forward(&json!({"type":"set_variable","name":name,"value":value}));
    let applied = answers.iter().filter(|(_, answer)| answer.is_ok()).count();
    if request["strict"] == true {
        let failed = answers
            .iter()
            .filter(|(_, answer)| answer.as_ref().is_err_and(|error| !undeclared(error)))
            .count();
        if failed > 0 {
            return Err(format!(
                "{name} was applied by {applied} of {} running modules that may declare it",
                applied + failed
            ));
        }
    }
    if applied > 0 {
        if name == osh::AGENT_DECISION && value == "none" {
            crate::agent_tools::note_decision_cleared();
        }
        Ok(json!({"applied": applied}))
    } else {
        Err(refusal(answers))
    }
}

fn get_variables(forward: &dyn Fn(&Value) -> Answers) -> Result<Value, String> {
    let answers = forward(&json!({"type":"get_variables"}));
    if !answers.iter().any(|(_, answer)| answer.is_ok()) {
        return Err(refusal(answers));
    }
    let mut variables = Vec::new();
    for (service, answer) in answers {
        let Ok(answer) = answer else { continue };
        for variable in answer["variables"].as_array().into_iter().flatten() {
            variables
                .push(json!({"service":service,"name":variable["name"],"value":variable["value"]}));
        }
    }
    for variable in variables
        .iter()
        .filter(|variable| variable["name"] == osh::AGENT_DECISION)
    {
        crate::agent_tools::note_decision(&variable["value"]);
    }
    Ok(json!({"variables": variables}))
}

/// Answers one request line.
pub fn handle_line(line: &str, token: &str, forward: &dyn Fn(&Value) -> Answers) -> Value {
    let outcome = (|| {
        let request: Value = serde_json::from_str(line)
            .ok()
            .filter(Value::is_object)
            .ok_or("the request is not a JSON object")?;
        if !request["token"]
            .as_str()
            .is_some_and(|given| token_matches(token, given))
        {
            return Err(BAD_TOKEN.to_string());
        }
        match request["type"].as_str() {
            Some("ping") => Ok(json!({})),
            Some("set_variable") => set_variable(&request, forward),
            Some("get_variables") => get_variables(forward),
            _ => Err("unknown request type".to_string()),
        }
    })();
    match outcome {
        Ok(result) => json!({"ok":true,"result":result}),
        Err(error) => json!({"ok":false,"error":error}),
    }
}

fn reply(mut stream: &UnixStream, answer: &Value) -> bool {
    stream.write_all(format!("{answer}\n").as_bytes()).is_ok()
}

/// One request line as read by `read_line`.
enum Line {
    Text(Vec<u8>),
    TooLong,
    /// Closed, failed, or not complete by the deadline.
    Over,
}

/// Reads up to the next newline (or the end of the stream) into a line, keeping any bytes after it in `pending`.
/// The whole line must arrive by `deadline`, however the bytes are spread out.
fn read_line(mut stream: &UnixStream, pending: &mut Vec<u8>, deadline: Instant) -> Line {
    let mut chunk = [0u8; 1024];
    loop {
        if let Some(end) = pending.iter().position(|byte| *byte == b'\n') {
            if end > MAX_LINE {
                return Line::TooLong;
            }
            let mut line: Vec<u8> = pending.drain(..=end).collect();
            line.pop();
            return Line::Text(line);
        }
        if pending.len() > MAX_LINE {
            return Line::TooLong;
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() || stream.set_read_timeout(Some(left)).is_err() {
            return Line::Over;
        }
        match stream.read(&mut chunk) {
            Ok(0) if pending.is_empty() => return Line::Over,
            Ok(0) => return Line::Text(std::mem::take(pending)),
            Ok(read) => pending.extend_from_slice(&chunk[..read]),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return Line::Over,
        }
    }
}

/// Serves one connection until it closes, is late with a line, sends a line that is too long, or sends a wrong
/// token (answered, then closed).
fn serve(stream: UnixStream, token: &str, forward: &Forward) {
    if stream.set_write_timeout(Some(READ_TIMEOUT)).is_err() {
        return;
    }
    let mut pending = Vec::new();
    let mut deadline = Instant::now() + READ_TIMEOUT;
    loop {
        let line = match read_line(&stream, &mut pending, deadline) {
            Line::Text(line) => line,
            Line::TooLong => {
                reply(
                    &stream,
                    &json!({"ok":false,"error":"the request line is too long"}),
                );
                return;
            }
            Line::Over => return,
        };
        let answer = match std::str::from_utf8(&line) {
            Ok(text) => handle_line(text, token, forward.as_ref()),
            Err(_) => json!({"ok":false,"error":"the request is not UTF-8"}),
        };
        if !reply(&stream, &answer) || answer["error"] == BAD_TOKEN {
            return;
        }
        deadline = Instant::now() + READ_TIMEOUT;
    }
}

/// Creates the ingress directory when it is missing, then accepts it only as a real directory (never a symbolic
/// link) owned by this user, and makes it 0700. Anything else is refused, and ingress stays off.
fn prepare_dir(dir: &Path) -> Result<(), String> {
    let refused = |why: &str| {
        format!(
            "the ingress directory {} {why}, so agent hooks and osh cannot reach the app",
            dir.display()
        )
    };
    match fs::symlink_metadata(dir) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
            .map_err(|error| format!("could not create the ingress directory: {error}"))?,
        Err(error) => return Err(format!("could not inspect the ingress directory: {error}")),
        Ok(_) => {}
    }
    let metadata = fs::symlink_metadata(dir)
        .map_err(|error| format!("could not inspect the ingress directory: {error}"))?;
    if !metadata.file_type().is_dir() {
        return Err(refused(
            "is not a plain directory (a symbolic link or a file)",
        ));
    }
    // SAFETY: getuid has no preconditions and cannot fail.
    if metadata.uid() != unsafe { libc::getuid() } {
        return Err(refused("belongs to another user"));
    }
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700))
        .map_err(|error| format!("could not protect the ingress directory: {error}"))
}

/// A fresh 256-bit token as hex.
fn new_token() -> Result<String, String> {
    let mut bytes = [0u8; 32];
    fs::File::open("/dev/urandom")
        .and_then(|mut random| random.read_exact(&mut bytes))
        .map_err(|error| format!("could not create the ingress token: {error}"))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// The running ingress; dropping or stopping it removes the socket and the token.
pub struct Ingress {
    dir: PathBuf,
}

impl Ingress {
    /// Binds the socket in `dir` (replacing a stale one), writes a new token and serves on a background thread.
    pub fn start(dir: &Path, forward: Forward) -> Result<Self, String> {
        let failed = |what: &str, error: std::io::Error| format!("could not {what}: {error}");
        prepare_dir(dir)?;
        let socket = dir.join(SOCKET_FILE);
        match fs::remove_file(&socket) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                return Err(failed("remove the old ingress socket", error));
            }
            _ => {}
        }
        let listener = UnixListener::bind(&socket)
            .map_err(|error| failed("bind the ingress socket", error))?;
        fs::set_permissions(&socket, fs::Permissions::from_mode(0o600))
            .map_err(|error| failed("protect the ingress socket", error))?;
        let token = new_token()?;
        let staged = dir.join(format!("{TOKEN_FILE}.new"));
        let _ = fs::remove_file(&staged);
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&staged)
            .and_then(|mut file| file.write_all(token.as_bytes()))
            .and_then(|()| fs::rename(&staged, dir.join(TOKEN_FILE)))
            .map_err(|error| failed("write the ingress token", error))?;
        let token: Arc<str> = token.into();
        let active = Arc::new(AtomicUsize::new(0));
        thread::Builder::new()
            .name("ingress".into())
            .spawn(move || {
                for stream in listener.incoming() {
                    let Ok(stream) = stream else { continue };
                    if active.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
                        active.fetch_sub(1, Ordering::SeqCst);
                        let _ = stream.set_write_timeout(Some(READ_TIMEOUT));
                        reply(&stream, &json!({"ok":false,"error":"too many connections"}));
                        continue;
                    }
                    let (token, forward, served) = (
                        Arc::clone(&token),
                        Arc::clone(&forward),
                        Arc::clone(&active),
                    );
                    let spawned = thread::Builder::new()
                        .name("ingress-connection".into())
                        .spawn(move || {
                            serve(stream, &token, &forward);
                            served.fetch_sub(1, Ordering::SeqCst);
                        });
                    if spawned.is_err() {
                        active.fetch_sub(1, Ordering::SeqCst);
                    }
                }
            })
            .map_err(|error| failed("start the ingress thread", error))?;
        Ok(Self {
            dir: dir.to_path_buf(),
        })
    }

    /// Removes the socket and the token, so `osh` reports the app as not running.
    pub fn stop(&self) {
        let _ = fs::remove_file(self.dir.join(SOCKET_FILE));
        let _ = fs::remove_file(self.dir.join(TOKEN_FILE));
    }
}

impl Drop for Ingress {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::sync::Mutex;

    fn fixed(answers: Answers) -> impl Fn(&Value) -> Answers {
        move |_| answers.clone()
    }

    #[test]
    fn a_request_without_the_right_token_reaches_no_module() {
        let reached = Mutex::new(0);
        let forward = |_: &Value| {
            *reached.lock().unwrap() += 1;
            Vec::new()
        };
        for line in [
            r#"{"type":"get_variables"}"#,
            r#"{"token":"abd","type":"get_variables"}"#,
            r#"{"token":"ab","type":"get_variables"}"#,
            r#"{"token":7,"type":"ping"}"#,
        ] {
            let answer = handle_line(line, "abc", &forward);
            assert_eq!(answer["ok"], false, "{line}");
            assert_eq!(answer["error"], "the token is missing or wrong");
        }
        assert_eq!(*reached.lock().unwrap(), 0);
        assert_eq!(handle_line("[1]", "abc", &forward)["ok"], false);
        assert_eq!(
            handle_line(r#"{"token":"abc","type":"nope"}"#, "abc", &forward)["error"],
            "unknown request type"
        );
        assert_eq!(
            handle_line(r#"{"token":"abc","type":"ping"}"#, "abc", &forward)["ok"],
            true
        );
    }

    #[test]
    fn a_set_succeeds_when_one_module_declares_the_variable() {
        let line =
            r#"{"token":"abc","type":"set_variable","name":"agent_state","value":"waiting"}"#;
        let seen = Mutex::new(Vec::new());
        let forward = |body: &Value| {
            seen.lock().unwrap().push(body.clone());
            vec![
                (
                    "a".to_string(),
                    Err("variable agent_state is not declared".to_string()),
                ),
                ("b".to_string(), Ok(json!({}))),
            ]
        };
        let answer = handle_line(line, "abc", &forward);
        assert_eq!(answer, json!({"ok":true,"result":{"applied":1}}));
        // The token is never passed on.
        assert_eq!(
            seen.lock().unwrap()[0],
            json!({"type":"set_variable","name":"agent_state","value":"waiting"})
        );
        let refused = handle_line(
            line,
            "abc",
            &fixed(vec![
                ("a".into(), Err("not declared".into())),
                ("b".into(), Err("not declared".into())),
            ]),
        );
        assert_eq!(refused["error"], "not declared");
        let none = handle_line(line, "abc", &fixed(Vec::new()));
        assert_eq!(none["error"], NOT_RUNNING);
        let unnamed = handle_line(
            r#"{"token":"abc","type":"set_variable","value":1}"#,
            "abc",
            &fixed(Vec::new()),
        );
        assert_eq!(unnamed["error"], "set_variable needs a name");
    }

    #[test]
    fn variables_of_every_module_are_listed_with_their_service() {
        let forward = fixed(vec![
            (
                "a".into(),
                Ok(json!({"variables":[{"name":"busy","value":true}],"version":3})),
            ),
            ("b".into(), Err("this run has no variables".into())),
        ]);
        let answer = handle_line(r#"{"token":"abc","type":"get_variables"}"#, "abc", &forward);
        assert_eq!(
            answer["result"]["variables"],
            json!([{"service":"a","name":"busy","value":true}])
        );
    }

    fn exchange(stream: &mut UnixStream, line: &[u8]) -> Value {
        stream.write_all(line).unwrap();
        let mut answer = String::new();
        BufReader::new(&*stream).read_line(&mut answer).unwrap();
        serde_json::from_str(&answer).unwrap()
    }

    #[test]
    fn the_socket_serves_with_the_token_and_refuses_long_lines() {
        let temp = tempfile::tempdir().unwrap();
        let dir = dir(temp.path());
        let forward: Forward = Arc::new(|_| vec![("a".into(), Ok(json!({"variables":[]})))]);
        let ingress = Ingress::start(&dir, forward).unwrap();
        let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(&dir.join(SOCKET_FILE)), 0o600);
        assert_eq!(mode(&dir.join(TOKEN_FILE)), 0o600);
        let token = fs::read_to_string(dir.join(TOKEN_FILE)).unwrap();
        assert_eq!(token.len(), 64);

        let mut stream = UnixStream::connect(dir.join(SOCKET_FILE)).unwrap();
        let ok = exchange(
            &mut stream,
            format!("{{\"token\":\"{token}\",\"type\":\"get_variables\"}}\n").as_bytes(),
        );
        assert_eq!(ok["ok"], true, "{ok}");
        let wrong = exchange(&mut stream, b"{\"token\":\"x\",\"type\":\"ping\"}\n");
        assert_eq!(wrong["ok"], false);
        // A wrong token closes the connection after its answer (macOS refuses a timeout on a closed socket).
        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
        let mut rest = Vec::new();
        assert_eq!(stream.read_to_end(&mut rest).unwrap(), 0);
        let mut long = vec![b' '; MAX_LINE + 10];
        long.push(b'\n');
        let mut other = UnixStream::connect(dir.join(SOCKET_FILE)).unwrap();
        let refused = exchange(&mut other, &long);
        assert_eq!(refused["error"], "the request line is too long");

        // A restart replaces the socket and the token.
        drop(ingress);
        assert!(!dir.join(SOCKET_FILE).exists() && !dir.join(TOKEN_FILE).exists());
        let _again = Ingress::start(&dir, Arc::new(|_| Vec::new())).unwrap();
        assert_ne!(fs::read_to_string(dir.join(TOKEN_FILE)).unwrap(), token);
    }

    #[test]
    fn connections_beyond_the_cap_are_refused() {
        let temp = tempfile::tempdir().unwrap();
        let dir = dir(temp.path());
        let _ingress = Ingress::start(&dir, Arc::new(|_| Vec::new())).unwrap();
        let token = fs::read_to_string(dir.join(TOKEN_FILE)).unwrap();
        let mut held: Vec<UnixStream> = (0..MAX_CONNECTIONS)
            .map(|_| UnixStream::connect(dir.join(SOCKET_FILE)).unwrap())
            .collect();
        // Every held connection is being served once it has answered.
        for stream in &mut held {
            let pong = exchange(
                stream,
                format!("{{\"token\":\"{token}\",\"type\":\"ping\"}}\n").as_bytes(),
            );
            assert_eq!(pong["ok"], true);
        }
        let mut extra = UnixStream::connect(dir.join(SOCKET_FILE)).unwrap();
        extra
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut answer = String::new();
        BufReader::new(&mut extra).read_line(&mut answer).unwrap();
        assert!(answer.contains("too many connections"), "{answer}");
    }

    #[test]
    fn only_none_reaches_the_agent_decision_from_outside() {
        let reached = Mutex::new(Vec::new());
        let forward = |body: &Value| {
            reached.lock().unwrap().push(body.clone());
            vec![("a".to_string(), Ok(json!({})))]
        };
        for value in [r#""allow""#, r#""deny""#, r#""Allow""#, "true", "1"] {
            let line = format!(
                r#"{{"token":"abc","type":"set_variable","name":"agent_decision","value":{value}}}"#
            );
            let answer = handle_line(&line, "abc", &forward);
            assert_eq!(answer["ok"], false, "{value}");
            assert_eq!(answer["error"], osh::DECISION_REFUSED);
        }
        assert!(reached.lock().unwrap().is_empty(), "no module was asked");
        let cleared = handle_line(
            r#"{"token":"abc","type":"set_variable","name":"agent_decision","value":"none","strict":true}"#,
            "abc",
            &forward,
        );
        assert_eq!(cleared, json!({"ok":true,"result":{"applied":1}}));
        // Other variables, agent_state among them, are set as before.
        let state = handle_line(
            r#"{"token":"abc","type":"set_variable","name":"agent_state","value":"working"}"#,
            "abc",
            &forward,
        );
        assert_eq!(state["ok"], true);
        assert_eq!(reached.lock().unwrap().len(), 2);
    }

    #[test]
    fn a_strict_set_needs_every_declaring_module() {
        let line = r#"{"token":"abc","type":"set_variable","name":"agent_decision","value":"none","strict":true}"#;
        let one_failing = fixed(vec![
            ("a".into(), Ok(json!({}))),
            ("b".into(), Err("the module did not answer in time".into())),
        ]);
        let answer = handle_line(line, "abc", &one_failing);
        assert_eq!(answer["ok"], false);
        assert_eq!(
            answer["error"],
            "agent_decision was applied by 1 of 2 running modules that may declare it"
        );
        // Without strict, one module is enough, as before.
        let loose = line.replace(r#","strict":true"#, "");
        assert_eq!(handle_line(&loose, "abc", &one_failing)["ok"], true);
        // A module that does not declare the variable is not counted against it.
        let undeclared = fixed(vec![
            ("a".into(), Ok(json!({}))),
            (
                "b".into(),
                Err("variable agent_decision is not declared; no action was taken".into()),
            ),
            ("c".into(), Err("this run has no variables".into())),
        ]);
        assert_eq!(
            handle_line(line, "abc", &undeclared),
            json!({"ok":true,"result":{"applied":1}})
        );
        // Strict with nothing applied anywhere is still refused.
        let none = fixed(vec![("a".into(), Err("this run has no variables".into()))]);
        assert_eq!(handle_line(line, "abc", &none)["ok"], false);
    }

    #[test]
    fn a_slow_drip_client_loses_its_slot_at_the_deadline() {
        let temp = tempfile::tempdir().unwrap();
        let dir = dir(temp.path());
        let _ingress = Ingress::start(&dir, Arc::new(|_| Vec::new())).unwrap();
        let token = fs::read_to_string(dir.join(TOKEN_FILE)).unwrap();
        let started = Instant::now();
        // Every slot taken by a client that sends one byte every 200 ms and never a newline.
        let drips: Vec<_> = (0..MAX_CONNECTIONS)
            .map(|_| {
                let mut stream = UnixStream::connect(dir.join(SOCKET_FILE)).unwrap();
                thread::spawn(move || {
                    let mut sent = 0;
                    while started.elapsed() < READ_TIMEOUT * 3 {
                        if stream.write_all(b" ").is_err() {
                            break;
                        }
                        sent += 1;
                        thread::sleep(Duration::from_millis(200));
                    }
                    (started.elapsed(), sent)
                })
            })
            .collect();
        thread::sleep(READ_TIMEOUT + Duration::from_millis(600));
        // The deadline closed them all, although none was ever silent for long: a new client is served.
        let mut stream = UnixStream::connect(dir.join(SOCKET_FILE)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let pong = exchange(
            &mut stream,
            format!("{{\"token\":\"{token}\",\"type\":\"ping\"}}\n").as_bytes(),
        );
        assert_eq!(pong["ok"], true, "{pong}");
        for drip in drips {
            let (stopped, sent) = drip.join().unwrap();
            assert!(sent > 5, "{sent}");
            assert!(
                stopped < READ_TIMEOUT * 3,
                "the server closed the connection"
            );
        }
    }

    #[test]
    fn the_directory_must_be_a_real_directory_of_this_user() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        // A symbolic link to a directory is refused, and nothing is created where it points.
        let elsewhere = temp.path().join("elsewhere");
        fs::create_dir(&elsewhere).unwrap();
        fs::set_permissions(&elsewhere, fs::Permissions::from_mode(0o755)).unwrap();
        let linked = temp.path().join("linked");
        symlink(&elsewhere, &linked).unwrap();
        let error = Ingress::start(&linked, Arc::new(|_| Vec::new()))
            .err()
            .unwrap();
        assert!(error.contains("not a plain directory"), "{error}");
        assert_eq!(fs::read_dir(&elsewhere).unwrap().count(), 0);
        assert_eq!(
            fs::metadata(&elsewhere).unwrap().permissions().mode() & 0o777,
            0o755
        );
        // So is a file.
        let file = temp.path().join("file");
        fs::write(&file, "").unwrap();
        assert!(Ingress::start(&file, Arc::new(|_| Vec::new())).is_err());
        // A real directory of this user with loose permissions is tightened to 0700.
        let loose = temp.path().join("loose");
        fs::create_dir(&loose).unwrap();
        fs::set_permissions(&loose, fs::Permissions::from_mode(0o777)).unwrap();
        let _ingress = Ingress::start(&loose, Arc::new(|_| Vec::new())).unwrap();
        assert_eq!(
            fs::metadata(&loose).unwrap().permissions().mode() & 0o777,
            0o700
        );
        // A directory of another user (/usr, owned by root) is refused without touching it.
        if unsafe { libc::getuid() } != 0 {
            let error = prepare_dir(Path::new("/usr")).unwrap_err();
            assert!(error.contains("another user"), "{error}");
        }
    }
}
