//! The pure part of `osh` the desktop app shares: the hook configuration `osh agent setup` prints, and the names
//! and rules of the agent variables, which the app's socket enforces.
//!
//! The binary prints it for the `osh` that runs; the app's Settings shows the same text for the `osh` it bundles.
//! Only `serde_json` is needed, so the app depending on this library adds nothing.

use serde_json::{Value, json};

/// How long a `PostToolUse` hook may take (seconds); `osh agent state` answers in milliseconds.
const POST_TOOL_TIMEOUT: u64 = 5;

/// How long `agent ask` waits for the deck by default.
pub const ASK_DEFAULT_SECS: u64 = 55;

/// The text variable agent hooks set to show what an agent is doing (`idle`, `working`, `waiting`, `done`).
pub const AGENT_STATE: &str = "agent_state";
/// The text variable a deck key sets to answer an agent's permission prompt (`none`, `allow`, `deny`).
pub const AGENT_DECISION: &str = "agent_decision";
/// The optional number variable counting the agent sessions that wait for the deck.
pub const AGENT_WAITING: &str = "agent_waiting";
/// The one value of [`AGENT_DECISION`] a process outside the app may write: clearing it.
pub const DECISION_CLEARED: &str = "none";
/// Why the app's socket refuses any other value of [`AGENT_DECISION`].
pub const DECISION_REFUSED: &str =
    "only the deck can answer: agent_decision can be set only to none from outside the app";

/// Whether a process outside the app (through its local socket) may set variable `name` to `value`. Only a deck
/// key, an action inside the running module, writes `allow` or `deny` to [`AGENT_DECISION`]; from outside it can
/// only be cleared.
pub fn outside_write_allowed(name: &str, value: &Value) -> bool {
    name != AGENT_DECISION || value == DECISION_CLEARED
}

/// What the app and `osh agent setup` say when macOS runs the app from a translocated copy.
pub const TRANSLOCATED: &str = "Move OpenStreamHub to Applications and open it from there";

/// Whether `path` lies inside a macOS App Translocation mount: a read-only copy that moves at every launch, so a
/// hook calling an `osh` there stops working.
pub fn is_translocated(path: &str) -> bool {
    path.contains("/AppTranslocation/")
}

/// The coding agents `osh agent setup` writes a hook configuration for.
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum Agent {
    Claude,
    Codex,
}

/// A path as one `sh` word: as it is when it holds only safe characters, else single-quoted.
pub fn shell_word(path: &str) -> String {
    let safe = |c: char| c.is_ascii_alphanumeric() || "/._-+:@%,".contains(c);
    if !path.is_empty() && path.chars().all(safe) {
        path.to_string()
    } else {
        format!("'{}'", path.replace('\'', r"'\''"))
    }
}

/// The hook configuration for `agent`, calling `osh` (an absolute path): what `osh agent setup` prints.
pub fn setup(agent: Agent, osh: &str) -> String {
    let osh = shell_word(osh);
    let working = format!("{osh} agent state working");
    let ask = format!("{osh} agent ask --timeout {ASK_DEFAULT_SECS}");
    let done = format!("{osh} agent state done");
    let ended = format!("{osh} agent state end");
    match agent {
        Agent::Claude => {
            let hook = |command: &str, extra: Value| {
                let mut hook = json!({"type":"command","command":command});
                hook.as_object_mut()
                    .unwrap()
                    .extend(extra.as_object().cloned().unwrap_or_default());
                json!([{"hooks":[hook]}])
            };
            let config = json!({"hooks":{
                "UserPromptSubmit": hook(&working, json!({"async":true})),
                // Not async: it must finish before the Stop hook so a finished turn is not shown as working.
                "PostToolUse": hook(&working, json!({"timeout":POST_TOOL_TIMEOUT})),
                "PermissionRequest": hook(&ask, json!({"timeout":ASK_DEFAULT_SECS + 10})),
                "Stop": hook(&done, json!({"async":true})),
                "SessionEnd": hook(&ended, json!({"async":true})),
            }});
            serde_json::to_string_pretty(&config).unwrap_or_default()
        }
        Agent::Codex => {
            // TOML basic strings take the same escapes serde_json writes.
            let table = |event: &str, command: &str, timeout: Option<u64>| {
                let mut text = format!(
                    "[[hooks.{event}]]\n[[hooks.{event}.hooks]]\ntype = \"command\"\ncommand = {}\n",
                    json!(command)
                );
                if let Some(timeout) = timeout {
                    text.push_str(&format!("timeout = {timeout}\n"));
                }
                text
            };
            [
                "# OpenStreamHub deck hooks for Codex. Add these lines to ~/.codex/config.toml, next to any\n\
                 # [[hooks.*]] tables already there. Codex runs new hooks only after you review and trust them:\n\
                 # run /hooks in Codex once after adding them.\n"
                    .to_string(),
                table("UserPromptSubmit", &working, None),
                table("PostToolUse", &working, Some(POST_TOOL_TIMEOUT)),
                table("PermissionRequest", &ask, Some(ASK_DEFAULT_SECS + 10)),
                table("Stop", &done, None),
            ]
            .join("\n")
            .trim_end()
            .to_string()
        }
    }
}

/// How to merge the Claude Code snippet, printed on stderr so stdout stays plain JSON.
pub const CLAUDE_HEADER: &str = "# OpenStreamHub deck hooks for Claude Code. Merge the \"hooks\" object below into\n\
# ~/.claude/settings.json (or a project's .claude/settings.json). Where the file already has hooks for\n\
# UserPromptSubmit, PostToolUse, PermissionRequest, Stop or SessionEnd, add these entries to those arrays.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outside_writes_only_clear_the_decision() {
        assert!(outside_write_allowed(AGENT_DECISION, &json!("none")));
        for value in [json!("allow"), json!("deny"), json!("Allow"), json!(true)] {
            assert!(!outside_write_allowed(AGENT_DECISION, &value), "{value}");
        }
        assert!(outside_write_allowed(AGENT_STATE, &json!("waiting")));
        assert!(outside_write_allowed("busy", &json!(true)));
    }

    #[test]
    fn translocated_paths_are_recognised() {
        assert!(is_translocated(
            "/private/var/folders/x/T/AppTranslocation/ABC/d/OpenStreamHub.app/Contents/MacOS/osh"
        ));
        assert!(!is_translocated(
            "/Applications/OpenStreamHub.app/Contents/MacOS/osh"
        ));
    }

    #[test]
    fn setup_prints_valid_configuration_with_a_quoted_path() {
        assert_eq!(
            shell_word("/Applications/OpenStreamHub.app/Contents/MacOS/osh"),
            "/Applications/OpenStreamHub.app/Contents/MacOS/osh"
        );
        assert_eq!(
            shell_word("/Users/a b/it's/osh"),
            r"'/Users/a b/it'\''s/osh'"
        );

        let claude: Value = serde_json::from_str(&setup(Agent::Claude, "/Users/a b/osh")).unwrap();
        let hooks = &claude["hooks"];
        let hook = |event: &str| hooks[event][0]["hooks"][0].clone();
        assert_eq!(
            hook("PermissionRequest"),
            json!({"type":"command","command":"'/Users/a b/osh' agent ask --timeout 55","timeout":65})
        );
        assert_eq!(
            hook("UserPromptSubmit"),
            json!({"type":"command","command":"'/Users/a b/osh' agent state working","async":true})
        );
        assert_eq!(
            hook("Stop"),
            json!({"type":"command","command":"'/Users/a b/osh' agent state done","async":true})
        );
        assert_eq!(
            hook("PostToolUse"),
            json!({"type":"command","command":"'/Users/a b/osh' agent state working","timeout":5})
        );
        assert_eq!(
            hook("SessionEnd"),
            json!({"type":"command","command":"'/Users/a b/osh' agent state end","async":true})
        );
        assert_eq!(hooks.as_object().unwrap().len(), 5);

        let codex = setup(Agent::Codex, "/opt/osh\"x");
        assert!(codex.contains("/hooks"));
        let mut tables = Vec::new();
        for line in codex
            .lines()
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
        {
            if let Some(table) = line
                .strip_prefix("[[")
                .and_then(|line| line.strip_suffix("]]"))
            {
                tables.push(table.to_string());
                continue;
            }
            let (key, value) = line.split_once(" = ").expect(line);
            match key {
                "type" => assert_eq!(value, "\"command\""),
                "timeout" => assert!(value == "65" || value == "5", "{value}"),
                "command" => {
                    let command: String = serde_json::from_str(value).unwrap();
                    assert!(command.starts_with(r#"'/opt/osh"x' agent "#), "{command}");
                }
                _ => panic!("unexpected key {key}"),
            }
        }
        assert_eq!(
            tables,
            [
                "hooks.UserPromptSubmit",
                "hooks.UserPromptSubmit.hooks",
                "hooks.PostToolUse",
                "hooks.PostToolUse.hooks",
                "hooks.PermissionRequest",
                "hooks.PermissionRequest.hooks",
                "hooks.Stop",
                "hooks.Stop.hooks"
            ]
        );
        assert!(
            codex.contains("command = \"'/opt/osh\\\"x' agent ask --timeout 55\"\ntimeout = 65")
        );
    }
}
