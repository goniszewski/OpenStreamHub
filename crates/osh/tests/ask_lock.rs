//! The real `osh` binary: an `agent ask` that is killed while it waits releases the ask lock, so the next hooks
//! work again.
#![cfg(unix)]

use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixListener,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

/// A fake app that declares both agent variables, never answers on the deck, and logs every set as `name=value`.
fn fake_app(dir: &Path) -> Arc<Mutex<Vec<String>>> {
    let listener = UnixListener::bind(dir.join("s")).unwrap();
    std::fs::write(dir.join("t"), "abc\n").unwrap();
    let log = Arc::new(Mutex::new(Vec::new()));
    let seen = log.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { break };
            let mut line = String::new();
            if BufReader::new(&stream).read_line(&mut line).is_err() {
                continue;
            }
            let request: Value = serde_json::from_str(&line).unwrap_or(Value::Null);
            let answer = match request["type"].as_str() {
                Some("get_variables") => json!({"ok":true,"result":{"variables":[
                    {"service":"m4315","name":"agent_state","value":"idle"},
                    {"service":"m4315","name":"agent_decision","value":"none"},
                ]}}),
                Some("set_variable") => {
                    seen.lock().unwrap().push(format!(
                        "{}={}",
                        request["name"].as_str().unwrap_or_default(),
                        request["value"].as_str().unwrap_or_default()
                    ));
                    json!({"ok":true,"result":{"applied":1}})
                }
                _ => json!({"ok":true,"result":{}}),
            };
            let _ = (&stream).write_all(format!("{answer}\n").as_bytes());
        }
    });
    log
}

fn osh(dir: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_osh"));
    command
        .args(args)
        .env("OSH_SOCKET", dir.join("s"))
        .env("OSH_TOKEN_FILE", dir.join("t"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    command
}

fn temp_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("osh-it-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn a_killed_ask_releases_the_lock() {
    let dir = temp_dir();
    let log = fake_app(&dir);
    let mut asking = osh(&dir, &["agent", "ask", "--timeout", "30"])
        .spawn()
        .unwrap();
    asking
        .stdin
        .take()
        .unwrap()
        .write_all(br#"{"hook_event_name":"PermissionRequest"}"#)
        .unwrap();
    let started = Instant::now();
    while !log.lock().unwrap().contains(&"agent_state=waiting".into()) {
        assert!(started.elapsed() < Duration::from_secs(5), "the ask waits");
        std::thread::sleep(Duration::from_millis(20));
    }

    // While it waits, a second ask prints nothing at once and a state hook changes nothing.
    let second = osh(&dir, &["agent", "ask"]).output().unwrap();
    assert!(second.status.success());
    assert!(second.stdout.is_empty());
    let state = osh(&dir, &["agent", "state", "done"]).output().unwrap();
    assert!(state.status.success());
    assert_eq!(
        *log.lock().unwrap(),
        ["agent_decision=none", "agent_state=waiting"]
    );

    // Killed outright (no signal handler runs), it leaves the lock to the system, which releases it.
    asking.kill().unwrap();
    asking.wait().unwrap();
    let state = osh(&dir, &["agent", "state", "idle"]).output().unwrap();
    assert!(state.status.success());
    assert_eq!(log.lock().unwrap().last().unwrap(), "agent_state=idle");
    let _ = std::fs::remove_dir_all(&dir);
}
