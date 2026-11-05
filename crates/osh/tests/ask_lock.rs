//! The real `osh` binary: asks of several sessions queue, and an `agent ask` that is killed while it waits hands
//! its turn on, so the next session's prompt reaches the deck.
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
                    let value = match &request["value"] {
                        Value::String(text) => text.clone(),
                        other => other.to_string(),
                    };
                    seen.lock().unwrap().push(format!(
                        "{}={value}",
                        request["name"].as_str().unwrap_or_default()
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

fn count(log: &Mutex<Vec<String>>, set: &str) -> usize {
    log.lock()
        .unwrap()
        .iter()
        .filter(|seen| *seen == set)
        .count()
}

fn wait_for(log: &Mutex<Vec<String>>, set: &str, times: usize) {
    let started = Instant::now();
    while count(log, set) < times {
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "waiting for {set} x{times}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn ask(dir: &Path, session: &str) -> std::process::Child {
    let mut asking = osh(dir, &["agent", "ask", "--timeout", "30"])
        .spawn()
        .unwrap();
    asking
        .stdin
        .take()
        .unwrap()
        .write_all(
            format!(r#"{{"hook_event_name":"PermissionRequest","session_id":"{session}"}}"#)
                .as_bytes(),
        )
        .unwrap();
    asking
}

fn state(dir: &Path, session: &str, state: &str) {
    let mut hook = osh(dir, &["agent", "state", state]).spawn().unwrap();
    hook.stdin
        .take()
        .unwrap()
        .write_all(format!(r#"{{"session_id":"{session}"}}"#).as_bytes())
        .unwrap();
    assert!(hook.wait_with_output().unwrap().status.success());
}

#[test]
fn asks_of_two_sessions_queue_and_a_killed_ask_hands_its_turn_on() {
    let dir = temp_dir();
    let log = fake_app(&dir);
    let mut first = ask(&dir, "one");
    wait_for(&log, "agent_waiting=1", 1);
    // A second session's ask queues behind it: the deck counts both, and the second has not cleared the decision.
    let mut second = ask(&dir, "two");
    wait_for(&log, "agent_waiting=2", 1);
    let clears = count(&log, "agent_decision=none");
    // A state hook of a third session changes nothing visible while two wait.
    state(&dir, "three", "done");
    assert_eq!(log.lock().unwrap().last().unwrap(), "agent_waiting=2");
    assert_eq!(count(&log, "agent_state=waiting"), 3);

    // Killed outright (no signal handler runs), the first leaves the lock to the system and its entry to the
    // liveness check: the second takes its turn.
    first.kill().unwrap();
    first.wait().unwrap();
    wait_for(&log, "agent_decision=none", clears + 1);

    // When the second goes too, a later hook finds nobody waiting and shows the third session's finished work.
    second.kill().unwrap();
    second.wait().unwrap();
    state(&dir, "three", "done");
    assert_eq!(count(&log, "agent_state=done"), 1);
    assert_eq!(
        log.lock()
            .unwrap()
            .iter()
            .rev()
            .find(|s| s.starts_with("agent_waiting="))
            .unwrap(),
        "agent_waiting=0"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
