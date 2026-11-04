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
//! ```
//!
//! `OSH_SOCKET` and `OSH_TOKEN_FILE` override where the socket and the token are found.

use serde_json::{Value, json};
use std::{
    env,
    io::{BufRead, BufReader, Write},
    path::PathBuf,
    process::ExitCode,
    time::Duration,
};

/// The app's bundle identifier (tauri.conf.json), which names its data directory.
const APP_IDENTIFIER: &str = "dev.openstreamhub";
/// How long `osh` waits for the app, in total per step.
const TIMEOUT: Duration = Duration::from_secs(3);
/// The longest answer read back.
const MAX_ANSWER: u64 = 64 * 1024;

const USAGE: &str = "usage: osh set [--kind bool|number|text] <name> <value>\n       osh get [name]\n       osh ping";

#[derive(Debug, PartialEq)]
enum Command {
    Set { name: String, value: Value },
    Get { name: Option<String> },
    Ping,
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
        Some("-h" | "--help" | "help") => Err(Failure(0, USAGE.to_string())),
        _ => Err(usage("unknown command")),
    }
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

fn locate() -> Result<(PathBuf, PathBuf), Failure> {
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

fn not_running() -> Failure {
    Failure(3, "osh: OpenStreamHub is not running".to_string())
}

/// Sends one request line with the token and returns the answer's `result`.
#[cfg(unix)]
fn request(
    socket: &std::path::Path,
    token_file: &std::path::Path,
    mut body: Value,
) -> Result<Value, Failure> {
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
fn request(_: &std::path::Path, _: &std::path::Path, _: Value) -> Result<Value, Failure> {
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

/// Runs one command and returns what to print on stdout.
fn run(args: &[String]) -> Result<String, Failure> {
    let command = parse(args)?;
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
            let result = request(&socket, &token, json!({"type":"get_variables"}))?;
            let variables = result["variables"].as_array().cloned().unwrap_or_default();
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
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    match run(&args) {
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
            ExitCode::from(code)
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
}
