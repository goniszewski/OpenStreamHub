//! Report planning and event-record processing shared by replay and live runs.
//!
//! [`plan_report`] decodes one report and resolves each event through the
//! stateful [`ProfileSession`], without I/O. [`process_work`] writes the report
//! and event records, then dispatches planned actions. Replay calls both inline;
//! the live runtime plans on its input thread and processes on its worker.

use crate::{capture::encode_hex, live_runtime::LiveSummary};
use cooltek_sdk_service::{
    Action, ActionDispatcher, Binding, DeviceContext, InputEvent, InputSession, InputUpdate,
    KeypadResolution, ProfileSession,
};
use serde_json::{Value, json};
use std::{
    io::Write,
    sync::atomic::{AtomicBool, Ordering},
};

/// Largest JSON record, including its newline, that a service log may contain.
pub const MAX_RECORD_BYTES: usize = 65_536;

/// One normalized event with its selector and stateful resolution.
#[derive(Clone, Debug)]
pub struct PlannedEvent {
    /// Latest input represented by this entry.
    pub event: InputEvent,
    /// Flat selector, if any; schema-2 keypad routing reports `None`.
    pub binding: Option<Binding>,
    /// Action, keypad navigation, or unbound outcome.
    pub resolution: KeypadResolution,
    /// Input events represented, greater than one only after live coalescing.
    pub coalesced: u64,
}

impl PlannedEvent {
    /// The concrete action to dispatch, if any.
    pub fn action(&self) -> Option<&Action> {
        match &self.resolution {
            KeypadResolution::Action { action, .. } => Some(action),
            _ => None,
        }
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
            let resolution = session.resolve(device, &event)?;
            Ok(PlannedEvent {
                event,
                binding,
                resolution,
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
    })
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

/// Writes records for one work item and dispatches its actions in order.
///
/// Each intended action is logged before dispatch, so a failed log write never
/// causes an unrecorded action. A failed action is logged as `action_failed`;
/// it ends processing only when `stop_on_action_error` is set. Output errors
/// always end processing. A set `abort` flag drops remaining events.
pub fn process_work<D: ActionDispatcher, W: Write>(
    work: Work,
    dispatcher: &mut D,
    output: &mut W,
    abort: &AtomicBool,
    execute: bool,
    stop_on_action_error: bool,
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
    emit(output, report)?;
    summary.processed_report_count += work.report_count;
    for planned in work.events {
        if abort.load(Ordering::SeqCst) {
            break;
        }
        let (action, navigation, page_id) = match &planned.resolution {
            KeypadResolution::Action { action, page_id } => (Some(action), None, page_id.as_ref()),
            KeypadResolution::Navigation { navigation } => (None, Some(navigation), None),
            KeypadResolution::Unbound => (None, None, None),
        };
        let mut record = json!({
            "type":"event", "schema_version":1, "event":planned.event,
            "binding":planned.binding, "action":action, "navigation":navigation, "page_id":page_id,
            "status":if action.is_some() {"planned"} else if navigation.is_some() {"navigation"} else {"unbound"},
        });
        if planned.coalesced > 1 {
            record["coalesced_event_count"] = json!(planned.coalesced);
        }
        emit(output, record)?;
        summary.event_count += planned.coalesced;
        let Some(action) = action else {
            continue;
        };
        if abort.load(Ordering::SeqCst) {
            break;
        }
        if let Err(error) = dispatcher.dispatch(action) {
            summary.failed_action_count += planned.coalesced;
            emit(
                output,
                json!({"type":"action_failed", "event":planned.event, "action":action, "error":error}),
            )?;
            if stop_on_action_error {
                return Err(format!("action failed: {error}"));
            }
            continue;
        }
        summary.action_count += planned.coalesced;
        if execute {
            emit(
                output,
                json!({"type":"action_executed", "event":planned.event, "action":action}),
            )?;
        }
    }
    Ok(())
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
