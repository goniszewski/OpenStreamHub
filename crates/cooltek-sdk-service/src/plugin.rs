//! Plugin actions: the parameter values of an [`Action::Plugin`](crate::Action::Plugin), their checks, and the
//! bridge through which a module service asks the app that supervises it to run one.
//!
//! A plugin is a separate process the OpenStreamHub app starts and talks to (see `docs/plugins-design.md`). A module
//! service never reaches a plugin itself: when a key runs a plugin action it writes a `plugin_invoke` record to its
//! stdout with a request number, and waits for the app to write the plugin's answer back on its control channel as
//! `plugin_result` with the same number. Only the service decides that a press runs, as for every other action.

use crate::profile::{
    VariableValue, fill_in, is_invisible_text_character, validate_variable_name,
    variable_references,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap},
    io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

/// Why a plugin action cannot run in a service without a plugin bridge.
pub const PLUGIN_UNAVAILABLE: &str = "plugins are run by the OpenStreamHub app, and this run was not started by it; no action was taken";

/// Longest plugin id in bytes, such as `com.example.homeassistant`.
pub const MAX_PLUGIN_ID_BYTES: usize = 128;
/// Longest plugin action id in bytes.
pub const MAX_PLUGIN_ACTION_BYTES: usize = 64;
/// Most parameters one plugin action carries.
pub const MAX_PLUGIN_PARAMS: usize = 32;
/// Longest text parameter in UTF-8 bytes, before and after its variables are filled in.
pub const MAX_PLUGIN_TEXT_BYTES: usize = 2048;
/// Most UTF-8 bytes of all text parameters of one action together, after its variables are filled in.
pub const MAX_PLUGIN_PARAMS_BYTES: usize = 8192;

/// One parameter value of a plugin action: the manifest types text and choice are both text.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PluginValue {
    /// `true` or `false`.
    Bool(bool),
    /// A JSON number, integer or decimal.
    Number(serde_json::Number),
    /// One line of text, or the id of a choice.
    Text(String),
}

/// Checks a reverse-DNS plugin id: at least two dot-separated labels of `a-z`, `0-9` and `-`, each starting with a
/// letter or digit, at most [`MAX_PLUGIN_ID_BYTES`] in all.
pub fn validate_plugin_id(id: &str) -> Result<(), String> {
    let labels_ok = id.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && label.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
            && label
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    });
    if id.len() > MAX_PLUGIN_ID_BYTES || !id.contains('.') || !labels_ok {
        return Err(format!(
            "plugin id must be a reverse-DNS name such as com.example.tool (lowercase letters, digits, - and dots, at most {MAX_PLUGIN_ID_BYTES} bytes)"
        ));
    }
    Ok(())
}

/// Checks a plugin action id: 1 to [`MAX_PLUGIN_ACTION_BYTES`] of `a-z`, `0-9`, `_`, `-` and `.`, starting with a
/// letter.
pub fn validate_plugin_action_id(id: &str) -> Result<(), String> {
    if id.is_empty()
        || id.len() > MAX_PLUGIN_ACTION_BYTES
        || !id.starts_with(|c: char| c.is_ascii_lowercase())
        || !id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '-' | '.'))
    {
        return Err(format!(
            "plugin action must be 1 to {MAX_PLUGIN_ACTION_BYTES} of a-z, 0-9, _, - and ., starting with a letter"
        ));
    }
    Ok(())
}

/// A version as `MAJOR.MINOR.PATCH`, compared numerically.
pub fn parse_version(version: &str) -> Option<(u32, u32, u32)> {
    let mut parts = version.split('.');
    let mut next = || -> Option<u32> {
        let part = parts.next()?;
        if part.is_empty() || part.len() > 9 || !part.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        part.parse().ok()
    };
    let version = (next()?, next()?, next()?);
    parts.next().is_none().then_some(version)
}

/// Checks one text parameter: at most [`MAX_PLUGIN_TEXT_BYTES`], one line without control or invisible characters.
fn check_text(name: &str, text: &str) -> Result<(), String> {
    if text.len() > MAX_PLUGIN_TEXT_BYTES
        || text
            .chars()
            .any(|c| c.is_control() || is_invisible_text_character(c))
    {
        return Err(format!(
            "plugin parameter {name} must be one line of at most {MAX_PLUGIN_TEXT_BYTES} bytes without control or invisible characters"
        ));
    }
    Ok(())
}

/// Checks the parts of a plugin action that do not depend on its manifest. `variables` names the text parameters
/// whose `{{name}}` references are filled in when the key is pressed.
pub fn validate_plugin_action(
    plugin: &str,
    min_version: &str,
    action: &str,
    params: &BTreeMap<String, PluginValue>,
    variables: &[String],
) -> Result<(), String> {
    validate_plugin_id(plugin)?;
    parse_version(min_version)
        .ok_or("plugin min_version must be MAJOR.MINOR.PATCH, such as 1.0.0")?;
    validate_plugin_action_id(action)?;
    if params.len() > MAX_PLUGIN_PARAMS {
        return Err(format!(
            "a plugin action takes at most {MAX_PLUGIN_PARAMS} parameters"
        ));
    }
    let mut total = 0;
    for (name, value) in params {
        validate_variable_name(name).map_err(|_| {
            format!("plugin parameter name {name:?} must be 1 to 32 of a-z, 0-9 and _, starting with a letter")
        })?;
        if let PluginValue::Text(text) = value {
            check_text(name, text)?;
            total += text.len();
        }
    }
    if total > MAX_PLUGIN_PARAMS_BYTES {
        return Err(format!(
            "plugin text parameters must total at most {MAX_PLUGIN_PARAMS_BYTES} bytes"
        ));
    }
    for (index, name) in variables.iter().enumerate() {
        if variables[..index].contains(name) {
            return Err(format!("plugin variables lists parameter {name} twice"));
        }
        let Some(PluginValue::Text(text)) = params.get(name) else {
            return Err(format!(
                "plugin variables names {name}, which is not a text parameter of this action"
            ));
        };
        variable_references(text).map_err(|error| format!("plugin parameter {name}: {error}"))?;
    }
    Ok(())
}

/// The variable names the text parameters listed in `variables` refer to.
pub(crate) fn plugin_variable_references<'a>(
    params: &'a BTreeMap<String, PluginValue>,
    variables: &'a [String],
) -> Result<Vec<(&'a str, &'a str)>, String> {
    let mut found = Vec::new();
    for name in variables {
        if let Some(PluginValue::Text(text)) = params.get(name) {
            for reference in variable_references(text)
                .map_err(|error| format!("plugin parameter {name}: {error}"))?
            {
                found.push((name.as_str(), reference));
            }
        }
    }
    Ok(found)
}

/// The parameters of a plugin action with the variables of the text parameters named in `variables` filled in from
/// `lookup` when its key is pressed. Fails closed, so nothing is sent, when a filled-in text breaks the one-line rules
/// or the size limits. Errors name the parameter, never a value.
pub fn fill_in_plugin_params(
    params: &BTreeMap<String, PluginValue>,
    variables: &[String],
    lookup: impl Fn(&str) -> Option<VariableValue>,
) -> Result<BTreeMap<String, PluginValue>, String> {
    let mut filled = params.clone();
    let mut total = 0;
    for (name, value) in filled.iter_mut() {
        let PluginValue::Text(text) = value else {
            continue;
        };
        if variables.contains(name) {
            let new =
                fill_in(text, &lookup).map_err(|error| format!("parameter {name}: {error}"))?;
            if check_text(name, &new).is_err() {
                return Err(format!(
                    "parameter {name} with its variables filled in is not one line of at most {MAX_PLUGIN_TEXT_BYTES} bytes without control or invisible characters"
                ));
            }
            *text = new;
        }
        total += text.len();
    }
    if total > MAX_PLUGIN_PARAMS_BYTES {
        return Err(format!(
            "the text parameters with their variables filled in total more than {MAX_PLUGIN_PARAMS_BYTES} bytes"
        ));
    }
    Ok(filled)
}

/// Writes one JSON record to the service's stdout, in the same stream as its other records.
pub type RecordSink = Box<dyn Fn(&Value) -> io::Result<()> + Send + Sync>;

/// The answer the app gives for one invoke.
type Answer = Result<(), String>;

struct BridgeState {
    send: RecordSink,
    next: AtomicU64,
    pending: Mutex<HashMap<u64, mpsc::Sender<Answer>>>,
}

/// A module service's side of plugin invokes: it writes `plugin_invoke` records and waits for the matching
/// `plugin_result`, which the control channel hands to [`PluginBridge::complete`]. Cloning shares the state.
#[derive(Clone)]
pub struct PluginBridge(Arc<BridgeState>);

/// How often a waiting invoke looks at the cancellation flag.
const CANCEL_POLL: Duration = Duration::from_millis(50);

impl PluginBridge {
    /// A bridge that writes its records with `send`.
    pub fn new(send: RecordSink) -> Self {
        Self(Arc::new(BridgeState {
            send,
            next: AtomicU64::new(1),
            pending: Mutex::new(HashMap::new()),
        }))
    }

    fn pending(&self) -> std::sync::MutexGuard<'_, HashMap<u64, mpsc::Sender<Answer>>> {
        self.0
            .pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Asks the app to run `action` of `plugin` and waits up to `timeout` for the answer. A late answer, an
    /// unreachable app or a cancellation fails the press; the plugin may already have acted.
    pub fn invoke(
        &self,
        plugin: &str,
        min_version: &str,
        action: &str,
        params: &BTreeMap<String, PluginValue>,
        timeout: Duration,
        cancellation: Option<&AtomicBool>,
    ) -> Result<(), String> {
        let request = self.0.next.fetch_add(1, Ordering::SeqCst);
        let (sender, receiver) = mpsc::channel();
        self.pending().insert(request, sender);
        let record = json!({
            "type": "plugin_invoke",
            "request": request,
            "plugin": plugin,
            "min_version": min_version,
            "action": action,
            "params": params,
            "timeout_ms": u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX),
        });
        if let Err(error) = (self.0.send)(&record) {
            self.pending().remove(&request);
            return Err(format!(
                "the app could not be asked to run the plugin: {error}"
            ));
        }
        let deadline = Instant::now() + timeout;
        let answer = loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break Err(format!(
                    "{plugin} did not answer within {:.1} s",
                    timeout.as_secs_f32()
                ));
            }
            if cancellation.is_some_and(|flag| flag.load(Ordering::SeqCst)) {
                break Err("the run was cancelled while the plugin was working".into());
            }
            match receiver.recv_timeout(remaining.min(CANCEL_POLL)) {
                Ok(answer) => return answer,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    break Err("the plugin bridge closed".into());
                }
            }
        };
        self.pending().remove(&request);
        answer
    }

    /// Hands the app's answer to the invoke waiting for `request`. An unknown or late request is an error, so the
    /// control channel can say so.
    pub fn complete(&self, request: u64, answer: Result<(), String>) -> Result<(), String> {
        let waiter = self
            .pending()
            .remove(&request)
            .ok_or_else(|| format!("no plugin request {request} is waiting"))?;
        waiter
            .send(answer)
            .map_err(|_| format!("plugin request {request} stopped waiting"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    fn params(pairs: &[(&str, PluginValue)]) -> BTreeMap<String, PluginValue> {
        pairs
            .iter()
            .map(|(name, value)| (name.to_string(), value.clone()))
            .collect()
    }

    #[test]
    fn ids_and_versions_follow_their_rules() {
        assert!(validate_plugin_id("com.example.tool").is_ok());
        assert!(validate_plugin_id("io.github.user-1.x2").is_ok());
        for bad in [
            "tool",
            "Com.example",
            "com..x",
            "com.-x",
            "com.ex ample",
            ".com.x",
            "com.x.",
        ] {
            assert!(validate_plugin_id(bad).is_err(), "{bad}");
        }
        assert!(validate_plugin_id(&format!("com.{}", "a".repeat(125))).is_err());
        assert!(validate_plugin_action_id("turn_on.light-2").is_ok());
        for bad in ["", "On", "1st", "a b", &"a".repeat(65)] {
            assert!(validate_plugin_action_id(bad).is_err(), "{bad}");
        }
        assert_eq!(parse_version("1.20.3"), Some((1, 20, 3)));
        for bad in [
            "1.2",
            "1.2.3.4",
            "1.x.3",
            "",
            "1..3",
            "+1.2.3",
            "1234567890.0.0",
        ] {
            assert_eq!(parse_version(bad), None, "{bad}");
        }
        assert!(parse_version("1.10.0") > parse_version("1.9.9"));
    }

    #[test]
    fn values_parse_as_bool_number_or_text() {
        let parsed: BTreeMap<String, PluginValue> =
            serde_json::from_str(r#"{"a":true,"b":2.5,"c":"hi","d":-3}"#).unwrap();
        assert_eq!(parsed["a"], PluginValue::Bool(true));
        assert!(matches!(parsed["b"], PluginValue::Number(_)));
        assert_eq!(parsed["c"], PluginValue::Text("hi".into()));
        assert!(serde_json::from_str::<PluginValue>("null").is_err());
        assert!(serde_json::from_str::<PluginValue>("[1]").is_err());
        assert!(serde_json::from_str::<PluginValue>(r#"{"x":1}"#).is_err());
    }

    #[test]
    fn an_action_is_checked_without_its_manifest() {
        let ok = params(&[
            ("entity", PluginValue::Text("light.desk {{room}}".into())),
            ("on", PluginValue::Bool(true)),
        ]);
        assert!(
            validate_plugin_action("com.x.y", "1.0.0", "toggle", &ok, &["entity".into()]).is_ok()
        );
        let check = |params: &BTreeMap<String, PluginValue>, variables: &[&str]| {
            let variables: Vec<String> = variables.iter().map(|name| name.to_string()).collect();
            validate_plugin_action("com.x.y", "1.0.0", "toggle", params, &variables)
        };
        assert!(
            check(&ok, &["on"])
                .unwrap_err()
                .contains("not a text parameter")
        );
        assert!(
            check(&ok, &["gone"])
                .unwrap_err()
                .contains("not a text parameter")
        );
        assert!(
            check(&ok, &["entity", "entity"])
                .unwrap_err()
                .contains("twice")
        );
        let broken = params(&[("entity", PluginValue::Text("{{ room }}".into()))]);
        assert!(
            check(&broken, &[]).is_ok(),
            "variables off: {{ is plain text"
        );
        assert!(
            check(&broken, &["entity"])
                .unwrap_err()
                .starts_with("plugin parameter entity: every {{")
        );
        assert!(check(&params(&[("Bad", PluginValue::Bool(true))]), &[]).is_err());
        assert!(check(&params(&[("t", PluginValue::Text("a\nb".into()))]), &[]).is_err());
        assert!(check(&params(&[("t", PluginValue::Text("x".repeat(2049)))]), &[]).is_err());
        let many: Vec<(String, PluginValue)> = (0..33)
            .map(|i| (format!("p{i}"), PluginValue::Bool(true)))
            .collect();
        assert!(check(&many.into_iter().collect(), &[]).is_err());
        let big: BTreeMap<String, PluginValue> = (0..5)
            .map(|i| (format!("p{i}"), PluginValue::Text("x".repeat(2000))))
            .collect();
        assert!(check(&big, &[]).unwrap_err().contains("total"));
        assert!(validate_plugin_action("com.x.y", "1.0", "toggle", &ok, &[]).is_err());
    }

    #[test]
    fn only_listed_text_parameters_are_filled_in_and_checked_again() {
        let given = params(&[
            ("entity", PluginValue::Text("light.{{room}}".into())),
            ("raw", PluginValue::Text("{{room}}".into())),
            ("level", PluginValue::Number(3.into())),
        ]);
        let lookup = |value: &'static str| {
            move |name: &str| (name == "room").then(|| VariableValue::Text(value.into()))
        };
        let filled = fill_in_plugin_params(&given, &["entity".into()], lookup("desk")).unwrap();
        assert_eq!(filled["entity"], PluginValue::Text("light.desk".into()));
        assert_eq!(filled["raw"], PluginValue::Text("{{room}}".into()));
        assert_eq!(filled["level"], PluginValue::Number(3.into()));
        let error =
            fill_in_plugin_params(&given, &["entity".into()], lookup("a\u{200b}b")).unwrap_err();
        assert!(
            error.starts_with("parameter entity with its variables"),
            "{error}"
        );
        assert!(!error.contains('\u{200b}'));
        let error = fill_in_plugin_params(&given, &["entity".into()], |_| None).unwrap_err();
        assert_eq!(error, "parameter entity: variable room is not declared");
    }

    #[test]
    fn an_invoke_waits_for_its_own_answer() {
        let (sent_tx, sent_rx) = mpsc::channel::<Value>();
        let sent_tx = Mutex::new(sent_tx);
        let bridge = PluginBridge::new(Box::new(move |record| {
            sent_tx.lock().unwrap().send(record.clone()).unwrap();
            Ok(())
        }));
        let app = bridge.clone();
        let answering = thread::spawn(move || {
            let record = sent_rx.recv().unwrap();
            assert_eq!(record["type"], "plugin_invoke");
            assert_eq!(record["plugin"], "com.x.y");
            assert_eq!(record["params"], json!({"on": true}));
            let request = record["request"].as_u64().unwrap();
            assert!(app.complete(request + 100, Ok(())).is_err());
            app.complete(request, Err("light is offline".into()))
                .unwrap();
            // The second answer finds nobody waiting.
            assert!(app.complete(request, Ok(())).is_err());
        });
        let result = bridge.invoke(
            "com.x.y",
            "1.0.0",
            "toggle",
            &params(&[("on", PluginValue::Bool(true))]),
            Duration::from_secs(5),
            None,
        );
        answering.join().unwrap();
        assert_eq!(result, Err("light is offline".into()));
    }

    #[test]
    fn an_unanswered_invoke_fails_at_its_deadline_or_on_cancellation() {
        let bridge = PluginBridge::new(Box::new(|_| Ok(())));
        let started = Instant::now();
        let error = bridge
            .invoke(
                "com.x.y",
                "1.0.0",
                "a",
                &BTreeMap::new(),
                Duration::from_millis(120),
                None,
            )
            .unwrap_err();
        assert!(error.contains("did not answer"), "{error}");
        assert!(started.elapsed() >= Duration::from_millis(120));
        assert!(bridge.pending().is_empty());
        let cancelled = AtomicBool::new(true);
        let error = bridge
            .invoke(
                "com.x.y",
                "1.0.0",
                "a",
                &BTreeMap::new(),
                Duration::from_secs(5),
                Some(&cancelled),
            )
            .unwrap_err();
        assert!(error.contains("cancelled"), "{error}");
        let broken = PluginBridge::new(Box::new(|_| Err(io::Error::other("pipe closed"))));
        let error = broken
            .invoke(
                "com.x.y",
                "1.0.0",
                "a",
                &BTreeMap::new(),
                Duration::from_secs(5),
                None,
            )
            .unwrap_err();
        assert!(error.contains("pipe closed"), "{error}");
    }
}
