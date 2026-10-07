//! Report planning and event-record processing shared by replay and live runs.
//!
//! [`plan_report`] decodes one report and resolves each event through the
//! stateful [`ProfileSession`], without I/O. [`process_work`] writes the report
//! and event records, then dispatches planned actions. Replay calls both inline;
//! the live runtime plans on its input thread and processes on its worker.

use crate::{capture::encode_hex, live_runtime::LiveSummary};
use cooltek_sdk_service::{
    Action, ActionDispatcher, Binding, DeviceContext, InputEvent, InputSession, InputUpdate,
    ProfileResolution, ProfileSession, ServiceModule,
};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    io::Write,
    sync::atomic::{AtomicBool, Ordering},
};

/// Largest JSON record, including its newline, that a service log may contain.
pub const MAX_RECORD_BYTES: usize = 65_536;

/// `action_unavailable` reason for a `profile` action in a run without a
/// session manager (no `--profile-switch-requests`).
pub const PROFILE_SWITCH_UNAVAILABLE: &str =
    "profile switching needs the OpenStreamHub session manager; no action was taken";

/// The dial function that was current when an M131E event was routed.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ActiveDialFunction {
    /// Function name from the profile.
    pub name: String,
    /// Zero-based position in `dial_functions`.
    pub index: usize,
}

/// One normalized event with its selector and stateful resolution.
#[derive(Clone, Debug)]
pub struct PlannedEvent {
    /// Latest input represented by this entry.
    pub event: InputEvent,
    /// Flat selector, if any; schema-2 keypad routing and dial functions report `None`.
    pub binding: Option<Binding>,
    /// Action, keypad navigation, dial function switch, or unbound outcome.
    pub resolution: ProfileResolution,
    /// Function current while routing an M131E event of a profile with
    /// `dial_functions`; `None` for a switch, whose `navigation` names both sides.
    pub dial_function: Option<ActiveDialFunction>,
    /// Input events represented, greater than one only after live coalescing.
    pub coalesced: u64,
}

impl PlannedEvent {
    /// The concrete action to dispatch, if any. A dial function switch never has one.
    pub fn action(&self) -> Option<&Action> {
        match &self.resolution {
            ProfileResolution::Action { action, .. } => Some(action),
            _ => None,
        }
    }

    /// Whether this entry is a short-click dial function switch.
    pub fn is_dial_function_switch(&self) -> bool {
        matches!(self.resolution, ProfileResolution::DialFunction { .. })
    }
}

/// The diagnostic record and planned events for one or more input reports.
#[derive(Clone, Debug)]
pub struct Work {
    /// Latest raw report represented by this work item.
    pub raw: Vec<u8>,
    /// Timestamp of the latest report.
    pub timestamp_micros: u64,
    /// `decoded`, `unknown`, or `parse_error` for the latest report.
    pub status: &'static str,
    /// Parse error of the latest report.
    pub error: Option<String>,
    /// Planned events in input order (per control).
    pub events: Vec<PlannedEvent>,
    /// Reports represented, greater than one only after live coalescing.
    pub report_count: u64,
    /// Parse errors among earlier coalesced reports.
    pub coalesced_parse_error_count: u64,
    /// Events dropped while coalescing because the pending entry was full.
    pub dropped_event_count: u64,
    /// Dial function switches among `dropped_event_count`. The switch itself
    /// already took effect while planning; only its record is missing.
    pub dropped_dial_function_count: u64,
    /// Set by live coalescing when a switch after the last event was dropped,
    /// so the next event is never merged into that last event.
    pub merge_barrier: bool,
}

/// Decodes one report and resolves its events without hardware or host I/O.
///
/// Decoder errors become a `parse_error` work item; resolution errors are fatal.
pub fn plan_report(
    input: &mut InputSession,
    session: &mut ProfileSession,
    device: &DeviceContext,
    raw: &[u8],
    timestamp_micros: u64,
) -> Result<Work, String> {
    let (status, error, events) = match input.decode(raw, timestamp_micros) {
        Ok(InputUpdate::Events(events)) => ("decoded", None, events),
        Ok(InputUpdate::Unknown) => ("unknown", None, Vec::new()),
        Err(error) => ("parse_error", Some(error), Vec::new()),
    };
    let events = events
        .into_iter()
        .map(|event| {
            let binding = session.binding_for(device, &event).cloned();
            let current = active_dial_function(session, device);
            let resolution = session.route(device, &event)?;
            let dial_function = match resolution {
                ProfileResolution::DialFunction { .. } => None,
                _ => current,
            };
            Ok(PlannedEvent {
                event,
                binding,
                resolution,
                dial_function,
                coalesced: 1,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(Work {
        raw: raw.to_vec(),
        timestamp_micros,
        status,
        error,
        events,
        report_count: 1,
        coalesced_parse_error_count: 0,
        dropped_event_count: 0,
        dropped_dial_function_count: 0,
        merge_barrier: false,
    })
}

fn active_dial_function(
    session: &ProfileSession,
    device: &DeviceContext,
) -> Option<ActiveDialFunction> {
    if device.module != ServiceModule::M131E {
        return None;
    }
    Some(ActiveDialFunction {
        name: session.current_dial_function()?.name.clone(),
        index: session.dial_function_index()?,
    })
}

/// `service_start` details for active dial functions, or `None` when the
/// device is not the M131E dial or the profile has no `dial_functions`.
///
/// The current function lives only in the process: every restart, including
/// the desktop app's restarts on wake, reconnect, and profile switch, begins
/// again at the first function.
pub fn dial_function_details(session: &ProfileSession, device: &DeviceContext) -> Option<Value> {
    if device.module != ServiceModule::M131E {
        return None;
    }
    let functions = session.profile().dial_functions.as_ref()?;
    Some(json!({
        "count":functions.len(),
        "names":functions.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(),
        "initial_index":0,
        "short_press":"switches_function",
        "resets_on_restart":true,
    }))
}

/// The `service_start` record shared by replay and live sessions.
pub fn start_record(device: &DeviceContext, execute: bool, details: Value) -> Value {
    json!({
        "type":"service_start", "schema_version":1,
        "device":{"module":device.module,"serial_number":device.serial_number},
        "execution":execution(execute), "details":details,
    })
}

/// The `execution` label used by service records.
pub fn execution(execute: bool) -> &'static str {
    if execute { "macos" } else { "dry_run" }
}

/// The `service_end` record shared by replay and live sessions.
///
/// A present `error` always reports the reason as `error`.
pub fn end_record(
    reason: &str,
    error: Option<&str>,
    summary: &LiveSummary,
    execute: bool,
) -> Value {
    json!({
        "type":"service_end", "schema_version":1,
        "reason":if error.is_some() {"error"} else {reason}, "error":error,
        "report_count":summary.report_count, "queued_report_count":summary.queued_report_count,
        "processed_report_count":summary.processed_report_count,
        "coalesced_report_count":summary.coalesced_report_count,
        "discarded_report_count":summary.report_count.saturating_sub(summary.processed_report_count),
        "decoded_event_count":summary.decoded_event_count, "event_count":summary.event_count,
        "coalesced_event_count":summary.coalesced_event_count,
        "dropped_event_count":summary.dropped_event_count,
        "dial_function_switch_count":summary.dial_function_switch_count,
        "dropped_dial_function_count":summary.dropped_dial_function_count,
        "profile_request_count":summary.profile_request_count,
        "discarded_event_count":summary.decoded_event_count.saturating_sub(summary.event_count),
        "action_count":summary.action_count, "failed_action_count":summary.failed_action_count,
        "incomplete_action_count":summary.incomplete_action_count,
        "execution":execution(execute),
    })
}

/// Writes records for one work item and dispatches its actions in order.
///
/// Each intended action is logged before dispatch, so a failed log write never
/// causes an unrecorded action. A failed action is logged as `action_failed`;
/// it ends processing only when `stop_on_action_error` is set. Output errors
/// always end processing. A set `abort` flag drops remaining events.
///
/// A `profile` action never reaches the dispatcher. With
/// `profile_switch_requests` it is reported as `profile_switch_requested` for
/// the supervising session manager, which owns the saved profiles; without it
/// it is logged as `action_unavailable` and does nothing. Neither outcome is a
/// failure, so it never ends the session.
#[allow(clippy::too_many_arguments)]
pub fn process_work<D: ActionDispatcher, W: Write>(
    work: Work,
    dispatcher: &mut D,
    output: &mut W,
    abort: &AtomicBool,
    execute: bool,
    stop_on_action_error: bool,
    profile_switch_requests: bool,
    summary: &mut LiveSummary,
) -> Result<(), String> {
    let mut report = json!({
        "type":"report", "schema_version":1, "timestamp_micros":work.timestamp_micros,
        "raw_hex":encode_hex(&work.raw), "status":work.status, "error":work.error,
    });
    if work.report_count > 1 {
        report["coalesced_report_count"] = json!(work.report_count);
        report["coalesced_parse_error_count"] = json!(work.coalesced_parse_error_count);
    }
    if work.dropped_event_count > 0 {
        report["dropped_event_count"] = json!(work.dropped_event_count);
    }
    if work.dropped_dial_function_count > 0 {
        report["dropped_dial_function_count"] = json!(work.dropped_dial_function_count);
    }
    emit(output, report)?;
    summary.processed_report_count += work.report_count;
    for planned in work.events {
        if abort.load(Ordering::SeqCst) {
            break;
        }
        let (action, navigation, page_id, status) = match &planned.resolution {
            ProfileResolution::Action {
                action, page_id, ..
            } => (Some(action), Value::Null, page_id.as_ref(), "planned"),
            ProfileResolution::Navigation { navigation } => {
                (None, json!(navigation), None, "navigation")
            }
            // A switch is a state change only and is never dispatched.
            ProfileResolution::DialFunction { navigation } => {
                (None, json!(navigation), None, "dial_function")
            }
            ProfileResolution::Unbound => (None, Value::Null, None, "unbound"),
        };
        let mut record = json!({
            "type":"event", "schema_version":1, "event":planned.event,
            "binding":loggable_binding(&planned.binding), "action":action.map(loggable), "navigation":navigation, "page_id":page_id,
            "status":status,
        });
        if let Some(function) = &planned.dial_function {
            record["dial_function"] = json!(function);
        }
        if planned.coalesced > 1 {
            record["coalesced_event_count"] = json!(planned.coalesced);
        }
        emit(output, record)?;
        summary.event_count += planned.coalesced;
        if planned.is_dial_function_switch() {
            summary.dial_function_switch_count += planned.coalesced;
        }
        let Some(action) = action else {
            continue;
        };
        if abort.load(Ordering::SeqCst) {
            break;
        }
        if let Action::Profile { direction } = action {
            summary.profile_request_count += planned.coalesced;
            let record = if profile_switch_requests {
                json!({"type":"profile_switch_requested", "schema_version":1, "event":planned.event, "direction":direction})
            } else {
                json!({"type":"action_unavailable", "schema_version":1, "event":planned.event, "action":loggable(action), "reason":PROFILE_SWITCH_UNAVAILABLE})
            };
            emit(output, record)?;
            continue;
        }
        if let Err(error) = dispatcher.dispatch(action) {
            summary.failed_action_count += planned.coalesced;
            let mut failed = json!({"type":"action_failed", "schema_version":1, "event":planned.event, "action":loggable(action), "error":error});
            if let Some(function) = &planned.dial_function {
                failed["dial_function"] = json!(function);
            }
            emit(output, failed)?;
            if stop_on_action_error {
                return Err(format!("action failed: {error}"));
            }
            continue;
        }
        summary.action_count += planned.coalesced;
        if execute {
            let mut executed = json!({"type":"action_executed", "schema_version":1, "event":planned.event, "action":loggable(action)});
            if let Some(function) = &planned.dial_function {
                executed["dial_function"] = json!(function);
            }
            emit(output, executed)?;
        }
    }
    Ok(())
}

/// Redacts an action for a log record: snippet text and URL paths, queries and
/// user information (often passwords or tokens) are never written. Every other
/// action serializes unchanged.
pub fn loggable(action: &Action) -> Value {
    match action {
        Action::Text { text, press_enter } => {
            json!({"type":"text", "chars":text.chars().count(), "press_enter":press_enter})
        }
        Action::OpenUrl { url } => json!({"type":"open_url", "origin":url_origin(url)}),
        Action::MultiAction { actions } => {
            json!({"type":"multi_action", "actions":actions.iter().map(loggable).collect::<Vec<_>>()})
        }
        other => json!(other),
    }
}

/// `scheme://host[:port]` of a URL, without user information, path, query or fragment.
fn url_origin(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return "unrecognized".into();
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let authority = authority.rsplit('@').next().unwrap_or("");
    format!("{scheme}://{authority}")
}

/// Redacts the action inside a keypad or dial binding for a log record.
fn loggable_binding(binding: &Option<Binding>) -> Value {
    match binding {
        None => Value::Null,
        Some(binding) => {
            let mut value = json!(binding);
            value["action"] = loggable(&binding.action);
            value
        }
    }
}

/// Writes one bounded JSON line and flushes it as a complete record.
pub fn emit(output: &mut impl Write, value: Value) -> Result<(), String> {
    let mut bytes = serde_json::to_vec(&value).map_err(|error| error.to_string())?;
    bytes.push(b'\n');
    if bytes.len() > MAX_RECORD_BYTES {
        return Err(format!(
            "service log record exceeds {MAX_RECORD_BYTES} bytes"
        ));
    }
    output
        .write_all(&bytes)
        .map_err(|error| format!("service log write failed: {error}"))?;
    output
        .flush()
        .map_err(|error| format!("service log flush failed: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use cooltek_sdk_service::{Gesture, Modifier, ProfileDirection};

    /// Fails the test if a `profile` action ever reaches the host boundary.
    struct NoHost;

    impl ActionDispatcher for NoHost {
        fn dispatch(&mut self, action: &Action) -> Result<(), String> {
            panic!("{} reached the dispatcher", action.type_name())
        }
    }

    fn profile_work(direction: ProfileDirection) -> Work {
        Work {
            raw: vec![1],
            timestamp_micros: 7,
            status: "decoded",
            error: None,
            events: vec![PlannedEvent {
                event: InputEvent {
                    timestamp_micros: 7,
                    control: 3,
                    gesture: Gesture::Pressed,
                    value: None,
                },
                binding: None,
                resolution: ProfileResolution::Action {
                    action: Action::Profile { direction },
                    page_id: Some("root".into()),
                    dial_function: None,
                },
                dial_function: None,
                coalesced: 1,
            }],
            report_count: 1,
            coalesced_parse_error_count: 0,
            dropped_event_count: 0,
            dropped_dial_function_count: 0,
            merge_barrier: false,
        }
    }

    fn records(output: &[u8]) -> Vec<Value> {
        output
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_slice(line).unwrap())
            .collect()
    }

    #[test]
    fn profile_actions_are_requests_or_unavailable_and_never_dispatched() {
        for execute in [false, true] {
            let mut output = Vec::new();
            let mut summary = LiveSummary::default();
            let never = AtomicBool::new(false);
            process_work(
                profile_work(ProfileDirection::Previous),
                &mut NoHost,
                &mut output,
                &never,
                execute,
                true,
                true,
                &mut summary,
            )
            .unwrap();
            let records = records(&output);
            assert_eq!(records[1]["status"], "planned");
            assert_eq!(
                records[1]["action"],
                json!({"type":"profile","direction":"previous"})
            );
            assert_eq!(
                records[2],
                json!({"type":"profile_switch_requested","schema_version":1,
                    "event":{"timestamp_micros":7,"control":3,"gesture":"pressed"},
                    "direction":"previous"})
            );
            assert_eq!(records.len(), 3, "no action_executed record");
            assert_eq!(summary.profile_request_count, 1);
            assert_eq!(summary.action_count, 0);
            assert_eq!(summary.failed_action_count, 0);
        }

        // Standalone: a validated no-op, never an error, even with
        // stop_on_action_error.
        let mut output = Vec::new();
        let mut summary = LiveSummary::default();
        process_work(
            profile_work(ProfileDirection::Next),
            &mut NoHost,
            &mut output,
            &AtomicBool::new(false),
            true,
            true,
            false,
            &mut summary,
        )
        .unwrap();
        let records = records(&output);
        assert_eq!(records.len(), 3);
        assert_eq!(records[2]["type"], "action_unavailable");
        assert_eq!(
            records[2]["action"],
            json!({"type":"profile","direction":"next"})
        );
        assert_eq!(records[2]["reason"], PROFILE_SWITCH_UNAVAILABLE);
        assert_eq!(summary.profile_request_count, 1);
        assert_eq!(summary.failed_action_count, 0);
        let end = end_record("max_reports", None, &summary, true);
        assert_eq!(end["profile_request_count"], 1);
    }

    #[test]
    fn loggable_redacts_text_and_url_but_keeps_other_actions_unchanged() {
        let text = Action::Text {
            text: "hunter2 pässword".into(),
            press_enter: true,
        };
        assert_eq!(
            loggable(&text),
            json!({"type":"text", "chars":16, "press_enter":true})
        );
        let url = Action::OpenUrl {
            url: "https://user:pw@Example.com:8443/reset?token=s3cr3t#frag".into(),
        };
        assert_eq!(
            loggable(&url),
            json!({"type":"open_url", "origin":"https://Example.com:8443"})
        );
        let nested = Action::MultiAction {
            actions: vec![text.clone(), Action::ToggleMute, url],
        };
        let rendered = loggable(&nested).to_string();
        assert!(!rendered.contains("hunter2") && !rendered.contains("s3cr3t"));
        assert!(!rendered.contains("pw@") && !rendered.contains("reset"));
        assert_eq!(
            loggable(&nested)["actions"][1],
            json!({"type":"toggle_mute"})
        );
        for plain in [
            Action::Volume { delta: -5 },
            Action::ToggleMute,
            Action::Shortcut {
                key: "c".into(),
                modifiers: vec![Modifier::Command],
            },
            Action::LaunchApp {
                bundle_id: "com.apple.finder".into(),
            },
        ] {
            assert_eq!(loggable(&plain), json!(plain));
        }
    }
}
