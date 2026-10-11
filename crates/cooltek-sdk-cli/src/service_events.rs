//! Report planning and event-record processing shared by replay and live runs.
//!
//! [`plan_report`] decodes one report and resolves each event through the
//! stateful [`ProfileSession`], without I/O. [`process_work`] writes the report
//! and event records, then dispatches planned actions. Replay calls both inline;
//! the live runtime plans on its input thread and processes on its worker.

use crate::{capture::encode_hex, live_runtime::LiveSummary};
use cooltek_sdk_service::{
    Action, ActionDispatcher, Binding, DeviceContext, InputEvent, InputSession, InputUpdate,
    ProfileResolution, ProfileSession, RunningTimer, ServiceModule, TimerAlert, TimerKey,
    TimerTable, TimerToggle,
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

/// `action_failed` error for a `timer` press while [`TimerTable::MAX_RUNNING`]
/// countdowns already run.
pub const TIMER_LIMIT_REACHED: &str =
    "at most 8 timers can run at once; this timer was not started";

/// Why running timers were cancelled without a press.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TimerCancelReason {
    /// The input ended normally (a duration or report limit, or the end of a capture).
    SessionEnd,
    /// The session was interrupted or stopped (Ctrl-C, SIGTERM, Stop, sleep, an error).
    Stopped,
}

impl TimerCancelReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::SessionEnd => "session_end",
            Self::Stopped => "stopped",
        }
    }
}

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
    let mut record = json!({
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
        "unavailable_action_count":summary.unavailable_action_count,
        "discarded_event_count":summary.decoded_event_count.saturating_sub(summary.event_count),
        "action_count":summary.action_count, "failed_action_count":summary.failed_action_count,
        "incomplete_action_count":summary.incomplete_action_count,
        "execution":execution(execute),
    });
    // Only when a timer started, so sessions without timers log what they did before.
    if summary.timer_started_count > 0 {
        record["timer_started_count"] = json!(summary.timer_started_count);
        record["timer_cancelled_count"] = json!(summary.timer_cancelled_count);
        record["timer_expired_count"] = json!(summary.timer_expired_count);
        record["timer_alert_failed_count"] = json!(summary.timer_alert_failed_count);
        record["timer_alert_warning_count"] = json!(summary.timer_alert_warning_count);
    }
    record
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
///
/// A `timer` action never reaches the dispatcher either: it starts a countdown
/// in `timers` at `now_micros`, or cancels the countdown its binding already
/// runs, and is logged as `timer_started` or `timer_cancelled`. A press while
/// [`TimerTable::MAX_RUNNING`] countdowns run is an `action_failed`.
///
/// An action the dispatcher reports as [`ActionDispatcher::unavailable`] (an
/// `obs_scene` action, or a `multi_action` with one, in a run without an OBS
/// endpoint) is also logged as `action_unavailable` with that reason and never
/// dispatched: no step of such a `multi_action` runs.
#[allow(clippy::too_many_arguments)]
pub fn process_work<D: ActionDispatcher, W: Write>(
    work: Work,
    dispatcher: &mut D,
    output: &mut W,
    abort: &AtomicBool,
    execute: bool,
    stop_on_action_error: bool,
    profile_switch_requests: bool,
    timers: &mut TimerTable,
    now_micros: u64,
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
        if let Action::Timer { .. } = action {
            let key = timer_key(&planned);
            let record = match timers.toggle(key.clone(), action, now_micros) {
                Ok(TimerToggle::Started { running, .. }) => {
                    summary.timer_started_count += 1;
                    json!({"type":"timer_started", "schema_version":1, "event":planned.event, "timer":key, "action":loggable(action), "running_count":running})
                }
                Ok(TimerToggle::Cancelled(_)) => {
                    summary.timer_cancelled_count += 1;
                    json!({"type":"timer_cancelled", "schema_version":1, "event":planned.event, "timer":key, "action":loggable(action), "reason":"pressed_again"})
                }
                outcome @ (Ok(TimerToggle::Full) | Err(_)) => {
                    let error = outcome.err().unwrap_or_else(|| TIMER_LIMIT_REACHED.into());
                    summary.failed_action_count += planned.coalesced;
                    emit(
                        output,
                        json!({"type":"action_failed", "schema_version":1, "event":planned.event, "action":loggable(action), "error":error}),
                    )?;
                    if stop_on_action_error {
                        return Err(format!("action failed: {error}"));
                    }
                    continue;
                }
            };
            summary.action_count += planned.coalesced;
            emit(output, record)?;
            continue;
        }
        if let Some(reason) = dispatcher.unavailable(action) {
            summary.unavailable_action_count += planned.coalesced;
            let mut record = json!({"type":"action_unavailable", "schema_version":1, "event":planned.event, "action":loggable(action), "reason":reason});
            if let Some(function) = &planned.dial_function {
                record["dial_function"] = json!(function);
            }
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
/// user information (often passwords or tokens), timer labels and OBS scene
/// names are never written. Every other action serializes unchanged.
pub fn loggable(action: &Action) -> Value {
    match action {
        Action::Text { text, press_enter } => {
            json!({"type":"text", "chars":text.chars().count(), "press_enter":press_enter})
        }
        Action::OpenUrl { url } => json!({"type":"open_url", "origin":url_origin(url)}),
        Action::MultiAction { actions } => {
            json!({"type":"multi_action", "actions":actions.iter().map(loggable).collect::<Vec<_>>()})
        }
        // A macro's step list is the keys it types, so like a text snippet
        // only its shape is logged: never the keys themselves.
        Action::Macro { steps } => {
            let ms: u64 = steps
                .iter()
                .map(|step| u64::from(step.delay_before_ms) + u64::from(step.hold_ms))
                .sum();
            json!({"type":"macro", "steps":steps.len(), "ms":ms})
        }
        // The label is user text shown in a notification; only its length is logged.
        Action::Timer {
            seconds,
            label,
            sound,
        } => {
            let mut value = json!({"type":"timer", "seconds":seconds});
            if let Some(label) = label {
                value["label_chars"] = json!(label.chars().count());
            }
            if let Some(sound) = sound {
                value["sound"] = json!(sound);
            }
            value
        }
        // A scene name is user text too (it can name a private call); only its length is logged.
        Action::ObsScene { scene } => {
            json!({"type":"obs_scene", "scene_chars":scene.chars().count()})
        }
        // Source, filter and scene names are user text as well: only lengths are logged.
        Action::ObsFilter { source, filter, op } => json!({
            "type":"obs_filter",
            "source_chars":source.chars().count(),
            "filter_chars":filter.chars().count(),
            "op":op,
        }),
        Action::ObsSourceVisibility { scene, source, op } => json!({
            "type":"obs_source_visibility",
            "scene_chars":scene.chars().count(),
            "source_chars":source.chars().count(),
            "op":op,
        }),
        Action::ObsSceneCollection { name } => {
            json!({"type":"obs_scene_collection", "name_chars":name.chars().count()})
        }
        Action::ObsProfile { name } => {
            json!({"type":"obs_profile", "name_chars":name.chars().count()})
        }
        Action::ObsSetInputVolume { input, percent } => json!({
            "type":"obs_set_input_volume",
            "input_chars":input.chars().count(),
            "percent":percent,
        }),
        Action::ObsInputVolumeFromPosition { input } => json!({
            "type":"obs_input_volume_from_position",
            "input_chars":input.chars().count(),
        }),
        other => json!(other),
    }
}

/// The binding that owns a `timer` press: control, gesture, keypad page and dial function.
fn timer_key(planned: &PlannedEvent) -> TimerKey {
    let (page_id, dial_function) = match &planned.resolution {
        ProfileResolution::Action {
            page_id,
            dial_function,
            ..
        } => (page_id.clone(), dial_function.clone()),
        _ => (None, None),
    };
    TimerKey {
        control: planned.event.control,
        gesture: planned.event.gesture,
        page_id,
        dial_function,
    }
}

/// Ends every countdown due at `now_micros`, earliest first: logs
/// `timer_expired`, then hands the alert to `alert` (inline through the
/// dispatcher, or to a live session's alert thread). Nothing expires once
/// `abort` is set. Countdowns are taken one at a time: if writing a record
/// fails, that countdown is put back and the rest stay in the table, so the
/// session's final cancellation still reports each of them.
pub fn process_due_timers<W: Write>(
    timers: &mut TimerTable,
    now_micros: u64,
    output: &mut W,
    abort: &AtomicBool,
    summary: &mut LiveSummary,
    mut alert: impl FnMut(&RunningTimer, TimerAlert, &mut W, &mut LiveSummary) -> Result<(), String>,
) -> Result<(), String> {
    while !abort.load(Ordering::SeqCst) {
        let Some(timer) = timers.pop_due(now_micros) else {
            break;
        };
        if let Err(error) = emit(
            output,
            json!({"type":"timer_expired", "schema_version":1, "timer":timer.key, "action":loggable(&timer.action)}),
        ) {
            timers.restore(timer);
            return Err(error);
        }
        summary.timer_expired_count += 1;
        if let Some(details) = TimerAlert::from_action(&timer.action) {
            alert(&timer, details, output, summary)?;
        }
    }
    Ok(())
}

/// Runs an alert inline through `dispatcher` and logs its outcome. Used by
/// replay and by dispatchers without a separate alerter (dry runs, tests).
pub fn inline_alert<D: ActionDispatcher, W: Write>(
    dispatcher: &mut D,
) -> impl FnMut(&RunningTimer, TimerAlert, &mut W, &mut LiveSummary) -> Result<(), String> + '_ {
    move |timer, alert, output, summary| {
        let outcome = dispatcher.timer_alert(&alert);
        log_alert_outcome(output, &timer.key, &timer.action, outcome, summary)
    }
}

/// Logs one finished alert: nothing on success, `timer_alert_warning` when
/// the tools succeeded but reported a problem (such as a notification macOS
/// refused), or `timer_alert_failed`. Neither ends the session.
pub fn log_alert_outcome<W: Write>(
    output: &mut W,
    key: &TimerKey,
    action: &Action,
    outcome: Result<Option<String>, String>,
    summary: &mut LiveSummary,
) -> Result<(), String> {
    match outcome {
        Ok(None) => Ok(()),
        Ok(Some(warning)) => {
            summary.timer_alert_warning_count += 1;
            emit(
                output,
                json!({"type":"timer_alert_warning", "schema_version":1, "timer":key, "action":loggable(action), "warning":warning}),
            )
        }
        Err(error) => {
            summary.timer_alert_failed_count += 1;
            emit(
                output,
                json!({"type":"timer_alert_failed", "schema_version":1, "timer":key, "action":loggable(action), "error":error}),
            )
        }
    }
}

/// Cancels every running countdown when a session ends, logging each as
/// `timer_cancelled` with `reason`. Timers never outlive the process.
pub fn cancel_timers<W: Write>(
    timers: &mut TimerTable,
    reason: TimerCancelReason,
    output: &mut W,
    summary: &mut LiveSummary,
) -> Result<(), String> {
    for timer in timers.cancel_all() {
        summary.timer_cancelled_count += 1;
        emit(
            output,
            json!({"type":"timer_cancelled", "schema_version":1, "event":Value::Null, "timer":timer.key, "action":loggable(&timer.action), "reason":reason.as_str()}),
        )?;
    }
    Ok(())
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

/// Prefix of a failed or timed-out record write (for example `service_start`
/// to a reader that stopped reading). OpenStreamHub treats it as transient.
pub const LOG_WRITE_FAILED: &str = "service log write failed";

/// Prefix of a failed flush of a record. OpenStreamHub treats it as transient.
pub const LOG_FLUSH_FAILED: &str = "service log flush failed";

/// Why a live run refuses a schema-2 keypad profile on an M4315 without
/// `--display-sync`. OpenStreamHub matches its start ("nested keypad profiles
/// run live only with") and shows its own wording.
pub const NESTED_KEYPAD_REFUSAL: &str = "nested keypad profiles run live only with --display-sync (which requires --software-mode); in OpenStreamHub, set Module control to App in Settings";

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
        .map_err(|error| format!("{LOG_WRITE_FAILED}: {error}"))?;
    output
        .flush()
        .map_err(|error| format!("{LOG_FLUSH_FAILED}: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use cooltek_sdk_service::{Gesture, MacroStep, Modifier, ProfileDirection};

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
                &mut TimerTable::default(),
                0,
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
            &mut TimerTable::default(),
            0,
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

    /// Records timer alerts; any `dispatch` call fails the test, because a
    /// timer press must never reach the host.
    #[derive(Default)]
    struct Alerts {
        alerts: Vec<TimerAlert>,
        fail: bool,
        warning: Option<String>,
    }

    impl ActionDispatcher for Alerts {
        fn dispatch(&mut self, action: &Action) -> Result<(), String> {
            panic!("{} reached the dispatcher", action.type_name())
        }

        fn timer_alert(&mut self, alert: &TimerAlert) -> Result<Option<String>, String> {
            self.alerts.push(alert.clone());
            if self.fail {
                Err("timer notification failed: synthetic timeout".into())
            } else {
                Ok(self.warning.clone())
            }
        }
    }

    const HOSTILE: &str = "\" & do shell script \"id\" `id` $(id) secret-label";

    fn tea(seconds: u32) -> Action {
        Action::Timer {
            seconds,
            label: Some(HOSTILE.into()),
            sound: Some(cooltek_sdk_service::TimerSound::Ping),
        }
    }

    fn press(control: u8, micros: u64, action: Action) -> Work {
        let mut work = profile_work(ProfileDirection::Next);
        work.timestamp_micros = micros;
        work.events[0].event.control = control;
        work.events[0].event.timestamp_micros = micros;
        work.events[0].resolution = ProfileResolution::Action {
            action,
            page_id: Some("root".into()),
            dial_function: None,
        };
        work
    }

    struct Session {
        timers: TimerTable,
        output: Vec<u8>,
        summary: LiveSummary,
        host: Alerts,
        abort: AtomicBool,
    }

    impl Session {
        fn new() -> Self {
            Self {
                timers: TimerTable::default(),
                output: Vec::new(),
                summary: LiveSummary::default(),
                host: Alerts::default(),
                abort: AtomicBool::new(false),
            }
        }

        fn press(
            &mut self,
            control: u8,
            micros: u64,
            action: Action,
            stop: bool,
        ) -> Result<(), String> {
            process_work(
                press(control, micros, action),
                &mut self.host,
                &mut self.output,
                &self.abort,
                true,
                stop,
                false,
                &mut self.timers,
                micros,
                &mut self.summary,
            )
        }

        fn tick(&mut self, micros: u64) {
            process_due_timers(
                &mut self.timers,
                micros,
                &mut self.output,
                &self.abort,
                &mut self.summary,
                inline_alert(&mut self.host),
            )
            .unwrap();
        }

        /// Timer records only, without report and event records.
        fn timer_records(&self) -> Vec<Value> {
            records(&self.output)
                .into_iter()
                .filter(|record| !matches!(record["type"].as_str(), Some("report" | "event")))
                .collect()
        }
    }

    #[test]
    fn a_fake_clock_drives_a_timer_through_start_cancel_and_expiry() {
        let mut session = Session::new();
        session.press(2, 0, tea(5), true).unwrap();
        session.tick(4_999_999);
        session.press(2, 1_000_000, tea(5), true).unwrap();
        // Cancelled: nothing expires at the old deadline.
        session.tick(5_000_000);
        session.press(2, 2_000_000, tea(5), true).unwrap();
        session.tick(6_999_999);
        assert!(session.host.alerts.is_empty());
        session.tick(7_000_000);
        session.tick(60_000_000);
        let timer = json!({"control":2,"gesture":"pressed","page_id":"root","dial_function":null});
        let action = json!({"type":"timer","seconds":5,"label_chars":HOSTILE.chars().count(),"sound":"ping"});
        let event =
            |micros: u64| json!({"timestamp_micros":micros,"control":2,"gesture":"pressed"});
        assert_eq!(
            session.timer_records(),
            [
                json!({"type":"timer_started","schema_version":1,"event":event(0),"timer":timer,"action":action,"running_count":1}),
                json!({"type":"timer_cancelled","schema_version":1,"event":event(1_000_000),"timer":timer,"action":action,"reason":"pressed_again"}),
                json!({"type":"timer_started","schema_version":1,"event":event(2_000_000),"timer":timer,"action":action,"running_count":1}),
                json!({"type":"timer_expired","schema_version":1,"timer":timer,"action":action}),
            ]
        );
        assert_eq!(session.host.alerts.len(), 1, "one alert, never repeated");
        assert_eq!(session.host.alerts[0].label.as_deref(), Some(HOSTILE));
        assert_eq!(session.summary.action_count, 3);
        assert_eq!(session.summary.timer_started_count, 2);
        assert_eq!(session.summary.timer_cancelled_count, 1);
        assert_eq!(session.summary.timer_expired_count, 1);
        // The label is never logged, in any record.
        let log = String::from_utf8(session.output.clone()).unwrap();
        assert!(!log.contains("secret-label") && !log.contains("shell script"));
        let end = end_record("max_reports", None, &session.summary, true);
        assert_eq!(end["timer_started_count"], 2);
        assert_eq!(end["timer_expired_count"], 1);
        assert_eq!(end["timer_alert_failed_count"], 0);
        let quiet = end_record("max_reports", None, &LiveSummary::default(), true);
        assert!(
            quiet.get("timer_started_count").is_none(),
            "unchanged without timers"
        );
    }

    #[test]
    fn a_ninth_timer_fails_without_touching_the_running_eight() {
        let mut session = Session::new();
        for control in 0..8 {
            session.press(control, 0, tea(60), false).unwrap();
        }
        session.press(8, 0, tea(60), false).unwrap();
        let failed = records(&session.output).pop().unwrap();
        assert_eq!(failed["type"], "action_failed");
        assert_eq!(failed["error"], TIMER_LIMIT_REACHED);
        assert_eq!(session.timers.len(), 8);
        assert_eq!(session.summary.failed_action_count, 1);
        // A running timer can still be cancelled while the table is full.
        session.press(0, 1, tea(60), false).unwrap();
        assert_eq!(session.timers.len(), 7);
        for control in 0..8 {
            session.press(10 + control, 2, tea(60), false).ok();
        }
        assert!(
            session
                .press(9, 3, tea(60), true)
                .unwrap_err()
                .contains(TIMER_LIMIT_REACHED)
        );
    }

    #[test]
    fn stopping_cancels_every_timer_and_nothing_expires_after_abort() {
        let mut session = Session::new();
        session.press(0, 0, tea(1), true).unwrap();
        session.press(1, 0, tea(2), true).unwrap();
        session.abort.store(true, Ordering::SeqCst);
        session.tick(10_000_000);
        assert!(session.host.alerts.is_empty());
        assert_eq!(session.timers.len(), 2, "kept for the final cancellation");
        cancel_timers(
            &mut session.timers,
            TimerCancelReason::Stopped,
            &mut session.output,
            &mut session.summary,
        )
        .unwrap();
        assert!(session.timers.is_empty());
        let cancelled: Vec<_> = session
            .timer_records()
            .into_iter()
            .filter(|record| record["type"] == "timer_cancelled")
            .collect();
        assert_eq!(cancelled.len(), 2);
        assert!(
            cancelled
                .iter()
                .all(|record| record["reason"] == "stopped" && record["event"].is_null())
        );
        assert_eq!(session.summary.timer_expired_count, 0);
    }

    #[test]
    fn a_failed_alert_is_logged_and_never_ends_the_session() {
        let mut session = Session::new();
        session.host.fail = true;
        session.press(0, 0, tea(1), true).unwrap();
        session.tick(1_000_000);
        let records = session.timer_records();
        assert_eq!(records[1]["type"], "timer_expired");
        assert_eq!(records[2]["type"], "timer_alert_failed");
        assert_eq!(
            records[2]["error"],
            "timer notification failed: synthetic timeout"
        );
        assert_eq!(session.summary.timer_alert_failed_count, 1);
        assert_eq!(session.summary.failed_action_count, 0);
    }

    #[test]
    fn a_notification_warning_is_a_label_free_record_and_not_a_failure() {
        let mut session = Session::new();
        session.host.warning =
            Some("timer notification may not have been shown: connection invalid".into());
        session.press(0, 0, tea(1), true).unwrap();
        session.tick(1_000_000);
        let records = session.timer_records();
        assert_eq!(records.len(), 3);
        assert_eq!(records[2]["type"], "timer_alert_warning");
        assert_eq!(
            records[2]["warning"],
            "timer notification may not have been shown: connection invalid"
        );
        assert_eq!(records[2]["action"]["label_chars"], HOSTILE.chars().count());
        assert!(!records[2].to_string().contains("secret-label"));
        assert_eq!(session.summary.timer_alert_warning_count, 1);
        assert_eq!(session.summary.timer_alert_failed_count, 0);
        let end = end_record("max_reports", None, &session.summary, true);
        assert_eq!(end["timer_alert_warning_count"], 1);
    }

    #[test]
    fn a_failed_expiry_record_keeps_every_unprocessed_timer_in_the_table() {
        /// Accepts `budget` writes, then fails every write.
        struct Failing {
            budget: usize,
        }
        impl Write for Failing {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                if self.budget == 0 {
                    return Err(std::io::Error::other("synthetic log failure"));
                }
                self.budget -= 1;
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut timers = TimerTable::default();
        for control in 0..3 {
            let key = TimerKey {
                control,
                gesture: cooltek_sdk_service::Gesture::Pressed,
                page_id: None,
                dial_function: None,
            };
            timers.toggle(key, &tea(1 + u32::from(control)), 0).unwrap();
        }
        let mut summary = LiveSummary::default();
        let mut host = Alerts::default();
        // The first expiry record is written, the second fails.
        let error = process_due_timers(
            &mut timers,
            10_000_000,
            &mut Failing { budget: 1 },
            &AtomicBool::new(false),
            &mut summary,
            inline_alert(&mut host),
        )
        .unwrap_err();
        assert!(error.contains("synthetic log failure"));
        assert_eq!(summary.timer_expired_count, 1);
        assert_eq!(host.alerts.len(), 1);
        // The failed one and the one after it are still running, so the
        // session's final cancellation counts and reports them.
        assert_eq!(timers.len(), 2);
        let mut output = Vec::new();
        cancel_timers(
            &mut timers,
            TimerCancelReason::Stopped,
            &mut output,
            &mut summary,
        )
        .unwrap();
        assert_eq!(summary.timer_cancelled_count, 2);
        assert_eq!(records(&output).len(), 2);
    }

    #[test]
    fn invalid_timer_actions_fail_without_starting() {
        let mut session = Session::new();
        session
            .press(
                0,
                0,
                Action::Timer {
                    seconds: 0,
                    label: None,
                    sound: None,
                },
                false,
            )
            .unwrap();
        assert!(session.timers.is_empty());
        assert_eq!(
            records(&session.output).pop().unwrap()["type"],
            "action_failed"
        );
    }

    /// A dispatcher without OBS: `obs_scene` is unavailable; nothing may run.
    struct NoObs;

    impl ActionDispatcher for NoObs {
        fn dispatch(&mut self, action: &Action) -> Result<(), String> {
            panic!("{} reached the dispatcher", action.type_name())
        }

        fn unavailable(&self, action: &Action) -> Option<&'static str> {
            action
                .uses_obs()
                .then_some(cooltek_sdk_service::obs::OBS_UNAVAILABLE)
        }
    }

    fn action_work(action: Action) -> Work {
        let mut work = profile_work(ProfileDirection::Next);
        work.events[0].resolution = ProfileResolution::Action {
            action,
            page_id: None,
            dial_function: None,
        };
        work
    }

    #[test]
    fn obs_actions_without_an_endpoint_are_unavailable_never_failures() {
        let scene = Action::ObsScene {
            scene: "Live".into(),
        };
        let multi = Action::MultiAction {
            actions: vec![Action::ToggleMute, scene.clone()],
        };
        for (action, logged) in [
            (scene, json!({"type":"obs_scene","scene_chars":4})),
            (
                Action::ObsVirtualCamera {
                    op: cooltek_sdk_service::ObsOutputOp::Toggle,
                },
                json!({"type":"obs_virtual_camera","op":"toggle"}),
            ),
            (
                Action::ObsReplayBuffer {
                    op: cooltek_sdk_service::ObsReplayBufferOp::Save,
                },
                json!({"type":"obs_replay_buffer","op":"save"}),
            ),
            (
                Action::ObsStudioMode {
                    op: cooltek_sdk_service::ObsSwitchOp::Toggle,
                },
                json!({"type":"obs_studio_mode","op":"toggle"}),
            ),
            (Action::ObsTransition, json!({"type":"obs_transition"})),
            (
                Action::ObsFilter {
                    source: "Private cam".into(),
                    filter: "Blur".into(),
                    op: cooltek_sdk_service::ObsSwitchOp::Off,
                },
                json!({"type":"obs_filter","source_chars":11,"filter_chars":4,"op":"off"}),
            ),
            (
                Action::ObsSourceVisibility {
                    scene: "Live".into(),
                    source: "Private cam".into(),
                    op: cooltek_sdk_service::ObsSwitchOp::On,
                },
                json!({"type":"obs_source_visibility","scene_chars":4,"source_chars":11,"op":"on"}),
            ),
            (
                Action::ObsSceneCollection {
                    name: "Private show".into(),
                },
                json!({"type":"obs_scene_collection","name_chars":12}),
            ),
            (
                Action::ObsProfile {
                    name: "Work".into(),
                },
                json!({"type":"obs_profile","name_chars":4}),
            ),
            (
                Action::ObsSetInputVolume {
                    input: "Mic".into(),
                    percent: 40,
                },
                json!({"type":"obs_set_input_volume","input_chars":3,"percent":40}),
            ),
            (
                Action::ObsInputVolumeFromPosition {
                    input: "Mic".into(),
                },
                json!({"type":"obs_input_volume_from_position","input_chars":3}),
            ),
            (
                multi,
                json!({"type":"multi_action","actions":[{"type":"toggle_mute"},{"type":"obs_scene","scene_chars":4}]}),
            ),
        ] {
            for execute in [false, true] {
                let mut output = Vec::new();
                let mut summary = LiveSummary::default();
                // stop_on_action_error is set: unavailable never ends the session.
                process_work(
                    action_work(action.clone()),
                    &mut NoObs,
                    &mut output,
                    &AtomicBool::new(false),
                    execute,
                    true,
                    true,
                    &mut TimerTable::default(),
                    0,
                    &mut summary,
                )
                .unwrap();
                let records = records(&output);
                assert_eq!(records.len(), 3, "{records:?}");
                assert_eq!(records[1]["status"], "planned");
                assert_eq!(
                    records[2],
                    json!({"type":"action_unavailable","schema_version":1,
                        "event":{"timestamp_micros":7,"control":3,"gesture":"pressed"},
                        "action":logged,
                        "reason":cooltek_sdk_service::obs::OBS_UNAVAILABLE})
                );
                assert_eq!(summary.unavailable_action_count, 1);
                assert_eq!(summary.action_count, 0);
                assert_eq!(summary.failed_action_count, 0);
                let end = end_record("max_reports", None, &summary, execute);
                assert_eq!(end["unavailable_action_count"], 1);
                assert_eq!(end["failed_action_count"], 0);
            }
        }
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
        // A scene name is user text: only its length is logged, at any depth.
        for scene in ["Live", "BRB", "Zażółć 🎥 \"quoted\" {json}"] {
            let obs = Action::ObsScene {
                scene: scene.into(),
            };
            let count = scene.chars().count();
            assert_eq!(
                loggable(&obs),
                json!({"type":"obs_scene", "scene_chars":count})
            );
            let nested = Action::MultiAction {
                actions: vec![Action::ToggleMute, obs],
            };
            assert_eq!(
                loggable(&nested)["actions"][1],
                json!({"type":"obs_scene", "scene_chars":count})
            );
            assert!(!loggable(&nested).to_string().contains(scene));
        }
        // A macro's steps are the keys it types, so they are never logged.
        let macro_action = Action::Macro {
            steps: vec![
                MacroStep {
                    key: "h".into(),
                    modifiers: vec![],
                    delay_before_ms: 0,
                    hold_ms: 40,
                },
                MacroStep {
                    key: "unter2".into(),
                    modifiers: vec![],
                    delay_before_ms: 100,
                    hold_ms: 0,
                },
            ],
        };
        assert_eq!(
            loggable(&macro_action),
            json!({"type":"macro", "steps":2, "ms":140})
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
