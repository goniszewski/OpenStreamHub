//! Bounded live input and serialized action execution.
//!
//! The input owner timestamps, normalizes, and resolves reports before
//! enqueueing work. When the queue is full, later reports are coalesced into one
//! pending work item: consecutive relative volume steps on a control are summed,
//! absolute positions keep the latest value, and other events are appended up to
//! a fixed bound beyond which they are dropped and counted. Steps never merge
//! across a dial function switch or between different dial functions, and a
//! dropped switch is counted separately. The action worker's
//! writer must have bounded writes, and its dispatcher must respond to the
//! supplied cancellation token. Native HID feature cleanup has no exposed
//! timeout; worker deadlines do not establish a bound on a backend's
//! synchronous device operations.
//!
//! Optional keypad display synchronization ([`LiveDisplay`], opt-in and
//! hardware-unverified) runs on its own thread with a single coalescing
//! request slot. The input owner only records the newest page path there after
//! the work for that report was queued or coalesced, and never waits for the
//! display thread, so display work and display failures never dispatch,
//! suppress, or reorder an action. When the sink shares the input device, an
//! in-flight image write can delay reading input reports (they queue in the OS
//! meanwhile); the input loop itself keeps running its interrupt and limit
//! checks. Display outcomes reach the log through the action worker.

use crate::service_events::{
    PlannedEvent, Work, dial_function_details, emit, end_record, plan_report, process_work,
};
use cooltek_sdk_masterhub::{InputReader, ModuleKind, ReportTransport};
use cooltek_sdk_service::{
    Action, ActionDispatcher, DeviceContext, Gesture, InputSession, ProfileResolution,
    ProfileSession, ServiceModule,
    display_sync::{
        ApplyReport, DisplayCapabilities, DisplaySink, DisplaySync, DisplayTarget,
        KeyImageProvider, SinkError,
    },
};
use serde_json::{Value, json};
use std::{
    io::Write,
    sync::{
        Arc, Condvar, Mutex, PoisonError,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
    },
    thread,
    time::{Duration, Instant},
};

/// Maximum distinct events held in the coalesced pending work item.
pub const MAX_PENDING_EVENTS: usize = 256;
/// Read timeout while coalesced work waits for queue space, in milliseconds.
const PENDING_POLL_MS: i32 = 5;
/// Error reported when the worker panics while building resources or dispatching.
const WORKER_PANIC: &str = "action worker panicked; action outcome may be unknown";
/// Error reported when the input loop panics; cleanup still runs.
const INPUT_PANIC: &str = "input loop panicked";

/// Bounds for a single live interface.
#[derive(Clone, Copy, Debug)]
pub struct LiveOptions {
    /// Explicitly enable IPS module software mode and attempt hardware-mode cleanup.
    pub software_mode: bool,
    /// Stop accepting reports after this duration, if set.
    pub duration: Option<Duration>,
    /// Stop accepting reports after this count, if set.
    pub max_reports: Option<u64>,
    /// Positive read timeout, at most 1000 milliseconds.
    pub timeout_ms: i32,
    /// Maximum queued reports, excluding the worker's current report.
    pub queue_capacity: usize,
    /// Maximum time to drain accepted work after a normal input limit.
    pub drain_timeout: Duration,
    /// Maximum time to observe worker completion after cancellation.
    pub cancellation_timeout: Duration,
    /// Maximum time for the worker to build its resources and write `service_start`.
    pub startup_timeout: Duration,
    /// End the session on the first failed action instead of logging and continuing.
    pub stop_on_action_error: bool,
}

impl Default for LiveOptions {
    fn default() -> Self {
        Self {
            software_mode: false,
            duration: None,
            max_reports: None,
            timeout_ms: 100,
            queue_capacity: 64,
            drain_timeout: Duration::from_secs(10),
            cancellation_timeout: Duration::from_secs(2),
            startup_timeout: Duration::from_secs(5),
            stop_on_action_error: false,
        }
    }
}

/// Final counts distinguish received input from completed worker output.
#[derive(Clone, Debug, Default)]
pub struct LiveSummary {
    /// Nonempty reports received.
    pub report_count: u64,
    /// Reports accepted by the work queue, including coalesced reports.
    pub queued_report_count: u64,
    /// Reports whose diagnostic record (possibly coalesced) was written by the worker.
    pub processed_report_count: u64,
    /// Reports merged into an earlier pending work item while the queue was full.
    pub coalesced_report_count: u64,
    /// Normalized events decoded by the input owner.
    pub decoded_event_count: u64,
    /// Events whose planned, navigation, or unbound record was written.
    pub event_count: u64,
    /// Events merged into an earlier pending event on the same control.
    pub coalesced_event_count: u64,
    /// Events dropped because the pending work item was full.
    pub dropped_event_count: u64,
    /// Dial function switch records written; a switch never dispatches an action.
    pub dial_function_switch_count: u64,
    /// Dial function switches among `dropped_event_count`. The switch still
    /// took effect, because routing happens before queueing.
    pub dropped_dial_function_count: u64,
    /// Successfully dispatched actions, including dry-run validation.
    pub action_count: u64,
    /// Dispatched actions that reported failure.
    pub failed_action_count: u64,
    /// Matched bindings that did not complete successfully.
    pub incomplete_action_count: u64,
    /// Display syncs attempted; always 0 without [`LiveDisplay`].
    pub display_sync_count: u64,
    /// Display syncs that failed to plan or apply, or left any key unwritten.
    pub display_failure_count: u64,
}

/// Longest error text in a `display_sync` record, which lists at most 15 keys per field.
const DISPLAY_ERROR_CHARS: usize = 240;
/// Display records waiting for the action worker; later records are dropped and counted.
const DISPLAY_RECORD_QUEUE: usize = 32;

/// Opt-in keypad display synchronization for one live M4315 run.
///
/// Hardware-unverified: physical key order, orientation, pacing, and firmware
/// image acceptance are unknown. The sink and provider move to a dedicated
/// display thread; a display failure is logged as a `display_sync` record and
/// counted in `service_end`, but never ends the session or changes its result.
pub struct LiveDisplay {
    pub(crate) sink: Box<dyn DisplaySink + Send>,
    artwork: Box<dyn KeyImageProvider + Send>,
    pub(crate) write_gate: Arc<AtomicBool>,
}

impl LiveDisplay {
    /// Pairs a sink (for example `HidDisplaySink`) with prepared key artwork
    /// and the sink transport's write gate.
    ///
    /// The gate is required: pass the transport's own gate (for a shared
    /// device, `SharedTransport::write_gate`, as `display_sink::live_display`
    /// does) so closing it also stops an image mid-transfer at its next report.
    /// The runtime stores `true` when the input loop ends, before it waits for
    /// the display thread and restores hardware mode. Every later key write
    /// then fails at once without reaching the sink.
    pub fn new(
        sink: impl DisplaySink + Send + 'static,
        artwork: impl KeyImageProvider + Send + 'static,
        write_gate: Arc<AtomicBool>,
    ) -> Self {
        Self {
            sink: Box::new(sink),
            artwork: Box::new(artwork),
            write_gate,
        }
    }
}

/// Error for a key write refused after the write gate closed.
const DISPLAY_WRITES_CLOSED: &str = "display writes closed at session end";

struct BoxedSink {
    sink: Box<dyn DisplaySink + Send>,
    gate: Arc<AtomicBool>,
}

impl BoxedSink {
    fn open(&self) -> Result<(), SinkError> {
        if self.gate.load(Ordering::SeqCst) {
            Err(SinkError::Transport(DISPLAY_WRITES_CLOSED.into()))
        } else {
            Ok(())
        }
    }
}

impl DisplaySink for BoxedSink {
    fn capabilities(&self) -> DisplayCapabilities {
        self.sink.capabilities()
    }

    fn write_key_image(
        &mut self,
        key_index: u8,
        jpeg: &[u8],
        between_chunks: &mut dyn FnMut(usize),
    ) -> Result<(), SinkError> {
        self.open()?;
        self.sink.write_key_image(key_index, jpeg, between_chunks)
    }

    fn write_dial_image(
        &mut self,
        jpeg: &[u8],
        between_chunks: &mut dyn FnMut(usize),
    ) -> Result<(), SinkError> {
        self.open()?;
        self.sink.write_dial_image(jpeg, between_chunks)
    }
}

/// One pending page redraw; a newer request replaces an older one.
struct DisplayRequest {
    path: Vec<String>,
    trigger: &'static str,
}

#[derive(Default)]
struct DisplaySlot {
    pending: Option<DisplayRequest>,
    stopped: bool,
}

#[derive(Default)]
struct DisplayCounts {
    syncs: AtomicU64,
    failures: AtomicU64,
    coalesced: AtomicU64,
    dropped_records: AtomicU64,
}

/// The input owner's handle to the display thread.
struct DisplayThread {
    slot: Arc<(Mutex<DisplaySlot>, Condvar)>,
    counts: Arc<DisplayCounts>,
    write_gate: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl DisplayThread {
    fn spawn(
        display: LiveDisplay,
        mut session: ProfileSession,
        abort: Arc<AtomicBool>,
        records: SyncSender<Value>,
    ) -> Result<Self, String> {
        let slot = Arc::new((Mutex::new(DisplaySlot::default()), Condvar::new()));
        let counts = Arc::new(DisplayCounts::default());
        let (thread_slot, thread_counts) = (Arc::clone(&slot), Arc::clone(&counts));
        let write_gate = Arc::clone(&display.write_gate);
        let handle = thread::Builder::new()
            .name("masterhub-display".into())
            .spawn(move || {
                let LiveDisplay {
                    sink,
                    artwork,
                    write_gate: gate,
                } = display;
                // Built here: `DisplaySync` is not `Send`.
                let mut sync = DisplaySync::new(BoxedSink { sink, gate });
                while let Some(request) = next_display_request(&thread_slot, &abort) {
                    let record = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        sync_page(&mut sync, &mut session, &*artwork, &request, &thread_counts)
                    }));
                    let (record, panicked) = match record {
                        Ok(record) => (record, false),
                        Err(_) => {
                            // Later requests are ignored rather than counted.
                            let (lock, _) = &*thread_slot;
                            let mut slot = lock.lock().unwrap_or_else(PoisonError::into_inner);
                            slot.stopped = true;
                            slot.pending = None;
                            drop(slot);
                            thread_counts.failures.fetch_add(1, Ordering::SeqCst);
                            let record = display_record(
                                &request,
                                None,
                                &[],
                                Some("display sync panicked; display disabled for this run"),
                            );
                            (record, true)
                        }
                    };
                    if records.try_send(record).is_err() {
                        thread_counts.dropped_records.fetch_add(1, Ordering::SeqCst);
                    }
                    if panicked {
                        return;
                    }
                }
            })
            .map_err(|error| {
                format!("--display-sync setup failed: could not start display thread: {error}")
            })?;
        Ok(Self {
            slot,
            counts,
            write_gate,
            handle: Some(handle),
        })
    }

    /// Records the newest page to draw without waiting for the display thread.
    fn request(&self, session: &ProfileSession, trigger: &'static str) {
        let (lock, changed) = &*self.slot;
        let mut slot = lock.lock().unwrap_or_else(PoisonError::into_inner);
        if slot.stopped {
            return;
        }
        let request = DisplayRequest {
            path: session.path().to_vec(),
            trigger,
        };
        if slot.pending.replace(request).is_some() {
            self.counts.coalesced.fetch_add(1, Ordering::SeqCst);
        }
        changed.notify_all();
    }

    /// Discards any pending request and closes the write gate: no sync starts
    /// after this call, and an in-flight one fails its remaining writes at once.
    fn stop(&self) {
        self.write_gate.store(true, Ordering::SeqCst);
        let (lock, changed) = &*self.slot;
        let mut slot = lock.lock().unwrap_or_else(PoisonError::into_inner);
        slot.stopped = true;
        slot.pending = None;
        changed.notify_all();
    }

    /// Waits up to `timeout` for an in-flight sync; returns whether the thread exited.
    fn finish(&mut self, timeout: Duration) -> bool {
        self.stop();
        let Some(handle) = self.handle.take() else {
            return true;
        };
        let deadline = Instant::now() + timeout;
        while !handle.is_finished() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(1));
        }
        if handle.is_finished() {
            // A panic is already contained and logged inside the thread.
            let _ = handle.join();
            true
        } else {
            false
        }
    }
}

impl Drop for DisplayThread {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Blocks the display thread until a request arrives; `None` once stopped or aborted.
fn next_display_request(
    slot: &(Mutex<DisplaySlot>, Condvar),
    abort: &AtomicBool,
) -> Option<DisplayRequest> {
    let (lock, changed) = slot;
    let mut slot = lock.lock().unwrap_or_else(PoisonError::into_inner);
    loop {
        if slot.stopped || abort.load(Ordering::SeqCst) {
            return None;
        }
        if let Some(request) = slot.pending.take() {
            return Some(request);
        }
        // Bounded wait so a set interrupt flag is observed without a notification.
        slot = changed
            .wait_timeout(slot, Duration::from_millis(50))
            .unwrap_or_else(PoisonError::into_inner)
            .0;
    }
}

/// Plans and applies one page; every failure is reported in the record only.
fn sync_page<S: DisplaySink>(
    sync: &mut DisplaySync<S>,
    session: &mut ProfileSession,
    artwork: &dyn KeyImageProvider,
    request: &DisplayRequest,
    counts: &DisplayCounts,
) -> Value {
    counts.syncs.fetch_add(1, Ordering::SeqCst);
    let planned = session
        .set_path(&request.path)
        .and_then(|()| sync.plan_page(session, artwork).map_err(|e| e.to_string()));
    let (report, missing, error) = match planned {
        Ok(plan) => {
            let missing: Vec<u8> = plan.missing().iter().map(|r| r.key_index).collect();
            match sync.apply(plan) {
                Ok(report) => (Some(report), missing, None),
                Err(error) => (None, missing, Some(error.to_string())),
            }
        }
        Err(error) => (None, Vec::new(), Some(error)),
    };
    if error.is_some() || report.as_ref().is_some_and(|report| !report.is_complete()) {
        counts.failures.fetch_add(1, Ordering::SeqCst);
    }
    display_record(request, report.as_ref(), &missing, error.as_deref())
}

fn key_index(target: &DisplayTarget) -> Option<u8> {
    match target {
        DisplayTarget::Key(index) => Some(*index),
        DisplayTarget::Dial => None,
    }
}

fn bounded(text: &str) -> String {
    text.chars().take(DISPLAY_ERROR_CHARS).collect()
}

/// A `display_sync` record; every list has at most one entry per key.
fn display_record(
    request: &DisplayRequest,
    report: Option<&ApplyReport>,
    missing: &[u8],
    error: Option<&str>,
) -> Value {
    let written: Vec<u8> = report
        .map(|report| report.written.iter().filter_map(key_index).collect())
        .unwrap_or_default();
    let failed: Vec<Value> = report
        .map(|report| {
            report
                .failures
                .iter()
                .filter_map(|failure| {
                    let short = matches!(failure.error, SinkError::ShortWrite { .. });
                    key_index(&failure.target).map(|index| {
                        json!({
                            "key_index":index, "short_write":short,
                            "attempts":failure.attempts, "error":bounded(&failure.error.to_string()),
                        })
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let not_attempted: Vec<u8> = report
        .map(|report| report.not_attempted.iter().filter_map(key_index).collect())
        .unwrap_or_default();
    json!({
        "type":"display_sync", "schema_version":1, "trigger":request.trigger,
        "page_id":request.path.last(), "depth":request.path.len().saturating_sub(1),
        "written":written, "failed":failed, "not_attempted":not_attempted, "missing":missing,
        "error":error.map(bounded),
        "complete":error.is_none() && report.is_some_and(ApplyReport::is_complete),
    })
}

#[derive(Default)]
struct Finish {
    reason: String,
    error: Option<String>,
    reports: u64,
    queued: u64,
    coalesced_reports: u64,
    decoded_events: u64,
    coalesced_events: u64,
    dropped_events: u64,
    dropped_dial_functions: u64,
    matched_actions: u64,
}

struct WorkerOutcome<W> {
    summary: LiveSummary,
    error: Option<String>,
    output: Option<W>,
}

/// Reads input independently of the worker's action and log latency.
///
/// `factory` runs in the worker and receives its cooperative cancellation token.
/// The worker owns both dispatcher and writer. The caller's interrupt flag is
/// also set on internal cancellation; failure details are returned separately.
/// Pending work is discarded on cancellation; normal limits allow a finite drain.
/// A callback that ignores its bounds/cancellation contract is reported as an
/// unjoined-worker error. Its resources remain owned by that worker until it exits.
#[allow(clippy::too_many_arguments)]
pub fn run_live<T, D, W, F>(
    reader: InputReader<T>,
    session: ProfileSession,
    device: DeviceContext,
    execute: bool,
    details: Value,
    options: LiveOptions,
    interrupted: Arc<AtomicBool>,
    factory: F,
) -> Result<LiveSummary, String>
where
    T: ReportTransport,
    D: ActionDispatcher + Send + 'static,
    W: Write + Send + 'static,
    F: FnOnce(Arc<AtomicBool>) -> Result<(D, W), String> + Send + 'static,
{
    run_live_with_display(
        reader,
        session,
        device,
        execute,
        details,
        options,
        interrupted,
        None,
        factory,
    )
}

/// [`run_live`] with optional, opt-in keypad display synchronization.
///
/// `display` requires `options.software_mode`, an M4315 session, and a schema-2
/// keypad profile; anything else, or a display thread that cannot start, is
/// rejected before the worker starts or software mode is entered. The
/// first sync is requested right after software mode is enabled, and another
/// after each report whose work contains a keypad navigation, once that work
/// was queued or coalesced. No sync starts once the input loop has ended.
#[allow(clippy::too_many_arguments)]
pub fn run_live_with_display<T, D, W, F>(
    mut reader: InputReader<T>,
    session: ProfileSession,
    device: DeviceContext,
    execute: bool,
    details: Value,
    options: LiveOptions,
    interrupted: Arc<AtomicBool>,
    display: Option<LiveDisplay>,
    factory: F,
) -> Result<LiveSummary, String>
where
    T: ReportTransport,
    D: ActionDispatcher + Send + 'static,
    W: Write + Send + 'static,
    F: FnOnce(Arc<AtomicBool>) -> Result<(D, W), String> + Send + 'static,
{
    let origin = Instant::now();
    run_inner(
        &mut reader,
        session,
        device,
        execute,
        details,
        options,
        interrupted,
        display,
        factory,
        || u64::try_from(origin.elapsed().as_micros()).unwrap_or(u64::MAX),
    )
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
fn run_with_clock<T, D, W, F>(
    reader: &mut InputReader<T>,
    session: ProfileSession,
    device: DeviceContext,
    execute: bool,
    details: Value,
    options: LiveOptions,
    interrupted: Arc<AtomicBool>,
    factory: F,
    timestamp: impl FnMut() -> u64,
) -> Result<LiveSummary, String>
where
    T: ReportTransport,
    D: ActionDispatcher + Send + 'static,
    W: Write + Send + 'static,
    F: FnOnce(Arc<AtomicBool>) -> Result<(D, W), String> + Send + 'static,
{
    run_inner(
        reader,
        session,
        device,
        execute,
        details,
        options,
        interrupted,
        None,
        factory,
        timestamp,
    )
}

#[allow(clippy::too_many_arguments)]
fn run_inner<T, D, W, F>(
    reader: &mut InputReader<T>,
    mut session: ProfileSession,
    device: DeviceContext,
    execute: bool,
    details: Value,
    options: LiveOptions,
    interrupted: Arc<AtomicBool>,
    display: Option<LiveDisplay>,
    factory: F,
    mut timestamp: impl FnMut() -> u64,
) -> Result<LiveSummary, String>
where
    T: ReportTransport,
    D: ActionDispatcher + Send + 'static,
    W: Write + Send + 'static,
    F: FnOnce(Arc<AtomicBool>) -> Result<(D, W), String> + Send + 'static,
{
    validate_options(options, &device)?;
    if display.is_some() {
        validate_display(options, &device, &session)?;
    }
    let mut details = details;
    if let (Some(object), Some(functions)) = (
        details.as_object_mut(),
        dial_function_details(&session, &device),
    ) {
        object.insert("dial_functions".into(), functions);
    }
    let abort = Arc::clone(&interrupted);
    // Started before the worker, so a failed spawn leaves nothing to clean up.
    // Fail closed: folder navigation must not run live without its display.
    let (display_records, display_thread) = match display {
        Some(display) => {
            let (records_tx, records_rx) = mpsc::sync_channel(DISPLAY_RECORD_QUEUE);
            let spawned =
                DisplayThread::spawn(display, session.clone(), Arc::clone(&abort), records_tx)?;
            if let Some(object) = details.as_object_mut() {
                object.insert(
                    "display_sync".into(),
                    json!({"enabled":true, "hardware_verified":false}),
                );
            }
            (Some(records_rx), Some(spawned))
        }
        None => (None, None),
    };
    let mut display_thread = display_thread;
    let (sender, receiver) = mpsc::sync_channel(options.queue_capacity);
    let (ready_tx, ready_rx) = mpsc::sync_channel(1);
    let (done_tx, done_rx) = mpsc::sync_channel(1);
    let worker_abort = Arc::clone(&abort);
    let worker_device = device.clone();
    let handle = thread::Builder::new()
        .name("masterhub-actions".into())
        .spawn(move || {
            let panic_abort = Arc::clone(&worker_abort);
            let panic_ready = ready_tx.clone();
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                worker(
                    receiver,
                    ready_tx,
                    worker_abort,
                    worker_device,
                    execute,
                    options.stop_on_action_error,
                    details,
                    display_records,
                    factory,
                )
            }))
            .unwrap_or_else(|_| {
                panic_abort.store(true, Ordering::SeqCst);
                let error = WORKER_PANIC.to_owned();
                let _ = panic_ready.try_send(Err(error.clone()));
                WorkerOutcome {
                    summary: LiveSummary::default(),
                    error: Some(error),
                    output: None,
                }
            });
            // Dispatcher resources are dropped; the output owner returns to the coordinator.
            let _ = done_tx.send(outcome);
        })
        .map_err(|error| format!("could not start action worker: {error}"))?;

    let started = Instant::now();
    let mut input = InputSession::new(device.module);
    let mut mode_attempted = false;
    let mut counts = Finish::default();
    let mut pending: Option<Work> = None;
    // A panic in the input loop (decoding, routing, a transport) is contained
    // so the cleanup below, including the hardware-mode restore, still runs.
    let input_loop = std::panic::AssertUnwindSafe(|| -> Result<&str, String> {
        match ready_rx.recv_timeout(options.startup_timeout) {
            Ok(Ok(())) => {}
            Ok(Err(error)) => return Err(error),
            Err(error) => return Err(format!("action worker did not become ready: {error}")),
        }
        if interrupted.load(Ordering::SeqCst) {
            return Ok("interrupted");
        }
        if options.software_mode {
            mode_attempted = true;
            crate::mode::set(reader.transport(), mode_module(device.module), true)?;
            if let Some(display) = &display_thread {
                display.request(&session, "software_mode");
            }
        }
        loop {
            if interrupted.load(Ordering::SeqCst) {
                return Ok("interrupted");
            }
            if let Some(work) = pending.take() {
                pending = offer(&sender, work, &mut counts)?;
            }
            let mut timeout = if let Some(duration) = options.duration {
                let remaining = duration.saturating_sub(started.elapsed());
                if remaining.as_millis() == 0 {
                    return Ok("duration_elapsed");
                }
                remaining.as_millis().min(options.timeout_ms as u128) as i32
            } else {
                options.timeout_ms
            };
            if pending.is_some() {
                timeout = timeout.min(PENDING_POLL_MS);
            }
            let read = match reader.read_input(timeout) {
                Ok(read) => read,
                // A signal can interrupt the blocking read; that is a requested stop.
                Err(_) if interrupted.load(Ordering::SeqCst) => return Ok("interrupted"),
                Err(error) => return Err(format!("input read failed: {error}")),
            };
            if let Some(raw) = read {
                let now = timestamp();
                counts.reports += 1;
                if interrupted.load(Ordering::SeqCst) {
                    return Ok("interrupted");
                }
                let work = plan_report(&mut input, &mut session, &device, raw, now)?;
                counts.decoded_events += work.events.len() as u64;
                counts.matched_actions +=
                    work.events.iter().filter(|e| e.action().is_some()).count() as u64;
                let navigated = work.events.iter().any(|planned| {
                    matches!(planned.resolution, ProfileResolution::Navigation { .. })
                });
                pending = match pending.take() {
                    Some(mut earlier) => {
                        coalesce(&mut earlier, work, &mut counts);
                        Some(earlier)
                    }
                    None => offer(&sender, work, &mut counts)?,
                };
                // Only after the work was queued or coalesced, and never blocking.
                if navigated && let Some(display) = &display_thread {
                    display.request(&session, "navigation");
                }
                if options
                    .max_reports
                    .is_some_and(|limit| counts.reports >= limit)
                {
                    return Ok("max_reports");
                }
            }
        }
    });
    let result = std::panic::catch_unwind(input_loop).unwrap_or_else(|payload| {
        let message = payload
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
            .unwrap_or("non-string panic payload");
        Err(format!("{INPUT_PANIC}: {message}"))
    });
    // No display sync starts during drain or cancellation.
    if let Some(display) = &display_thread {
        display.stop();
    }
    let normal = matches!(result, Ok("duration_elapsed" | "max_reports"));
    // One drain budget covers both queueing pending work and worker completion.
    let drain_deadline = normal.then(|| Instant::now() + options.drain_timeout);
    let mut drain_error = None;
    if let Some(deadline) = drain_deadline
        && let Some(work) = pending.take()
    {
        drain_error = flush_pending(&sender, work, &abort, deadline, &mut counts);
    }
    if !normal {
        abort.store(true, Ordering::SeqCst);
    }
    input.reset();
    session.reset();
    counts.reason = result.as_ref().copied().unwrap_or("error").to_owned();
    counts.error = combine_errors(result.err(), drain_error);
    // The write gate closed when the loop ended, so an in-flight sync fails its
    // remaining writes at once. Wait (bounded) for the thread so its record
    // reaches the worker before the queue closes. If a write is stuck inside
    // the backend, the thread outlives this wait: `display_stopped` is false,
    // the hardware-mode restore below may find the device busy, and no further
    // image report can start because the gate stays closed. A stalled sink is
    // not an error by itself.
    let display_stopped = display_thread
        .as_mut()
        .map(|display| display.finish(options.cancellation_timeout));
    // Close the queue before monitoring drain/cancellation. Device cleanup is
    // attempted after that finite worker phase, including when the worker fails.
    drop(sender);
    let outcome = await_worker(
        &done_rx,
        &abort,
        drain_deadline,
        options.cancellation_timeout,
    );
    let mut summary = LiveSummary::default();
    let mut output = None;
    match outcome {
        Ok(outcome) => {
            summary = outcome.summary;
            output = outcome.output;
            counts.error = combine_errors(counts.error.take(), outcome.error);
            let deadline = Instant::now() + options.cancellation_timeout;
            while !handle.is_finished() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(1));
            }
            let join_error = if handle.is_finished() {
                handle
                    .join()
                    .err()
                    .map(|_| "action worker panicked".to_owned())
            } else {
                Some(
                    "worker completion was reported but thread did not exit; thread was not joined"
                        .into(),
                )
            };
            counts.error = combine_errors(counts.error.take(), join_error);
        }
        Err(error) => counts.error = combine_errors(counts.error.take(), Some(error)),
    }
    let cleanup = if mode_attempted {
        restore_hardware_mode(reader.transport(), device.module)
    } else {
        Ok(())
    };
    counts.error = combine_errors(counts.error.take(), cleanup.err());
    if normal && abort.load(Ordering::SeqCst) && counts.error.is_none() {
        counts.reason = "interrupted".into();
    }
    summary.report_count = counts.reports;
    summary.queued_report_count = counts.queued;
    summary.coalesced_report_count = counts.coalesced_reports;
    summary.decoded_event_count = counts.decoded_events;
    summary.coalesced_event_count = counts.coalesced_events;
    summary.dropped_event_count = counts.dropped_events;
    summary.dropped_dial_function_count = counts.dropped_dial_functions;
    summary.incomplete_action_count = counts.matched_actions.saturating_sub(summary.action_count);
    if let Some(display) = &display_thread {
        summary.display_sync_count = display.counts.syncs.load(Ordering::SeqCst);
        summary.display_failure_count = display.counts.failures.load(Ordering::SeqCst);
    }
    if let Some(mut output) = output {
        let mut record = end_record(&counts.reason, counts.error.as_deref(), &summary, execute);
        if let (Some(object), Some(display), Some(stopped)) =
            (record.as_object_mut(), &display_thread, display_stopped)
        {
            for (key, value) in [
                ("display_sync_count", json!(summary.display_sync_count)),
                (
                    "display_failure_count",
                    json!(summary.display_failure_count),
                ),
                (
                    "display_coalesced_request_count",
                    json!(display.counts.coalesced.load(Ordering::SeqCst)),
                ),
                (
                    "display_dropped_record_count",
                    json!(display.counts.dropped_records.load(Ordering::SeqCst)),
                ),
                ("display_stopped", json!(stopped)),
            ] {
                object.insert(key.into(), value);
            }
        }
        let footer = emit(&mut output, record);
        counts.error = combine_errors(counts.error.take(), footer.err());
    }
    match counts.error {
        Some(error) => Err(error),
        None => Ok(summary),
    }
}

/// Offers work to the queue; returns it back when the queue is full.
fn offer(
    sender: &SyncSender<Work>,
    work: Work,
    counts: &mut Finish,
) -> Result<Option<Work>, String> {
    let reports = work.report_count;
    match sender.try_send(work) {
        Ok(()) => {
            counts.queued += reports;
            Ok(None)
        }
        Err(TrySendError::Full(work)) => Ok(Some(work)),
        Err(TrySendError::Disconnected(_)) => Err("action worker disconnected".into()),
    }
}

/// Retries pending work within the drain budget after a normal input limit.
fn flush_pending(
    sender: &SyncSender<Work>,
    mut work: Work,
    abort: &AtomicBool,
    deadline: Instant,
    counts: &mut Finish,
) -> Option<String> {
    loop {
        match offer(sender, work, counts) {
            Ok(None) => return None,
            Ok(Some(returned)) => work = returned,
            Err(error) => return Some(error),
        }
        if abort.load(Ordering::SeqCst) {
            return None;
        }
        if Instant::now() >= deadline {
            abort.store(true, Ordering::SeqCst);
            return Some("action queue drain deadline exceeded; pending actions cancelled".into());
        }
        thread::sleep(Duration::from_millis(1));
    }
}

/// Merges a later report into pending work that is waiting for queue space.
fn coalesce(pending: &mut Work, next: Work, counts: &mut Finish) {
    counts.coalesced_reports += next.report_count;
    pending.coalesced_parse_error_count +=
        next.coalesced_parse_error_count + u64::from(pending.status == "parse_error");
    pending.report_count += next.report_count;
    pending.raw = next.raw;
    pending.timestamp_micros = next.timestamp_micros;
    pending.status = next.status;
    pending.error = next.error;
    pending.dropped_event_count += next.dropped_event_count;
    pending.dropped_dial_function_count += next.dropped_dial_function_count;
    for planned in next.events {
        // Merge only into the immediately preceding event so the order of
        // events across controls is preserved. A dropped dial function switch
        // after that event is a barrier: steps on both sides stay separate.
        if !pending.merge_barrier
            && let Some(existing) = pending
                .events
                .last_mut()
                .filter(|existing| existing.event.control == planned.event.control)
            && let Some(merged) = merge_event(existing, &planned)
        {
            *existing = merged;
            counts.coalesced_events += 1;
        } else if pending.events.len() < MAX_PENDING_EVENTS {
            pending.events.push(planned);
            pending.merge_barrier = false;
        } else {
            pending.dropped_event_count += planned.coalesced;
            counts.dropped_events += planned.coalesced;
            if planned.is_dial_function_switch() {
                pending.dropped_dial_function_count += planned.coalesced;
                counts.dropped_dial_functions += planned.coalesced;
                pending.merge_barrier = true;
            }
        }
    }
}

/// Merges two consecutive continuous inputs on one control, if they are equivalent.
///
/// Relative volume steps sum (clamped to the valid ±100 range), absolute
/// positions keep the latest value, and unbound continuous inputs keep only the
/// latest event. Button transitions, navigation, dial function switches, and
/// other actions never merge, and neither do inputs routed through different
/// dial functions.
fn merge_event(earlier: &PlannedEvent, later: &PlannedEvent) -> Option<PlannedEvent> {
    if earlier.event.gesture != later.event.gesture
        || earlier.dial_function != later.dial_function
        || earlier.is_dial_function_switch()
        || later.is_dial_function_switch()
        || !matches!(
            later.event.gesture,
            Gesture::Clockwise
                | Gesture::CounterClockwise
                | Gesture::Left
                | Gesture::Right
                | Gesture::Up
                | Gesture::Down
                | Gesture::PositionChanged
        )
    {
        return None;
    }
    let resolution = match (&earlier.resolution, &later.resolution) {
        (
            ProfileResolution::Action {
                action: Action::Volume { delta: first },
                page_id: None,
                dial_function: first_function,
            },
            ProfileResolution::Action {
                action: Action::Volume { delta: second },
                page_id: None,
                dial_function: second_function,
            },
        ) if first_function == second_function => {
            let delta = (first + second).clamp(-100, 100);
            if delta == 0 {
                return None;
            }
            ProfileResolution::Action {
                action: Action::Volume { delta },
                page_id: None,
                dial_function: second_function.clone(),
            }
        }
        (
            ProfileResolution::Action {
                action: Action::SetVolume { .. },
                page_id: None,
                dial_function: first_function,
            },
            ProfileResolution::Action {
                action: Action::SetVolume { .. },
                page_id: None,
                dial_function: second_function,
            },
        ) if first_function == second_function => later.resolution.clone(),
        (ProfileResolution::Unbound, ProfileResolution::Unbound) => later.resolution.clone(),
        _ => return None,
    };
    Some(PlannedEvent {
        event: later.event,
        binding: later.binding.clone(),
        resolution,
        dial_function: later.dial_function.clone(),
        coalesced: earlier.coalesced + later.coalesced,
    })
}

fn validate_options(options: LiveOptions, device: &DeviceContext) -> Result<(), String> {
    if !(1..=1000).contains(&options.timeout_ms) {
        return Err("read timeout must be between 1 and 1000 ms".into());
    }
    if !(1..=4096).contains(&options.queue_capacity) {
        return Err("queue capacity must be between 1 and 4096 reports".into());
    }
    if options.drain_timeout.is_zero()
        || options.drain_timeout > Duration::from_secs(60)
        || options.cancellation_timeout.is_zero()
        || options.cancellation_timeout > Duration::from_secs(10)
        || options.startup_timeout.is_zero()
        || options.startup_timeout > Duration::from_secs(30)
    {
        return Err(
            "worker drain/cancellation/startup budgets must be positive and bounded".into(),
        );
    }
    if options.max_reports == Some(0) || options.duration.is_some_and(|duration| duration.is_zero())
    {
        return Err("input limits must be positive".into());
    }
    if options.software_mode && !crate::mode::supported(mode_module(device.module)) {
        return Err("software mode requires an M131E or M4315 session".into());
    }
    Ok(())
}

/// Restores hardware mode, retrying once when a shared device was busy.
///
/// A display write can hold a shared device; its lock wait is bounded and
/// reports `SHARED_BUSY`. Any other failure is not retried, as before.
fn restore_hardware_mode<T: ReportTransport + ?Sized>(
    transport: &T,
    module: ServiceModule,
) -> Result<(), String> {
    let first = match crate::mode::set(transport, mode_module(module), false) {
        Ok(()) => return Ok(()),
        Err(error) => error,
    };
    if !first.contains(crate::display_sink::SHARED_BUSY) {
        return Err(format!("restoring module hardware mode failed: {first}"));
    }
    crate::mode::set(transport, mode_module(module), false).map_err(|retry| {
        format!("restoring module hardware mode failed: first attempt: {first}; retry: {retry}")
    })
}

fn validate_display(
    options: LiveOptions,
    device: &DeviceContext,
    session: &ProfileSession,
) -> Result<(), String> {
    if device.module != ServiceModule::M4315 {
        return Err("display sync requires an M4315 keypad session".into());
    }
    if !options.software_mode {
        return Err("display sync requires software mode".into());
    }
    if session.profile().keypad.is_none() {
        return Err("display sync requires a schema-2 keypad profile".into());
    }
    Ok(())
}

fn mode_module(module: ServiceModule) -> Option<ModuleKind> {
    match module {
        ServiceModule::M131E => Some(ModuleKind::M131E),
        ServiceModule::M4315 => Some(ModuleKind::M4315),
        _ => None,
    }
}

/// Waits for the worker outcome. `drain_deadline` is set after a normal input
/// limit; without it the worker is already cancelled.
fn await_worker<W>(
    done: &Receiver<WorkerOutcome<W>>,
    abort: &AtomicBool,
    drain_deadline: Option<Instant>,
    cancellation_timeout: Duration,
) -> Result<WorkerOutcome<W>, String> {
    let mut cancelled_at = drain_deadline.is_none().then(Instant::now);
    let mut drain_error = None;
    loop {
        match done.try_recv() {
            Ok(mut outcome) => {
                outcome.error = combine_errors(drain_error, outcome.error);
                return Ok(outcome);
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                return Err("action worker exited without a completion record".into());
            }
            Err(mpsc::TryRecvError::Empty) => {}
        }
        if cancelled_at.is_none() && abort.load(Ordering::SeqCst) {
            cancelled_at = Some(Instant::now());
        }
        if cancelled_at.is_none()
            && drain_deadline.is_some_and(|deadline| Instant::now() >= deadline)
        {
            drain_error =
                Some("action queue drain deadline exceeded; pending actions cancelled".to_owned());
            abort.store(true, Ordering::SeqCst);
            cancelled_at = Some(Instant::now());
        }
        if cancelled_at.is_some_and(|time| time.elapsed() >= cancellation_timeout) {
            return Err(combine_errors(
                drain_error,
                Some(
                    "action worker did not stop within cancellation budget; thread was not joined"
                        .into(),
                ),
            )
            .unwrap());
        }
        match done.recv_timeout(Duration::from_millis(5)) {
            Ok(mut outcome) => {
                outcome.error = combine_errors(drain_error, outcome.error);
                return Ok(outcome);
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err("action worker exited without a completion record".into());
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn worker<D, W, F>(
    receiver: Receiver<Work>,
    ready: SyncSender<Result<(), String>>,
    abort: Arc<AtomicBool>,
    device: DeviceContext,
    execute: bool,
    stop_on_action_error: bool,
    details: Value,
    display_records: Option<Receiver<Value>>,
    factory: F,
) -> WorkerOutcome<W>
where
    D: ActionDispatcher,
    W: Write,
    F: FnOnce(Arc<AtomicBool>) -> Result<(D, W), String>,
{
    let mut outcome = WorkerOutcome {
        summary: LiveSummary::default(),
        error: None,
        output: None,
    };
    let (mut dispatcher, mut output) = match factory(Arc::clone(&abort)) {
        Ok(resources) => resources,
        Err(error) => {
            abort.store(true, Ordering::SeqCst);
            let _ = ready.send(Err(error.clone()));
            outcome.error = Some(error);
            return outcome;
        }
    };
    if let Err(error) = emit(
        &mut output,
        crate::service_events::start_record(&device, execute, details),
    ) {
        abort.store(true, Ordering::SeqCst);
        let _ = ready.send(Err(error.clone()));
        outcome.error = Some(error);
        outcome.output = Some(output);
        return outcome;
    }
    let _ = ready.send(Ok(()));
    let mut display_records = display_records;
    loop {
        if let Some(records) = &display_records
            && let Err(error) = log_display_records(records, &mut output)
        {
            // An output failure ends the session as for any other record.
            outcome.error = Some(error);
            abort.store(true, Ordering::SeqCst);
            display_records = None;
        }
        let work = match &display_records {
            // Poll so display records are logged while no input arrives.
            Some(_) => match receiver.recv_timeout(Duration::from_millis(20)) {
                Ok(work) => work,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            },
            None => match receiver.recv() {
                Ok(work) => work,
                Err(_) => break,
            },
        };
        if abort.load(Ordering::SeqCst) {
            continue;
        }
        // Contain a dispatch panic here so the writer survives to log `service_end`.
        let processed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            process_work(
                work,
                &mut dispatcher,
                &mut output,
                &abort,
                execute,
                stop_on_action_error,
                &mut outcome.summary,
            )
        }))
        .unwrap_or_else(|_| Err(WORKER_PANIC.to_owned()));
        if let Err(error) = processed {
            outcome.error = Some(error);
            abort.store(true, Ordering::SeqCst);
        }
    }
    // The display thread has stopped (or was detached) before the queue closed.
    if let Some(records) = &display_records
        && let Err(error) = log_display_records(records, &mut output)
    {
        outcome.error = combine_errors(outcome.error.take(), Some(error));
    }
    outcome.output = Some(output);
    outcome
}

/// Logs every display record received so far, without waiting for more.
fn log_display_records(records: &Receiver<Value>, output: &mut impl Write) -> Result<(), String> {
    while let Ok(record) = records.try_recv() {
        emit(output, record)?;
    }
    Ok(())
}

fn combine_errors(first: Option<String>, second: Option<String>) -> Option<String> {
    match (first, second) {
        (None, None) => None,
        (Some(error), None) | (None, Some(error)) => Some(error),
        (Some(error), Some(other)) if error == other => Some(error),
        (Some(error), Some(other)) if other.starts_with(&format!("{error}; ")) => Some(other),
        (Some(error), Some(other)) if error.starts_with(&format!("{other}; ")) => Some(error),
        (Some(error), Some(other)) => Some(format!("{error}; {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cooltek_sdk_masterhub::dial;
    use cooltek_sdk_masterhub::hidapi::{HidError, HidResult};
    use cooltek_sdk_service::Profile;
    use serde_json::json;
    use std::{
        cell::RefCell,
        collections::VecDeque,
        io,
        sync::{
            Condvar, Mutex,
            atomic::{AtomicU64, AtomicUsize},
        },
    };

    #[derive(Clone, Default)]
    struct Output(Arc<Mutex<Vec<u8>>>);
    impl Write for Output {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    impl Output {
        fn rows(&self) -> Vec<Value> {
            String::from_utf8(self.0.lock().unwrap().clone())
                .unwrap()
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect()
        }
    }
    type Gate = Arc<(Mutex<bool>, Condvar)>;
    struct Actions {
        received: Arc<Mutex<Vec<Action>>>,
        stop: Arc<AtomicBool>,
        gate: Option<Gate>,
        began: Option<SyncSender<()>>,
        fail: bool,
        interrupt: bool,
        panic: bool,
    }
    impl ActionDispatcher for Actions {
        fn dispatch(&mut self, action: &Action) -> Result<(), String> {
            self.received.lock().unwrap().push(action.clone());
            if self.panic {
                panic!("synthetic dispatcher panic");
            }
            if let Some(began) = self.began.take() {
                began.send(()).unwrap();
            }
            if let Some(gate) = self.gate.take() {
                let (lock, changed) = &*gate;
                let mut released = lock.lock().unwrap();
                while !*released {
                    if self.stop.load(Ordering::SeqCst) {
                        return Err("synthetic action cancelled".into());
                    }
                    released = changed
                        .wait_timeout(released, Duration::from_millis(1))
                        .unwrap()
                        .0;
                }
            }
            if self.interrupt {
                self.stop.store(true, Ordering::SeqCst);
            }
            if self.fail {
                Err("synthetic action failed".into())
            } else {
                Ok(())
            }
        }
    }
    enum Step {
        Raw(Vec<u8>, u64),
        Unplug,
        /// Runs on the input thread, then reads as a timeout (no report).
        Run(Box<dyn FnMut()>),
    }
    struct Transport {
        steps: RefCell<VecDeque<Step>>,
        clock: Arc<AtomicU64>,
        reads: Arc<AtomicUsize>,
        writes: Arc<Mutex<Vec<Vec<u8>>>>,
        timeouts: Arc<Mutex<Vec<i32>>>,
        wait_second: Option<Receiver<()>>,
        signal_second: Option<SyncSender<()>>,
        fail_enable: bool,
        fail_cleanup: bool,
    }
    impl Transport {
        fn new(steps: impl IntoIterator<Item = Step>) -> Self {
            Self {
                steps: RefCell::new(steps.into_iter().collect()),
                clock: Arc::new(AtomicU64::new(0)),
                reads: Arc::new(AtomicUsize::new(0)),
                writes: Arc::new(Mutex::new(Vec::new())),
                timeouts: Arc::new(Mutex::new(Vec::new())),
                wait_second: None,
                signal_second: None,
                fail_enable: false,
                fail_cleanup: false,
            }
        }
    }
    fn hid_error(message: &str) -> HidError {
        HidError::HidApiError {
            message: message.into(),
        }
    }
    impl ReportTransport for Transport {
        fn read_timeout(&self, buffer: &mut [u8], timeout: i32) -> HidResult<usize> {
            self.timeouts.lock().unwrap().push(timeout);
            let read = self.reads.fetch_add(1, Ordering::SeqCst);
            if read == 1
                && let Some(wait) = &self.wait_second
            {
                wait.recv_timeout(Duration::from_secs(2)).unwrap();
            }
            match self.steps.borrow_mut().pop_front() {
                Some(Step::Raw(raw, timestamp)) => {
                    buffer[..raw.len()].copy_from_slice(&raw);
                    self.clock.store(timestamp, Ordering::SeqCst);
                    if read == 1
                        && let Some(signal) = &self.signal_second
                    {
                        signal.send(()).unwrap();
                    }
                    Ok(raw.len())
                }
                Some(Step::Unplug) => Err(hid_error("synthetic unplug")),
                Some(Step::Run(mut step)) => {
                    step();
                    Ok(0)
                }
                None => {
                    thread::sleep(Duration::from_millis(timeout as u64));
                    Ok(0)
                }
            }
        }
        fn get_feature_report(&self, _: &mut [u8]) -> HidResult<usize> {
            panic!("runtime must not query features")
        }
        fn send_feature_report(&self, bytes: &[u8]) -> HidResult<()> {
            self.writes.lock().unwrap().push(bytes.to_vec());
            if self.fail_enable && bytes[2] == 1 {
                Err(hid_error("enable failed"))
            } else if self.fail_cleanup && bytes[2] == 0 {
                Err(hid_error("cleanup failed"))
            } else {
                Ok(())
            }
        }
    }
    fn dial(selector: u8, value: u8, time: u64) -> Step {
        Step::Raw(vec![1, 0x44, 3, 0, selector, 0, value], time)
    }
    fn profile() -> ProfileSession {
        ProfileSession::new(Profile::from_json(br#"{"schema_version":1,"name":"test","bindings":[
            {"module":"m131e","control":0,"gesture":"right","action":{"type":"volume","delta":5}},
            {"module":"m131e","control":0,"gesture":"pressed","action":{"type":"volume","delta":1}},
            {"module":"m131e","control":0,"gesture":"short_press","action":{"type":"volume","delta":2}},
            {"module":"m131e","control":0,"gesture":"long_press","action":{"type":"toggle_mute"}}
        ]}"#).unwrap()).unwrap()
    }
    fn device() -> DeviceContext {
        DeviceContext {
            module: ServiceModule::M131E,
            serial_number: None,
        }
    }
    fn options() -> LiveOptions {
        LiveOptions {
            max_reports: Some(1),
            timeout_ms: 5,
            drain_timeout: Duration::from_secs(2),
            cancellation_timeout: Duration::from_secs(2),
            ..LiveOptions::default()
        }
    }
    fn modes() -> Vec<Vec<u8>> {
        vec![
            dial::software_mode_report(true).to_vec(),
            dial::software_mode_report(false).to_vec(),
        ]
    }
    fn simple_actions(stop: Arc<AtomicBool>, received: Arc<Mutex<Vec<Action>>>) -> Actions {
        Actions {
            stop,
            received,
            gate: None,
            began: None,
            fail: false,
            interrupt: false,
            panic: false,
        }
    }
    fn run_fake(
        transport: Transport,
        options: LiveOptions,
        interrupted: Arc<AtomicBool>,
        output: Output,
        factory: impl FnOnce(Arc<AtomicBool>) -> Actions + Send + 'static,
    ) -> Result<LiveSummary, String> {
        let clock = Arc::clone(&transport.clock);
        run_with_clock(
            &mut InputReader::new(transport),
            profile(),
            device(),
            true,
            json!({"source":"synthetic"}),
            options,
            interrupted,
            move |stop| Ok((factory(stop), output)),
            || clock.load(Ordering::SeqCst),
        )
    }

    #[test]
    fn new_modules_preserve_values_and_resolve_actions_before_worker_dispatch() {
        for (module, tag, payload, expected) in [
            (
                ServiceModule::M4315,
                0x34,
                {
                    let mut payload = vec![2, 15];
                    payload.extend([0; 15]);
                    payload[2] = 1;
                    payload
                },
                vec![Action::ToggleMute],
            ),
            (
                ServiceModule::M005S,
                0x2a,
                vec![2, 5, 63, 25, 50, 75, 100],
                vec![Action::SetVolume { percent: 63 }],
            ),
            (
                ServiceModule::M032LE,
                0x2a,
                vec![1, 2, 0, 0xff],
                vec![Action::Volume { delta: 5 }],
            ),
        ] {
            let mut raw = vec![1, tag, payload.len() as u8, 0];
            raw.extend(payload);
            let transport = Transport::new([Step::Raw(raw, 123)]);
            let clock = Arc::clone(&transport.clock);
            let writes = Arc::clone(&transport.writes);
            let received = Arc::new(Mutex::new(Vec::new()));
            let actions = Arc::clone(&received);
            let output = Output::default();
            let writer = output.clone();
            let engine = ProfileSession::new(
                Profile::from_json(include_bytes!(
                    "../../../examples/profiles/all-modules.json"
                ))
                .unwrap(),
            )
            .unwrap();
            let result = run_with_clock(
                &mut InputReader::new(transport),
                engine,
                DeviceContext {
                    module,
                    serial_number: None,
                },
                false,
                json!({"source":"synthetic"}),
                options(),
                Arc::new(AtomicBool::new(false)),
                move |stop| Ok((simple_actions(stop, actions), writer)),
                || clock.load(Ordering::SeqCst),
            )
            .unwrap();
            assert_eq!(*received.lock().unwrap(), expected);
            assert_eq!(result.action_count, 1);
            assert!(writes.lock().unwrap().is_empty());
            let rows = output.rows();
            let planned = rows.iter().find(|row| row["status"] == "planned").unwrap();
            assert_eq!(
                planned["action"],
                serde_json::to_value(&expected[0]).unwrap()
            );
            if module == ServiceModule::M005S {
                assert_eq!(planned["binding"]["action"]["type"], "volume_from_position");
                assert_eq!(planned["event"]["value"]["raw"], 63);
            }
        }
    }

    #[test]
    fn key_mode_is_explicit_and_restored_even_when_enabling_fails() {
        for fail_enable in [false, true] {
            let mut raw = vec![1, 0x34, 17, 0, 2, 15];
            raw.extend([0; 15]);
            let mut transport = Transport::new([Step::Raw(raw, 123)]);
            transport.fail_enable = fail_enable;
            let clock = Arc::clone(&transport.clock);
            let writes = Arc::clone(&transport.writes);
            let output = Output::default();
            let actions = Arc::new(Mutex::new(Vec::new()));
            let mut configuration = options();
            configuration.software_mode = true;
            let result = run_with_clock(
                &mut InputReader::new(transport),
                profile(),
                DeviceContext {
                    module: ServiceModule::M4315,
                    serial_number: None,
                },
                false,
                json!({"source":"synthetic"}),
                configuration,
                Arc::new(AtomicBool::new(false)),
                move |stop| Ok((simple_actions(stop, actions), output)),
                || clock.load(Ordering::SeqCst),
            );
            if fail_enable {
                let error = result.unwrap_err();
                assert!(error.contains("enable failed"), "{error}");
                assert!(!error.contains(INPUT_PANIC), "{error}");
            } else {
                result.unwrap();
            }
            assert_eq!(
                *writes.lock().unwrap(),
                vec![
                    cooltek_sdk_masterhub::keypad::software_mode_report(true).to_vec(),
                    cooltek_sdk_masterhub::keypad::software_mode_report(false).to_vec(),
                ]
            );
        }
    }

    #[test]
    fn normal_limit_drains_actions_and_keeps_default_device_access_read_only() {
        let transport = Transport::new([dial(3, 0, 10)]);
        let writes = Arc::clone(&transport.writes);
        let received = Arc::new(Mutex::new(Vec::new()));
        let actions = Arc::clone(&received);
        let output = Output::default();
        let result = run_fake(
            transport,
            options(),
            Arc::new(AtomicBool::new(false)),
            output.clone(),
            move |stop| simple_actions(stop, actions),
        )
        .unwrap();
        assert_eq!(result.report_count, 1);
        assert_eq!(result.processed_report_count, 1);
        assert_eq!(result.action_count, 1);
        assert_eq!(result.incomplete_action_count, 0);
        assert!(writes.lock().unwrap().is_empty());
        assert_eq!(*received.lock().unwrap(), vec![Action::Volume { delta: 5 }]);
        let rows = output.rows();
        // Live event records share replay's shape, including keypad fields.
        let event = rows.iter().find(|row| row["type"] == "event").unwrap();
        for key in [
            "event",
            "binding",
            "action",
            "navigation",
            "page_id",
            "status",
        ] {
            assert!(event.get(key).is_some(), "missing {key}: {event}");
        }
        let executed = rows
            .iter()
            .find(|row| row["type"] == "action_executed")
            .unwrap();
        assert_eq!(executed["schema_version"], 1);
        assert_eq!(rows.last().unwrap()["reason"], "max_reports");
    }

    #[test]
    fn input_reads_and_gesture_timing_continue_during_a_blocked_action() {
        for (held, expected) in [
            (599_999, Action::Volume { delta: 2 }),
            (600_000, Action::ToggleMute),
        ] {
            let (began_tx, began_rx) = mpsc::sync_channel(1);
            let (read_tx, read_rx) = mpsc::sync_channel(1);
            let gate = Arc::new((Mutex::new(false), Condvar::new()));
            let mut transport = Transport::new([dial(4, 1, 10_000), dial(4, 0, 10_000 + held)]);
            transport.wait_second = Some(began_rx);
            transport.signal_second = Some(read_tx);
            let reads = Arc::clone(&transport.reads);
            let release = Arc::clone(&gate);
            let controller = thread::spawn(move || {
                read_rx.recv_timeout(Duration::from_secs(2)).unwrap();
                assert_eq!(reads.load(Ordering::SeqCst), 2);
                *release.0.lock().unwrap() = true;
                release.1.notify_all();
            });
            let received = Arc::new(Mutex::new(Vec::new()));
            let actions = Arc::clone(&received);
            let mut configuration = options();
            configuration.max_reports = Some(2);
            let output = Output::default();
            let summary = run_fake(
                transport,
                configuration,
                Arc::new(AtomicBool::new(false)),
                output.clone(),
                move |stop| {
                    let mut actions = simple_actions(stop, actions);
                    actions.gate = Some(gate);
                    actions.began = Some(began_tx);
                    actions
                },
            )
            .unwrap();
            controller.join().unwrap();
            assert_eq!(
                *received.lock().unwrap(),
                vec![Action::Volume { delta: 1 }, expected]
            );
            assert_eq!(summary.decoded_event_count, 3);
            assert_eq!(summary.event_count, 3);
            let gesture = output
                .rows()
                .into_iter()
                .rfind(|row| row["type"] == "event")
                .unwrap();
            assert_eq!(gesture["event"]["timestamp_micros"], 10_000 + held);
        }
    }

    #[test]
    fn burst_beyond_queue_capacity_is_coalesced_without_ending_the_session() {
        let (began_tx, began_rx) = mpsc::sync_channel(1);
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let mut transport = Transport::new((1..=10).map(|time| dial(3, 0, time)));
        transport.wait_second = Some(began_rx);
        let reads = Arc::clone(&transport.reads);
        let writes = Arc::clone(&transport.writes);
        let release = Arc::clone(&gate);
        // Hold the first action until every report has been read, so reports
        // 3..=10 arrive while the capacity-1 queue is full.
        let controller = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(2);
            while reads.load(Ordering::SeqCst) < 10 && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(1));
            }
            *release.0.lock().unwrap() = true;
            release.1.notify_all();
        });
        let mut configuration = options();
        configuration.max_reports = Some(10);
        configuration.software_mode = true;
        configuration.queue_capacity = 1;
        let received = Arc::new(Mutex::new(Vec::new()));
        let actions = Arc::clone(&received);
        let output = Output::default();
        let summary = run_fake(
            transport,
            configuration,
            Arc::new(AtomicBool::new(false)),
            output.clone(),
            move |stop| {
                let mut actions = simple_actions(stop, actions);
                actions.gate = Some(gate);
                actions.began = Some(began_tx);
                actions
            },
        )
        .unwrap();
        controller.join().unwrap();
        assert_eq!(
            *received.lock().unwrap(),
            vec![
                Action::Volume { delta: 5 },
                Action::Volume { delta: 5 },
                Action::Volume { delta: 40 },
            ]
        );
        assert_eq!(summary.report_count, 10);
        assert_eq!(summary.processed_report_count, 10);
        assert_eq!(summary.coalesced_report_count, 7);
        assert_eq!(summary.coalesced_event_count, 7);
        assert_eq!(summary.dropped_event_count, 0);
        assert_eq!(summary.event_count, 10);
        assert_eq!(summary.action_count, 10);
        assert_eq!(summary.incomplete_action_count, 0);
        assert_eq!(*writes.lock().unwrap(), modes());
        let rows = output.rows();
        let coalesced = rows
            .iter()
            .find(|row| row["type"] == "report" && row["coalesced_report_count"] == 8)
            .unwrap();
        assert_eq!(coalesced["timestamp_micros"], 10);
        let event = rows
            .iter()
            .find(|row| row["type"] == "event" && row["coalesced_event_count"] == 8)
            .unwrap();
        assert_eq!(event["action"], json!({"type":"volume","delta":40}));
        let footer = rows.last().unwrap();
        assert_eq!(footer["reason"], "max_reports");
        assert_eq!(footer["discarded_report_count"], 0);
        assert_eq!(footer["coalesced_report_count"], 7);
    }

    fn planned(control: u8, gesture: Gesture, resolution: ProfileResolution) -> PlannedEvent {
        PlannedEvent {
            event: cooltek_sdk_service::InputEvent {
                timestamp_micros: 0,
                control,
                gesture,
                value: None,
            },
            binding: None,
            resolution,
            dial_function: None,
            coalesced: 1,
        }
    }

    fn action(action: Action) -> ProfileResolution {
        ProfileResolution::Action {
            action,
            page_id: None,
            dial_function: None,
        }
    }

    fn work(events: Vec<PlannedEvent>) -> Work {
        Work {
            raw: vec![1],
            timestamp_micros: 0,
            status: "decoded",
            error: None,
            events,
            report_count: 1,
            coalesced_parse_error_count: 0,
            dropped_event_count: 0,
            dropped_dial_function_count: 0,
            merge_barrier: false,
        }
    }

    #[test]
    fn coalescing_sums_steps_keeps_last_position_and_never_merges_buttons() {
        let volume = |delta| planned(0, Gesture::Clockwise, action(Action::Volume { delta }));
        let merged = merge_event(&volume(60), &volume(60)).unwrap();
        assert_eq!(merged.resolution, action(Action::Volume { delta: 100 }));
        assert_eq!(merged.coalesced, 2);
        let position = |percent| {
            planned(
                1,
                Gesture::PositionChanged,
                action(Action::SetVolume { percent }),
            )
        };
        assert_eq!(
            merge_event(&position(10), &position(70))
                .unwrap()
                .resolution,
            action(Action::SetVolume { percent: 70 })
        );
        let press = planned(0, Gesture::Pressed, action(Action::ToggleMute));
        assert!(merge_event(&press, &press).is_none());
        let counter = planned(
            0,
            Gesture::CounterClockwise,
            action(Action::Volume { delta: -5 }),
        );
        assert!(merge_event(&volume(5), &counter).is_none());
        let shortcut = planned(
            0,
            Gesture::Clockwise,
            action(Action::Shortcut {
                key: "v".into(),
                modifiers: vec![],
            }),
        );
        assert!(merge_event(&shortcut, &shortcut).is_none());
    }

    #[test]
    fn coalescing_preserves_per_control_order_and_drops_beyond_the_bound() {
        let mut counts = Finish::default();
        let step = || planned(0, Gesture::Clockwise, action(Action::Volume { delta: 5 }));
        let press = || planned(0, Gesture::Pressed, action(Action::ToggleMute));
        let mut pending = work(vec![step()]);
        coalesce(&mut pending, work(vec![press(), step()]), &mut counts);
        // A press between two steps prevents merging across it.
        assert_eq!(pending.events.len(), 3);
        coalesce(&mut pending, work(vec![step()]), &mut counts);
        assert_eq!(pending.events.len(), 3);
        assert_eq!(pending.events[2].coalesced, 2);
        let presses = (0..MAX_PENDING_EVENTS).map(|_| press()).collect();
        coalesce(&mut pending, work(presses), &mut counts);
        assert_eq!(pending.events.len(), MAX_PENDING_EVENTS);
        assert_eq!(pending.dropped_event_count, 3);
        assert_eq!(counts.dropped_events, 3);
        assert_eq!(pending.report_count, 4);
        assert_eq!(counts.coalesced_reports, 3);
    }

    #[test]
    fn coalescing_never_reorders_events_across_controls() {
        let mut counts = Finish::default();
        let step = |control| {
            planned(
                control,
                Gesture::Clockwise,
                action(Action::Volume { delta: 5 }),
            )
        };
        let mut pending = work(vec![step(0)]);
        coalesce(&mut pending, work(vec![step(1)]), &mut counts);
        coalesce(&mut pending, work(vec![step(0)]), &mut counts);
        let controls: Vec<_> = pending.events.iter().map(|e| e.event.control).collect();
        assert_eq!(controls, vec![0, 1, 0]);
        assert_eq!(counts.coalesced_events, 0);
        coalesce(&mut pending, work(vec![step(0)]), &mut counts);
        assert_eq!(pending.events.len(), 3);
        assert_eq!(pending.events[2].coalesced, 2);
    }

    fn dial_functions_profile(functions: &str) -> ProfileSession {
        let json = format!(
            r#"{{"schema_version":1,"name":"dial","bindings":[],"dial_functions":{functions}}}"#
        );
        ProfileSession::new(Profile::from_json(json.as_bytes()).unwrap()).unwrap()
    }

    fn two_functions() -> ProfileSession {
        dial_functions_profile(
            r#"[
            {"name":"Coarse","bindings":[{"gesture":"right","action":{"type":"volume","delta":5}}]},
            {"name":"Fine","bindings":[{"gesture":"right","action":{"type":"volume","delta":1}},
                {"gesture":"long_press","action":{"type":"toggle_mute"}}]}
        ]"#,
        )
    }

    /// Plans raw dial reports through the real decoder and profile session.
    fn plan_dial(
        input: &mut InputSession,
        session: &mut ProfileSession,
        selector: u8,
        value: u8,
        time: u64,
    ) -> Work {
        plan_report(
            input,
            session,
            &device(),
            &[1, 0x44, 3, 0, selector, 0, value],
            time,
        )
        .unwrap()
    }

    fn function_actions(pending: &Work) -> Vec<(Option<String>, Action)> {
        pending
            .events
            .iter()
            .filter_map(|planned| match &planned.resolution {
                ProfileResolution::Action {
                    action,
                    dial_function,
                    ..
                } => Some((dial_function.clone(), action.clone())),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn coalescing_never_sums_rotations_across_a_short_click() {
        let mut input = InputSession::new(ServiceModule::M131E);
        let mut session = two_functions();
        let mut counts = Finish::default();
        let mut pending = plan_dial(&mut input, &mut session, 3, 0, 1);
        for work in [
            plan_dial(&mut input, &mut session, 3, 0, 2),
            plan_dial(&mut input, &mut session, 4, 1, 3),
            plan_dial(&mut input, &mut session, 4, 0, 100_003),
            plan_dial(&mut input, &mut session, 3, 0, 100_004),
            plan_dial(&mut input, &mut session, 3, 0, 100_005),
        ] {
            coalesce(&mut pending, work, &mut counts);
        }
        assert_eq!(
            function_actions(&pending),
            vec![
                (Some("Coarse".into()), Action::Volume { delta: 10 }),
                (Some("Fine".into()), Action::Volume { delta: 2 }),
            ]
        );
        let switches: Vec<_> = pending
            .events
            .iter()
            .filter(|planned| planned.is_dial_function_switch())
            .collect();
        assert_eq!(switches.len(), 1);
        assert!(switches[0].action().is_none());
        assert_eq!(counts.coalesced_events, 2);
        assert_eq!(counts.dropped_dial_functions, 0);
    }

    #[test]
    fn steps_from_different_functions_never_merge_even_when_adjacent() {
        let step = |function: &str| PlannedEvent {
            dial_function: Some(crate::service_events::ActiveDialFunction {
                name: function.into(),
                index: 0,
            }),
            resolution: ProfileResolution::Action {
                action: Action::Volume { delta: 5 },
                page_id: None,
                dial_function: Some(function.into()),
            },
            ..planned(0, Gesture::Right, action(Action::Volume { delta: 5 }))
        };
        assert!(merge_event(&step("A"), &step("B")).is_none());
        let merged = merge_event(&step("A"), &step("A")).unwrap();
        assert_eq!(merged.dial_function.unwrap().name, "A");
    }

    #[test]
    fn a_switch_dropped_under_pressure_is_counted_and_blocks_merging() {
        // One function: both sides share a name, so only the barrier separates them.
        let mut session = dial_functions_profile(
            r#"[{"name":"Only","bindings":[{"gesture":"right","action":{"type":"volume","delta":5}}]}]"#,
        );
        let mut input = InputSession::new(ServiceModule::M131E);
        let mut counts = Finish::default();
        let mut pending = plan_dial(&mut input, &mut session, 3, 0, 1);
        let filler = (1..MAX_PENDING_EVENTS)
            .map(|_| planned(0, Gesture::Pressed, ProfileResolution::Unbound))
            .collect();
        coalesce(&mut pending, work(filler), &mut counts);
        // Keep a rotation last so a later rotation could otherwise merge into it.
        pending.events.pop();
        coalesce(
            &mut pending,
            plan_dial(&mut input, &mut session, 3, 0, 2),
            &mut counts,
        );
        assert_eq!(pending.events.len(), MAX_PENDING_EVENTS);
        assert_eq!(pending.events.last().unwrap().coalesced, 1);
        for work in [
            plan_dial(&mut input, &mut session, 4, 1, 3),
            plan_dial(&mut input, &mut session, 4, 0, 100_003),
            plan_dial(&mut input, &mut session, 3, 0, 100_004),
        ] {
            coalesce(&mut pending, work, &mut counts);
        }
        assert_eq!(counts.dropped_dial_functions, 1);
        assert_eq!(pending.dropped_dial_function_count, 1);
        // pressed, released, short_press, and the rotation after the switch.
        assert_eq!(counts.dropped_events, 4);
        assert_eq!(pending.events.last().unwrap().coalesced, 1);
        assert!(pending.merge_barrier);
    }

    #[test]
    fn live_rotation_click_rotation_dispatches_each_functions_action_under_coalescing() {
        let (began_tx, began_rx) = mpsc::sync_channel(1);
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let steps = vec![
            dial(3, 0, 1),
            dial(3, 0, 2),
            dial(3, 0, 3),
            dial(3, 0, 4),
            dial(4, 1, 5),
            dial(4, 0, 100_005),
            dial(3, 0, 100_006),
            dial(3, 0, 100_007),
            dial(4, 1, 200_000),
            dial(4, 0, 900_000),
        ];
        let total = steps.len();
        let mut transport = Transport::new(steps);
        transport.wait_second = Some(began_rx);
        let reads = Arc::clone(&transport.reads);
        let clock = Arc::clone(&transport.clock);
        let release = Arc::clone(&gate);
        let controller = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(2);
            while reads.load(Ordering::SeqCst) < total && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(1));
            }
            *release.0.lock().unwrap() = true;
            release.1.notify_all();
        });
        let mut configuration = options();
        configuration.max_reports = Some(total as u64);
        configuration.queue_capacity = 1;
        let received = Arc::new(Mutex::new(Vec::new()));
        let actions = Arc::clone(&received);
        let output = Output::default();
        let writer = output.clone();
        let summary = run_with_clock(
            &mut InputReader::new(transport),
            two_functions(),
            device(),
            true,
            json!({"source":"synthetic"}),
            configuration,
            Arc::new(AtomicBool::new(false)),
            move |stop| {
                let mut actions = simple_actions(stop, actions);
                actions.gate = Some(gate);
                actions.began = Some(began_tx);
                Ok((actions, writer))
            },
            move || clock.load(Ordering::SeqCst),
        )
        .unwrap();
        controller.join().unwrap();
        assert_eq!(
            *received.lock().unwrap(),
            vec![
                Action::Volume { delta: 5 },
                Action::Volume { delta: 5 },
                Action::Volume { delta: 10 },
                Action::Volume { delta: 2 },
                Action::ToggleMute,
            ]
        );
        assert_eq!(summary.dial_function_switch_count, 1);
        assert_eq!(summary.dropped_dial_function_count, 0);
        let rows = output.rows();
        let start = &rows[0];
        assert_eq!(
            start["details"]["dial_functions"]["names"],
            json!(["Coarse", "Fine"])
        );
        assert_eq!(
            start["details"]["dial_functions"]["resets_on_restart"],
            true
        );
        let switch: Vec<_> = rows
            .iter()
            .filter(|row| row["status"] == "dial_function")
            .collect();
        assert_eq!(switch.len(), 1);
        assert_eq!(
            switch[0]["navigation"],
            json!({"from_function":"Coarse","to_function":"Fine","index":1,"count":2})
        );
        assert!(switch[0]["action"].is_null());
        let functions: Vec<_> = rows
            .iter()
            .filter(|row| row["type"] == "action_executed")
            .map(|row| row["dial_function"]["name"].clone())
            .collect();
        assert_eq!(
            functions,
            vec![
                json!("Coarse"),
                json!("Coarse"),
                json!("Coarse"),
                json!("Fine"),
                json!("Fine")
            ]
        );
        // Five executed actions for five planned events; the click added none.
        assert_eq!(functions.len(), 5);
        let footer = rows.last().unwrap();
        assert_eq!(footer["dial_function_switch_count"], 1);
        assert_eq!(footer["dropped_dial_function_count"], 0);
    }

    #[test]
    fn enable_unplug_and_cleanup_failures_retain_original_causes() {
        for (fail_enable, fail_cleanup, source) in [
            (true, false, "enable failed"),
            (false, false, "synthetic unplug"),
            (false, true, "synthetic unplug"),
        ] {
            let mut transport = Transport::new([Step::Unplug]);
            transport.fail_enable = fail_enable;
            transport.fail_cleanup = fail_cleanup;
            let writes = Arc::clone(&transport.writes);
            let mut configuration = options();
            configuration.software_mode = true;
            let error = run_fake(
                transport,
                configuration,
                Arc::new(AtomicBool::new(false)),
                Output::default(),
                |stop| simple_actions(stop, Arc::new(Mutex::new(Vec::new()))),
            )
            .unwrap_err();
            assert!(error.contains(source));
            assert_eq!(error.contains("cleanup failed"), fail_cleanup);
            assert_eq!(*writes.lock().unwrap(), modes());
        }
    }

    #[test]
    fn read_timeouts_honor_idle_deadline_and_restore_mode() {
        let transport = Transport::new([]);
        let writes = Arc::clone(&transport.writes);
        let timeouts = Arc::clone(&transport.timeouts);
        let mut configuration = options();
        configuration.software_mode = true;
        configuration.duration = Some(Duration::from_millis(15));
        configuration.timeout_ms = 100;
        let result = run_fake(
            transport,
            configuration,
            Arc::new(AtomicBool::new(false)),
            Output::default(),
            |stop| simple_actions(stop, Arc::new(Mutex::new(Vec::new()))),
        )
        .unwrap();
        assert_eq!(result.report_count, 0);
        assert!(
            timeouts
                .lock()
                .unwrap()
                .iter()
                .all(|timeout| (1..=15).contains(timeout))
        );
        assert_eq!(*writes.lock().unwrap(), modes());
    }

    #[test]
    fn interrupted_action_drops_remaining_actions_from_the_same_report() {
        let engine = ProfileSession::new(Profile::from_json(br#"{"schema_version":1,"name":"test","bindings":[
            {"module":"m003k","control":0,"gesture":"clockwise","action":{"type":"volume","delta":5}},
            {"module":"m003k","control":1,"gesture":"clockwise","action":{"type":"volume","delta":5}}
        ]}"#).unwrap()).unwrap();
        let received = Arc::new(Mutex::new(Vec::new()));
        let actions = Arc::clone(&received);
        let mut reader = InputReader::new(Transport::new([Step::Raw(
            vec![1, 0x2a, 5, 0, 0, 3, 1, 1, 0xff],
            1,
        )]));
        let output = Output::default();
        let result = run_with_clock(
            &mut reader,
            engine,
            DeviceContext {
                module: ServiceModule::M003K,
                serial_number: None,
            },
            true,
            json!({}),
            options(),
            Arc::new(AtomicBool::new(false)),
            move |stop| {
                let mut actions = simple_actions(stop, actions);
                actions.interrupt = true;
                Ok((actions, output))
            },
            || 1,
        )
        .unwrap();
        assert_eq!(received.lock().unwrap().len(), 1);
        assert_eq!(result.incomplete_action_count, 1);
    }

    #[test]
    fn finite_drain_timeout_cancels_the_current_action_and_reports_discarded_work() {
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let mut configuration = options();
        configuration.drain_timeout = Duration::from_millis(15);
        configuration.software_mode = true;
        let transport = Transport::new([dial(3, 0, 1)]);
        let writes = Arc::clone(&transport.writes);
        let output = Output::default();
        let error = run_fake(
            transport,
            configuration,
            Arc::new(AtomicBool::new(false)),
            output.clone(),
            move |stop| {
                let mut actions = simple_actions(stop, Arc::new(Mutex::new(Vec::new())));
                actions.gate = Some(gate);
                actions
            },
        )
        .unwrap_err();
        assert!(error.contains("drain deadline exceeded"));
        let footer = output.rows().last().unwrap().clone();
        assert_eq!(footer["reason"], "error");
        assert_eq!(footer["action_count"], 0);
        assert_eq!(footer["incomplete_action_count"], 1);
        assert_eq!(*writes.lock().unwrap(), modes());
    }

    #[test]
    fn dispatcher_failure_is_logged_and_the_session_continues_by_default() {
        let received = Arc::new(Mutex::new(Vec::new()));
        let actions = Arc::clone(&received);
        let mut configuration = options();
        configuration.max_reports = Some(3);
        configuration.software_mode = true;
        let transport = Transport::new([dial(3, 0, 1), dial(3, 0, 2), dial(3, 0, 3)]);
        let writes = Arc::clone(&transport.writes);
        let output = Output::default();
        let summary = run_fake(
            transport,
            configuration,
            Arc::new(AtomicBool::new(false)),
            output.clone(),
            move |stop| {
                let mut actions = simple_actions(stop, actions);
                actions.fail = true;
                actions
            },
        )
        .unwrap();
        assert_eq!(received.lock().unwrap().len(), 3);
        assert_eq!(summary.failed_action_count, 3);
        assert_eq!(summary.action_count, 0);
        assert_eq!(summary.processed_report_count, 3);
        assert_eq!(*writes.lock().unwrap(), modes());
        let rows = output.rows();
        let failed: Vec<_> = rows
            .iter()
            .filter(|row| row["type"] == "action_failed")
            .collect();
        assert_eq!(failed.len(), 3);
        assert!(failed.iter().all(|row| row["schema_version"] == 1));
        let footer = rows.last().unwrap();
        assert_eq!(footer["reason"], "max_reports");
        assert_eq!(footer["failed_action_count"], 3);
    }

    #[test]
    fn stop_on_action_error_restores_the_failure_stops_session_behavior() {
        let received = Arc::new(Mutex::new(Vec::new()));
        let actions = Arc::clone(&received);
        let mut configuration = options();
        configuration.max_reports = Some(3);
        configuration.stop_on_action_error = true;
        let error = run_fake(
            Transport::new([dial(3, 0, 1), dial(3, 0, 2), dial(3, 0, 3)]),
            configuration,
            Arc::new(AtomicBool::new(false)),
            Output::default(),
            move |stop| {
                let mut actions = simple_actions(stop, actions);
                actions.fail = true;
                actions
            },
        )
        .unwrap_err();
        assert!(error.contains("synthetic action failed"));
        assert_eq!(received.lock().unwrap().len(), 1);
    }

    #[test]
    fn output_failure_prevents_an_unlogged_action_and_restores_mode() {
        struct FailedOutput {
            lines: usize,
        }
        impl Write for FailedOutput {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                self.lines += 1;
                if self.lines > 1 {
                    Err(io::Error::other("synthetic output failure"))
                } else {
                    Ok(bytes.len())
                }
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let transport = Transport::new([dial(3, 0, 1)]);
        let writes = Arc::clone(&transport.writes);
        let received = Arc::new(Mutex::new(Vec::new()));
        let actions = Arc::clone(&received);
        let mut configuration = options();
        configuration.software_mode = true;
        let error = run_live(
            InputReader::new(transport),
            profile(),
            device(),
            true,
            json!({}),
            configuration,
            Arc::new(AtomicBool::new(false)),
            move |stop| Ok((simple_actions(stop, actions), FailedOutput { lines: 0 })),
        )
        .unwrap_err();
        assert!(error.contains("synthetic output failure"));
        assert!(received.lock().unwrap().is_empty());
        assert_eq!(*writes.lock().unwrap(), modes());
    }

    #[test]
    fn worker_panic_is_detected_even_without_further_input() {
        let mut configuration = options();
        configuration.max_reports = None;
        let started = Instant::now();
        let output = Output::default();
        let error = run_fake(
            Transport::new([dial(3, 0, 1)]),
            configuration,
            Arc::new(AtomicBool::new(false)),
            output.clone(),
            |stop| {
                let mut actions = simple_actions(stop, Arc::new(Mutex::new(Vec::new())));
                actions.panic = true;
                actions
            },
        )
        .unwrap_err();
        assert!(error.contains("worker panicked"));
        assert!(started.elapsed() < Duration::from_secs(2));
        // The writer survives a dispatch panic, so the session still has a footer.
        let footer = output.rows().last().unwrap().clone();
        assert_eq!(footer["type"], "service_end");
        assert_eq!(footer["reason"], "error");
        assert!(
            footer["error"]
                .as_str()
                .unwrap()
                .contains("worker panicked")
        );
        assert_eq!(footer["incomplete_action_count"], 1);
    }

    #[test]
    fn read_error_after_an_interrupt_ends_as_interrupted_and_restores_mode() {
        struct SignalledRead {
            flag: Arc<AtomicBool>,
            writes: Arc<Mutex<Vec<Vec<u8>>>>,
        }
        impl ReportTransport for SignalledRead {
            fn read_timeout(&self, _: &mut [u8], _: i32) -> HidResult<usize> {
                self.flag.store(true, Ordering::SeqCst);
                Err(hid_error("read interrupted"))
            }
            fn get_feature_report(&self, _: &mut [u8]) -> HidResult<usize> {
                panic!("runtime must not query features")
            }
            fn send_feature_report(&self, bytes: &[u8]) -> HidResult<()> {
                self.writes.lock().unwrap().push(bytes.to_vec());
                Ok(())
            }
        }
        let flag = Arc::new(AtomicBool::new(false));
        let writes = Arc::new(Mutex::new(Vec::new()));
        let output = Output::default();
        let writer = output.clone();
        let mut configuration = options();
        configuration.software_mode = true;
        run_with_clock(
            &mut InputReader::new(SignalledRead {
                flag: Arc::clone(&flag),
                writes: Arc::clone(&writes),
            }),
            profile(),
            device(),
            false,
            json!({}),
            configuration,
            flag,
            move |stop| {
                Ok((
                    simple_actions(stop, Arc::new(Mutex::new(Vec::new()))),
                    writer,
                ))
            },
            || 0,
        )
        .unwrap();
        let footer = output.rows().last().unwrap().clone();
        assert_eq!(footer["reason"], "interrupted");
        assert!(footer["error"].is_null());
        assert_eq!(*writes.lock().unwrap(), modes());
    }

    #[test]
    fn worker_startup_uses_its_own_timeout_rather_than_the_cancellation_budget() {
        let mut configuration = options();
        configuration.cancellation_timeout = Duration::from_millis(10);
        configuration.startup_timeout = Duration::from_secs(2);
        let output = Output::default();
        let writer = output.clone();
        let clock = Arc::new(AtomicU64::new(0));
        let time = Arc::clone(&clock);
        run_with_clock(
            &mut InputReader::new(Transport::new([dial(3, 0, 1)])),
            profile(),
            device(),
            false,
            json!({}),
            configuration,
            Arc::new(AtomicBool::new(false)),
            move |stop| {
                thread::sleep(Duration::from_millis(50));
                Ok((
                    simple_actions(stop, Arc::new(Mutex::new(Vec::new()))),
                    writer,
                ))
            },
            move || time.load(Ordering::SeqCst),
        )
        .unwrap();
        assert_eq!(output.rows().last().unwrap()["reason"], "max_reports");
        let mut invalid = options();
        invalid.startup_timeout = Duration::ZERO;
        assert!(validate_options(invalid, &device()).is_err());
    }

    #[test]
    fn worker_await_shares_the_drain_deadline_already_spent_on_flushing() {
        let (_done_tx, done_rx) = mpsc::sync_channel::<WorkerOutcome<Output>>(1);
        let abort = AtomicBool::new(false);
        let started = Instant::now();
        // The deadline was consumed while flushing; waiting must not restart it.
        let error = await_worker(&done_rx, &abort, Some(started), Duration::from_millis(20))
            .err()
            .unwrap();
        assert!(error.contains("drain deadline exceeded"));
        assert!(abort.load(Ordering::SeqCst));
        assert!(started.elapsed() < Duration::from_millis(500));
    }

    #[test]
    fn invalid_configuration_never_constructs_the_worker_or_writes_modes() {
        let mut invalid = options();
        invalid.queue_capacity = 0;
        let error = run_live(
            InputReader::new(Transport::new([])),
            profile(),
            device(),
            true,
            json!({}),
            invalid,
            Arc::new(AtomicBool::new(false)),
            |_| -> Result<(Actions, Output), String> { panic!("must not construct worker") },
        )
        .unwrap_err();
        assert!(error.contains("queue capacity"));
    }

    #[test]
    fn rapid_input_burst_keeps_owned_bytes_and_complete_ordered_counts() {
        let transport = Transport::new((0..20).map(|index| dial(3, (index % 2) as u8, index + 1)));
        let output = Output::default();
        let mut configuration = options();
        configuration.max_reports = Some(20);
        let summary = run_fake(
            transport,
            configuration,
            Arc::new(AtomicBool::new(false)),
            output.clone(),
            |stop| simple_actions(stop, Arc::new(Mutex::new(Vec::new()))),
        )
        .unwrap();
        assert_eq!(summary.report_count, 20);
        assert_eq!(summary.processed_report_count, 20);
        assert_eq!(summary.event_count, 20);
        assert_eq!(summary.action_count, 10);
        assert_eq!(summary.incomplete_action_count, 0);
        let rows = output.rows();
        let reports: Vec<_> = rows.iter().filter(|row| row["type"] == "report").collect();
        for (index, report) in reports.iter().enumerate() {
            assert_eq!(
                report["raw_hex"],
                if index % 2 == 0 {
                    "01440300030000"
                } else {
                    "01440300030001"
                }
            );
            assert_eq!(report["timestamp_micros"], index + 1);
        }
        assert_eq!(rows.last().unwrap()["discarded_report_count"], 0);
    }

    #[test]
    fn uncooperative_dispatcher_is_reported_without_an_unbounded_join_and_cleanup_still_runs() {
        struct Uncooperative {
            gate: Gate,
            dropped: SyncSender<()>,
        }
        impl ActionDispatcher for Uncooperative {
            fn dispatch(&mut self, _: &Action) -> Result<(), String> {
                let mut released = self.gate.0.lock().unwrap();
                while !*released {
                    released = self.gate.1.wait(released).unwrap();
                }
                Ok(())
            }
        }
        impl Drop for Uncooperative {
            fn drop(&mut self) {
                let _ = self.dropped.send(());
            }
        }
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let worker_gate = Arc::clone(&gate);
        let (dropped_tx, dropped_rx) = mpsc::sync_channel(1);
        let transport = Transport::new([dial(3, 0, 1)]);
        let writes = Arc::clone(&transport.writes);
        let mut configuration = options();
        configuration.software_mode = true;
        configuration.drain_timeout = Duration::from_millis(15);
        configuration.cancellation_timeout = Duration::from_millis(100);
        let started = Instant::now();
        let error = run_live(
            InputReader::new(transport),
            profile(),
            device(),
            true,
            json!({}),
            configuration,
            Arc::new(AtomicBool::new(false)),
            move |_| {
                Ok((
                    Uncooperative {
                        gate: worker_gate,
                        dropped: dropped_tx,
                    },
                    Output::default(),
                ))
            },
        )
        .unwrap_err();
        // Release the intentionally broken test adapter before asserting, so
        // the detached test worker cannot outlive this test.
        *gate.0.lock().unwrap() = true;
        gate.1.notify_all();
        dropped_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(error.contains("thread was not joined"));
        assert!(started.elapsed() < Duration::from_secs(2));
        assert_eq!(*writes.lock().unwrap(), modes());
    }

    use crate::display_artwork::PreparedKeyArtwork;
    use cooltek_sdk_service::display_sync::FakeDisplaySink;

    type WriteHook = Box<dyn FnMut(usize) + Send>;

    /// A [`FakeDisplaySink`] observable from the test thread, with an optional
    /// hook that runs before each attempt and may block like a stalled device.
    struct SharedSink {
        fake: Arc<Mutex<FakeDisplaySink>>,
        before_write: Option<WriteHook>,
    }
    impl DisplaySink for SharedSink {
        fn capabilities(&self) -> DisplayCapabilities {
            self.fake.lock().unwrap().capabilities
        }
        fn write_key_image(
            &mut self,
            key_index: u8,
            jpeg: &[u8],
            between_chunks: &mut dyn FnMut(usize),
        ) -> Result<(), SinkError> {
            let attempts = self.fake.lock().unwrap().writes.len();
            if let Some(hook) = &mut self.before_write {
                hook(attempts);
            }
            self.fake
                .lock()
                .unwrap()
                .write_key_image(key_index, jpeg, between_chunks)
        }
        fn write_dial_image(
            &mut self,
            _: &[u8],
            _: &mut dyn FnMut(usize),
        ) -> Result<(), SinkError> {
            panic!("keypad display sync never writes the dial")
        }
    }

    fn keypad_profile() -> ProfileSession {
        ProfileSession::new(
            Profile::from_json(include_bytes!(
                "../../../examples/profiles/keypad-folders.json"
            ))
            .unwrap(),
        )
        .unwrap()
    }
    fn keypad_device() -> DeviceContext {
        DeviceContext {
            module: ServiceModule::M4315,
            serial_number: None,
        }
    }
    fn keys(pressed: &[usize], time: u64) -> Step {
        let mut raw = vec![1, 0x34, 17, 0, 2, 15];
        raw.extend([0; 15]);
        for key in pressed {
            raw[6 + key] = 1;
        }
        Step::Raw(raw, time)
    }
    fn wait_until(mut condition: impl FnMut() -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(2);
        while !condition() {
            if Instant::now() >= deadline {
                return false;
            }
            thread::sleep(Duration::from_millis(1));
        }
        true
    }
    fn written(fake: &Arc<Mutex<FakeDisplaySink>>) -> usize {
        fake.lock().unwrap().writes.len()
    }
    fn wait_for_writes(fake: &Arc<Mutex<FakeDisplaySink>>, count: usize) -> Step {
        let fake = Arc::clone(fake);
        Step::Run(Box::new(move || {
            assert!(wait_until(|| written(&fake) >= count), "display writes");
        }))
    }
    fn keypad_options(max_reports: u64) -> LiveOptions {
        LiveOptions {
            software_mode: true,
            max_reports: Some(max_reports),
            ..options()
        }
    }
    struct DisplayRun {
        result: Result<LiveSummary, String>,
        rows: Vec<Value>,
        actions: Vec<Action>,
        mode_writes: Vec<Vec<u8>>,
    }
    fn run_display(
        steps: Vec<Step>,
        options: LiveOptions,
        interrupted: Arc<AtomicBool>,
        display: Option<LiveDisplay>,
        received: Arc<Mutex<Vec<Action>>>,
    ) -> DisplayRun {
        let transport = Transport::new(steps);
        let clock = Arc::clone(&transport.clock);
        let writes = Arc::clone(&transport.writes);
        let output = Output::default();
        let writer = output.clone();
        let actions = Arc::clone(&received);
        let result = run_inner(
            &mut InputReader::new(transport),
            keypad_profile(),
            keypad_device(),
            false,
            json!({"source":"synthetic"}),
            options,
            interrupted,
            display,
            move |stop| Ok((simple_actions(stop, actions), writer)),
            move || clock.load(Ordering::SeqCst),
        );
        let actions = received.lock().unwrap().clone();
        let mode_writes = writes.lock().unwrap().clone();
        DisplayRun {
            result,
            rows: output.rows(),
            actions,
            mode_writes,
        }
    }
    fn display_rows(rows: &[Value]) -> Vec<&Value> {
        rows.iter()
            .filter(|row| row["type"] == "display_sync")
            .collect()
    }
    fn keypad_modes() -> Vec<Vec<u8>> {
        vec![
            cooltek_sdk_masterhub::keypad::software_mode_report(true).to_vec(),
            cooltek_sdk_masterhub::keypad::software_mode_report(false).to_vec(),
        ]
    }
    fn shared_display(fake: &Arc<Mutex<FakeDisplaySink>>, hook: Option<WriteHook>) -> LiveDisplay {
        LiveDisplay::new(
            SharedSink {
                fake: Arc::clone(fake),
                before_write: hook,
            },
            PreparedKeyArtwork::prepare().unwrap(),
            Arc::new(AtomicBool::new(false)),
        )
    }

    #[test]
    fn without_a_display_a_keypad_run_logs_no_display_records_or_fields() {
        let run = run_display(
            vec![keys(&[1], 10), keys(&[], 20)],
            keypad_options(2),
            Arc::new(AtomicBool::new(false)),
            None,
            Arc::default(),
        );
        let summary = run.result.unwrap();
        assert_eq!(summary.display_sync_count, 0);
        assert!(display_rows(&run.rows).is_empty());
        let footer = run.rows.last().unwrap();
        assert!(footer.get("display_sync_count").is_none());
        assert!(run.rows[0]["details"].get("display_sync").is_none());
    }

    #[test]
    fn display_requires_software_mode_an_m4315_and_a_keypad_profile_before_any_worker() {
        let fake = Arc::new(Mutex::new(FakeDisplaySink::default()));
        for (options, device, session, expected) in [
            (
                options(),
                keypad_device(),
                keypad_profile(),
                "software mode",
            ),
            (keypad_options(1), device(), keypad_profile(), "M4315"),
            (keypad_options(1), keypad_device(), profile(), "schema-2"),
        ] {
            let error = run_inner(
                &mut InputReader::new(Transport::new([])),
                session,
                device,
                false,
                json!({}),
                options,
                Arc::new(AtomicBool::new(false)),
                Some(shared_display(&fake, None)),
                |_| -> Result<(Actions, Output), String> { panic!("must not construct worker") },
                || 0,
            )
            .unwrap_err();
            assert!(error.contains(expected), "{error}");
        }
        assert!(fake.lock().unwrap().writes.is_empty());
    }

    #[test]
    fn first_sync_after_software_mode_draws_fifteen_keys_and_navigation_redraws_the_diff() {
        // The expected navigation diff, from an offline sync of the same pages.
        let mut offline = DisplaySync::new(FakeDisplaySink::default());
        let artwork = PreparedKeyArtwork::prepare().unwrap();
        let mut session = keypad_profile();
        offline.sync_page(&session, &artwork).unwrap();
        session
            .set_path(&["root".into(), "development".into()])
            .unwrap();
        let diff: Vec<u8> = offline
            .plan_page(&session, &artwork)
            .unwrap()
            .writes()
            .iter()
            .map(|write| write.request.key_index)
            .collect();
        assert!(!diff.is_empty() && diff.len() < 15);

        let fake = Arc::new(Mutex::new(FakeDisplaySink::default()));
        let run = run_display(
            vec![
                wait_for_writes(&fake, 15),
                keys(&[1], 10),
                wait_for_writes(&fake, 15 + diff.len()),
                keys(&[], 20),
            ],
            keypad_options(2),
            Arc::new(AtomicBool::new(false)),
            Some(shared_display(&fake, None)),
            Arc::default(),
        );
        let summary = run.result.unwrap();
        assert_eq!(run.mode_writes, keypad_modes());
        let targets = fake.lock().unwrap().successful();
        let expected: Vec<_> = (0..15)
            .chain(diff.iter().copied())
            .map(DisplayTarget::Key)
            .collect();
        assert_eq!(targets, expected);
        let records = display_rows(&run.rows);
        assert_eq!(records.len(), 2);
        assert_eq!(records[0]["trigger"], "software_mode");
        assert_eq!(records[0]["schema_version"], 1);
        assert_eq!(records[0]["page_id"], "root");
        assert_eq!(records[0]["written"], json!((0..15).collect::<Vec<u8>>()));
        assert_eq!(records[0]["complete"], true);
        assert_eq!(records[1]["trigger"], "navigation");
        assert_eq!(records[1]["page_id"], "development");
        assert_eq!(records[1]["depth"], 1);
        assert_eq!(records[1]["written"], json!(diff));
        assert_eq!(records[1]["missing"], json!([]));
        assert_eq!(summary.display_sync_count, 2);
        assert_eq!(summary.display_failure_count, 0);
        let footer = run.rows.last().unwrap();
        assert_eq!(footer["reason"], "max_reports");
        assert_eq!(footer["display_sync_count"], 2);
        assert_eq!(footer["display_failure_count"], 0);
        assert_eq!(footer["display_stopped"], true);
        assert_eq!(
            run.rows[0]["details"]["display_sync"],
            json!({"enabled":true, "hardware_verified":false})
        );
    }

    #[test]
    fn a_short_write_is_logged_while_actions_still_dispatch_and_the_run_succeeds() {
        let fake = Arc::new(Mutex::new(FakeDisplaySink::default()));
        fake.lock().unwrap().fail_next(
            DisplayTarget::Key(3),
            SinkError::ShortWrite {
                written: 10,
                expected: 65,
            },
        );
        let run = run_display(
            vec![wait_for_writes(&fake, 4), keys(&[0], 10), keys(&[], 20)],
            keypad_options(2),
            Arc::new(AtomicBool::new(false)),
            Some(shared_display(&fake, None)),
            Arc::default(),
        );
        let summary = run.result.unwrap();
        assert_eq!(run.actions, vec![Action::ToggleMute]);
        assert_eq!(summary.action_count, 1);
        let records = display_rows(&run.rows);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0]["written"], json!([0, 1, 2]));
        assert_eq!(records[0]["failed"][0]["key_index"], 3);
        assert_eq!(records[0]["failed"][0]["short_write"], true);
        assert_eq!(records[0]["failed"][0]["attempts"], 1);
        assert_eq!(
            records[0]["not_attempted"],
            json!((4..15).collect::<Vec<u8>>())
        );
        assert_eq!(records[0]["complete"], false);
        assert_eq!(summary.display_failure_count, 1);
        let footer = run.rows.last().unwrap();
        assert_eq!(footer["reason"], "max_reports");
        assert!(footer["error"].is_null());
        assert_eq!(footer["display_failure_count"], 1);
        assert_eq!(run.mode_writes, keypad_modes());
    }

    /// The only sync covered all 15 keys: each was written or refused by the
    /// closed write gate without reaching the sink.
    fn assert_page_written_or_refused(rows: &[Value], fake: &Arc<Mutex<FakeDisplaySink>>) {
        let records = display_rows(rows);
        assert_eq!(records.len(), 1);
        let written_list = records[0]["written"].as_array().unwrap();
        // Key 0 passed the gate before the sink blocked, so it completes.
        assert_eq!(written_list.first(), Some(&json!(0)));
        let written_keys = written_list.len();
        let failed = records[0]["failed"].as_array().unwrap();
        assert_eq!(written_keys + failed.len(), 15);
        assert!(failed.iter().all(|failure| {
            failure["error"]
                .as_str()
                .unwrap()
                .contains(DISPLAY_WRITES_CLOSED)
        }));
        assert_eq!(written(fake), written_keys);
    }

    #[test]
    fn a_stalled_sink_never_blocks_input_or_dispatch() {
        let fake = Arc::new(Mutex::new(FakeDisplaySink::default()));
        let gate: Gate = Arc::new((Mutex::new(false), Condvar::new()));
        let (began_tx, began_rx) = mpsc::sync_channel(1);
        let sink_gate = Arc::clone(&gate);
        let hook: WriteHook = Box::new(move |attempts| {
            if attempts == 0 {
                began_tx.send(()).unwrap();
                let (lock, changed) = &*sink_gate;
                let released = lock.lock().unwrap();
                let _unused = changed
                    .wait_timeout_while(released, Duration::from_secs(5), |open| !*open)
                    .unwrap();
            }
        });
        let received = Arc::new(Mutex::new(Vec::new()));
        let observed = Arc::clone(&received);
        let release = Arc::clone(&gate);
        let controller = thread::spawn(move || {
            // Both actions dispatch while the display thread is still blocked.
            let dispatched = wait_until(|| observed.lock().unwrap().len() == 2);
            let still_blocked = !*release.0.lock().unwrap();
            *release.0.lock().unwrap() = true;
            release.1.notify_all();
            dispatched && still_blocked
        });
        let run = run_display(
            vec![
                Step::Run(Box::new(move || {
                    began_rx.recv_timeout(Duration::from_secs(2)).unwrap();
                })),
                keys(&[0], 10),
                keys(&[], 20),
                keys(&[0], 30),
                keys(&[], 40),
            ],
            keypad_options(4),
            Arc::new(AtomicBool::new(false)),
            Some(shared_display(&fake, Some(hook))),
            received,
        );
        assert!(controller.join().unwrap());
        let summary = run.result.unwrap();
        assert_eq!(run.actions, vec![Action::ToggleMute, Action::ToggleMute]);
        assert_eq!(summary.report_count, 4);
        // After release the in-flight page either writes a key or, once the
        // session's write gate closed, refuses it; nothing else starts.
        assert_page_written_or_refused(&run.rows, &fake);
        assert_eq!(run.rows.last().unwrap()["display_stopped"], true);
    }

    #[test]
    fn no_sync_starts_after_the_interrupt_flag_is_set() {
        let fake = Arc::new(Mutex::new(FakeDisplaySink::default()));
        let gate: Gate = Arc::new((Mutex::new(false), Condvar::new()));
        let (began_tx, began_rx) = mpsc::sync_channel(1);
        let sink_gate = Arc::clone(&gate);
        let hook: WriteHook = Box::new(move |attempts| {
            if attempts == 0 {
                began_tx.send(()).unwrap();
                let (lock, changed) = &*sink_gate;
                let released = lock.lock().unwrap();
                let _unused = changed
                    .wait_timeout_while(released, Duration::from_secs(5), |open| !*open)
                    .unwrap();
            }
        });
        let interrupted = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&interrupted);
        let release = Arc::clone(&gate);
        let run = run_display(
            vec![
                Step::Run(Box::new(move || {
                    began_rx.recv_timeout(Duration::from_secs(2)).unwrap();
                })),
                // A navigation request is queued while the first sync is blocked.
                keys(&[1], 10),
                Step::Run(Box::new(move || {
                    flag.store(true, Ordering::SeqCst);
                    *release.0.lock().unwrap() = true;
                    release.1.notify_all();
                })),
            ],
            keypad_options(10),
            interrupted,
            Some(shared_display(&fake, Some(hook))),
            Arc::default(),
        );
        let summary = run.result.unwrap();
        assert_page_written_or_refused(&run.rows, &fake);
        let records = display_rows(&run.rows);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0]["trigger"], "software_mode");
        assert_eq!(summary.display_sync_count, 1);
        let footer = run.rows.last().unwrap();
        assert_eq!(footer["reason"], "interrupted");
        assert_eq!(footer["display_stopped"], true);
        assert_eq!(run.mode_writes, keypad_modes());
    }

    #[test]
    fn pending_display_requests_coalesce_to_the_newest_page() {
        let fake = Arc::new(Mutex::new(FakeDisplaySink::default()));
        let gate: Gate = Arc::new((Mutex::new(false), Condvar::new()));
        let (began_tx, began_rx) = mpsc::sync_channel(1);
        let sink_gate = Arc::clone(&gate);
        let hook: WriteHook = Box::new(move |attempts| {
            if attempts == 0 {
                began_tx.send(()).unwrap();
                let (lock, changed) = &*sink_gate;
                let released = lock.lock().unwrap();
                let _unused = changed
                    .wait_timeout_while(released, Duration::from_secs(5), |open| !*open)
                    .unwrap();
            }
        });
        let (records_tx, records_rx) = mpsc::sync_channel(DISPLAY_RECORD_QUEUE);
        let mut session = keypad_profile();
        let mut display = DisplayThread::spawn(
            shared_display(&fake, Some(hook)),
            session.clone(),
            Arc::new(AtomicBool::new(false)),
            records_tx,
        )
        .unwrap();
        display.request(&session, "software_mode");
        began_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        session
            .set_path(&["root".into(), "development".into()])
            .unwrap();
        display.request(&session, "navigation");
        session.set_path(&["root".into()]).unwrap();
        display.request(&session, "navigation");
        *gate.0.lock().unwrap() = true;
        gate.1.notify_all();
        let first = records_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let second = records_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(display.finish(Duration::from_secs(2)));
        assert_eq!(first["trigger"], "software_mode");
        // Only the newest request ran: back at root, which is already drawn.
        assert_eq!(second["page_id"], "root");
        assert_eq!(second["written"], json!([]));
        assert_eq!(display.counts.coalesced.load(Ordering::SeqCst), 1);
        assert_eq!(display.counts.syncs.load(Ordering::SeqCst), 2);
        assert!(records_rx.try_recv().is_err());
        assert_eq!(written(&fake), 15);
    }

    use crate::display_sink::{SHARED_BUSY, SHARED_WRITES_CLOSED, SharedTransport};

    /// A thread-safe M4315 HID fake whose first output report blocks on a gate,
    /// like a display write stuck inside the backend while holding the device.
    struct GatedHid {
        log: Arc<Mutex<Vec<&'static str>>>,
        began: Mutex<Option<SyncSender<()>>>,
        returned: Mutex<Option<SyncSender<()>>>,
        gate: Gate,
    }
    impl ReportTransport for GatedHid {
        fn read_timeout(&self, _: &mut [u8], _: i32) -> HidResult<usize> {
            Ok(0)
        }
        fn get_feature_report(&self, _: &mut [u8]) -> HidResult<usize> {
            panic!("runtime must not query features")
        }
        fn send_feature_report(&self, bytes: &[u8]) -> HidResult<()> {
            self.log
                .lock()
                .unwrap()
                .push(if bytes[2] == 1 { "on" } else { "off" });
            Ok(())
        }
        fn write_output(&self, data: &[u8]) -> HidResult<usize> {
            self.log.lock().unwrap().push("out");
            if let Some(began) = self.began.lock().unwrap().take() {
                began.send(()).unwrap();
                let (lock, changed) = &*self.gate;
                let open = lock.lock().unwrap();
                let open = changed
                    .wait_timeout_while(open, Duration::from_secs(10), |open| !*open)
                    .unwrap()
                    .0;
                drop(open);
                if let Some(returned) = self.returned.lock().unwrap().take() {
                    returned.send(()).unwrap();
                }
            }
            Ok(data.len())
        }
        fn get_report_descriptor(&self, buffer: &mut [u8]) -> HidResult<usize> {
            // Report ID 2 with 64 eight-bit output fields.
            let descriptor = [0x85, 0x02, 0x75, 0x08, 0x95, 64, 0x91, 0x02];
            buffer[..descriptor.len()].copy_from_slice(&descriptor);
            Ok(descriptor.len())
        }
    }

    struct GatedRun {
        result: Result<LiveSummary, String>,
        rows: Vec<Value>,
        log: Arc<Mutex<Vec<&'static str>>>,
        gate: Gate,
        shared: SharedTransport<GatedHid>,
        returned: Receiver<()>,
    }

    /// Runs a keypad session whose first display report blocks; `release`
    /// decides when the controller opens the gate (given the shared transport).
    fn run_gated(
        lock_timeout: Duration,
        release: impl FnOnce(&SharedTransport<GatedHid>, &Gate) + Send + 'static,
    ) -> GatedRun {
        let log = Arc::new(Mutex::new(Vec::new()));
        let gate: Gate = Arc::new((Mutex::new(false), Condvar::new()));
        let (began_tx, began_rx) = mpsc::sync_channel(1);
        let (returned_tx, returned_rx) = mpsc::sync_channel(1);
        let shared = SharedTransport::with_lock_timeout(
            GatedHid {
                log: Arc::clone(&log),
                began: Mutex::new(Some(began_tx)),
                returned: Mutex::new(Some(returned_tx)),
                gate: Arc::clone(&gate),
            },
            lock_timeout,
        );
        let display = crate::display_sink::live_display(
            &shared,
            Some(ModuleKind::M4315),
            PreparedKeyArtwork::prepare().unwrap(),
        )
        .unwrap();
        let interrupted = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&interrupted);
        let controlled = shared.clone();
        let controller_gate = Arc::clone(&gate);
        let controller = thread::spawn(move || {
            // The display write is now stuck holding the device; stop the run.
            began_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            flag.store(true, Ordering::SeqCst);
            release(&controlled, &controller_gate);
        });
        let output = Output::default();
        let writer = output.clone();
        let mut configuration = keypad_options(10);
        configuration.cancellation_timeout = Duration::from_millis(200);
        let handle = shared.clone();
        let result = run_inner(
            &mut InputReader::new(shared),
            keypad_profile(),
            keypad_device(),
            false,
            json!({}),
            configuration,
            interrupted,
            Some(display),
            move |stop| {
                Ok((
                    simple_actions(stop, Arc::new(Mutex::new(Vec::new()))),
                    writer,
                ))
            },
            || 0,
        );
        controller.join().unwrap();
        GatedRun {
            result,
            rows: output.rows(),
            log,
            gate,
            shared: handle,
            returned: returned_rx,
        }
    }

    #[test]
    fn a_stuck_display_write_yields_to_the_restore_and_no_report_follows_it() {
        let run = run_gated(Duration::from_secs(5), |shared, gate| {
            // Release the stuck write only once the restore is waiting for it.
            assert!(wait_until(|| shared.priority_waiters() > 0));
            *gate.0.lock().unwrap() = true;
            gate.1.notify_all();
        });
        let summary = run.result.unwrap();
        // One report went out before cleanup; the closed gate refused the rest.
        assert_eq!(*run.log.lock().unwrap(), vec!["on", "out", "off"]);
        assert_eq!(summary.report_count, 0);
        let footer = run.rows.last().unwrap();
        assert_eq!(footer["reason"], "interrupted");
        assert_eq!(footer["display_stopped"], false);
    }

    #[test]
    fn a_restore_that_never_gets_the_device_retries_once_and_the_run_still_ends() {
        let run = run_gated(Duration::from_millis(30), |_, _| {});
        let error = run.result.unwrap_err();
        assert!(error.contains("first attempt"), "{error}");
        assert!(error.contains("retry"), "{error}");
        assert_eq!(error.matches(SHARED_BUSY).count(), 2, "{error}");
        assert_eq!(*run.log.lock().unwrap(), vec!["on", "out"]);
        let footer = run.rows.last().unwrap();
        assert_eq!(footer["reason"], "error");
        assert_eq!(footer["display_stopped"], false);
        // Free the detached display thread: the stuck write returns, and any
        // further report is refused by the gate that closed at loop end.
        *run.gate.0.lock().unwrap() = true;
        run.gate.1.notify_all();
        run.returned.recv_timeout(Duration::from_secs(5)).unwrap();
        let refused = run.shared.write_output(&[2, 0]).unwrap_err().to_string();
        assert!(refused.contains(SHARED_WRITES_CLOSED), "{refused}");
        assert_eq!(*run.log.lock().unwrap(), vec!["on", "out"]);
    }

    #[test]
    fn a_panicking_sink_stops_the_display_thread_and_later_requests_are_ignored() {
        struct Panics;
        impl DisplaySink for Panics {
            fn capabilities(&self) -> DisplayCapabilities {
                FakeDisplaySink::default().capabilities
            }
            fn write_key_image(
                &mut self,
                _: u8,
                _: &[u8],
                _: &mut dyn FnMut(usize),
            ) -> Result<(), SinkError> {
                panic!("synthetic sink panic")
            }
            fn write_dial_image(
                &mut self,
                _: &[u8],
                _: &mut dyn FnMut(usize),
            ) -> Result<(), SinkError> {
                panic!("synthetic sink panic")
            }
        }
        let (records_tx, records_rx) = mpsc::sync_channel(DISPLAY_RECORD_QUEUE);
        let session = keypad_profile();
        let mut display = DisplayThread::spawn(
            LiveDisplay::new(
                Panics,
                PreparedKeyArtwork::prepare().unwrap(),
                Arc::new(AtomicBool::new(false)),
            ),
            session.clone(),
            Arc::new(AtomicBool::new(false)),
            records_tx,
        )
        .unwrap();
        display.request(&session, "software_mode");
        let record = records_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(record["error"].as_str().unwrap().contains("panicked"));
        display.request(&session, "navigation");
        display.request(&session, "navigation");
        assert!(display.finish(Duration::from_secs(2)));
        assert_eq!(display.counts.coalesced.load(Ordering::SeqCst), 0);
        assert_eq!(display.counts.syncs.load(Ordering::SeqCst), 1);
        assert_eq!(display.counts.failures.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn an_input_loop_panic_still_restores_hardware_mode() {
        let transport = Transport::new([Step::Run(Box::new(|| panic!("synthetic read panic")))]);
        let writes = Arc::clone(&transport.writes);
        let output = Output::default();
        let mut configuration = options();
        configuration.software_mode = true;
        let error = run_fake(
            transport,
            configuration,
            Arc::new(AtomicBool::new(false)),
            output.clone(),
            |stop| simple_actions(stop, Arc::new(Mutex::new(Vec::new()))),
        )
        .unwrap_err();
        assert!(error.contains(INPUT_PANIC), "{error}");
        assert!(error.contains("synthetic read panic"), "{error}");
        assert_eq!(*writes.lock().unwrap(), modes());
        let footer = output.rows().last().unwrap().clone();
        assert_eq!(footer["reason"], "error");
    }
}
