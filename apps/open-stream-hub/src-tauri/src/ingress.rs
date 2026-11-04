//! A local socket through which processes outside the app (agent hooks, the `osh` tool) read and set the
//! variables of the running profiles.
//!
//! The socket and a token live in `<app data>/ingress/` (directory 0700, files 0600). The token is new at every
//! launch; a request must carry it. Protocol: one JSON object per line, one JSON line back.
//!
//! - `{"token":"…","type":"ping"}`
//! - `{"token":"…","type":"set_variable","name":"agent_state","value":"waiting"}`: the value as in a profile
//!   (`true`, `3`, `"text"`). Sent to every running module; it succeeds when at least one declares the variable.
//! - `{"token":"…","type":"get_variables"}`: `result.variables` is `[{"service":..,"name":..,"value":..}]`.
//!
//! Answers are `{"ok":true,"result":{..}}` or `{"ok":false,"error":".."}`.

use crate::service::Services;
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    os::unix::{
        fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};

/// The longest request line.
pub const MAX_LINE: usize = 8 * 1024;
/// Connections served at once; more are answered with an error and closed.
pub const MAX_CONNECTIONS: usize = 8;
/// How long a connection may stay silent.
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

fn set_variable(request: &Value, forward: &dyn Fn(&Value) -> Answers) -> Result<Value, String> {
    let name = request["name"]
        .as_str()
        .ok_or("set_variable needs a name")?;
    let value = request.get("value").ok_or("set_variable needs a value")?;
    let answers = forward(&json!({"type":"set_variable","name":name,"value":value}));
    let applied = answers.iter().filter(|(_, answer)| answer.is_ok()).count();
    if applied > 0 {
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
            return Err("the token is missing or wrong".to_string());
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

/// Serves one connection until it closes, goes silent, or sends a line that is too long.
fn serve(stream: UnixStream, token: &str, forward: &Forward) {
    if stream.set_read_timeout(Some(READ_TIMEOUT)).is_err()
        || stream.set_write_timeout(Some(READ_TIMEOUT)).is_err()
    {
        return;
    }
    let mut reader = BufReader::new(&stream);
    loop {
        let mut line = Vec::new();
        match reader
            .by_ref()
            .take(MAX_LINE as u64 + 1)
            .read_until(b'\n', &mut line)
        {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        if line.last() == Some(&b'\n') {
            line.pop();
        }
        if line.len() > MAX_LINE {
            reply(
                &stream,
                &json!({"ok":false,"error":"the request line is too long"}),
            );
            return;
        }
        let answer = match std::str::from_utf8(&line) {
            Ok(text) => handle_line(text, token, forward.as_ref()),
            Err(_) => json!({"ok":false,"error":"the request is not UTF-8"}),
        };
        if !reply(&stream, &answer) {
            return;
        }
    }
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
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
            .map_err(|error| failed("create the ingress directory", error))?;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))
            .map_err(|error| failed("protect the ingress directory", error))?;
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
}
