//! Bounded live input and serialized action execution.
//!
//! The input owner timestamps and normalizes reports before enqueueing work.
//! Queue overflow ends the session. The action worker's writer must have bounded
//! writes, and its dispatcher must respond to the supplied cancellation token.
//! Native HID feature cleanup has no exposed timeout; worker deadlines do not
//! establish a bound on a backend's synchronous device operations.

use crate::capture::encode_hex;
use cooltek_sdk_masterhub::{InputReader, ReportTransport, dial};
use cooltek_sdk_service::{
    ActionDispatcher, Binding, BindingEngine, DeviceContext, InputEvent, InputSession, InputUpdate,
    ServiceModule,
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

/// Bounds for a single live interface.
#[derive(Clone, Copy, Debug)]
pub struct LiveOptions {
    /// Explicitly enable dial software mode and attempt hardware-mode cleanup.
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
        }
    }
}

/// Final counts distinguish received input from completed worker output.
#[derive(Clone, Debug, Default)]
pub struct LiveSummary {
    /// Nonempty reports received, including a report rejected by queue overflow.
    pub report_count: u64,
    /// Reports accepted by the work queue.
    pub queued_report_count: u64,
    /// Reports whose diagnostic record was written by the worker.
    pub processed_report_count: u64,
    /// Normalized events decoded by the input owner.
    pub decoded_event_count: u64,
    /// Events whose planned or unbound record was written.
    pub event_count: u64,
    /// Successfully dispatched actions, including dry-run validation.
    pub action_count: u64,
    /// Matched bindings that did not complete successfully.
    pub incomplete_action_count: u64,
}

#[derive(Default)]
struct Finish {
    reason: String,
    error: Option<String>,
    reports: u64,
    queued: u64,
    decoded_events: u64,
    matched_actions: u64,
}

struct Work {
    raw: Vec<u8>,
    timestamp_micros: u64,
    status: &'static str,
    error: Option<String>,
    events: Vec<(InputEvent, Option<Binding>)>,
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
    engine: BindingEngine,
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
        engine,
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
    engine: BindingEngine,
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
            dial::set_software_mode(reader.transport(), true).map_err(|error| error.to_string())?;
        }
        loop {
            if interrupted.load(Ordering::SeqCst) {
                return Ok("interrupted");
            }
            let timeout = if let Some(duration) = options.duration {
                let remaining = duration.saturating_sub(started.elapsed());
                if remaining.as_millis() == 0 {
                    return Ok("duration_elapsed");
                }
                remaining.as_millis().min(options.timeout_ms as u128) as i32
            } else {
                options.timeout_ms
            };
            if let Some(raw) = reader
                .read_input(timeout)
                .map_err(|error| format!("input read failed: {error}"))?
            {
                let now = timestamp();
                counts.reports += 1;
                if interrupted.load(Ordering::SeqCst) {
                    return Ok("interrupted");
                }
                let (status, error, events) = match input.decode(raw, now) {
                    Ok(InputUpdate::Events(events)) => ("decoded", None, events),
                    Ok(InputUpdate::Unknown) => ("unknown", None, Vec::new()),
                    Err(error) => ("parse_error", Some(error), Vec::new()),
                };
                let events: Vec<_> = events
                    .into_iter()
                    .map(|event| {
                        let binding = engine.binding_for(&device, &event).cloned();
                        counts.decoded_events += 1;
                        counts.matched_actions += u64::from(binding.is_some());
                        (event, binding)
                    })
                    .collect();
                let work = Work {
                    raw: raw.to_vec(),
                    timestamp_micros: now,
                    status,
                    error,
                    events,
                };
                match sender.try_send(work) {
                    Ok(()) => counts.queued += 1,
                    Err(TrySendError::Full(_)) => {
                        return Err(format!(
                            "action queue overflow (capacity {} reports); stopping session",
                            options.queue_capacity
                        ));
                    }
                    Err(TrySendError::Disconnected(_)) => {
                        return Err("action worker disconnected".into());
                    }
                }
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
    if !normal {
        abort.store(true, Ordering::SeqCst);
    }
    input.reset();
    counts.reason = result.as_ref().copied().unwrap_or("error").to_owned();
    counts.error = result.err();
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
        dial::set_software_mode(reader.transport(), false)
            .map_err(|error| format!("restoring dial hardware mode failed: {error}"))
    } else {
        Ok(())
    };
    counts.error = combine_errors(counts.error.take(), cleanup.err());
    if normal && abort.load(Ordering::SeqCst) && counts.error.is_none() {
        counts.reason = "interrupted".into();
    }
    summary.report_count = counts.reports;
    summary.queued_report_count = counts.queued;
    summary.decoded_event_count = counts.decoded_events;
    summary.incomplete_action_count = counts.matched_actions.saturating_sub(summary.action_count);
    if let Some(mut output) = output {
        let footer = emit(
            &mut output,
            json!({
                "type":"service_end", "schema_version":1,
                "reason":if counts.error.is_some() {"error"} else {counts.reason.as_str()}, "error":counts.error,
                "report_count":summary.report_count, "queued_report_count":summary.queued_report_count,
                "processed_report_count":summary.processed_report_count,
                "discarded_report_count":summary.report_count.saturating_sub(summary.processed_report_count),
                "decoded_event_count":summary.decoded_event_count, "event_count":summary.event_count,
                "discarded_event_count":summary.decoded_event_count.saturating_sub(summary.event_count),
                "action_count":summary.action_count, "incomplete_action_count":summary.incomplete_action_count,
                "execution":if execute {"macos"} else {"dry_run"},
            }),
        );
        counts.error = combine_errors(counts.error.take(), footer.err());
    }
    match counts.error {
        Some(error) => Err(error),
        None => Ok(summary),
    }
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
    if options.software_mode && device.module != ServiceModule::M131E {
        return Err("software mode requires an M131E session".into());
    }
    Ok(())
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
        json!({
            "type":"service_start", "schema_version":1,
            "device":{"module":device.module,"serial_number":device.serial_number},
            "execution":if execute {"macos"} else {"dry_run"}, "details":details,
        }),
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
            &mut outcome.summary,
        ) {
            outcome.error = Some(error);
            abort.store(true, Ordering::SeqCst);
        }
    }
    outcome.output = Some(output);
    outcome
}

fn process_work<D: ActionDispatcher, W: Write>(
    work: Work,
    dispatcher: &mut D,
    output: &mut W,
    abort: &AtomicBool,
    execute: bool,
    summary: &mut LiveSummary,
) -> Result<(), String> {
    emit(
        output,
        json!({
            "type":"report", "schema_version":1, "timestamp_micros":work.timestamp_micros,
            "raw_hex":encode_hex(&work.raw), "status":work.status, "error":work.error,
        }),
    )?;
    summary.processed_report_count += 1;
    for (event, binding) in work.events {
        if abort.load(Ordering::SeqCst) {
            break;
        }
        emit(
            output,
            json!({
                "type":"event", "schema_version":1, "event":event, "binding":binding,
                "status":if binding.is_some() {"planned"} else {"unbound"},
            }),
        )?;
        summary.event_count += 1;
        if let Some(binding) = binding {
            if abort.load(Ordering::SeqCst) {
                break;
            }
            if let Err(error) = dispatcher.dispatch(&binding.action) {
                abort.store(true, Ordering::SeqCst);
                let logged = emit(
                    output,
                    json!({"type":"action_failed", "event":event, "action":binding.action, "error":error}),
                );
                return Err(
                    combine_errors(Some(format!("action failed: {error}")), logged.err()).unwrap(),
                );
            }
            summary.action_count += 1;
            if execute {
                emit(
                    output,
                    json!({"type":"action_executed", "event":event, "action":binding.action}),
                )?;
            }
        }
    }
    Ok(())
}

fn emit(output: &mut impl Write, value: Value) -> Result<(), String> {
    let mut bytes = serde_json::to_vec(&value).map_err(|error| error.to_string())?;
    bytes.push(b'\n');
    if bytes.len() > 65_536 {
        return Err("live log record exceeds 65536 bytes".into());
    }
    output
        .write_all(&bytes)
        .map_err(|error| format!("live log write failed: {error}"))?;
    output
        .flush()
        .map_err(|error| format!("live log flush failed: {error}"))
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
    use cooltek_sdk_masterhub::hidapi::{HidError, HidResult};
    use cooltek_sdk_service::{Action, Profile};
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
    fn profile() -> BindingEngine {
        BindingEngine::new(Profile::from_json(br#"{"schema_version":1,"name":"test","bindings":[
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
        assert_eq!(output.rows().last().unwrap()["reason"], "max_reports");
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
    fn queue_overflow_is_explicit_and_cancels_pending_work_with_one_cleanup() {
        let (began_tx, began_rx) = mpsc::sync_channel(1);
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let mut transport = Transport::new([dial(3, 0, 1), dial(3, 0, 2), dial(3, 0, 3)]);
        transport.wait_second = Some(began_rx);
        let writes = Arc::clone(&transport.writes);
        let mut configuration = options();
        configuration.max_reports = None;
        configuration.software_mode = true;
        configuration.queue_capacity = 1;
        let received = Arc::new(Mutex::new(Vec::new()));
        let actions = Arc::clone(&received);
        let output = Output::default();
        let error = run_fake(
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
        .unwrap_err();
        assert!(error.contains("action queue overflow"));
        assert_eq!(*writes.lock().unwrap(), modes());
        assert_eq!(received.lock().unwrap().len(), 1);
        let footer = output.rows().last().unwrap().clone();
        assert_eq!(footer["reason"], "error");
        assert_eq!(footer["report_count"], 3);
        assert_eq!(footer["queued_report_count"], 2);
        assert_eq!(footer["processed_report_count"], 1);
        assert_eq!(footer["incomplete_action_count"], 3);
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
        let engine = BindingEngine::new(Profile::from_json(br#"{"schema_version":1,"name":"test","bindings":[
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
    fn dispatcher_failure_stops_further_work_without_retrying() {
        let received = Arc::new(Mutex::new(Vec::new()));
        let actions = Arc::clone(&received);
        let error = run_fake(
            Transport::new([dial(3, 0, 1)]),
            options(),
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
