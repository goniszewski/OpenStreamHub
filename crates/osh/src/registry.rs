//! What each agent session is doing, so several sessions can share one deck.
//!
//! Hooks of different sessions run as separate `osh` processes. Each records its session's state in a small file
//! next to the app's socket (`agents.json`, guarded by `agents.lock`); the deck's `agent_state` is the summary of
//! all sessions: `waiting` if any waits for the deck, else `working` if any works, else `done` if any finished,
//! else `idle`. `agent_waiting` counts the sessions that wait.

use serde_json::{Map, Value, json};
use std::{collections::BTreeMap, path::Path};

/// A session that keeps its state this long without a hook is forgotten.
pub const STALE_MS: u64 = 12 * 60 * 60 * 1000;
/// A finished session stops counting as `done` after this long (the deck is idle again).
pub const DONE_MS: u64 = 30 * 60 * 1000;
/// A waiting entry older than this is dropped whatever its process says (pids get reused): the longest ask plus a margin.
pub const WAITING_MS: u64 = 11 * 60 * 1000;
/// Session name used when a hook sends none (a hand-typed `osh agent state working`).
pub const DEFAULT_SESSION: &str = "default";

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    /// `working`, `waiting`, `settling` or `done`. `settling` is an ask that has its turn and lets the deck show
    /// that the previous prompt is over; it counts as working.
    pub state: String,
    /// When the session entered this state, in milliseconds since the epoch.
    pub since: u64,
    /// The `osh agent ask` that owns a `waiting` entry; 0 when no process does.
    pub pid: u32,
}

pub type Sessions = BTreeMap<String, Entry>;

#[derive(Debug, PartialEq)]
pub struct Summary {
    pub state: &'static str,
    pub waiting: i32,
}

/// A session id a hook sent, made safe to keep: short, plain characters, else the default session.
pub fn session_id(hook: &Value) -> String {
    hook.get("session_id")
        .and_then(Value::as_str)
        .filter(|id| {
            !id.is_empty()
                && id.len() <= 64
                && id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        })
        .unwrap_or(DEFAULT_SESSION)
        .to_string()
}

pub fn summary(sessions: &Sessions) -> Summary {
    let count = |state: &str| {
        sessions
            .values()
            .filter(|entry| entry.state == state)
            .count()
    };
    let working = count("working") + count("settling");
    let state = if count("settling") > 0 {
        // Between two prompts the deck shows Working, so a repeated press is seen not to answer the next one.
        "working"
    } else if count("waiting") > 0 {
        "waiting"
    } else if working > 0 {
        "working"
    } else if count("done") > 0 {
        "done"
    } else {
        "idle"
    };
    Summary {
        state,
        waiting: count("waiting") as i32,
    }
}

/// Forgets what is out of date: sessions silent for [`STALE_MS`], finished ones older than [`DONE_MS`], and
/// waiting ones whose ask is gone (`alive` says whether a process exists).
pub fn prune(sessions: &mut Sessions, now: u64, alive: &dyn Fn(u32) -> bool) {
    sessions.retain(|_, entry| {
        let age = now.saturating_sub(entry.since);
        age <= STALE_MS
            && !(entry.state == "done" && age > DONE_MS)
            && !(matches!(entry.state.as_str(), "waiting" | "settling")
                && entry.pid != 0
                && (age > WAITING_MS || !alive(entry.pid)))
    });
}

/// The waiting session that has waited longest: the one whose ask goes first.
pub fn front(sessions: &Sessions) -> Option<&str> {
    sessions
        .iter()
        .filter(|(_, entry)| entry.state == "waiting")
        .min_by_key(|(id, entry)| (entry.since, id.as_str()))
        .map(|(id, _)| id.as_str())
}

/// The registry key of an ask: its session and its process, so two asks of one session (a subagent, parallel
/// tools) keep separate places in line.
pub fn ask_key(session: &str, pid: u32) -> String {
    format!("{session}#{pid}")
}

/// Records `state` for `id`: `idle` and `end` forget it. Entering the state it is already in keeps its place.
pub fn record(sessions: &mut Sessions, id: &str, state: &str, now: u64, pid: u32) {
    match state {
        "working" | "waiting" | "settling" | "done" => {
            if sessions.get(id).is_some_and(|entry| entry.state == state) {
                return;
            }
            sessions.insert(
                id.to_string(),
                Entry {
                    state: state.to_string(),
                    since: now,
                    pid,
                },
            );
        }
        _ => {
            sessions.remove(id);
        }
    }
}

pub fn load(path: &Path) -> Sessions {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Sessions::new();
    };
    let Ok(Value::Object(map)) = serde_json::from_str::<Value>(&text) else {
        return Sessions::new();
    };
    map.into_iter()
        .filter_map(|(id, value)| {
            Some((
                id,
                Entry {
                    state: value["state"].as_str()?.to_string(),
                    since: value["since"].as_u64()?,
                    pid: value["pid"].as_u64().unwrap_or(0) as u32,
                },
            ))
        })
        .filter(|(_, entry)| {
            matches!(
                entry.state.as_str(),
                "working" | "waiting" | "settling" | "done"
            )
        })
        .collect()
}

/// Writes the registry through a temporary file and a rename, so a reader never sees half of it.
pub fn save(path: &Path, sessions: &Sessions) -> std::io::Result<()> {
    let map: Map<String, Value> = sessions
        .iter()
        .map(|(id, e)| {
            (
                id.clone(),
                json!({"state":e.state,"since":e.since,"pid":e.pid}),
            )
        })
        .collect();
    let temp = path.with_extension("tmp");
    std::fs::write(&temp, Value::Object(map).to_string())?;
    std::fs::rename(&temp, path)
}

/// Whether process `pid` exists (0 means no process owns the entry, which never goes stale this way).
#[cfg(unix)]
pub fn alive(pid: u32) -> bool {
    unsafe extern "C" {
        fn kill(pid: i32, signal: i32) -> i32;
    }
    const ESRCH: i32 = 3;
    if pid == 0 || pid > i32::MAX as u32 {
        return true;
    }
    // Signal 0 only checks; EPERM still means the process exists.
    unsafe {
        kill(pid as i32, 0) == 0 || std::io::Error::last_os_error().raw_os_error() != Some(ESRCH)
    }
}

#[cfg(not(unix))]
pub fn alive(_: u32) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(state: &str, since: u64, pid: u32) -> Entry {
        Entry {
            state: state.into(),
            since,
            pid,
        }
    }

    #[test]
    fn the_summary_puts_waiting_before_working_before_done() {
        let mut sessions = Sessions::new();
        assert_eq!(
            summary(&sessions),
            Summary {
                state: "idle",
                waiting: 0
            }
        );
        sessions.insert("a".into(), entry("done", 1, 0));
        assert_eq!(summary(&sessions).state, "done");
        sessions.insert("b".into(), entry("working", 2, 0));
        assert_eq!(summary(&sessions).state, "working");
        sessions.insert("c".into(), entry("waiting", 3, 7));
        sessions.insert("d".into(), entry("waiting", 4, 8));
        assert_eq!(
            summary(&sessions),
            Summary {
                state: "waiting",
                waiting: 2
            }
        );
    }

    #[test]
    fn the_longest_waiting_session_is_in_front() {
        let mut sessions = Sessions::new();
        assert_eq!(front(&sessions), None);
        sessions.insert("late".into(), entry("waiting", 20, 1));
        sessions.insert("early".into(), entry("waiting", 10, 2));
        sessions.insert("busy".into(), entry("working", 1, 0));
        assert_eq!(front(&sessions), Some("early"));
        sessions.insert("tie".into(), entry("waiting", 10, 3));
        assert_eq!(front(&sessions), Some("early"));
    }

    #[test]
    fn two_asks_of_one_session_have_separate_places() {
        let mut sessions = Sessions::new();
        record(&mut sessions, &ask_key("s", 1), "waiting", 5, 1);
        record(&mut sessions, &ask_key("s", 2), "waiting", 6, 2);
        // A state hook of the session does not touch either ask.
        record(&mut sessions, "s", "done", 7, 0);
        assert_eq!(
            summary(&sessions),
            Summary {
                state: "waiting",
                waiting: 2
            }
        );
        // The first ends: the second stays, in front.
        record(&mut sessions, &ask_key("s", 1), "idle", 8, 1);
        assert_eq!(front(&sessions), Some("s#2"));
        record(&mut sessions, "s", "end", 9, 0);
        assert_eq!(sessions.len(), 1);
    }

    #[test]
    fn a_settling_ask_counts_as_working_not_waiting() {
        let mut sessions = Sessions::new();
        record(&mut sessions, "a#1", "settling", 5, 1);
        assert_eq!(
            summary(&sessions),
            Summary {
                state: "working",
                waiting: 0
            }
        );
        assert_eq!(front(&sessions), None);
        // Others still queued are counted, but the deck keeps saying Working until the settling ask is done.
        record(&mut sessions, "b#2", "waiting", 6, 2);
        assert_eq!(
            summary(&sessions),
            Summary {
                state: "working",
                waiting: 1
            }
        );
    }

    #[test]
    fn repeating_a_state_keeps_the_place_in_line() {
        let mut sessions = Sessions::new();
        record(&mut sessions, "a", "waiting", 5, 0);
        record(&mut sessions, "a", "waiting", 9, 0);
        assert_eq!(sessions["a"].since, 5);
    }

    #[test]
    fn old_and_orphaned_entries_are_pruned() {
        let mut sessions = Sessions::new();
        let now = STALE_MS * 2;
        sessions.insert("ancient".into(), entry("working", 0, 0));
        sessions.insert("old-done".into(), entry("done", now - DONE_MS - 1, 0));
        sessions.insert("done".into(), entry("done", now - 10, 0));
        sessions.insert("dead".into(), entry("waiting", now - 10, 99));
        sessions.insert("live".into(), entry("waiting", now - 10, 7));
        sessions.insert("handset".into(), entry("waiting", now - 10, 0));
        // A reused pid does not keep an old waiting entry alive.
        sessions.insert("reused".into(), entry("waiting", now - WAITING_MS - 1, 7));
        prune(&mut sessions, now, &|pid| pid == 7);
        let kept: Vec<&str> = sessions.keys().map(String::as_str).collect();
        assert_eq!(kept, ["done", "handset", "live"]);
    }

    #[test]
    fn session_ids_are_plain_or_the_default() {
        assert_eq!(session_id(&json!({"session_id":"abc-123_X"})), "abc-123_X");
        for bad in [
            json!({}),
            json!({"session_id":""}),
            json!({"session_id":"a/b"}),
            json!({"session_id":"x".repeat(65)}),
            json!({"session_id":5}),
        ] {
            assert_eq!(session_id(&bad), DEFAULT_SESSION);
        }
    }

    #[test]
    fn the_file_round_trips_and_a_broken_file_is_empty() {
        let dir = std::env::temp_dir().join(format!("osh-reg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("agents.json");
        assert!(load(&path).is_empty());
        let mut sessions = Sessions::new();
        sessions.insert("a".into(), entry("waiting", 5, 42));
        save(&path, &sessions).unwrap();
        assert_eq!(load(&path), sessions);
        std::fs::write(&path, "{nope").unwrap();
        assert!(load(&path).is_empty());
        std::fs::write(&path, r#"{"a":{"state":"bogus","since":1}}"#).unwrap();
        assert!(load(&path).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn this_process_is_alive() {
        assert!(alive(std::process::id()));
        assert!(alive(0));
    }
}
