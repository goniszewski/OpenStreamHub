//! Bounded live input and serialized action execution.
//!
//! The input owner timestamps, normalizes, and resolves reports before
//! enqueueing work. When the queue is full, later reports are coalesced into one
//! pending work item: consecutive relative volume steps on a control are summed,
//! absolute positions keep the latest value, and other events are appended up to
//! a fixed bound beyond which they are dropped and counted. The action worker's
//! writer must have bounded writes, and its dispatcher must respond to the
//! supplied cancellation token. Native HID feature cleanup has no exposed
//! timeout; worker deadlines do not establish a bound on a backend's
//! synchronous device operations.

use crate::service_events::{PlannedEvent, Work, emit, execution, plan_report, process_work};
use cooltek_sdk_masterhub::{InputReader, ModuleKind, ReportTransport};
use cooltek_sdk_service::{
    Action, ActionDispatcher, DeviceContext, Gesture, InputSession, KeypadResolution,
    ProfileSession, ServiceModule,
};
use serde_json::{Value, json};
use std::{
    io::Write,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
    },
    thread,
    time::{Duration, Instant},
};

/// Maximum distinct events held in the coalesced pending work item.
pub const MAX_PENDING_EVENTS: usize = 256;
/// Read timeout while coalesced work waits for queue space, in milliseconds.
const PENDING_POLL_MS: i32 = 5;

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
    /// Successfully dispatched actions, including dry-run validation.
    pub action_count: u64,
    /// Dispatched actions that reported failure.
    pub failed_action_count: u64,
    /// Matched bindings that did not complete successfully.
    pub incomplete_action_count: u64,
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
    mut reader: InputReader<T>,
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
    let origin = Instant::now();
    run_with_clock(
        &mut reader,
        session,
        device,
        execute,
        details,
        options,
        interrupted,
        factory,
        || u64::try_from(origin.elapsed().as_micros()).unwrap_or(u64::MAX),
    )
}

#[allow(clippy::too_many_arguments)]
fn run_with_clock<T, D, W, F>(
    reader: &mut InputReader<T>,
    mut session: ProfileSession,
    device: DeviceContext,
    execute: bool,
    details: Value,
    options: LiveOptions,
    interrupted: Arc<AtomicBool>,
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
    let abort = Arc::clone(&interrupted);
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
                    factory,
                )
            }))
            .unwrap_or_else(|_| {
                panic_abort.store(true, Ordering::SeqCst);
                let error = "action worker panicked; action outcome may be unknown".to_owned();
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
    let result: Result<&str, String> = (|| {
        match ready_rx.recv_timeout(options.cancellation_timeout) {
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
            if let Some(raw) = reader
                .read_input(timeout)
                .map_err(|error| format!("input read failed: {error}"))?
            {
                let now = timestamp();
                counts.reports += 1;
                if interrupted.load(Ordering::SeqCst) {
                    return Ok("interrupted");
                }
                let work = plan_report(&mut input, &mut session, &device, raw, now)?;
                counts.decoded_events += work.events.len() as u64;
                counts.matched_actions +=
                    work.events.iter().filter(|e| e.action().is_some()).count() as u64;
                pending = match pending.take() {
                    Some(mut earlier) => {
                        coalesce(&mut earlier, work, &mut counts);
                        Some(earlier)
                    }
                    None => offer(&sender, work, &mut counts)?,
                };
                if options
                    .max_reports
                    .is_some_and(|limit| counts.reports >= limit)
                {
                    return Ok("max_reports");
                }
            }
        }
    })();
    let normal = matches!(result, Ok("duration_elapsed" | "max_reports"));
    let mut drain_error = None;
    if normal && let Some(work) = pending.take() {
        drain_error = flush_pending(&sender, work, &abort, options.drain_timeout, &mut counts);
    }
    if !normal {
        abort.store(true, Ordering::SeqCst);
    }
    input.reset();
    session.reset();
    counts.reason = result.as_ref().copied().unwrap_or("error").to_owned();
    counts.error = combine_errors(result.err(), drain_error);
    // Close the queue before monitoring drain/cancellation. Device cleanup is
    // attempted after that finite worker phase, including when the worker fails.
    drop(sender);
    let outcome = await_worker(&done_rx, &abort, options, normal);
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
        crate::mode::set(reader.transport(), mode_module(device.module), false)
            .map_err(|error| format!("restoring module hardware mode failed: {error}"))
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
    summary.incomplete_action_count = counts.matched_actions.saturating_sub(summary.action_count);
    if let Some(mut output) = output {
        let footer = emit(
            &mut output,
            json!({
                "type":"service_end", "schema_version":1,
                "reason":if counts.error.is_some() {"error"} else {counts.reason.as_str()}, "error":counts.error,
                "report_count":summary.report_count, "queued_report_count":summary.queued_report_count,
                "processed_report_count":summary.processed_report_count,
                "coalesced_report_count":summary.coalesced_report_count,
                "discarded_report_count":summary.report_count.saturating_sub(summary.processed_report_count),
                "decoded_event_count":summary.decoded_event_count, "event_count":summary.event_count,
                "coalesced_event_count":summary.coalesced_event_count,
                "dropped_event_count":summary.dropped_event_count,
                "discarded_event_count":summary.decoded_event_count.saturating_sub(summary.event_count),
                "action_count":summary.action_count, "failed_action_count":summary.failed_action_count,
                "incomplete_action_count":summary.incomplete_action_count,
                "execution":execution(execute),
            }),
        );
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
    budget: Duration,
    counts: &mut Finish,
) -> Option<String> {
    let started = Instant::now();
    loop {
        match offer(sender, work, counts) {
            Ok(None) => return None,
            Ok(Some(returned)) => work = returned,
            Err(error) => return Some(error),
        }
        if abort.load(Ordering::SeqCst) {
            return None;
        }
        if started.elapsed() >= budget {
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
    for planned in next.events {
        if let Some(existing) = pending
            .events
            .iter_mut()
            .rev()
            .find(|existing| existing.event.control == planned.event.control)
            && let Some(merged) = merge_event(existing, &planned)
        {
            *existing = merged;
            counts.coalesced_events += 1;
        } else if pending.events.len() < MAX_PENDING_EVENTS {
            pending.events.push(planned);
        } else {
            pending.dropped_event_count += planned.coalesced;
            counts.dropped_events += planned.coalesced;
        }
    }
}

/// Merges two consecutive continuous inputs on one control, if they are equivalent.
///
/// Relative volume steps sum (clamped to the valid ±100 range), absolute
/// positions keep the latest value, and unbound continuous inputs keep only the
/// latest event. Button transitions, navigation, and other actions never merge.
fn merge_event(earlier: &PlannedEvent, later: &PlannedEvent) -> Option<PlannedEvent> {
    if earlier.event.gesture != later.event.gesture
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
            KeypadResolution::Action {
                action: Action::Volume { delta: first },
                page_id: None,
            },
            KeypadResolution::Action {
                action: Action::Volume { delta: second },
                page_id: None,
            },
        ) => {
            let delta = (first + second).clamp(-100, 100);
            if delta == 0 {
                return None;
            }
            KeypadResolution::Action {
                action: Action::Volume { delta },
                page_id: None,
            }
        }
        (
            KeypadResolution::Action {
                action: Action::SetVolume { .. },
                page_id: None,
            },
            KeypadResolution::Action {
                action: Action::SetVolume { .. },
                page_id: None,
            },
        )
        | (KeypadResolution::Unbound, KeypadResolution::Unbound) => later.resolution.clone(),
        _ => return None,
    };
    Some(PlannedEvent {
        event: later.event,
        binding: later.binding.clone(),
        resolution,
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
    {
        return Err("worker drain/cancellation budgets must be positive and bounded".into());
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

fn mode_module(module: ServiceModule) -> Option<ModuleKind> {
    match module {
        ServiceModule::M131E => Some(ModuleKind::M131E),
        ServiceModule::M4315 => Some(ModuleKind::M4315),
        _ => None,
    }
}

fn await_worker<W>(
    done: &Receiver<WorkerOutcome<W>>,
    abort: &AtomicBool,
    options: LiveOptions,
    normal: bool,
) -> Result<WorkerOutcome<W>, String> {
    let started = Instant::now();
    let mut cancelled_at = (!normal).then_some(started);
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
        if cancelled_at.is_none() && started.elapsed() >= options.drain_timeout {
            drain_error =
                Some("action queue drain deadline exceeded; pending actions cancelled".to_owned());
            abort.store(true, Ordering::SeqCst);
            cancelled_at = Some(Instant::now());
        }
        if cancelled_at.is_some_and(|time| time.elapsed() >= options.cancellation_timeout) {
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
    while let Ok(work) = receiver.recv() {
        if abort.load(Ordering::SeqCst) {
            continue;
        }
        if let Err(error) = process_work(
            work,
            &mut dispatcher,
            &mut output,
            &abort,
            execute,
            stop_on_action_error,
            &mut outcome.summary,
        ) {
            outcome.error = Some(error);
            abort.store(true, Ordering::SeqCst);
        }
    }
    outcome.output = Some(output);
    outcome
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
            assert_eq!(result.is_err(), fail_enable);
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

    fn planned(control: u8, gesture: Gesture, resolution: KeypadResolution) -> PlannedEvent {
        PlannedEvent {
            event: cooltek_sdk_service::InputEvent {
                timestamp_micros: 0,
                control,
                gesture,
                value: None,
            },
            binding: None,
            resolution,
            coalesced: 1,
        }
    }

    fn action(action: Action) -> KeypadResolution {
        KeypadResolution::Action {
            action,
            page_id: None,
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
        assert_eq!(
            rows.iter()
                .filter(|row| row["type"] == "action_failed")
                .count(),
            3
        );
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
        let error = run_fake(
            Transport::new([dial(3, 0, 1)]),
            configuration,
            Arc::new(AtomicBool::new(false)),
            Output::default(),
            |stop| {
                let mut actions = simple_actions(stop, Arc::new(Mutex::new(Vec::new())));
                actions.panic = true;
                actions
            },
        )
        .unwrap_err();
        assert!(error.contains("worker panicked"));
        assert!(started.elapsed() < Duration::from_secs(2));
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
}
