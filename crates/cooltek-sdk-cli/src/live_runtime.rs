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
//! Optional display synchronization ([`LiveDisplay`], opt-in and
//! hardware-unverified) runs on its own thread with a single coalescing
//! request slot: M4315 key pages, or the M131E dial (a dial function name or a
//! clock, at most one dial write per [`DIAL_MIN_INTERVAL`]). The input owner
//! only records the newest page path and dial function there after the work
//! for that report was queued or coalesced, and never waits for the display
//! thread, so display work and display failures never dispatch, suppress, or
//! reorder an action. When the sink shares the input device, an
//! in-flight image write can delay reading input reports (they queue in the OS
//! meanwhile); the input loop itself keeps running its interrupt and limit
//! checks. Display outcomes reach the log through the action worker.

use crate::dial_artwork::DialArtwork;
use crate::display_artwork::PressedKey;
use crate::service_events::{
    PlannedEvent, TimerCancelReason, Work, cancel_timers, dial_function_details, emit, end_record,
    log_alert_outcome, plan_report, process_due_timers, process_work,
};
use crate::timer_face::{CountdownKey, TimerFaces};
use cooltek_sdk_masterhub::{InputReader, ModuleKind, ReportTransport};
use cooltek_sdk_service::{
    Action, ActionDispatcher, DeviceContext, Gesture, InputSession, KeypadLayout,
    ProfileResolution, ProfileSession, ServiceModule, TimerTable,
    display_sync::{
        ApplyReport, DialImageRequest, DisplayCapabilities, DisplaySink, DisplaySync,
        DisplayTarget, KeyImageProvider, SinkError,
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
/// Longest wait between expiry checks while a countdown runs and no input arrives.
const TIMER_POLL: Duration = Duration::from_millis(250);
/// Error reported when the input loop panics; cleanup still runs.
const INPUT_PANIC: &str = "input loop panicked";
/// Error reported when the coordinator panics after the input loop; the
/// hardware-mode restore still runs.
const COORDINATOR_PANIC: &str = "live session coordinator panicked";

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
    /// Report `profile` actions as `profile_switch_requested` for a supervising
    /// session manager instead of logging them as unavailable.
    pub profile_switch_requests: bool,
    /// Set by the stdin watchdog (`--exit-on-stdin-eof`) before it sets the
    /// interrupt flag: a stop it requested ends with reason `stdin_closed`
    /// instead of `interrupted`. `None` (the default) keeps `interrupted`.
    pub stdin_closed: Option<&'static AtomicBool>,
    /// A replacement profile engine, handed over by the supervisor's control channel. The input loop takes it
    /// between reports, so the next report is planned with the new bindings. Not used by runs with a display, whose
    /// own thread holds a copy of the engine and the artwork.
    pub profile_updates: Option<&'static Mutex<Option<ProfileSession>>>,
    /// The dial's backlight (percent), set right after software mode is entered. Only for an M131E run in software
    /// mode; `None` leaves the backlight as the module has it.
    pub dial_backlight: Option<u8>,
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
            profile_switch_requests: false,
            stdin_closed: None,
            profile_updates: None,
            dial_backlight: None,
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
    /// `profile` actions reported as a switch request or as unavailable;
    /// never dispatched, so not in `action_count`.
    pub profile_request_count: u64,
    /// `timer` presses that started a countdown (counted in `action_count`).
    pub timer_started_count: u64,
    /// Countdowns cancelled by a second press or by the end of the session.
    pub timer_cancelled_count: u64,
    /// Countdowns that ran to the end.
    pub timer_expired_count: u64,
    /// Expired countdowns whose notification or sound failed, or was skipped
    /// because the session stopped first.
    pub timer_alert_failed_count: u64,
    /// Expired countdowns whose notification tool succeeded but reported a
    /// problem, such as a notification macOS refused to show.
    pub timer_alert_warning_count: u64,
    /// Actions logged as `action_unavailable` because the dispatcher cannot run
    /// them in this run (such as `obs_scene` without an OBS endpoint); never
    /// dispatched, so not in `action_count` or `failed_action_count`.
    pub unavailable_action_count: u64,
    /// Display syncs attempted; always 0 without [`LiveDisplay`].
    pub display_sync_count: u64,
    /// Display syncs that failed to plan or apply, or left any key unwritten.
    pub display_failure_count: u64,
}

/// Longest error text in a `display_sync` record, which lists at most 15 keys per field.
const DISPLAY_ERROR_CHARS: usize = 240;
/// Display records waiting for the action worker; later records are dropped and counted.
const DISPLAY_RECORD_QUEUE: usize = 32;

/// Shortest time between two dial writes: the dial is redrawn at most about once a second
/// (an animated picture uses [`DIAL_ANIMATION_INTERVAL`]).
pub const DIAL_MIN_INTERVAL: Duration = Duration::from_secs(1);
/// Shortest time between two dial writes while an animated picture plays. A
/// frame near the size limit took 86 to 121 ms to write on a real M131E, so
/// this leaves about half the interval spare.
pub const DIAL_ANIMATION_INTERVAL: Duration = Duration::from_millis(200);
/// Trigger of a dial write started by an animation frame coming due.
const DIAL_ANIMATION_TRIGGER: &str = "animation";
/// How long a failed animation frame holds the animation back before the next
/// try, so a failing dial logs once in a while instead of at every frame.
const DIAL_ANIMATION_RETRY: Duration = Duration::from_secs(5);
/// One clean animation frame in this many is logged, with its write time; the
/// rest are counted in `syncs` only, as the keypad's timed frames are.
const DIAL_ANIMATION_LOG_EVERY: u32 = 50;
/// How often the dial thread re-checks its clock and a rate-limited request.
const DIAL_TICK: Duration = Duration::from_millis(100);
/// Shortest wait while an animation plays, so a frame due in a moment does not
/// spin the thread.
const DIAL_ANIMATION_TICK_FLOOR: Duration = Duration::from_millis(5);

/// Opt-in display synchronization for one live M4315 or M131E run.
///
/// Hardware-unverified: physical key order, orientation, pacing, and firmware
/// image acceptance are unknown. The sink and provider move to a dedicated
/// display thread; a display failure is logged as a `display_sync` record and
/// counted in `service_end`, but never ends the session or changes its result.
pub struct LiveDisplay {
    pub(crate) sink: Box<dyn DisplaySink + Send>,
    content: DisplayContent,
    pub(crate) write_gate: Arc<AtomicBool>,
    press_feedback: bool,
    /// The countdowns of running timers, drawn on their keys (keys display).
    timer_faces: TimerFaces,
}

/// What a [`LiveDisplay`] draws.
enum DisplayContent {
    /// M4315 key pages from prepared key artwork.
    Keys(Box<dyn KeyImageProvider + Send>),
    /// The M131E dial, rate limited to one write per `min_interval`.
    Dial {
        artwork: DialArtwork,
        min_interval: Duration,
    },
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
            content: DisplayContent::Keys(Box::new(artwork)),
            write_gate,
            press_feedback: false,
            timer_faces: TimerFaces::default(),
        }
    }

    /// Dims a key while it is held down: one extra key write on press and one
    /// on release, drawn on the display thread so an action never waits for it.
    /// Dropped for the rest of the run when a press write takes longer than
    /// [`PRESS_FEEDBACK_MAX_WRITE`], since feedback that late reads as lag.
    /// Keys displays only; a dial display ignores it.
    pub fn with_press_feedback(mut self, on: bool) -> Self {
        self.press_feedback = on;
        self
    }

    /// Pairs a dial sink with prepared dial artwork and the sink transport's
    /// write gate, under the same gate contract as [`Self::new`]. The dial is
    /// written at most once per [`DIAL_MIN_INTERVAL`], only when its face
    /// changes (a dial function switch or a new clock minute).
    pub fn dial(
        sink: impl DisplaySink + Send + 'static,
        artwork: DialArtwork,
        write_gate: Arc<AtomicBool>,
    ) -> Self {
        Self {
            sink: Box::new(sink),
            content: DisplayContent::Dial {
                artwork,
                min_interval: DIAL_MIN_INTERVAL,
            },
            write_gate,
            press_feedback: false,
            timer_faces: TimerFaces::default(),
        }
    }

    /// True for a dial display.
    fn is_dial(&self) -> bool {
        matches!(self.content, DisplayContent::Dial { .. })
    }

    #[cfg(test)]
    fn with_dial_interval(mut self, interval: Duration) -> Self {
        if let DisplayContent::Dial { min_interval, .. } = &mut self.content {
            *min_interval = interval;
        }
        self
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

/// One pending redraw; a newer request replaces an older one.
struct DisplayRequest {
    /// Keypad page path (keys).
    path: Vec<String>,
    /// Current dial function index (dial).
    dial_function: Option<usize>,
    /// The key held down, drawn dimmed (keys, with press feedback on).
    pressed: Option<u8>,
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
    /// Whether a held key is drawn dimmed (a keys display with the option on).
    press_feedback: bool,
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
        let press_feedback = display.press_feedback && !display.is_dial();
        let handle = thread::Builder::new()
            .name("masterhub-display".into())
            .spawn(move || {
                let LiveDisplay {
                    sink,
                    content,
                    write_gate: gate,
                    press_feedback,
                    timer_faces,
                } = display;
                let mut press_feedback = press_feedback;
                let gate_closed = Arc::clone(&gate);
                // Built here: `DisplaySync` is not `Send`.
                let mut sync = DisplaySync::new(BoxedSink { sink, gate });
                // Logs one sync; a panic disables the display for this run.
                let finish = |record: std::thread::Result<Value>, failed: Value| {
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
                            (failed, true)
                        }
                    };
                    if records.try_send(record).is_err() {
                        thread_counts.dropped_records.fetch_add(1, Ordering::SeqCst);
                    }
                    panicked
                };
                match content {
                    DisplayContent::Keys(mut artwork) => {
                        let mut current: Option<DisplayRequest> = None;
                        let started = Instant::now();
                        let mut animate = true;
                        let mut last_sync: Option<Instant> = None;
                        let faces_seen = std::cell::Cell::new(timer_faces.version());
                        loop {
                            // Frames are chosen from the run clock as of now, so a slow
                            // write never pushes later frames back; the gap is measured
                            // from the end of the previous sync, so the thread always
                            // idles between animation syncs.
                            let delay = match current.as_ref() {
                                Some(request) if animate => {
                                    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                                        || {
                                            artwork.advance(page_of(request), started.elapsed());
                                            animation_wait(
                                                [
                                                    artwork.next_frame_in(),
                                                    timer_faces.next_change_in(Instant::now()),
                                                ]
                                                .into_iter()
                                                .flatten()
                                                .min(),
                                                last_sync.map(|last| last.elapsed()),
                                                ANIMATION_MIN_INTERVAL,
                                            )
                                        },
                                    )) {
                                        Ok(delay) => delay,
                                        // Logged and handled like a panicking sync.
                                        Err(payload) => {
                                            let failed = display_record(
                                                request,
                                                None,
                                                &[],
                                                Some(DISPLAY_PANIC),
                                            );
                                            finish(Err(payload), failed);
                                            return;
                                        }
                                    }
                                }
                                _ => None,
                            };
                            match next_display_request(&thread_slot, &abort, delay, &|| {
                                let version = timer_faces.version();
                                version != faces_seen.replace(version)
                            }) {
                                Wake::Stopped => return,
                                Wake::Request(request) => current = Some(request),
                                Wake::Tick => {
                                    if let Some(request) = current.as_mut() {
                                        request.trigger = ANIMATION_TRIGGER;
                                    }
                                }
                            }
                            if gate_closed.load(Ordering::SeqCst) || abort.load(Ordering::SeqCst) {
                                return;
                            }
                            let Some(request) = current.as_ref() else {
                                continue;
                            };
                            let timed = request.trigger == ANIMATION_TRIGGER;
                            let closed = || {
                                gate_closed.load(Ordering::SeqCst) || abort.load(Ordering::SeqCst)
                            };
                            let held = request.pressed.filter(|_| press_feedback);
                            let began = Instant::now();
                            let record =
                                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                    artwork.advance(page_of(request), started.elapsed());
                                    let countdowns = timer_faces.snapshot();
                                    let counted = CountdownKey {
                                        inner: &*artwork,
                                        timers: &countdowns,
                                        now: Instant::now(),
                                    };
                                    let shown = PressedKey {
                                        inner: &counted,
                                        pressed: held,
                                    };
                                    sync_page(
                                        &mut sync,
                                        &mut session,
                                        &shown,
                                        request,
                                        &thread_counts,
                                        &|| timed && closed(),
                                    )
                                }));
                            last_sync = Some(Instant::now());
                            if press_feedback
                                && request.trigger == PRESS_TRIGGER
                                && began.elapsed() > PRESS_FEEDBACK_MAX_WRITE
                            {
                                press_feedback = false;
                            }
                            let elapsed_micros =
                                u64::try_from(began.elapsed().as_micros()).unwrap_or(u64::MAX);
                            let record = match record {
                                // A Stop interrupted an animation frame and every failure was
                                // its refusal: nothing is logged; no further sync may start.
                                Ok(None) => return,
                                Ok(Some(mut record)) => {
                                    record["elapsed_micros"] = json!(elapsed_micros);
                                    Ok(record)
                                }
                                Err(payload) => Err(payload),
                            };
                            let clean = record.as_ref().is_ok_and(|record| {
                                record["complete"] == true
                                    && record["missing"]
                                        .as_array()
                                        .is_none_or(|missing| missing.is_empty())
                            });
                            // A failed frame pauses timed animation, and an incomplete
                            // animation frame does too, so a failure is logged once rather
                            // than per tick; no timer retries a short write. Input and
                            // explicit page requests still draw, and the next one that
                            // completes cleanly resumes animation.
                            let failed_write = record.as_ref().is_ok_and(|record| {
                                record["error"].is_string()
                                    || record["failed"]
                                        .as_array()
                                        .is_some_and(|failed| !failed.is_empty())
                            });
                            if failed_write || (timed && !clean) {
                                animate = false;
                            } else if !timed && clean {
                                animate = true;
                            }
                            // Clean animation frames are counted in `syncs` but not logged:
                            // at up to ten a second they would flood the run log.
                            if timed && clean {
                                continue;
                            }
                            let failed = display_record(request, None, &[], Some(DISPLAY_PANIC));
                            if finish(record, failed) {
                                return;
                            }
                        }
                    }
                    DisplayContent::Dial {
                        artwork,
                        min_interval,
                    } => {
                        let mut dial = DialState::default();
                        let closed =
                            || gate_closed.load(Ordering::SeqCst) || abort.load(Ordering::SeqCst);
                        loop {
                            match next_display_request(
                                &thread_slot,
                                &abort,
                                Some(dial.tick(&artwork, min_interval, closed())),
                                &|| false,
                            ) {
                                Wake::Stopped => return,
                                Wake::Request(request) => dial.requested(request),
                                Wake::Tick => {}
                            }
                            // Choosing the face (reading the clock or sampling the
                            // CPU) runs inside the same panic guard as the write.
                            let mut attempted = None;
                            let attempt =
                                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                    let (request, trigger) =
                                        dial.due(&artwork, min_interval, &closed)?;
                                    attempted = Some((request.clone(), trigger));
                                    Some(sync_dial(
                                        &mut sync,
                                        &artwork,
                                        &request,
                                        trigger,
                                        &thread_counts,
                                    ))
                                }));
                            let record = match attempt {
                                Ok(None) => continue,
                                Ok(Some(record)) => Ok(record),
                                Err(payload) => Err(payload),
                            };
                            let outcome = record
                                .as_ref()
                                .map_or(DialOutcome::Failed, |(_, outcome)| *outcome);
                            // A panic before a face was chosen has no face or trigger.
                            let (request, trigger) = attempted.unwrap_or_else(|| {
                                let request = DialImageRequest {
                                    profile: String::new(),
                                    function: None,
                                };
                                (request, "panic")
                            });
                            // Clean animation frames are counted in `syncs` and mostly
                            // not logged: at five a second they would flood the log.
                            if trigger == DIAL_ANIMATION_TRIGGER
                                && outcome == DialOutcome::Done
                                && !record
                                    .as_ref()
                                    .is_ok_and(|(record, _)| record["error"].is_string())
                            {
                                dial.finished(outcome);
                                if !dial.logs_clean_frame() {
                                    continue;
                                }
                                // This one is the logged sample.
                                let Ok((record, _)) = record else { continue };
                                if finish(Ok(record), Value::Null) {
                                    return;
                                }
                                continue;
                            }
                            let failed =
                                dial_record(trigger, &request, None, false, Some(DISPLAY_PANIC));
                            if finish(record.map(|(record, _)| record), failed) {
                                return;
                            }
                            dial.finished(outcome);
                        }
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
            press_feedback,
            handle: Some(handle),
        })
    }

    /// Records the newest page and dial function to draw without waiting for
    /// the display thread.
    fn request(&self, session: &ProfileSession, trigger: &'static str) {
        self.request_pressed(session, trigger, None);
    }

    /// As [`Self::request`], with the key held down at this moment.
    fn request_pressed(
        &self,
        session: &ProfileSession,
        trigger: &'static str,
        pressed: Option<u8>,
    ) {
        let (lock, changed) = &*self.slot;
        let mut slot = lock.lock().unwrap_or_else(PoisonError::into_inner);
        if slot.stopped {
            return;
        }
        let request = DisplayRequest {
            path: session.path().to_vec(),
            dial_function: session.dial_function_index(),
            pressed,
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

/// Prefix of the error when the action worker thread cannot be spawned
/// (resource pressure). OpenStreamHub treats it as transient.
pub const WORKER_START_FAILED: &str = "could not start action worker";

/// Prefix of the error when the action worker never reported ready (it did
/// not write `service_start` in time). OpenStreamHub treats it as transient.
pub const WORKER_NOT_READY: &str = "action worker did not become ready";

/// Trigger of a key sync started by a key going down or up.
const PRESS_TRIGGER: &str = "press";

/// Longest a press-feedback write may take before feedback is dropped for the run.
const PRESS_FEEDBACK_MAX_WRITE: Duration = Duration::from_millis(150);

/// Trigger of a key sync started by an animation frame deadline, not a request.
const ANIMATION_TRIGGER: &str = "animation";

/// Least time from one key sync to the next animation-timed one, however many
/// keys' frame deadlines fall in between. Frames are still chosen from the run
/// clock, so a key whose deadline was deferred skips ahead rather than drifting.
const ANIMATION_MIN_INTERVAL: Duration = Duration::from_millis(100);

/// The page a key request draws.
fn page_of(request: &DisplayRequest) -> &str {
    request
        .path
        .last()
        .map(String::as_str)
        .unwrap_or(KeypadLayout::ROOT_PAGE_ID)
}

/// Wait before the next animation sync: the earliest frame deadline, but no
/// sooner than `min_interval` after the previous sync. `None` never wakes.
fn animation_wait(
    next_frame: Option<Duration>,
    since_last_sync: Option<Duration>,
    min_interval: Duration,
) -> Option<Duration> {
    let gap = since_last_sync.map_or(Duration::ZERO, |since| min_interval.saturating_sub(since));
    next_frame.map(|next| next.max(gap))
}

/// Record error for a sync that panicked.
const DISPLAY_PANIC: &str = "display sync panicked; display disabled for this run";

/// Why the display thread woke.
enum Wake {
    Request(DisplayRequest),
    /// `tick` elapsed without a request.
    Tick,
    /// Stopped or aborted: no further sync may start.
    Stopped,
}

/// Blocks the display thread until a request arrives, or at most `tick`.
fn next_display_request(
    slot: &(Mutex<DisplaySlot>, Condvar),
    abort: &AtomicBool,
    tick: Option<Duration>,
    changed_elsewhere: &dyn Fn() -> bool,
) -> Wake {
    let deadline = tick.map(|tick| Instant::now() + tick);
    let (lock, changed) = slot;
    let mut slot = lock.lock().unwrap_or_else(PoisonError::into_inner);
    loop {
        if slot.stopped || abort.load(Ordering::SeqCst) {
            return Wake::Stopped;
        }
        if let Some(request) = slot.pending.take() {
            return Wake::Request(request);
        }
        // Something outside the request slot (a countdown) needs a redraw.
        if changed_elsewhere() {
            return Wake::Tick;
        }
        let mut wait = Duration::from_millis(50);
        if let Some(deadline) = deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Wake::Tick;
            }
            wait = wait.min(remaining);
        }
        // Bounded wait so a set interrupt flag is observed without a notification.
        slot = changed
            .wait_timeout(slot, wait)
            .unwrap_or_else(PoisonError::into_inner)
            .0;
    }
}

/// Least time between two dial writes: `min_interval`, or the shorter
/// animation interval while an animated picture plays.
fn dial_interval(artwork: &DialArtwork, min_interval: Duration) -> Duration {
    if artwork.is_animated() {
        min_interval.min(DIAL_ANIMATION_INTERVAL)
    } else {
        min_interval
    }
}

/// The dial thread's view of what to draw and when it last tried.
#[derive(Default)]
struct DialState {
    /// Set by the first request, sent once software mode is on: nothing is
    /// drawn before it.
    started: bool,
    dial_function: Option<usize>,
    /// Trigger of the newest request not yet drawn.
    trigger: Option<&'static str>,
    /// The face last attempted, written or not. A short write is never
    /// repeated for the same face; the next face change redraws.
    last_face: Option<Option<String>>,
    last_attempt: Option<Instant>,
    /// The face whose plain transport failure already used its one retry.
    /// Cleared by a successful write and whenever another face is attempted.
    retried: Option<Option<String>>,
    /// The face waiting for its one retry after a plain transport failure.
    retry_pending: Option<Option<String>>,
    /// The newest attempt drew an animation frame.
    last_was_animation: bool,
    /// An animation frame failed: no more frames are tried until this moment
    /// or the next request, so a failing dial logs once in a while instead of
    /// at every frame.
    animation_paused_until: Option<Instant>,
    /// Clean animation frames drawn since one was last logged.
    animation_unlogged: u32,
}

/// How one dial attempt ended, for the retry bookkeeping.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DialOutcome {
    /// Written, or unchanged and so nothing to write.
    Done,
    /// A plain transport failure while the write gate was open.
    TransportFailed,
    /// A short write, a closed gate, or a planning error: never retried.
    Failed,
}

impl DialState {
    /// How long to sleep before looking again: the usual tick, or while an
    /// animation plays the time until the next draw can happen. When the frame
    /// on the clock is not the one drawn that is when the rate limit opens;
    /// otherwise it is when the frame ends. Waking at the usual tick instead
    /// would draw each frame up to a tick late, and the lateness would pile up
    /// until a frame was skipped. A paused animation waits at the usual tick
    /// for the next request.
    fn tick(&self, artwork: &DialArtwork, min_interval: Duration, closed: bool) -> Duration {
        if !self.started || closed || self.animation_paused() {
            return DIAL_TICK;
        }
        let Some(until_frame) = artwork.next_frame_in() else {
            return DIAL_TICK;
        };
        let pending =
            self.last_face.as_ref() != Some(&artwork.request(self.dial_function).function);
        let wait = if pending {
            let interval = dial_interval(artwork, min_interval);
            self.last_attempt.map_or(Duration::ZERO, |last| {
                interval.saturating_sub(last.elapsed())
            })
        } else {
            until_frame
        };
        wait.clamp(DIAL_ANIMATION_TICK_FLOOR, DIAL_TICK)
    }

    fn animation_paused(&self) -> bool {
        self.animation_paused_until
            .is_some_and(|until| Instant::now() < until)
    }

    /// Whether a clean animation frame's record is written to the log: the
    /// first, then one in [`DIAL_ANIMATION_LOG_EVERY`].
    fn logs_clean_frame(&mut self) -> bool {
        let log = self.animation_unlogged == 0;
        self.animation_unlogged = (self.animation_unlogged + 1) % DIAL_ANIMATION_LOG_EVERY;
        log
    }

    fn requested(&mut self, request: DisplayRequest) {
        self.started = true;
        self.dial_function = request.dial_function;
        self.trigger = Some(request.trigger);
        self.animation_paused_until = None;
    }

    /// The request to draw now, if the face changed and the rate limit allows.
    fn due(
        &mut self,
        artwork: &DialArtwork,
        min_interval: Duration,
        closed: &dyn Fn() -> bool,
    ) -> Option<(DialImageRequest, &'static str)> {
        // Checked right before building the request, which is where a CPU face
        // samples: no sample starts once the write gate has closed.
        if !self.started || closed() {
            return None;
        }
        let request = artwork.request(self.dial_function);
        // A CPU baseline alone would only draw `--`; wait for the next sample.
        if artwork.awaiting_first_sample() {
            return None;
        }
        let animation = request
            .function
            .as_deref()
            .is_some_and(|face| face.starts_with("image:"));
        if self.last_face.as_ref() == Some(&request.function) {
            self.trigger = None;
            return None;
        }
        if animation && self.animation_paused() && self.trigger.is_none() {
            return None;
        }
        let min_interval = dial_interval(artwork, min_interval);
        if self
            .last_attempt
            .is_some_and(|last| last.elapsed() < min_interval)
        {
            return None;
        }
        let face = Some(request.function.clone());
        if self.retried != face {
            self.retried = None;
        }
        let retrying = self.retry_pending.take() == face;
        self.last_face = face;
        self.last_attempt = Some(Instant::now());
        self.last_was_animation = animation;
        // What changes a face without a request: the clock's minute or a new
        // CPU percentage.
        let timed = match request.function.as_deref() {
            Some(face) if face.starts_with("clock:") || face.starts_with("analog:") => {
                Some("clock")
            }
            Some(face) if face.starts_with("cpu:") => Some("cpu"),
            Some(face) if face.starts_with("image:") => Some(DIAL_ANIMATION_TRIGGER),
            _ => None,
        };
        let trigger = match (self.trigger.take(), timed) {
            _ if retrying => "retry",
            // In clock and CPU modes a function switch changes nothing; time did.
            (Some("dial_function") | None, Some(timed)) => timed,
            (Some(trigger), _) => trigger,
            (None, None) => "clock",
        };
        Some((request, trigger))
    }

    /// Retry bookkeeping after an attempt. After a plain transport failure
    /// (not a short write, never with the write gate closed), the same face may
    /// be tried once more once the interval has passed, so a busy device does
    /// not leave a function name stale until the next click. A face that
    /// already used its retry is left alone until a successful write or
    /// another face resets it.
    fn finished(&mut self, outcome: DialOutcome) {
        if self.last_was_animation && outcome != DialOutcome::Done {
            self.animation_paused_until = Some(Instant::now() + DIAL_ANIMATION_RETRY);
        }
        match outcome {
            DialOutcome::Done => self.retried = None,
            DialOutcome::TransportFailed
                if self.last_face.is_some() && self.retried != self.last_face =>
            {
                self.retried = self.last_face.clone();
                self.retry_pending = self.last_face.take();
            }
            DialOutcome::TransportFailed | DialOutcome::Failed => {}
        }
    }
}

/// Plans and applies one dial face; every failure is reported in the record only.
fn sync_dial<S: DisplaySink>(
    sync: &mut DisplaySync<S>,
    artwork: &DialArtwork,
    request: &DialImageRequest,
    trigger: &'static str,
    counts: &DisplayCounts,
) -> (Value, DialOutcome) {
    counts.syncs.fetch_add(1, Ordering::SeqCst);
    let started = Instant::now();
    let (report, missing, error) = match sync.plan_dial(request.clone(), artwork) {
        Ok(plan) => {
            let missing = plan.is_missing();
            match sync.apply_dial(plan) {
                Ok(report) => (Some(report), missing, None),
                Err(error) => (None, missing, Some(error.to_string())),
            }
        }
        Err(error) => (None, false, Some(error.to_string())),
    };
    if missing || error.is_some() || report.as_ref().is_some_and(|report| !report.is_complete()) {
        counts.failures.fetch_add(1, Ordering::SeqCst);
    }
    let outcome = match &report {
        Some(report) if error.is_none() && report.is_complete() => DialOutcome::Done,
        Some(report)
            if report.failures.iter().all(|failure| {
                matches!(&failure.error, SinkError::Transport(message)
                    if !message.contains(DISPLAY_WRITES_CLOSED)
                        && !message.contains(crate::display_sink::SHARED_WRITES_CLOSED))
            }) && !report.failures.is_empty() =>
        {
            DialOutcome::TransportFailed
        }
        _ => DialOutcome::Failed,
    };
    let mut record = dial_record(trigger, request, report.as_ref(), missing, error.as_deref());
    // How long planning and the write took, to pace animation against.
    if report
        .as_ref()
        .is_some_and(|report| report.written.contains(&DisplayTarget::Dial))
    {
        record["elapsed_micros"] = json!(started.elapsed().as_micros() as u64);
    }
    (record, outcome)
}

/// A dial `display_sync` record.
fn dial_record(
    trigger: &'static str,
    request: &DialImageRequest,
    report: Option<&ApplyReport>,
    missing: bool,
    error: Option<&str>,
) -> Value {
    let failed = report
        .and_then(|report| report.failures.first())
        .map(|failure| {
            json!({
                "short_write":matches!(failure.error, SinkError::ShortWrite { .. }),
                "attempts":failure.attempts, "error":bounded(&failure.error.to_string()),
            })
        });
    json!({
        "type":"display_sync", "schema_version":1, "target":"dial", "trigger":trigger,
        "face":request.function,
        "written":report.is_some_and(|report| report.written.contains(&DisplayTarget::Dial)),
        "failed":failed, "missing":missing, "error":error.map(bounded),
        "complete":!missing && error.is_none() && report.is_some_and(ApplyReport::is_complete),
    })
}

/// Plans and applies one page; every failure is reported in the record only.
/// Returns `None`, counting no failure, when `interrupted` holds once the
/// writes end and the closed write gate refused every key that failed: the run
/// stopped, so the refusals are not faults. Any other failure in the same frame
/// is still logged and counted. `syncs` counts every sync started, including
/// an interrupted one, since it may already have written keys.
fn sync_page<S: DisplaySink>(
    sync: &mut DisplaySync<S>,
    session: &mut ProfileSession,
    artwork: &dyn KeyImageProvider,
    request: &DisplayRequest,
    counts: &DisplayCounts,
    interrupted: &dyn Fn() -> bool,
) -> Option<Value> {
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
    let only_refused = error.is_none()
        && missing.is_empty()
        && report.as_ref().is_some_and(|report| {
            report.not_attempted.is_empty()
                && report.failures.iter().all(|failure| {
                    matches!(&failure.error, SinkError::Transport(message)
                        if message.contains(DISPLAY_WRITES_CLOSED)
                            || message.contains(crate::display_sink::SHARED_WRITES_CLOSED))
                })
        });
    if only_refused && interrupted() {
        return None;
    }
    if error.is_some() || report.as_ref().is_some_and(|report| !report.is_complete()) {
        counts.failures.fetch_add(1, Ordering::SeqCst);
    }
    Some(display_record(
        request,
        report.as_ref(),
        &missing,
        error.as_deref(),
    ))
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

/// [`run_live`] with optional, opt-in keypad or dial display synchronization.
///
/// A keypad `display` requires `options.software_mode`, an M4315 session, and a
/// schema-2 keypad profile; a dial `display` ([`LiveDisplay::dial`]) requires
/// software mode and an M131E session. Anything else, or a display thread that
/// cannot start, is rejected before the worker starts or software mode is
/// entered. The first sync is requested right after software mode is enabled,
/// and another after each report whose work contains a keypad navigation or a
/// dial function switch, once that work was queued or coalesced. No sync starts
/// once the input loop has ended.
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
    if let Some(display) = &display {
        validate_display(options, &device, &session, display.is_dial())?;
    }
    let mut details = details;
    if let (Some(object), Some(functions)) = (
        details.as_object_mut(),
        dial_function_details(&session, &device),
    ) {
        object.insert("dial_functions".into(), functions);
    }
    let abort = Arc::clone(&interrupted);
    // The countdowns the worker publishes for a keys display to draw.
    let timer_faces = display
        .as_ref()
        .filter(|display| !display.is_dial())
        .map(|display| display.timer_faces.clone());
    // Started before the worker, so a failed spawn leaves nothing to clean up.
    // Fail closed: folder navigation must not run live without its display.
    let (display_records, display_thread) = match display {
        Some(display) => {
            let mut summary = json!({"enabled":true, "hardware_verified":false});
            if let DisplayContent::Dial {
                artwork,
                min_interval,
            } = &display.content
            {
                summary["target"] = json!("dial");
                summary["dial_content"] = json!(artwork.content().as_str());
                summary["min_interval_ms"] = json!(min_interval.as_millis() as u64);
            }
            let (records_tx, records_rx) = mpsc::sync_channel(DISPLAY_RECORD_QUEUE);
            let spawned =
                DisplayThread::spawn(display, session.clone(), Arc::clone(&abort), records_tx)?;
            if let Some(object) = details.as_object_mut() {
                object.insert("display_sync".into(), summary);
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
                    options.profile_switch_requests,
                    details,
                    display_records,
                    timer_faces,
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
        .map_err(|error| format!("{WORKER_START_FAILED}: {error}"))?;

    let started = Instant::now();
    let mut input = InputSession::new(device.module);
    let mut mode_attempted = false;
    let mut counts = Finish::default();
    let mut pending: Option<Work> = None;
    // The key currently held down, for press feedback.
    let mut held: Option<u8> = None;
    // A panic in the input loop (decoding, routing, a transport) is contained
    // so the cleanup below, including the hardware-mode restore, still runs.
    let input_loop = std::panic::AssertUnwindSafe(|| -> Result<&str, String> {
        match ready_rx.recv_timeout(options.startup_timeout) {
            Ok(Ok(())) => {}
            Ok(Err(error)) => return Err(error),
            Err(error) => return Err(format!("{WORKER_NOT_READY}: {error}")),
        }
        if interrupted.load(Ordering::SeqCst) {
            return Ok("interrupted");
        }
        if options.software_mode {
            mode_attempted = true;
            crate::mode::set(reader.transport(), mode_module(device.module), true)?;
            if let Some(percent) = options.dial_backlight {
                // A failed backlight write is not a failed run: the dial just keeps its own brightness.
                let _ = cooltek_sdk_masterhub::dial::set_backlight(reader.transport(), percent);
            }
            if let Some(display) = &display_thread {
                display.request(&session, "software_mode");
            }
            panic_point("software_mode_enabled");
        }
        loop {
            if interrupted.load(Ordering::SeqCst) {
                return Ok("interrupted");
            }
            if let Some(updates) = options.profile_updates
                && let Ok(mut slot) = updates.lock()
                && let Some(next) = slot.take()
            {
                session = next;
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
                let work_gestures: Vec<(u8, Gesture)> = work
                    .events
                    .iter()
                    .map(|planned| (planned.event.control, planned.event.gesture))
                    .collect();
                counts.decoded_events += work.events.len() as u64;
                counts.matched_actions +=
                    work.events.iter().filter(|e| e.action().is_some()).count() as u64;
                let navigated = work.events.iter().any(|planned| {
                    matches!(planned.resolution, ProfileResolution::Navigation { .. })
                });
                let switched = work
                    .events
                    .iter()
                    .any(PlannedEvent::is_dial_function_switch);
                pending = match pending.take() {
                    Some(mut earlier) => {
                        coalesce(&mut earlier, work, &mut counts);
                        Some(earlier)
                    }
                    None => offer(&sender, work, &mut counts)?,
                };
                // Only after the work was queued or coalesced, and never blocking.
                if let Some(display) = &display_thread {
                    let was_held = held;
                    if display.press_feedback {
                        for planned in &work_gestures {
                            match planned.1 {
                                Gesture::Pressed => held = Some(planned.0),
                                Gesture::Released if held == Some(planned.0) => held = None,
                                _ => {}
                            }
                        }
                    }
                    if navigated {
                        display.request_pressed(&session, "navigation", held);
                    } else if switched {
                        display.request(&session, "dial_function");
                    } else if held != was_held {
                        display.request_pressed(&session, PRESS_TRIGGER, held);
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
    });
    let result = std::panic::catch_unwind(input_loop)
        .unwrap_or_else(|payload| Err(format!("{INPUT_PANIC}: {}", panic_message(&*payload))));
    // A panic after the input loop (display coordination, drain, worker
    // completion) is contained too, so the hardware-mode restore below runs on
    // every exit path once software mode was attempted. The loop's outcome is
    // recorded first so a later panic cannot discard it.
    let normal = matches!(result, Ok("duration_elapsed" | "max_reports"));
    counts.reason = result.as_ref().copied().unwrap_or("error").to_owned();
    counts.error = result.err();
    let coordination = std::panic::AssertUnwindSafe(|| {
        // No display sync starts during drain or cancellation.
        if let Some(display) = &display_thread {
            display.stop();
        }
        panic_point("display_stopped");
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
        counts.error = combine_errors(counts.error.take(), drain_error);
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
        panic_point("queue_closed");
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
        panic_point("before_restore");
        (display_stopped, summary, output)
    });
    let (display_stopped, mut summary, output) = std::panic::catch_unwind(coordination)
        .unwrap_or_else(|payload| {
            // The queue sender and worker handle were dropped while unwinding;
            // cancel the worker and keep the display gate closed for the restore.
            abort.store(true, Ordering::SeqCst);
            if let Some(display) = &display_thread {
                display.stop();
            }
            counts.reason = "error".into();
            counts.error = combine_errors(
                counts.error.take(),
                Some(format!("{COORDINATOR_PANIC}: {}", panic_message(&*payload))),
            );
            (None, LiveSummary::default(), None)
        });
    let cleanup = if mode_attempted {
        restore_hardware_mode(reader.transport(), device.module)
    } else {
        Ok(())
    };
    counts.error = combine_errors(counts.error.take(), cleanup.err());
    if normal && abort.load(Ordering::SeqCst) && counts.error.is_none() {
        counts.reason = "interrupted".into();
    }
    if counts.reason == "interrupted"
        && options
            .stdin_closed
            .is_some_and(|closed| closed.load(Ordering::SeqCst))
    {
        counts.reason = "stdin_closed".into();
    }
    summary.report_count = counts.reports;
    summary.queued_report_count = counts.queued;
    summary.coalesced_report_count = counts.coalesced_reports;
    summary.decoded_event_count = counts.decoded_events;
    summary.coalesced_event_count = counts.coalesced_events;
    summary.dropped_event_count = counts.dropped_events;
    summary.dropped_dial_function_count = counts.dropped_dial_functions;
    summary.incomplete_action_count = counts.matched_actions.saturating_sub(
        summary.action_count + summary.profile_request_count + summary.unavailable_action_count,
    );
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

/// Folds already-queued work into `work` while each next item is one absolute
/// position (a fader) that merges into the last event (see [`merge_event`]). The first item
/// that does not merge is parked in `carried` so the order of everything else
/// is unchanged.
fn absorb_queued(mut work: Work, receiver: &Receiver<Work>, carried: &mut Option<Work>) -> Work {
    while let Ok(next) = receiver.try_recv() {
        let mergeable = !work.merge_barrier
            && next.dropped_event_count == 0
            && next.dropped_dial_function_count == 0
            && next.coalesced_parse_error_count == 0
            && next.status == "decoded"
            && next
                .events
                .iter()
                .all(|planned| planned.event.gesture == Gesture::PositionChanged)
            && matches!(next.events.as_slice(), [planned]
            if work.events.last().is_some_and(|last| {
                last.event.control == planned.event.control && merge_event(last, planned).is_some()
            }));
        if !mergeable {
            *carried = Some(next);
            break;
        }
        coalesce(&mut work, next, &mut Finish::default());
    }
    work
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
        // A fader backlog on the microphone keeps only the latest level.
        (
            ProfileResolution::Action {
                action: Action::SetMicInputVolume { .. },
                page_id: None,
                dial_function: first_function,
            },
            ProfileResolution::Action {
                action: Action::SetMicInputVolume { .. },
                page_id: None,
                dial_function: second_function,
            },
        ) if first_function == second_function => later.resolution.clone(),
        // A fader backlog on one OBS input keeps only the latest level; different
        // inputs never merge.
        (
            ProfileResolution::Action {
                action: Action::ObsSetInputVolume { input: first, .. },
                page_id: None,
                dial_function: first_function,
            },
            ProfileResolution::Action {
                action: Action::ObsSetInputVolume { input: second, .. },
                page_id: None,
                dial_function: second_function,
            },
        ) if first == second && first_function == second_function => later.resolution.clone(),
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

/// Returns a panic payload's message, if it is a string.
fn panic_message(payload: &(dyn std::any::Any + Send)) -> &str {
    payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("non-string panic payload")
}

#[cfg(test)]
thread_local! {
    /// Named coordinator point at which a test injects a panic.
    static PANIC_AT: std::cell::Cell<Option<&'static str>> = const { std::cell::Cell::new(None) };
}

/// Panics when a test armed `point` on this thread; a no-op outside tests.
#[cfg(test)]
fn panic_point(point: &'static str) {
    if PANIC_AT.with(|armed| armed.get()) == Some(point) {
        panic!("injected panic at {point}");
    }
}

#[cfg(not(test))]
fn panic_point(_: &'static str) {}

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
    dial: bool,
) -> Result<(), String> {
    if dial {
        if device.module != ServiceModule::M131E {
            return Err("dial display sync requires an M131E dial session".into());
        }
        if !options.software_mode {
            return Err("display sync requires software mode".into());
        }
        return Ok(());
    }
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
    profile_switch_requests: bool,
    details: Value,
    display_records: Option<Receiver<Value>>,
    timer_faces: Option<TimerFaces>,
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
    // Countdowns live only in this worker: they are polled between work items,
    // never block the input thread, and are cancelled when the queue closes.
    // Their alerts run on a separate alert thread when the dispatcher offers
    // one, so a playing sound never delays the next action.
    let mut timers = TimerTable::default();
    let mut alerts = Alerts::Unstarted;
    // A work item read ahead that did not merge; it runs next.
    let mut carried: Option<Work> = None;
    let origin = Instant::now();
    let now = || u64::try_from(origin.elapsed().as_micros()).unwrap_or(u64::MAX);
    loop {
        if let Some(records) = &display_records
            && let Err(error) = log_display_records(records, &mut output)
        {
            // An output failure ends the session as for any other record.
            outcome.error = Some(error);
            abort.store(true, Ordering::SeqCst);
            display_records = None;
        }
        alerts.observe_stop(&abort);
        if let Err(error) = alerts.log_results(&mut output, &mut outcome.summary) {
            outcome.error = combine_errors(outcome.error.take(), Some(error));
            abort.store(true, Ordering::SeqCst);
        }
        if !timers.is_empty() {
            let expired = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                if abort.load(Ordering::SeqCst) {
                    // Stopped: nothing expires or plays any more.
                    return cancel_timers(
                        &mut timers,
                        TimerCancelReason::Stopped,
                        &mut output,
                        &mut outcome.summary,
                    );
                }
                process_due_timers(
                    &mut timers,
                    now(),
                    &mut output,
                    &abort,
                    &mut outcome.summary,
                    |timer, alert, output, summary| {
                        alerts.submit(&mut dispatcher, &abort, timer, alert, output, summary)
                    },
                )
            }))
            .unwrap_or_else(|_| Err(WORKER_PANIC.to_owned()));
            if let Err(error) = expired {
                outcome.error = combine_errors(outcome.error.take(), Some(error));
                abort.store(true, Ordering::SeqCst);
            }
        }
        // Keys with a running countdown show it; one that ended or was cancelled gets its picture back.
        if let Some(faces) = &timer_faces {
            faces.publish(&timers, now());
        }
        // Poll while display records, countdowns or alert results are pending;
        // otherwise block.
        let polling = display_records.is_some() || alerts.pending() > 0;
        let wait = match (polling, timers.next_deadline()) {
            (false, None) => None,
            (display, deadline) => {
                let mut wait = if display {
                    Duration::from_millis(20)
                } else {
                    TIMER_POLL
                };
                if let Some(deadline) = deadline {
                    wait = wait.min(Duration::from_micros(deadline.saturating_sub(now())));
                }
                Some(wait.max(Duration::from_millis(1)))
            }
        };
        let work = match carried.take() {
            Some(work) => work,
            None => match wait {
                Some(wait) => match receiver.recv_timeout(wait) {
                    Ok(work) => work,
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                },
                None => match receiver.recv() {
                    Ok(work) => work,
                    Err(_) => break,
                },
            },
        };
        // While this work was waiting, a slow action (a volume change takes
        // about 0.2 s) may have let more of the same continuous input queue
        // up. Fold those into it so only the latest value runs.
        let work = absorb_queued(work, &receiver, &mut carried);
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
                profile_switch_requests,
                &mut timers,
                now(),
                &mut outcome.summary,
            )
        }))
        .unwrap_or_else(|_| Err(WORKER_PANIC.to_owned()));
        if let Err(error) = processed {
            outcome.error = Some(error);
            abort.store(true, Ordering::SeqCst);
        }
    }
    // The queue closed: no countdown survives the session.
    let reason = if abort.load(Ordering::SeqCst) {
        TimerCancelReason::Stopped
    } else {
        TimerCancelReason::SessionEnd
    };
    if let Err(error) = cancel_timers(&mut timers, reason, &mut output, &mut outcome.summary) {
        outcome.error = combine_errors(outcome.error.take(), Some(error));
    }
    if let Some(faces) = &timer_faces {
        faces.publish(&timers, now());
    }
    // Queued alerts are skipped; a stop kills the one in flight at once, a
    // normal end after ALERT_GRACE. The thread is then joined.
    if let Err(error) = alerts.finish(&mut output, &mut outcome.summary, &abort) {
        outcome.error = combine_errors(outcome.error.take(), Some(error));
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

/// Most timer alerts submitted to the alert thread and not yet reported back
/// (queued, in flight, or finished but unlogged). Equal to the timer limit.
const ALERT_LIMIT: usize = TimerTable::MAX_RUNNING;
/// How long a running alert may go on after a normal end of input (a
/// duration or report limit) before it is stopped like on a Stop. Most sounds
/// end within it; the end of a session never waits for a whole alert.
const ALERT_GRACE: Duration = Duration::from_secs(2);
/// How long to wait for the alert thread once its alert was stopped: killing
/// and reaping a subprocess takes at most about 1 s.
const ALERT_KILL_WAIT: Duration = Duration::from_secs(3);
/// `timer_alert_failed` error for an alert that never started after a stop.
const ALERT_SKIPPED: &str = "the session stopped before the alert ran; nothing was shown or played";
/// `timer_alert_failed` error for an alert that never started after a normal end.
const ALERT_SKIPPED_AT_END: &str =
    "the session ended before the alert ran; nothing was shown or played";
/// `timer_alert_failed` error for a running alert stopped by a Stop.
const ALERT_STOPPED: &str = "the session stopped while the alert ran; it was cut off";
/// `timer_alert_failed` error for a running alert stopped at a normal end.
const ALERT_ENDED: &str = "the session ended while the alert ran; it was cut off";
/// `timer_alert_failed` error when [`ALERT_LIMIT`] alerts are already pending.
const ALERT_QUEUE_FULL: &str = "too many timer alerts are pending; nothing was shown or played";
/// `timer_alert_failed` error for an alert whose thread did not report back.
const ALERT_UNKNOWN: &str = "alert outcome unknown: the timer alert thread did not stop";

/// Where expired timers' alerts run, chosen at the first expiry.
enum Alerts {
    Unstarted,
    /// Through `ActionDispatcher::timer_alert` on the worker (dispatchers
    /// without a separate alerter: dry runs and tests; instant).
    Inline,
    Thread(AlertThread),
    /// The alert thread could not start: every alert is logged as failed and
    /// the session goes on.
    Failed(String),
}

impl Alerts {
    fn submit<D: ActionDispatcher, W: Write>(
        &mut self,
        dispatcher: &mut D,
        abort: &Arc<AtomicBool>,
        timer: &cooltek_sdk_service::RunningTimer,
        alert: cooltek_sdk_service::TimerAlert,
        output: &mut W,
        summary: &mut LiveSummary,
    ) -> Result<(), String> {
        if matches!(self, Self::Unstarted) {
            let kill = Arc::new(AtomicBool::new(abort.load(Ordering::SeqCst)));
            *self = match dispatcher.timer_alerter(Arc::clone(&kill)) {
                Some(alerter) => {
                    Self::started(AlertThread::spawn(alerter, kill, Arc::clone(abort)))
                }
                None => Self::Inline,
            };
        }
        let outcome = match self {
            Self::Thread(thread) => match thread.submit(timer, alert) {
                Ok(()) => return Ok(()),
                Err(error) => Err(error),
            },
            Self::Failed(error) => Err(format!("{error}; nothing was shown or played")),
            _ => dispatcher.timer_alert(&alert),
        };
        log_alert_outcome(output, &timer.key, &timer.action, outcome, summary)
    }

    /// A started alert thread, or the reason it could not start.
    fn started(spawned: Result<AlertThread, String>) -> Self {
        match spawned {
            Ok(thread) => Self::Thread(thread),
            Err(error) => Self::Failed(error),
        }
    }

    /// Kills a running alert as soon as the worker sees the stop flag.
    fn observe_stop(&self, abort: &AtomicBool) {
        if let Self::Thread(thread) = self
            && abort.load(Ordering::SeqCst)
        {
            thread.kill.store(true, Ordering::SeqCst);
        }
    }

    fn pending(&self) -> usize {
        match self {
            Self::Thread(thread) => thread.pending.len(),
            _ => 0,
        }
    }

    fn log_results<W: Write>(
        &mut self,
        output: &mut W,
        summary: &mut LiveSummary,
    ) -> Result<(), String> {
        match self {
            Self::Thread(thread) => thread.log_results(output, summary),
            _ => Ok(()),
        }
    }

    fn finish<W: Write>(
        &mut self,
        output: &mut W,
        summary: &mut LiveSummary,
        abort: &AtomicBool,
    ) -> Result<(), String> {
        match self {
            Self::Thread(thread) => thread.finish(output, summary, abort),
            _ => Ok(()),
        }
    }
}

struct AlertJob {
    timer: cooltek_sdk_service::RunningTimer,
    alert: cooltek_sdk_service::TimerAlert,
}

struct AlertDone {
    timer: cooltek_sdk_service::RunningTimer,
    outcome: Result<Option<String>, String>,
}

/// One thread per session that runs timer alerts in order, so a playing
/// sound never blocks the action worker. Its alerter is cancelled by `kill`,
/// which the worker sets as soon as it sees the stop flag, and at the end of
/// the session after [`ALERT_GRACE`]: a running subprocess is then killed and
/// reaped, and no alert starts once the session stops or ends. Only the
/// worker writes records: results come back over a channel and are logged by it.
struct AlertThread {
    jobs: Option<SyncSender<AlertJob>>,
    done: Receiver<AlertDone>,
    /// Set at the end of the session: queued alerts are skipped.
    closing: Arc<AtomicBool>,
    /// The alerter's cancellation flag.
    kill: Arc<AtomicBool>,
    /// Submitted alerts not yet reported back, in submission (= result) order.
    pending: std::collections::VecDeque<cooltek_sdk_service::RunningTimer>,
    handle: Option<thread::JoinHandle<()>>,
}

impl AlertThread {
    fn spawn(
        mut alerter: Box<dyn cooltek_sdk_service::TimerAlerter>,
        kill: Arc<AtomicBool>,
        abort: Arc<AtomicBool>,
    ) -> Result<Self, String> {
        let (jobs, job_rx) = mpsc::sync_channel::<AlertJob>(ALERT_LIMIT);
        // At most ALERT_LIMIT results are ever outstanding, so sends never block.
        let (done_tx, done) = mpsc::sync_channel(ALERT_LIMIT);
        let closing = Arc::new(AtomicBool::new(false));
        let (stop, killed) = (Arc::clone(&closing), Arc::clone(&kill));
        #[cfg(test)]
        if FAIL_ALERT_SPAWN.with(std::cell::Cell::get) {
            return Err("could not start timer alert thread: injected failure".into());
        }
        let handle = thread::Builder::new()
            .name("masterhub-timer-alerts".into())
            .spawn(move || {
                let stopped = || abort.load(Ordering::SeqCst);
                while let Ok(job) = job_rx.recv() {
                    let outcome = if stopped() {
                        Err(ALERT_SKIPPED.to_owned())
                    } else if stop.load(Ordering::SeqCst) || killed.load(Ordering::SeqCst) {
                        Err(ALERT_SKIPPED_AT_END.to_owned())
                    } else {
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            alerter.alert(&job.alert)
                        }))
                        .unwrap_or_else(|_| Err("timer alert panicked".to_owned()))
                        .map_err(|error| {
                            // A cut-off alert says why, not just "cancelled".
                            match (killed.load(Ordering::SeqCst), stopped()) {
                                (true, true) => format!("{ALERT_STOPPED} ({error})"),
                                (true, false) => format!("{ALERT_ENDED} ({error})"),
                                (false, _) => error,
                            }
                        })
                    };
                    if done_tx
                        .send(AlertDone {
                            timer: job.timer,
                            outcome,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .map_err(|error| format!("could not start timer alert thread: {error}"))?;
        Ok(Self {
            jobs: Some(jobs),
            done,
            closing,
            kill,
            pending: std::collections::VecDeque::new(),
            handle: Some(handle),
        })
    }

    fn submit(
        &mut self,
        timer: &cooltek_sdk_service::RunningTimer,
        alert: cooltek_sdk_service::TimerAlert,
    ) -> Result<(), String> {
        if self.pending.len() >= ALERT_LIMIT {
            return Err(ALERT_QUEUE_FULL.into());
        }
        let sender = self.jobs.as_ref().ok_or(ALERT_SKIPPED)?;
        match sender.try_send(AlertJob {
            timer: timer.clone(),
            alert,
        }) {
            Ok(()) => {
                self.pending.push_back(timer.clone());
                Ok(())
            }
            Err(TrySendError::Full(_)) => Err(ALERT_QUEUE_FULL.into()),
            Err(TrySendError::Disconnected(_)) => {
                Err("the timer alert thread stopped; nothing was shown or played".into())
            }
        }
    }

    /// Logs every finished alert without waiting.
    fn log_results<W: Write>(
        &mut self,
        output: &mut W,
        summary: &mut LiveSummary,
    ) -> Result<(), String> {
        while let Ok(done) = self.done.try_recv() {
            self.pending.pop_front();
            log_alert_outcome(
                output,
                &done.timer.key,
                &done.timer.action,
                done.outcome,
                summary,
            )?;
        }
        Ok(())
    }

    /// Skips queued alerts, lets a running one go on for at most
    /// [`ALERT_GRACE`] (none after a stop), then stops it, logs every result
    /// and joins the thread. Alerts never turn a clean end into an error:
    /// only a thread that ignores the kill is reported, with each alert it
    /// never reported logged as failed.
    fn finish<W: Write>(
        &mut self,
        output: &mut W,
        summary: &mut LiveSummary,
        abort: &AtomicBool,
    ) -> Result<(), String> {
        self.closing.store(true, Ordering::SeqCst);
        if abort.load(Ordering::SeqCst) {
            self.kill.store(true, Ordering::SeqCst);
        }
        self.jobs = None;
        let Some(handle) = self.handle.take() else {
            return Ok(());
        };
        let mut logged = Ok(());
        for wait in [ALERT_GRACE, ALERT_KILL_WAIT] {
            let deadline = Instant::now() + wait;
            while !handle.is_finished() && Instant::now() < deadline {
                if abort.load(Ordering::SeqCst) {
                    self.kill.store(true, Ordering::SeqCst);
                }
                if logged.is_ok() {
                    logged = self.log_results(output, summary);
                }
                thread::sleep(Duration::from_millis(5));
            }
            self.kill.store(true, Ordering::SeqCst);
        }
        if logged.is_ok() {
            logged = self.log_results(output, summary);
        }
        if !handle.is_finished() {
            let unreported = self.pending.len();
            for timer in std::mem::take(&mut self.pending) {
                if logged.is_ok() {
                    logged = log_alert_outcome(
                        output,
                        &timer.key,
                        &timer.action,
                        Err(ALERT_UNKNOWN.into()),
                        summary,
                    );
                }
            }
            return Err(combine_errors(
                logged.err(),
                Some(format!(
                    "timer alert thread did not stop within {} s after it was cancelled; {unreported} alert(s) unreported; thread was not joined",
                    ALERT_KILL_WAIT.as_secs()
                )),
            )
            .unwrap());
        }
        let joined = handle
            .join()
            .map_err(|_| "timer alert thread panicked".to_owned());
        logged.and(joined)
    }
}

impl Drop for AlertThread {
    fn drop(&mut self) {
        // Reached without `finish` only while unwinding: stop everything.
        self.closing.store(true, Ordering::SeqCst);
        self.kill.store(true, Ordering::SeqCst);
    }
}

#[cfg(test)]
thread_local! {
    /// Makes the next alert-thread start on this (worker) thread fail.
    static FAIL_ALERT_SPAWN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
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

        /// Recorded as the timer action it came from; nothing is shown or played.
        fn timer_alert(
            &mut self,
            alert: &cooltek_sdk_service::TimerAlert,
        ) -> Result<Option<String>, String> {
            self.received.lock().unwrap().push(alert_action(alert));
            Ok(None)
        }
    }
    /// The timer action an alert came from, for recording.
    fn alert_action(alert: &cooltek_sdk_service::TimerAlert) -> Action {
        Action::Timer {
            seconds: alert.seconds,
            label: alert.label.clone(),
            sound: Some(alert.sound),
        }
    }
    /// [`Actions`] plus a separate alerter, so alerts run on the alert thread.
    /// Builds an alerter from the cancellation flag the runtime passes.
    type AlerterBuilder =
        Box<dyn FnOnce(Arc<AtomicBool>) -> Box<dyn cooltek_sdk_service::TimerAlerter> + Send>;
    struct WithAlerter {
        actions: Actions,
        alerter: Option<AlerterBuilder>,
        /// Make the alert thread fail to start (checked on the worker thread).
        fail_spawn: bool,
        /// (release, seen): `seen` is set when an action is dispatched while
        /// the slow alerter is still blocked (`release` unset).
        probe: Option<(Arc<AtomicBool>, Arc<AtomicBool>)>,
        /// A real OBS client for `obs_scene`, cancelled by the session's stop flag.
        obs: Option<cooltek_sdk_service::obs::ObsClient>,
    }
    impl ActionDispatcher for WithAlerter {
        fn dispatch(&mut self, action: &Action) -> Result<(), String> {
            if let Some((release, seen)) = &self.probe
                && !release.load(Ordering::SeqCst)
            {
                seen.store(true, Ordering::SeqCst);
            }
            if let (Action::ObsScene { scene }, Some(obs)) = (action, &self.obs) {
                self.actions.received.lock().unwrap().push(action.clone());
                return obs.set_current_program_scene(scene);
            }
            self.actions.dispatch(action)
        }
        fn timer_alert(
            &mut self,
            alert: &cooltek_sdk_service::TimerAlert,
        ) -> Result<Option<String>, String> {
            self.actions.timer_alert(alert)
        }
        fn timer_alerter(
            &mut self,
            cancel: Arc<AtomicBool>,
        ) -> Option<Box<dyn cooltek_sdk_service::TimerAlerter>> {
            FAIL_ALERT_SPAWN.with(|fail| fail.set(self.fail_spawn));
            self.alerter.take().map(|build| build(cancel))
        }
    }
    /// Shared observations of a [`SlowAlerter`].
    #[derive(Default)]
    struct AlertLog {
        started: Vec<Action>,
        /// Alerts that ended because the stop flag was set mid-alert.
        killed: usize,
        dropped: bool,
    }
    /// Stands in for `osascript` + `afplay`: each alert blocks until `release`
    /// is set or its cancellation flag is (like a killed subprocess), for at
    /// most 10 s. With `ignore_stop` it behaves like a stuck tool.
    struct SlowAlerter {
        log: Arc<Mutex<AlertLog>>,
        release: Arc<AtomicBool>,
        stop: Arc<AtomicBool>,
        ignore_stop: bool,
    }
    impl cooltek_sdk_service::TimerAlerter for SlowAlerter {
        fn alert(
            &mut self,
            alert: &cooltek_sdk_service::TimerAlert,
        ) -> Result<Option<String>, String> {
            self.log.lock().unwrap().started.push(alert_action(alert));
            let deadline = Instant::now() + Duration::from_secs(10);
            while !self.release.load(Ordering::SeqCst) && Instant::now() < deadline {
                if !self.ignore_stop && self.stop.load(Ordering::SeqCst) {
                    self.log.lock().unwrap().killed += 1;
                    return Err("/usr/bin/afplay: action execution cancelled".into());
                }
                thread::sleep(Duration::from_millis(1));
            }
            Ok(None)
        }
    }
    impl Drop for SlowAlerter {
        fn drop(&mut self) {
            self.log.lock().unwrap().dropped = true;
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
        /// Shared event log: mode reports are logged as `on` and `off`.
        log: Option<Arc<Mutex<Vec<&'static str>>>>,
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
                log: None,
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
            if let Some(log) = &self.log {
                log.lock()
                    .unwrap()
                    .push(if bytes[2] == 1 { "on" } else { "off" });
            }
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

    fn timer_session(
        steps: Vec<Step>,
        options: LiveOptions,
        interrupted: Arc<AtomicBool>,
    ) -> (Result<LiveSummary, String>, Vec<Action>, Vec<Value>) {
        timer_session_with(steps, options, interrupted, None, None, false)
    }

    /// Keys 1 and 4 start one-second timers, key 2 a one-hour timer, key 3
    /// toggles mute, key 5 switches to the OBS scene `Live`. With an
    /// `alerter`, alerts run on the alert thread.
    fn timer_session_with(
        steps: Vec<Step>,
        options: LiveOptions,
        interrupted: Arc<AtomicBool>,
        alerter: Option<AlerterBuilder>,
        probe: Option<(Arc<AtomicBool>, Arc<AtomicBool>)>,
        fail_spawn: bool,
    ) -> (Result<LiveSummary, String>, Vec<Action>, Vec<Value>) {
        timer_session_with_obs(
            steps,
            options,
            interrupted,
            alerter,
            probe,
            fail_spawn,
            None,
        )
    }

    /// [`timer_session_with`], plus a real OBS client for `obs_scene` on
    /// `127.0.0.1:obs_port` when given.
    fn timer_session_with_obs(
        steps: Vec<Step>,
        options: LiveOptions,
        interrupted: Arc<AtomicBool>,
        alerter: Option<AlerterBuilder>,
        probe: Option<(Arc<AtomicBool>, Arc<AtomicBool>)>,
        fail_spawn: bool,
        obs_port: Option<u16>,
    ) -> (Result<LiveSummary, String>, Vec<Action>, Vec<Value>) {
        let transport = Transport::new(steps);
        let clock = Arc::clone(&transport.clock);
        let received = Arc::new(Mutex::new(Vec::new()));
        let actions = Arc::clone(&received);
        let output = Output::default();
        let writer = output.clone();
        let engine = ProfileSession::new(
            Profile::from_json(br#"{"schema_version":1,"name":"timers","bindings":[
                {"module":"m4315","control":0,"gesture":"pressed","action":{"type":"timer","seconds":1,"label":"Tea","sound":"none"}},
                {"module":"m4315","control":1,"gesture":"pressed","action":{"type":"timer","seconds":3600}},
                {"module":"m4315","control":2,"gesture":"pressed","action":{"type":"toggle_mute"}},
                {"module":"m4315","control":3,"gesture":"pressed","action":{"type":"timer","seconds":1,"sound":"funk"}},
                {"module":"m4315","control":4,"gesture":"pressed","action":{"type":"obs_scene","scene":"Live"}}
            ]}"#)
            .unwrap(),
        )
        .unwrap();
        let result = run_with_clock(
            &mut InputReader::new(transport),
            engine,
            DeviceContext {
                module: ServiceModule::M4315,
                serial_number: None,
            },
            true,
            json!({"source":"synthetic"}),
            options,
            interrupted,
            move |stop| {
                let obs = obs_port.map(|port| {
                    cooltek_sdk_service::obs::ObsClient::new(
                        cooltek_sdk_service::obs::ObsEndpoint::new("127.0.0.1", port).unwrap(),
                        None,
                    )
                    .with_cancellation(Arc::clone(&stop))
                });
                Ok((
                    WithAlerter {
                        actions: simple_actions(stop, actions),
                        alerter,
                        fail_spawn,
                        probe,
                        obs,
                    },
                    writer,
                ))
            },
            || clock.load(Ordering::SeqCst),
        );
        let received = received.lock().unwrap().clone();
        (result, received, output.rows())
    }

    /// Polls `condition` for up to 5 s on the input thread.
    fn eventually(mut condition: impl FnMut() -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if condition() {
                return true;
            }
            thread::sleep(Duration::from_millis(2));
        }
        false
    }

    /// A [`SlowAlerter`] built on the cancellation flag the runtime passes.
    fn slow_alerter(ignore_stop: bool) -> (AlerterBuilder, Arc<Mutex<AlertLog>>, Arc<AtomicBool>) {
        let log = Arc::new(Mutex::new(AlertLog::default()));
        let release = Arc::new(AtomicBool::new(false));
        let (shared, freed) = (Arc::clone(&log), Arc::clone(&release));
        let build: AlerterBuilder = Box::new(move |stop| {
            Box::new(SlowAlerter {
                log: shared,
                release: freed,
                stop,
                ignore_stop,
            })
        });
        (build, log, release)
    }

    #[test]
    fn two_simultaneous_expiries_never_block_a_key_action() {
        let interrupted = Arc::new(AtomicBool::new(false));
        let (alerter, log, release) = slow_alerter(false);
        let unblocked = Arc::new(AtomicBool::new(false));
        let (seen, waited, freed, finished) = (
            Arc::clone(&log),
            Arc::clone(&unblocked),
            Arc::clone(&release),
            Arc::clone(&log),
        );
        let probe = (Arc::clone(&release), Arc::clone(&unblocked));
        let (result, received, rows) = timer_session_with(
            vec![
                keys(&[0], 1),
                keys(&[], 2),
                keys(&[3], 3),
                keys(&[], 4),
                // The first alert is now running (and blocked) on the alert thread.
                Step::Run(Box::new(move || {
                    assert!(eventually(|| !seen.lock().unwrap().started.is_empty()));
                })),
                keys(&[2], 5),
                // The worker must dispatch the key while the alert is blocked.
                Step::Run(Box::new(move || {
                    assert!(eventually(|| waited.load(Ordering::SeqCst)));
                })),
                Step::Run(Box::new(move || freed.store(true, Ordering::SeqCst))),
                Step::Run(Box::new(move || {
                    assert!(eventually(|| finished.lock().unwrap().started.len() == 2));
                })),
                keys(&[], 6),
            ],
            LiveOptions {
                max_reports: Some(6),
                timeout_ms: 5,
                ..LiveOptions::default()
            },
            interrupted,
            Some(alerter),
            Some(probe),
            false,
        );
        let summary = result.unwrap();
        assert!(unblocked.load(Ordering::SeqCst));
        // Mute ran on the worker; alerts never went through the dispatcher.
        assert_eq!(received, [Action::ToggleMute]);
        assert_eq!(of_type(&rows, "action_executed").len(), 1);
        assert_eq!(of_type(&rows, "timer_expired").len(), 2);
        let log = log.lock().unwrap();
        assert_eq!(log.started.len(), 2);
        assert_eq!(log.killed, 0);
        assert!(
            log.dropped,
            "the alert thread ended and dropped its alerter"
        );
        assert!(of_type(&rows, "timer_alert_failed").is_empty());
        assert_eq!(summary.timer_expired_count, 2);
        assert_eq!(rows.last().unwrap()["reason"], "max_reports");
    }

    #[test]
    fn a_stop_kills_the_running_alert_and_starts_no_other() {
        let interrupted = Arc::new(AtomicBool::new(false));
        let (alerter, log, _release) = slow_alerter(false);
        let (seen, stop) = (Arc::clone(&log), Arc::clone(&interrupted));
        let (result, received, rows) = timer_session_with(
            vec![
                keys(&[0], 1),
                keys(&[], 2),
                keys(&[3], 3),
                keys(&[], 4),
                Step::Run(Box::new(move || {
                    assert!(eventually(|| !seen.lock().unwrap().started.is_empty()));
                    // Let the second expiry queue behind the first.
                    thread::sleep(Duration::from_millis(100));
                    stop.store(true, Ordering::SeqCst);
                })),
            ],
            LiveOptions {
                timeout_ms: 5,
                ..LiveOptions::default()
            },
            interrupted,
            Some(alerter),
            None,
            false,
        );
        result.unwrap();
        assert!(received.is_empty());
        let log = log.lock().unwrap();
        assert_eq!(log.started.len(), 1, "nothing starts after the stop");
        assert_eq!(log.killed, 1, "the running alert saw the stop");
        assert!(log.dropped, "the alert thread was joined");
        let failed = of_type(&rows, "timer_alert_failed");
        assert_eq!(failed.len(), 2);
        let cut = failed[0]["error"].as_str().unwrap();
        assert!(
            cut.starts_with(ALERT_STOPPED) && cut.contains("cancelled"),
            "{cut}"
        );
        assert_eq!(failed[1]["error"], ALERT_SKIPPED);
        assert_eq!(rows.last().unwrap()["reason"], "interrupted");
        assert_eq!(rows.last().unwrap()["timer_alert_failed_count"], 2);
    }

    #[test]
    fn a_stop_cuts_off_both_a_running_alert_and_an_obs_request_in_flight() {
        // A loopback "OBS" that accepts and then never answers.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let connected = Arc::new(AtomicBool::new(false));
        let (release, wait) = std::sync::mpsc::channel::<()>();
        let accepted = Arc::clone(&connected);
        let server = thread::spawn(move || {
            let (_stream, _) = listener.accept().unwrap();
            accepted.store(true, Ordering::SeqCst);
            let _ = wait.recv_timeout(Duration::from_secs(10));
        });
        let interrupted = Arc::new(AtomicBool::new(false));
        let (alerter, log, _release) = slow_alerter(false);
        let stopped_at = Arc::new(Mutex::new(None));
        let (seen, stop, stamp) = (
            Arc::clone(&log),
            Arc::clone(&interrupted),
            Arc::clone(&stopped_at),
        );
        let (result, received, rows) = timer_session_with_obs(
            vec![
                keys(&[0], 1),
                keys(&[], 2),
                // The countdown has ended and its alert is running (blocked).
                Step::Run(Box::new(move || {
                    assert!(eventually(|| !seen.lock().unwrap().started.is_empty()));
                })),
                keys(&[4], 3),
                keys(&[], 4),
                // The OBS request is in flight: connected, waiting for an answer.
                Step::Run(Box::new(move || {
                    assert!(eventually(|| connected.load(Ordering::SeqCst)));
                    *stamp.lock().unwrap() = Some(Instant::now());
                    stop.store(true, Ordering::SeqCst);
                })),
            ],
            LiveOptions {
                timeout_ms: 5,
                ..LiveOptions::default()
            },
            interrupted,
            Some(alerter),
            None,
            false,
            Some(port),
        );
        let ended = stopped_at.lock().unwrap().unwrap().elapsed();
        release.send(()).unwrap();
        server.join().unwrap();
        result.unwrap();
        // Both are cut off at once: under the 3 s OBS deadline and far under
        // the 10 s the slow alert would otherwise take.
        assert!(ended < Duration::from_secs(2), "{ended:?}");
        assert_eq!(
            received,
            [Action::ObsScene {
                scene: "Live".into()
            }]
        );
        let log = log.lock().unwrap();
        assert_eq!(log.started.len(), 1);
        assert_eq!(log.killed, 1, "the running alert saw the stop");
        assert!(log.dropped, "the alert thread was joined");
        let failed = of_type(&rows, "action_failed");
        assert_eq!(failed.len(), 1, "{rows:?}");
        assert_eq!(
            failed[0]["action"],
            json!({"type":"obs_scene","scene_chars":4})
        );
        let error = failed[0]["error"].as_str().unwrap();
        assert!(error.contains("cancelled before OBS answered"), "{error}");
        let cut = of_type(&rows, "timer_alert_failed");
        assert_eq!(cut.len(), 1, "{rows:?}");
        let cut = cut[0]["error"].as_str().unwrap();
        assert!(cut.starts_with(ALERT_STOPPED), "{cut}");
        assert_eq!(rows.last().unwrap()["reason"], "interrupted");
    }

    #[test]
    fn a_normal_end_cuts_a_running_alert_off_quickly_without_a_drain_error() {
        let (alerter, log, _release) = slow_alerter(false);
        let seen = Arc::clone(&log);
        let started = Instant::now();
        let (result, _, rows) = timer_session_with(
            vec![
                keys(&[0], 1),
                // The alert is running (blocked) when the report limit is reached.
                Step::Run(Box::new(move || {
                    assert!(eventually(|| !seen.lock().unwrap().started.is_empty()));
                })),
                keys(&[], 2),
            ],
            LiveOptions {
                max_reports: Some(2),
                timeout_ms: 5,
                ..LiveOptions::default()
            },
            Arc::new(AtomicBool::new(false)),
            Some(alerter),
            None,
            false,
        );
        let elapsed = started.elapsed();
        let summary = result.unwrap();
        // About 1 s countdown + 2 s grace; never the whole alert or a drain error.
        assert!(elapsed < Duration::from_secs(6), "{elapsed:?}");
        let end = rows.last().unwrap();
        assert_eq!(end["reason"], "max_reports");
        assert!(end["error"].is_null(), "{end}");
        let log = log.lock().unwrap();
        assert_eq!(
            log.killed, 1,
            "the alert was stopped after the grace period"
        );
        assert!(log.dropped);
        let failed = of_type(&rows, "timer_alert_failed");
        assert_eq!(failed.len(), 1);
        assert!(
            failed[0]["error"]
                .as_str()
                .unwrap()
                .starts_with(ALERT_ENDED)
        );
        assert_eq!(summary.timer_alert_failed_count, 1);
        assert_eq!(summary.failed_action_count, 0);
    }

    #[test]
    fn an_alert_thread_that_cannot_start_fails_each_alert_and_the_session_goes_on() {
        let (alerter, log, _release) = slow_alerter(false);
        let (result, received, rows) = timer_session_with(
            vec![
                keys(&[0], 1),
                keys(&[], 2),
                keys(&[3], 3),
                keys(&[], 4),
                Step::Run(Box::new(|| thread::sleep(Duration::from_millis(1_500)))),
                keys(&[2], 5),
                keys(&[], 6),
            ],
            LiveOptions {
                max_reports: Some(6),
                timeout_ms: 5,
                ..LiveOptions::default()
            },
            Arc::new(AtomicBool::new(false)),
            Some(alerter),
            None,
            true,
        );
        result.unwrap();
        assert_eq!(received, [Action::ToggleMute], "later keys still run");
        assert_eq!(of_type(&rows, "timer_expired").len(), 2);
        let failed = of_type(&rows, "timer_alert_failed");
        assert_eq!(failed.len(), 2, "every expiry has an outcome");
        for record in failed {
            let error = record["error"].as_str().unwrap();
            assert!(
                error.contains("could not start timer alert thread")
                    && error.ends_with("nothing was shown or played"),
                "{error}"
            );
        }
        assert!(log.lock().unwrap().started.is_empty());
        assert_eq!(rows.last().unwrap()["reason"], "max_reports");
    }

    #[test]
    fn a_stuck_alert_thread_is_reported_with_its_unreported_alerts() {
        let (alerter, log, _release) = slow_alerter(true);
        let seen = Arc::clone(&log);
        let (result, _, rows) = timer_session_with(
            vec![
                keys(&[0], 1),
                Step::Run(Box::new(move || {
                    assert!(eventually(|| !seen.lock().unwrap().started.is_empty()));
                })),
                keys(&[], 2),
            ],
            LiveOptions {
                max_reports: Some(2),
                timeout_ms: 5,
                ..LiveOptions::default()
            },
            Arc::new(AtomicBool::new(false)),
            Some(alerter),
            None,
            false,
        );
        let error = result.unwrap_err();
        assert!(error.contains("1 alert(s) unreported"), "{error}");
        assert!(error.contains("thread was not joined"), "{error}");
        let failed = of_type(&rows, "timer_alert_failed");
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0]["error"], ALERT_UNKNOWN);
    }

    fn of_type<'a>(rows: &'a [Value], kind: &str) -> Vec<&'a Value> {
        rows.iter().filter(|row| row["type"] == kind).collect()
    }

    #[test]
    fn timers_expire_on_the_worker_and_the_rest_are_cancelled_when_input_ends() {
        let (result, received, rows) = timer_session(
            vec![keys(&[0], 1), keys(&[], 2), keys(&[1], 3), keys(&[], 4)],
            LiveOptions {
                // A 1.5 s margin past the one-second deadline, for loaded machines.
                duration: Some(Duration::from_millis(2_500)),
                timeout_ms: 5,
                ..LiveOptions::default()
            },
            Arc::new(AtomicBool::new(false)),
        );
        let summary = result.unwrap();
        // Only the expired one-second timer reached the host, as an alert.
        assert_eq!(
            received,
            [Action::Timer {
                seconds: 1,
                label: Some("Tea".into()),
                sound: Some(cooltek_sdk_service::TimerSound::None),
            }]
        );
        assert_eq!(of_type(&rows, "timer_started").len(), 2);
        let expired = of_type(&rows, "timer_expired");
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0]["timer"]["control"], 0);
        assert_eq!(
            expired[0]["action"],
            json!({"type":"timer","seconds":1,"label_chars":3,"sound":"none"})
        );
        let cancelled = of_type(&rows, "timer_cancelled");
        assert_eq!(cancelled.len(), 1);
        assert_eq!(cancelled[0]["timer"]["control"], 1);
        assert_eq!(cancelled[0]["reason"], "session_end");
        assert_eq!(summary.action_count, 2);
        assert_eq!(summary.incomplete_action_count, 0);
        let end = rows.last().unwrap();
        assert_eq!(end["type"], "service_end");
        assert_eq!(end["timer_started_count"], 2);
        assert_eq!(end["timer_expired_count"], 1);
        assert_eq!(end["timer_cancelled_count"], 1);
        assert!(!rows.iter().any(|row| row.to_string().contains("Tea")));
    }

    #[test]
    fn an_interrupted_session_cancels_running_timers_without_any_alert() {
        let interrupted = Arc::new(AtomicBool::new(false));
        let stop = Arc::clone(&interrupted);
        let (result, received, rows) = timer_session(
            vec![
                keys(&[0], 1),
                keys(&[], 2),
                // Give the worker time to start the countdown, then stop.
                Step::Run(Box::new(|| thread::sleep(Duration::from_millis(200)))),
                Step::Run(Box::new(move || stop.store(true, Ordering::SeqCst))),
            ],
            LiveOptions {
                timeout_ms: 5,
                ..LiveOptions::default()
            },
            interrupted,
        );
        result.unwrap();
        assert!(received.is_empty(), "no alert after a stop");
        let cancelled = of_type(&rows, "timer_cancelled");
        assert_eq!(cancelled.len(), 1);
        assert_eq!(cancelled[0]["reason"], "stopped");
        assert!(of_type(&rows, "timer_expired").is_empty());
        assert_eq!(rows.last().unwrap()["reason"], "interrupted");
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
    fn a_profile_handed_over_by_the_supervisor_plans_the_next_report() {
        let swapped = ProfileSession::new(
            Profile::from_json(
                br#"{"schema_version":1,"name":"swapped","bindings":[
                {"module":"m131e","control":0,"gesture":"right","action":{"type":"volume","delta":9}}
            ]}"#,
            )
            .unwrap(),
        )
        .unwrap();
        let slot: &'static Mutex<Option<ProfileSession>> =
            Box::leak(Box::new(Mutex::new(Some(swapped))));
        let transport = Transport::new([dial(3, 0, 10)]);
        let received = Arc::new(Mutex::new(Vec::new()));
        let actions = Arc::clone(&received);
        run_fake(
            transport,
            LiveOptions {
                profile_updates: Some(slot),
                ..options()
            },
            Arc::new(AtomicBool::new(false)),
            Output::default(),
            move |stop| simple_actions(stop, actions),
        )
        .unwrap();
        // The turn that the original profile binds to +5 volume plans the swapped profile's +9, and the slot is used up.
        assert_eq!(*received.lock().unwrap(), vec![Action::Volume { delta: 9 }]);
        assert!(slot.lock().unwrap().is_none());
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
    fn a_waiting_worker_folds_queued_positions_and_parks_the_first_other_item() {
        let position = |percent| {
            work(vec![planned(
                0,
                Gesture::PositionChanged,
                action(Action::SetMicInputVolume { percent }),
            )])
        };
        let (sender, receiver) = mpsc::sync_channel(8);
        for percent in [20, 30, 40] {
            sender.send(position(percent)).unwrap();
        }
        let press = work(vec![planned(
            0,
            Gesture::Pressed,
            action(Action::ToggleMute),
        )]);
        sender.send(press).unwrap();
        sender.send(position(90)).unwrap();
        let mut carried = None;
        let folded = absorb_queued(position(10), &receiver, &mut carried);
        assert_eq!(folded.report_count, 4);
        assert_eq!(
            folded.events[0].resolution,
            action(Action::SetMicInputVolume { percent: 40 })
        );
        let parked = carried.expect("the button press is parked, not merged");
        assert_eq!(parked.events[0].event.gesture, Gesture::Pressed);
        // The later position stays queued behind it, in order.
        assert!(receiver.try_recv().is_ok());
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
        let obs = |input: &str, percent| {
            planned(
                1,
                Gesture::PositionChanged,
                action(Action::ObsSetInputVolume {
                    input: input.into(),
                    percent,
                }),
            )
        };
        assert_eq!(
            merge_event(&obs("Mic", 10), &obs("Mic", 70))
                .unwrap()
                .resolution,
            action(Action::ObsSetInputVolume {
                input: "Mic".into(),
                percent: 70
            })
        );
        assert!(merge_event(&obs("Mic", 10), &obs("Desktop", 70)).is_none());
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
    fn closed_stdin_stops_like_a_signal_restores_mode_and_says_stdin_closed() {
        let (stdin, writer) = io::pipe().unwrap();
        let interrupted = Arc::new(AtomicBool::new(false));
        let closed: &'static AtomicBool = Box::leak(Box::new(AtomicBool::new(false)));
        crate::interrupt::watch_stdin(stdin, Arc::clone(&interrupted), closed).unwrap();
        // No report limit: only the closed pipe can end this run.
        let transport = Transport::new([]);
        let writes = Arc::clone(&transport.writes);
        let output = Output::default();
        let writer_output = output.clone();
        let configuration = LiveOptions {
            software_mode: true,
            max_reports: None,
            stdin_closed: Some(closed),
            ..options()
        };
        let parent = thread::spawn(move || {
            thread::sleep(Duration::from_millis(100));
            let dropped = Instant::now();
            drop(writer);
            dropped
        });
        run_with_clock(
            &mut InputReader::new(transport),
            profile(),
            device(),
            false,
            json!({}),
            configuration,
            interrupted,
            move |stop| {
                Ok((
                    simple_actions(stop, Arc::new(Mutex::new(Vec::new()))),
                    writer_output,
                ))
            },
            || 0,
        )
        .unwrap();
        let dropped = parent.join().unwrap();
        assert!(dropped.elapsed() < Duration::from_secs(1));
        let footer = output.rows().last().unwrap().clone();
        assert_eq!(footer["type"], "service_end");
        assert_eq!(footer["reason"], "stdin_closed");
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
        run_display_with(
            keypad_profile(),
            steps,
            options,
            interrupted,
            display,
            received,
        )
    }
    fn run_display_with(
        session: ProfileSession,
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
            session,
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
    fn a_held_folder_key_stays_dimmed_after_it_navigates_until_release_and_the_option_gates_it() {
        for on in [true, false] {
            let fake = Arc::new(Mutex::new(FakeDisplaySink::default()));
            let run = run_display(
                vec![
                    wait_for_writes(&fake, 15),
                    keys(&[1], 10),
                    // The folder opens on the press; let that redraw land before the release.
                    Step::Run(Box::new({
                        let fake = Arc::clone(&fake);
                        move || {
                            assert!(wait_until(|| written(&fake) > 15), "navigation writes");
                            thread::sleep(Duration::from_millis(150));
                        }
                    })),
                    keys(&[], 20),
                    // Let the release redraw land before the run ends.
                    Step::Run(Box::new(|| thread::sleep(Duration::from_millis(150)))),
                    keys(&[], 30),
                ],
                keypad_options(3),
                Arc::new(AtomicBool::new(false)),
                Some(shared_display(&fake, None).with_press_feedback(on)),
                Arc::default(),
            );
            run.result.unwrap();
            let triggers: Vec<_> = display_rows(&run.rows)
                .iter()
                .map(|record| record["trigger"].as_str().unwrap().to_owned())
                .collect();
            if on {
                // The new page draws key 1 dimmed; the release writes it back, alone.
                assert_eq!(triggers, ["software_mode", "navigation", "press"]);
                let rows = display_rows(&run.rows);
                assert_eq!(rows[2]["written"], json!([1]));
            } else {
                assert_eq!(triggers, ["software_mode", "navigation"]);
            }
        }
    }

    #[test]
    fn a_running_timer_key_counts_down_in_orange_and_gets_its_picture_back() {
        let mut slots = vec![serde_json::Value::Null; 15];
        slots[2] = json!({"type":"action","label":"Timer","bindings":[
            {"gesture":"pressed","action":{"type":"timer","seconds":3,"sound":"none"}}]});
        let profile = json!({"schema_version":2,"name":"timer","bindings":[],
            "keypad":{"pages":[{"id":"root","name":"Keys","slots":slots}]}});
        let session =
            ProfileSession::new(Profile::from_json(profile.to_string().as_bytes()).unwrap())
                .unwrap();
        let orange_on_key_2 = |fake: &Arc<Mutex<FakeDisplaySink>>| {
            fake.lock()
                .unwrap()
                .writes
                .iter()
                .filter(|write| write.target == DisplayTarget::Key(2))
                .map(|write| {
                    image::load_from_memory(&write.jpeg)
                        .unwrap()
                        .to_rgb8()
                        .pixels()
                        .filter(|pixel| {
                            pixel.0[0] > 200 && (80..200).contains(&pixel.0[1]) && pixel.0[2] < 90
                        })
                        .count()
                })
                .collect::<Vec<_>>()
        };
        let fake = Arc::new(Mutex::new(FakeDisplaySink::default()));
        let run = run_display_with(
            session,
            vec![
                wait_for_writes(&fake, 15),
                keys(&[2], 10),
                keys(&[], 20),
                // The countdown draws within a tick and then redraws as seconds pass.
                Step::Run(Box::new({
                    let fake = Arc::clone(&fake);
                    let counts = orange_on_key_2;
                    move || {
                        assert!(
                            wait_until(|| counts(&fake).iter().any(|&count| count > 40)),
                            "a countdown frame"
                        );
                        // The timer ends after three seconds and the key's own picture returns.
                        let deadline = Instant::now() + Duration::from_secs(6);
                        while counts(&fake).last().is_none_or(|&count| count > 0)
                            && Instant::now() < deadline
                        {
                            thread::sleep(Duration::from_millis(20));
                        }
                        assert_eq!(counts(&fake).last(), Some(&0), "the picture is back");
                        assert!(
                            counts(&fake).iter().filter(|&&count| count > 40).count() >= 2,
                            "the countdown redrew as seconds passed"
                        );
                    }
                })),
                keys(&[], 30),
            ],
            keypad_options(3),
            Arc::new(AtomicBool::new(false)),
            Some(shared_display(&fake, None)),
            Arc::default(),
        );
        run.result.unwrap();
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

    #[test]
    fn animation_waits_for_the_earliest_frame_but_never_sooner_than_the_gap() {
        let ms = Duration::from_millis;
        let gap = ANIMATION_MIN_INTERVAL;
        assert_eq!(animation_wait(None, None, gap), None);
        assert_eq!(animation_wait(None, Some(ms(0)), gap), None);
        assert_eq!(animation_wait(Some(ms(30)), None, gap), Some(ms(30)));
        // Two keys 30 ms apart: the second waits out the gap after the first.
        assert_eq!(animation_wait(Some(ms(30)), Some(ms(0)), gap), Some(gap));
        assert_eq!(
            animation_wait(Some(ms(30)), Some(ms(80)), gap),
            Some(ms(30))
        );
        assert_eq!(animation_wait(Some(ms(5)), Some(ms(80)), gap), Some(ms(20)));
        assert_eq!(
            animation_wait(Some(ms(250)), Some(ms(10)), gap),
            Some(ms(250))
        );
        assert_eq!(animation_wait(Some(ms(1)), Some(ms(500)), gap), Some(ms(1)));
    }

    /// A key display thread drawing `profile`'s artwork into `fake`.
    fn animated_display(
        fake: &Arc<Mutex<FakeDisplaySink>>,
        profile: Profile,
        abort: &Arc<AtomicBool>,
        hook: Option<WriteHook>,
    ) -> (DisplayThread, ProfileSession, Receiver<Value>) {
        let display = LiveDisplay::new(
            SharedSink {
                fake: Arc::clone(fake),
                before_write: hook,
            },
            PreparedKeyArtwork::prepare_profile(&profile).unwrap(),
            Arc::new(AtomicBool::new(false)),
        );
        let session = ProfileSession::new(profile).unwrap();
        let (records_tx, records_rx) = mpsc::sync_channel(DISPLAY_RECORD_QUEUE);
        let display =
            DisplayThread::spawn(display, session.clone(), Arc::clone(abort), records_tx).unwrap();
        display.request(&session, "software_mode");
        (display, session, records_rx)
    }
    fn blinking_root_key_3() -> Profile {
        use crate::display_artwork::tests::{key_artwork, profile_with};
        profile_with(vec![
            (0, "3", key_artwork(&[(10, 100), (200, 100)], 0)),
            (0, "5", key_artwork(&[(60, 100)], 1)),
        ])
    }
    fn successful_after(fake: &Arc<Mutex<FakeDisplaySink>>, skip: usize) -> Vec<DisplayTarget> {
        fake.lock().unwrap().successful()[skip..].to_vec()
    }

    /// A key display thread over a plain profile, with press feedback `on`.
    fn pressable_display(
        fake: &Arc<Mutex<FakeDisplaySink>>,
        on: bool,
        abort: &Arc<AtomicBool>,
    ) -> (DisplayThread, ProfileSession, Receiver<Value>) {
        use crate::display_artwork::tests::{key_artwork, profile_with};
        let profile = profile_with(vec![(0, "3", key_artwork(&[(120, 100)], 1))]);
        let display = LiveDisplay::new(
            SharedSink {
                fake: Arc::clone(fake),
                before_write: None,
            },
            PreparedKeyArtwork::prepare_profile(&profile).unwrap(),
            Arc::new(AtomicBool::new(false)),
        )
        .with_press_feedback(on);
        let session = ProfileSession::new(profile).unwrap();
        let (records_tx, records_rx) = mpsc::sync_channel(DISPLAY_RECORD_QUEUE);
        let display =
            DisplayThread::spawn(display, session.clone(), Arc::clone(abort), records_tx).unwrap();
        display.request(&session, "software_mode");
        (display, session, records_rx)
    }

    #[test]
    fn a_held_key_is_drawn_dimmed_and_restored_with_one_write_each() {
        let fake = Arc::new(Mutex::new(FakeDisplaySink::default()));
        let abort = Arc::new(AtomicBool::new(false));
        let (mut display, session, records) = pressable_display(&fake, true, &abort);
        assert_eq!(
            records.recv_timeout(Duration::from_secs(2)).unwrap()["trigger"],
            "software_mode"
        );
        assert_eq!(written(&fake), 15);
        let resting = fake.lock().unwrap().writes[3].jpeg.clone();
        display.request_pressed(&session, PRESS_TRIGGER, Some(3));
        assert_eq!(
            records.recv_timeout(Duration::from_secs(2)).unwrap()["trigger"],
            "press"
        );
        assert_eq!(written(&fake), 16);
        let dimmed = fake.lock().unwrap().writes[15].clone();
        assert_eq!(dimmed.target, DisplayTarget::Key(3));
        assert_ne!(dimmed.jpeg, resting);
        display.request_pressed(&session, PRESS_TRIGGER, None);
        assert_eq!(
            records.recv_timeout(Duration::from_secs(2)).unwrap()["trigger"],
            "press"
        );
        assert_eq!(written(&fake), 17);
        let restored = fake.lock().unwrap().writes[16].clone();
        assert_eq!(restored.target, DisplayTarget::Key(3));
        assert_eq!(restored.jpeg, resting);
        assert!(display.finish(Duration::from_secs(2)));
    }

    #[test]
    fn a_press_draws_nothing_extra_while_the_option_is_off() {
        let fake = Arc::new(Mutex::new(FakeDisplaySink::default()));
        let abort = Arc::new(AtomicBool::new(false));
        let (mut display, session, records) = pressable_display(&fake, false, &abort);
        records.recv_timeout(Duration::from_secs(2)).unwrap();
        display.request_pressed(&session, PRESS_TRIGGER, Some(3));
        records.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(written(&fake), 15);
        assert!(display.finish(Duration::from_secs(2)));
    }

    #[test]
    fn animation_ticks_redraw_only_changed_keys_and_log_nothing_when_clean() {
        let fake = Arc::new(Mutex::new(FakeDisplaySink::default()));
        let abort = Arc::new(AtomicBool::new(false));
        let started = Instant::now();
        let (mut display, _, records) =
            animated_display(&fake, blinking_root_key_3(), &abort, None);
        assert!(wait_until(|| written(&fake) >= 15 + 3), "animation writes");
        assert!(display.finish(Duration::from_secs(2)));
        // Three 100 ms frames after the page draw take at least 300 ms.
        assert!(started.elapsed() >= Duration::from_millis(300));
        let after_page = successful_after(&fake, 15);
        assert!(after_page.len() >= 3);
        assert!(
            after_page
                .iter()
                .all(|target| *target == DisplayTarget::Key(3))
        );
        let first = records.try_recv().unwrap();
        assert_eq!(first["trigger"], "software_mode");
        assert_eq!(first["written"], json!((0..15).collect::<Vec<u8>>()));
        assert!(records.try_recv().is_err(), "clean frames are not logged");
        let syncs = display.counts.syncs.load(Ordering::SeqCst);
        assert!(syncs as usize > after_page.len(), "{syncs}");
        assert_eq!(display.counts.failures.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_failed_animation_frame_is_logged_once_and_a_clean_request_resumes_animation() {
        use crate::display_artwork::tests::{key_artwork, profile_with};
        let fake = Arc::new(Mutex::new(FakeDisplaySink::default()));
        // The page draw and the first frame succeed; the second frame is short.
        fake.lock().unwrap().fail_after(
            DisplayTarget::Key(3),
            2,
            SinkError::ShortWrite {
                written: 10,
                expected: 65,
            },
        );
        let profile = profile_with(vec![
            (0, "3", key_artwork(&[(10, 100), (200, 100)], 0)),
            (1, "4", key_artwork(&[(30, 100), (90, 100)], 0)),
        ]);
        let abort = Arc::new(AtomicBool::new(false));
        let (mut display, mut session, records) = animated_display(&fake, profile, &abort, None);
        assert_eq!(
            records.recv_timeout(Duration::from_secs(2)).unwrap()["trigger"],
            "software_mode"
        );
        let failed = records.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(failed["trigger"], "animation");
        assert_eq!(failed["page_id"], "root");
        assert_eq!(failed["failed"][0]["key_index"], 3);
        assert_eq!(failed["failed"][0]["short_write"], true);
        assert_eq!(failed["complete"], false);
        let attempts = written(&fake);
        assert_eq!(attempts, 15 + 2);
        // Well past several frame deadlines, nothing else was attempted.
        thread::sleep(Duration::from_millis(350));
        assert_eq!(written(&fake), attempts);
        assert!(records.try_recv().is_err());
        // A clean explicit page request draws and resumes animation there.
        session
            .set_path(&["root".into(), "development".into()])
            .unwrap();
        display.request(&session, "navigation");
        let navigation = records.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(navigation["trigger"], "navigation");
        assert_eq!(navigation["complete"], true);
        let drawn = written(&fake);
        assert!(drawn > attempts);
        assert!(
            wait_until(|| written(&fake) >= drawn + 3),
            "animation resumed"
        );
        assert!(display.finish(Duration::from_secs(2)));
        let resumed = successful_after(&fake, drawn - 1);
        assert!(
            resumed
                .iter()
                .all(|target| *target == DisplayTarget::Key(4))
        );
        // The resumed frames were clean: no further lines and no further failures.
        assert!(records.try_recv().is_err());
        assert_eq!(display.counts.failures.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn stopping_during_an_animation_frame_logs_and_counts_no_failure() {
        use crate::display_artwork::tests::{key_artwork, profile_with};
        let fake = Arc::new(Mutex::new(FakeDisplaySink::default()));
        let gate: Gate = Arc::new((Mutex::new(false), Condvar::new()));
        let (began_tx, began_rx) = mpsc::sync_channel(1);
        let sink_gate = Arc::clone(&gate);
        // Holds the first animation write (key 3) until the test has stopped
        // the display, so key 4 of the same frame is refused by the closed gate.
        let hook: WriteHook = Box::new(move |attempts| {
            if attempts == 15 {
                began_tx.send(()).unwrap();
                let (lock, changed) = &*sink_gate;
                let released = lock.lock().unwrap();
                let _unused = changed
                    .wait_timeout_while(released, Duration::from_secs(5), |open| !*open)
                    .unwrap();
            }
        });
        let profile = profile_with(vec![
            (0, "3", key_artwork(&[(10, 100), (200, 100)], 0)),
            (0, "4", key_artwork(&[(30, 100), (90, 100)], 0)),
        ]);
        let abort = Arc::new(AtomicBool::new(false));
        let (mut display, _, records) = animated_display(&fake, profile, &abort, Some(hook));
        began_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        display.stop();
        *gate.0.lock().unwrap() = true;
        gate.1.notify_all();
        assert!(display.finish(Duration::from_secs(2)));
        // Key 3 was in flight and completed; key 4 never reached the sink.
        assert_eq!(written(&fake), 16);
        assert_eq!(records.try_recv().unwrap()["trigger"], "software_mode");
        assert!(records.try_recv().is_err(), "a Stop is not a failed frame");
        assert_eq!(display.counts.failures.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn animation_syncs_leave_the_minimum_gap_between_them() {
        use crate::display_artwork::tests::{key_artwork, profile_with};
        use std::sync::OnceLock;
        let fake = Arc::new(Mutex::new(FakeDisplaySink::default()));
        // Each write is tagged with the number of syncs started so far, so the
        // writes of one sync share a tag regardless of timing.
        let counts: Arc<OnceLock<Arc<DisplayCounts>>> = Arc::default();
        let writes = Arc::new(Mutex::new(Vec::new()));
        let (hook_counts, log) = (Arc::clone(&counts), Arc::clone(&writes));
        // Slow writes: the gap must follow the end of a sync, not its start.
        let hook: WriteHook = Box::new(move |attempts| {
            if attempts >= 15 {
                thread::sleep(Duration::from_millis(30));
            }
            let sync = hook_counts.get().unwrap().syncs.load(Ordering::SeqCst);
            log.lock().unwrap().push((sync, Instant::now()));
        });
        // Deadlines 50 ms apart (100, 150, 200, 300 ms...) are never drawn 50 ms apart.
        let profile = profile_with(vec![
            (0, "3", key_artwork(&[(10, 100), (200, 100)], 0)),
            (0, "4", key_artwork(&[(30, 150), (90, 150)], 0)),
        ]);
        let display = LiveDisplay::new(
            SharedSink {
                fake: Arc::clone(&fake),
                before_write: Some(hook),
            },
            PreparedKeyArtwork::prepare_profile(&profile).unwrap(),
            Arc::new(AtomicBool::new(false)),
        );
        let session = ProfileSession::new(profile).unwrap();
        let (records_tx, _records) = mpsc::sync_channel(DISPLAY_RECORD_QUEUE);
        let mut display = DisplayThread::spawn(
            display,
            session.clone(),
            Arc::new(AtomicBool::new(false)),
            records_tx,
        )
        .unwrap();
        counts.set(Arc::clone(&display.counts)).ok().unwrap();
        // (first, last) write time of each animation sync; tag 1 is the page draw.
        let sync_windows = |writes: Vec<(u64, Instant)>| {
            let mut syncs: Vec<(u64, Instant, Instant)> = Vec::new();
            for (sync, time) in writes.into_iter().filter(|(sync, _)| *sync > 1) {
                match syncs.last_mut() {
                    Some(last) if last.0 == sync => last.2 = time,
                    _ => syncs.push((sync, time, time)),
                }
            }
            syncs
        };
        display.request(&session, "software_mode");
        // Wait for syncs that actually wrote: the started-sync counter also
        // counts syncs with nothing to draw, so on a slow runner it can reach
        // its target while fewer than four syncs have written anything.
        assert!(
            wait_until(|| sync_windows(writes.lock().unwrap().clone()).len() >= 5),
            "animation syncs"
        );
        assert!(display.finish(Duration::from_secs(2)));
        let syncs = sync_windows(writes.lock().unwrap().clone());
        assert!(syncs.len() >= 4, "{}", syncs.len());
        for pair in syncs.windows(2) {
            let gap = pair[1].1 - pair[0].2;
            assert!(gap >= ANIMATION_MIN_INTERVAL, "sync {}: {gap:?}", pair[1].0);
        }
    }

    #[test]
    fn a_real_failure_in_a_frame_interrupted_by_stop_is_still_logged() {
        use crate::display_artwork::tests::{key_artwork, profile_with};
        let fake = Arc::new(Mutex::new(FakeDisplaySink::default()));
        fake.lock().unwrap().fail_after(
            DisplayTarget::Key(3),
            1,
            SinkError::Transport("device busy".into()),
        );
        let gate: Gate = Arc::new((Mutex::new(false), Condvar::new()));
        let (began_tx, began_rx) = mpsc::sync_channel(1);
        let sink_gate = Arc::clone(&gate);
        // In the first animation frame key 3 fails (attempt 15), then key 4 is
        // held until the test has stopped the display, so key 5 is refused.
        let hook: WriteHook = Box::new(move |attempts| {
            if attempts == 16 {
                began_tx.send(()).unwrap();
                let (lock, changed) = &*sink_gate;
                let released = lock.lock().unwrap();
                let _unused = changed
                    .wait_timeout_while(released, Duration::from_secs(5), |open| !*open)
                    .unwrap();
            }
        });
        let frames = || key_artwork(&[(10, 100), (200, 100)], 0);
        let profile = profile_with(vec![
            (0, "3", frames()),
            (0, "4", frames()),
            (0, "5", frames()),
        ]);
        let abort = Arc::new(AtomicBool::new(false));
        let (mut display, _, records) = animated_display(&fake, profile, &abort, Some(hook));
        began_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        display.stop();
        *gate.0.lock().unwrap() = true;
        gate.1.notify_all();
        assert!(display.finish(Duration::from_secs(2)));
        assert_eq!(records.try_recv().unwrap()["trigger"], "software_mode");
        let failed = records.try_recv().unwrap();
        assert_eq!(failed["trigger"], "animation");
        assert_eq!(failed["written"], json!([4]));
        assert_eq!(failed["failed"][0]["key_index"], 3);
        assert!(
            failed["failed"][0]["error"]
                .as_str()
                .unwrap()
                .contains("device busy")
        );
        assert_eq!(failed["failed"][1]["key_index"], 5);
        assert!(records.try_recv().is_err());
        assert_eq!(display.counts.failures.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn an_unclean_explicit_sync_leaves_animation_paused() {
        use crate::display_artwork::tests::{key_artwork, profile_with};
        let fake = Arc::new(Mutex::new(FakeDisplaySink::default()));
        let short = || SinkError::ShortWrite {
            written: 10,
            expected: 65,
        };
        // The second animation frame fails, then so does the navigation draw.
        fake.lock()
            .unwrap()
            .fail_after(DisplayTarget::Key(3), 2, short());
        fake.lock()
            .unwrap()
            .fail_after(DisplayTarget::Key(4), 1, short());
        let profile = profile_with(vec![
            (0, "3", key_artwork(&[(10, 100), (200, 100)], 0)),
            (1, "4", key_artwork(&[(30, 100), (90, 100)], 0)),
        ]);
        let abort = Arc::new(AtomicBool::new(false));
        let (mut display, mut session, records) = animated_display(&fake, profile, &abort, None);
        for trigger in ["software_mode", "animation"] {
            let record = records.recv_timeout(Duration::from_secs(2)).unwrap();
            assert_eq!(record["trigger"], trigger);
        }
        session
            .set_path(&["root".into(), "development".into()])
            .unwrap();
        display.request(&session, "navigation");
        let navigation = records.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(navigation["trigger"], "navigation");
        assert_eq!(navigation["complete"], false);
        let attempts = written(&fake);
        thread::sleep(Duration::from_millis(350));
        assert_eq!(written(&fake), attempts, "animation stays paused");
        assert!(display.finish(Duration::from_secs(2)));
        assert!(records.try_recv().is_err());
        assert_eq!(display.counts.failures.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn closing_the_gate_or_aborting_ends_animation_ticks() {
        for abort_run in [false, true] {
            let fake = Arc::new(Mutex::new(FakeDisplaySink::default()));
            let abort = Arc::new(AtomicBool::new(false));
            let (mut display, _, _records) =
                animated_display(&fake, blinking_root_key_3(), &abort, None);
            assert!(wait_until(|| written(&fake) > 15), "first frame");
            if abort_run {
                abort.store(true, Ordering::SeqCst);
                let handle = display.handle.as_ref().unwrap();
                assert!(wait_until(|| handle.is_finished()), "thread exits on abort");
            } else {
                assert!(display.finish(Duration::from_secs(2)));
            }
            let attempts = written(&fake);
            thread::sleep(Duration::from_millis(250));
            assert_eq!(written(&fake), attempts, "abort: {abort_run}");
            assert!(display.finish(Duration::from_secs(2)));
        }
    }

    #[test]
    fn static_profiles_never_tick() {
        use crate::display_artwork::tests::{key_artwork, profile_with};
        for profile in [
            profile_with(Vec::new()),
            profile_with(vec![(0, "3", key_artwork(&[(10, 100)], 0))]),
        ] {
            let fake = Arc::new(Mutex::new(FakeDisplaySink::default()));
            let abort = Arc::new(AtomicBool::new(false));
            let (mut display, _, records) = animated_display(&fake, profile, &abort, None);
            assert!(records.recv_timeout(Duration::from_secs(2)).is_ok());
            thread::sleep(Duration::from_millis(250));
            assert!(display.finish(Duration::from_secs(2)));
            assert_eq!(written(&fake), 15);
            assert_eq!(display.counts.syncs.load(Ordering::SeqCst), 1);
            assert!(records.try_recv().is_err());
        }
    }

    use crate::dial_artwork::{Clock, DialContent, ManualClock, WallTime};
    use cooltek_sdk_service::display_sync::DialImageProvider;

    /// A dial-only sink over a shared [`FakeDisplaySink`] that logs each
    /// attempt reaching it (`dial`) and when it happened.
    struct DialSink {
        fake: Arc<Mutex<FakeDisplaySink>>,
        log: Option<Arc<Mutex<Vec<&'static str>>>>,
        times: Arc<Mutex<Vec<Instant>>>,
    }
    impl DisplaySink for DialSink {
        fn capabilities(&self) -> DisplayCapabilities {
            DisplayCapabilities {
                key_count: 0,
                dial: true,
            }
        }
        fn write_key_image(
            &mut self,
            _: u8,
            _: &[u8],
            _: &mut dyn FnMut(usize),
        ) -> Result<(), SinkError> {
            panic!("dial display sync never writes keys")
        }
        fn write_dial_image(
            &mut self,
            jpeg: &[u8],
            between_chunks: &mut dyn FnMut(usize),
        ) -> Result<(), SinkError> {
            if let Some(log) = &self.log {
                log.lock().unwrap().push("dial");
            }
            self.times.lock().unwrap().push(Instant::now());
            self.fake
                .lock()
                .unwrap()
                .write_dial_image(jpeg, between_chunks)
        }
    }

    fn clock_artwork(content: DialContent, clock: impl Clock + 'static) -> DialArtwork {
        DialArtwork::prepare(content, two_functions().profile(), Box::new(clock)).unwrap()
    }

    struct DialHarness {
        display: LiveDisplay,
        fake: Arc<Mutex<FakeDisplaySink>>,
        times: Arc<Mutex<Vec<Instant>>>,
    }

    fn dial_harness(
        artwork: DialArtwork,
        interval: Duration,
        log: Option<Arc<Mutex<Vec<&'static str>>>>,
    ) -> DialHarness {
        let fake = Arc::new(Mutex::new(FakeDisplaySink::default()));
        let times = Arc::new(Mutex::new(Vec::new()));
        let sink = DialSink {
            fake: Arc::clone(&fake),
            log,
            times: Arc::clone(&times),
        };
        let display = LiveDisplay::dial(sink, artwork, Arc::new(AtomicBool::new(false)))
            .with_dial_interval(interval);
        DialHarness {
            display,
            fake,
            times,
        }
    }

    fn dial_writes(fake: &Arc<Mutex<FakeDisplaySink>>) -> Vec<Vec<u8>> {
        let fake = fake.lock().unwrap();
        fake.writes
            .iter()
            .filter(|write| write.fault.is_none())
            .map(|write| write.jpeg.clone())
            .collect()
    }

    fn run_dial(
        transport: Transport,
        session: ProfileSession,
        options: LiveOptions,
        display: Option<LiveDisplay>,
        received: Arc<Mutex<Vec<Action>>>,
    ) -> DisplayRun {
        let clock = Arc::clone(&transport.clock);
        let writes = Arc::clone(&transport.writes);
        let output = Output::default();
        let writer = output.clone();
        let actions = Arc::clone(&received);
        let result = run_inner(
            &mut InputReader::new(transport),
            session,
            device(),
            false,
            json!({"source":"synthetic"}),
            options,
            Arc::new(AtomicBool::new(false)),
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

    #[test]
    fn dial_display_requires_software_mode_and_an_m131e_before_any_worker() {
        let clock = ManualClock::new(WallTime::new(9, 0));
        for (options, device, expected) in [
            (options(), device(), "software mode"),
            (keypad_options(1), keypad_device(), "M131E"),
        ] {
            let harness = dial_harness(
                clock_artwork(DialContent::Clock, clock.clone()),
                Duration::from_millis(10),
                None,
            );
            let transport = Transport::new([]);
            let writes = Arc::clone(&transport.writes);
            let error = run_inner(
                &mut InputReader::new(transport),
                two_functions(),
                device,
                false,
                json!({}),
                options,
                Arc::new(AtomicBool::new(false)),
                Some(harness.display),
                |_| -> Result<(Actions, Output), String> { panic!("must not construct worker") },
                || 0,
            )
            .unwrap_err();
            assert!(error.contains(expected), "{error}");
            assert!(harness.fake.lock().unwrap().writes.is_empty());
            // Fail closed: no mode command was sent.
            assert!(writes.lock().unwrap().is_empty());
        }
    }

    #[test]
    fn the_dials_backlight_is_set_right_after_software_mode() {
        let clock = ManualClock::new(WallTime::new(9, 0));
        let harness = dial_harness(
            clock_artwork(DialContent::Function, clock),
            Duration::from_millis(20),
            None,
        );
        let fake = Arc::clone(&harness.fake);
        let transport = Transport::new([wait_for_writes(&fake, 1), dial(3, 0, 5)]);
        let run = run_dial(
            transport,
            two_functions(),
            LiveOptions {
                software_mode: true,
                max_reports: Some(1),
                dial_backlight: Some(40),
                ..options()
            },
            Some(harness.display),
            Arc::default(),
        );
        run.result.unwrap();
        assert_eq!(
            run.mode_writes,
            vec![
                dial::software_mode_report(true).to_vec(),
                dial::backlight_report(40).unwrap().to_vec(),
                dial::software_mode_report(false).to_vec(),
            ]
        );
    }

    #[test]
    fn a_dial_function_switch_redraws_the_dial_and_the_click_dispatches_nothing() {
        let clock = ManualClock::new(WallTime::new(9, 0));
        let expected = clock_artwork(DialContent::Function, clock.clone());
        let harness = dial_harness(
            clock_artwork(DialContent::Function, clock),
            Duration::from_millis(20),
            None,
        );
        let fake = Arc::clone(&harness.fake);
        let transport = Transport::new([
            wait_for_writes(&fake, 1),
            dial(4, 1, 5),
            dial(4, 0, 100_005),
            wait_for_writes(&fake, 2),
            dial(3, 0, 100_006),
        ]);
        let run = run_dial(
            transport,
            two_functions(),
            LiveOptions {
                software_mode: true,
                max_reports: Some(3),
                ..options()
            },
            Some(harness.display),
            Arc::default(),
        );
        let summary = run.result.unwrap();
        assert_eq!(run.mode_writes, modes());
        // The rotation after the click ran the second function's action only.
        assert_eq!(run.actions, vec![Action::Volume { delta: 1 }]);
        assert_eq!(
            dial_writes(&fake),
            vec![
                expected.function_face(0).unwrap().to_vec(),
                expected.function_face(1).unwrap().to_vec()
            ]
        );
        let records = display_rows(&run.rows);
        assert_eq!(records.len(), 2);
        assert_eq!(records[0]["target"], "dial");
        assert_eq!(records[0]["trigger"], "software_mode");
        assert_eq!(records[0]["face"], "function:0");
        assert_eq!(records[0]["written"], true);
        assert_eq!(records[0]["complete"], true);
        assert_eq!(records[1]["trigger"], "dial_function");
        assert_eq!(records[1]["face"], "function:1");
        assert_eq!(summary.display_sync_count, 2);
        assert_eq!(summary.display_failure_count, 0);
        assert_eq!(
            run.rows[0]["details"]["display_sync"],
            json!({"enabled":true, "hardware_verified":false, "target":"dial",
                "dial_content":"function", "min_interval_ms":20})
        );
        let footer = run.rows.last().unwrap();
        assert_eq!(footer["display_sync_count"], 2);
        assert_eq!(footer["display_stopped"], true);
    }

    #[test]
    fn the_clock_redraws_on_a_new_minute_only_and_at_most_once_per_interval() {
        assert_eq!(DIAL_MIN_INTERVAL, Duration::from_secs(1));
        // Long enough that the two clock changes below always land inside one
        // interval, even on a heavily loaded test runner.
        let interval = Duration::from_secs(2);
        let clock = ManualClock::new(WallTime::new(9, 0));
        let expected = clock_artwork(DialContent::Clock, clock.clone());
        let harness = dial_harness(
            clock_artwork(DialContent::Clock, clock.clone()),
            interval,
            None,
        );
        let (records_tx, records_rx) = mpsc::sync_channel(DISPLAY_RECORD_QUEUE);
        let session = two_functions();
        let mut display = DisplayThread::spawn(
            harness.display,
            session.clone(),
            Arc::new(AtomicBool::new(false)),
            records_tx,
        )
        .unwrap();
        // Nothing is drawn before the first request (sent once software mode is on).
        thread::sleep(Duration::from_millis(250));
        assert!(written(&harness.fake) == 0);
        display.request(&session, "software_mode");
        assert!(wait_until(|| written(&harness.fake) == 1));
        // Two new minutes right after a write: only the newest is drawn, once
        // the interval since that write has passed.
        clock.set(WallTime::new(9, 1));
        thread::sleep(Duration::from_millis(30));
        clock.set(WallTime::new(9, 2));
        let deadline = Instant::now() + Duration::from_secs(10);
        while written(&harness.fake) < 2 {
            assert!(Instant::now() < deadline, "the new minute was never drawn");
            thread::sleep(Duration::from_millis(5));
        }
        // The same minute is never redrawn, however often it is requested.
        display.request(&session, "dial_function");
        thread::sleep(Duration::from_millis(400));
        assert!(display.finish(Duration::from_secs(2)));
        let writes = dial_writes(&harness.fake);
        assert_eq!(writes.len(), 2);
        let request = expected.request(None);
        assert_eq!(request.function.as_deref(), Some("clock:09:02"));
        assert_eq!(writes[1], expected.dial_image(&request).unwrap());
        let times = harness.times.lock().unwrap().clone();
        // Encoding time varies a little between faces; allow for it.
        assert!(times[1] - times[0] >= interval - Duration::from_millis(50));
        let records: Vec<Value> = records_rx.try_iter().collect();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0]["face"], "clock:09:00");
        assert_eq!(records[1]["face"], "clock:09:02");
        assert_eq!(records[1]["trigger"], "clock");
        assert_eq!(display.counts.syncs.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_plain_transport_failure_is_retried_once_after_the_interval() {
        let busy = || SinkError::Transport(crate::display_sink::SHARED_BUSY.into());
        // Function content: without a retry a busy device would leave the old
        // name up until the next click.
        let start = |faults: usize| {
            let harness = dial_harness(
                clock_artwork(DialContent::Function, ManualClock::new(WallTime::new(9, 0))),
                Duration::from_millis(50),
                None,
            );
            for _ in 0..faults {
                harness
                    .fake
                    .lock()
                    .unwrap()
                    .fail_next(DisplayTarget::Dial, busy());
            }
            let (records_tx, records_rx) = mpsc::sync_channel(DISPLAY_RECORD_QUEUE);
            let session = two_functions();
            let display = DisplayThread::spawn(
                harness.display,
                session.clone(),
                Arc::new(AtomicBool::new(false)),
                records_tx,
            )
            .unwrap();
            display.request(&session, "software_mode");
            (display, harness.fake, harness.times, records_rx, session)
        };

        // One busy failure: the same face is redrawn once, after the interval.
        let (mut display, fake, times, records, _) = start(1);
        let first = records.recv_timeout(Duration::from_secs(2)).unwrap();
        let second = records.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(first["written"], false);
        assert_eq!(first["failed"]["short_write"], false);
        assert_eq!(second["trigger"], "retry");
        assert_eq!(second["face"], "function:0");
        assert_eq!(second["written"], true);
        thread::sleep(Duration::from_millis(200));
        assert!(display.finish(Duration::from_secs(2)));
        assert_eq!(written(&fake), 2);
        let times = times.lock().unwrap().clone();
        assert!(times[1] - times[0] >= Duration::from_millis(40));

        // Two busy failures: one retry only, then nothing until the face changes.
        let (mut display, fake, _, records, mut session) = start(2);
        records.recv_timeout(Duration::from_secs(2)).unwrap();
        let retried = records.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(retried["written"], false);
        thread::sleep(Duration::from_millis(300));
        assert_eq!(written(&fake), 2);
        session.set_dial_function(1).unwrap();
        display.request(&session, "dial_function");
        let next = records.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(display.finish(Duration::from_secs(2)));
        assert_eq!(next["face"], "function:1");
        assert_eq!(next["written"], true);
        assert_eq!(written(&fake), 3);
    }

    #[test]
    fn a_face_can_use_its_retry_again_after_another_face_and_labels_follow_the_face() {
        let busy = || SinkError::Transport(crate::display_sink::SHARED_BUSY.into());
        // Function 0 fails twice (attempt and retry), 1 is written, then 0 fails
        // once more: it gets a fresh retry, which succeeds.
        let harness = dial_harness(
            clock_artwork(DialContent::Function, ManualClock::new(WallTime::new(9, 0))),
            Duration::from_millis(50),
            None,
        );
        {
            let mut fake = harness.fake.lock().unwrap();
            fake.fail_next(DisplayTarget::Dial, busy());
            fake.fail_next(DisplayTarget::Dial, busy());
            fake.fail_after(DisplayTarget::Dial, 1, busy());
        }
        let (records_tx, records) = mpsc::sync_channel(DISPLAY_RECORD_QUEUE);
        let mut session = two_functions();
        let mut display = DisplayThread::spawn(
            harness.display,
            session.clone(),
            Arc::new(AtomicBool::new(false)),
            records_tx,
        )
        .unwrap();
        let next = || records.recv_timeout(Duration::from_secs(2)).unwrap();
        display.request(&session, "software_mode");
        let (first, retry) = (next(), next());
        assert_eq!(
            (first["written"].clone(), retry["trigger"].clone()),
            (json!(false), json!("retry"))
        );
        assert_eq!(retry["written"], false);
        session.set_dial_function(1).unwrap();
        display.request(&session, "dial_function");
        let one = next();
        assert_eq!(
            (one["face"].clone(), one["written"].clone()),
            (json!("function:1"), json!(true))
        );
        assert_eq!(one["trigger"], "dial_function");
        session.set_dial_function(0).unwrap();
        display.request(&session, "dial_function");
        let (again, retried) = (next(), next());
        assert!(display.finish(Duration::from_secs(2)));
        assert_eq!(again["face"], "function:0");
        assert_eq!(again["written"], false);
        assert_eq!(retried["trigger"], "retry");
        assert_eq!(retried["written"], true);
        assert_eq!(written(&harness.fake), 5);

        // Clock mode: a failure, then the minute rolls over before the retry.
        // The new minute is drawn instead, labelled `clock`, not `retry`.
        let clock = ManualClock::new(WallTime::new(9, 0));
        let harness = dial_harness(
            clock_artwork(DialContent::Clock, clock.clone()),
            Duration::from_millis(200),
            None,
        );
        harness
            .fake
            .lock()
            .unwrap()
            .fail_next(DisplayTarget::Dial, busy());
        let (records_tx, records) = mpsc::sync_channel(DISPLAY_RECORD_QUEUE);
        let mut display = DisplayThread::spawn(
            harness.display,
            session.clone(),
            Arc::new(AtomicBool::new(false)),
            records_tx,
        )
        .unwrap();
        display.request(&session, "software_mode");
        let failed = records.recv_timeout(Duration::from_secs(2)).unwrap();
        clock.set(WallTime::new(9, 1));
        let rolled = records.recv_timeout(Duration::from_secs(2)).unwrap();
        thread::sleep(Duration::from_millis(400));
        assert!(display.finish(Duration::from_secs(2)));
        assert_eq!(failed["written"], false);
        assert_eq!(rolled["face"], "clock:09:01");
        assert_eq!(rolled["trigger"], "clock");
        assert_eq!(rolled["written"], true);
        // The stale 9:00 face is never retried after the new minute was drawn.
        assert_eq!(written(&harness.fake), 2);
    }

    #[test]
    fn writes_refused_by_the_closed_gate_are_never_retried() {
        let mut state = DialState {
            started: true,
            last_face: Some(Some("function:0".into())),
            ..DialState::default()
        };
        state.finished(DialOutcome::Failed);
        assert_eq!(state.retry_pending, None);
        assert!(state.last_face.is_some());
        // The classification: a closed gate is `Failed`, a busy device is a retry.
        let mut sink = FakeDisplaySink::default();
        sink.fail_next(
            DisplayTarget::Dial,
            SinkError::Transport(DISPLAY_WRITES_CLOSED.into()),
        );
        sink.fail_next(
            DisplayTarget::Dial,
            SinkError::Transport(crate::display_sink::SHARED_WRITES_CLOSED.into()),
        );
        sink.fail_next(
            DisplayTarget::Dial,
            SinkError::Transport(crate::display_sink::SHARED_BUSY.into()),
        );
        let mut sync = DisplaySync::new(sink);
        let artwork = clock_artwork(DialContent::Clock, ManualClock::new(WallTime::new(9, 0)));
        let counts = DisplayCounts::default();
        let outcomes: Vec<DialOutcome> = (0..4)
            .map(|_| {
                sync_dial(
                    &mut sync,
                    &artwork,
                    &artwork.request(None),
                    "clock",
                    &counts,
                )
                .1
            })
            .collect();
        assert_eq!(
            outcomes,
            [
                DialOutcome::Failed,
                DialOutcome::Failed,
                DialOutcome::TransportFailed,
                DialOutcome::Done
            ]
        );
    }

    #[test]
    fn a_failed_dial_write_is_not_retried_until_the_face_changes() {
        let clock = ManualClock::new(WallTime::new(9, 0));
        let harness = dial_harness(
            clock_artwork(DialContent::Clock, clock.clone()),
            Duration::from_millis(20),
            None,
        );
        harness.fake.lock().unwrap().fail_next(
            DisplayTarget::Dial,
            SinkError::ShortWrite {
                written: 10,
                expected: 65,
            },
        );
        let (records_tx, records_rx) = mpsc::sync_channel(DISPLAY_RECORD_QUEUE);
        let session = two_functions();
        let mut display = DisplayThread::spawn(
            harness.display,
            session.clone(),
            Arc::new(AtomicBool::new(false)),
            records_tx,
        )
        .unwrap();
        display.request(&session, "software_mode");
        let failed = records_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(failed["written"], false);
        assert_eq!(failed["failed"]["short_write"], true);
        assert_eq!(failed["failed"]["attempts"], 1);
        assert_eq!(failed["complete"], false);
        // Many intervals pass and requests repeat; the same face is not resent.
        display.request(&session, "dial_function");
        thread::sleep(Duration::from_millis(300));
        assert_eq!(written(&harness.fake), 1);
        clock.set(WallTime::new(9, 1));
        let redrawn = records_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(display.finish(Duration::from_secs(2)));
        assert_eq!(redrawn["written"], true);
        assert_eq!(redrawn["face"], "clock:09:01");
        assert_eq!(written(&harness.fake), 2);
        assert_eq!(display.counts.failures.load(Ordering::SeqCst), 1);
    }

    /// Reports a new minute on every call, so every dial tick has a new face.
    struct Racing(AtomicU64);
    impl Clock for Racing {
        fn now(&self) -> WallTime {
            let tick = self.0.fetch_add(1, Ordering::SeqCst);
            WallTime::new((tick / 60 % 24) as u8, (tick % 60) as u8)
        }
    }

    #[test]
    fn dial_writes_reach_the_sink_only_between_enabling_and_restoring_hardware_mode() {
        // One log shared by the mode transport (`on`/`off`) and the dial sink
        // (`dial`), so their relative order is exact.
        let log = Arc::new(Mutex::new(Vec::new()));
        let harness = dial_harness(
            clock_artwork(DialContent::Clock, Racing(AtomicU64::new(0))),
            Duration::from_millis(10),
            Some(Arc::clone(&log)),
        );
        let fake = Arc::clone(&harness.fake);
        let mut transport = Transport::new([wait_for_writes(&fake, 3), dial(3, 0, 10)]);
        transport.log = Some(Arc::clone(&log));
        let run = run_dial(
            transport,
            profile(),
            LiveOptions {
                software_mode: true,
                max_reports: Some(1),
                ..options()
            },
            Some(harness.display),
            Arc::default(),
        );
        run.result.unwrap();
        // Writes kept being due; give a detached thread time to misbehave.
        thread::sleep(Duration::from_millis(200));
        let log = log.lock().unwrap().clone();
        assert_eq!(log.first(), Some(&"on"), "{log:?}");
        assert_eq!(log.last(), Some(&"off"), "{log:?}");
        assert_eq!(log.iter().filter(|&&event| event == "off").count(), 1);
        assert!(log.iter().filter(|&&event| event == "dial").count() >= 3);
        assert_eq!(run.mode_writes, modes());
        assert_eq!(run.rows.last().unwrap()["display_stopped"], true);
    }

    /// Replays CPU percentages (then nothing), logging each sample in `log`.
    struct LoggedCpu {
        readings: std::collections::VecDeque<Option<u8>>,
        log: Option<Arc<Mutex<Vec<&'static str>>>>,
        calls: Arc<AtomicU64>,
    }
    impl crate::dial_artwork::StatSource for LoggedCpu {
        fn sample(&mut self) -> Option<crate::dial_artwork::StatReading> {
            if let Some(log) = &self.log {
                log.lock().unwrap().push("sample");
            }
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.readings
                .pop_front()
                .flatten()
                .map(|percent| crate::dial_artwork::StatReading {
                    kind: crate::dial_artwork::StatKind::Cpu,
                    percent,
                })
        }
    }

    fn cpu_artwork(
        readings: impl IntoIterator<Item = Option<u8>>,
        sample_interval: Duration,
        log: Option<Arc<Mutex<Vec<&'static str>>>>,
    ) -> (DialArtwork, Arc<AtomicU64>) {
        let calls = Arc::new(AtomicU64::new(0));
        let source = LoggedCpu {
            readings: readings.into_iter().collect(),
            log,
            calls: Arc::clone(&calls),
        };
        let artwork = clock_artwork(DialContent::Cpu, ManualClock::new(WallTime::new(9, 0)))
            .with_stat_source(Box::new(source))
            .with_cpu_interval(sample_interval);
        (artwork, calls)
    }

    #[test]
    fn the_cpu_face_redraws_only_when_the_percentage_changes() {
        // Samples every 20 ms: 30 three times, 31, then nothing (dashes).
        let (artwork, calls) = cpu_artwork(
            [Some(30), Some(30), Some(30), Some(31)],
            Duration::from_millis(20),
            None,
        );
        let harness = dial_harness(artwork, Duration::from_millis(20), None);
        let (records_tx, records_rx) = mpsc::sync_channel(DISPLAY_RECORD_QUEUE);
        let session = two_functions();
        let mut display = DisplayThread::spawn(
            harness.display,
            session.clone(),
            Arc::new(AtomicBool::new(false)),
            records_tx,
        )
        .unwrap();
        // Nothing is sampled or drawn before software mode is on.
        thread::sleep(Duration::from_millis(250));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(written(&harness.fake), 0);
        display.request(&session, "software_mode");
        assert!(wait_until(|| written(&harness.fake) == 3));
        // A dial click changes nothing on a CPU face.
        display.request(&session, "dial_function");
        thread::sleep(Duration::from_millis(300));
        assert!(display.finish(Duration::from_secs(2)));
        assert_eq!(written(&harness.fake), 3);
        let records: Vec<Value> = records_rx.try_iter().collect();
        let faces: Vec<&str> = records
            .iter()
            .map(|record| record["face"].as_str().unwrap())
            .collect();
        assert_eq!(faces, ["cpu:30", "cpu:31", "cpu:--"]);
        let triggers: Vec<&str> = records
            .iter()
            .map(|record| record["trigger"].as_str().unwrap())
            .collect();
        assert_eq!(triggers, ["software_mode", "cpu", "cpu"]);
        assert!(records.iter().all(|record| record["written"] == true));
        assert_eq!(display.counts.syncs.load(Ordering::SeqCst), 3);
        assert_eq!(display.counts.failures.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn an_animated_dial_picture_redraws_each_frame_faster_than_the_default_interval() {
        let profile = crate::dial_artwork::animated_test_profile(&[250, 250, 250], 0);
        let artwork = DialArtwork::prepare(
            DialContent::Image,
            &profile,
            Box::new(ManualClock::new(WallTime::new(9, 0))),
        )
        .unwrap();
        // The default one-second dial interval would allow one write here.
        let harness = dial_harness(artwork, DIAL_MIN_INTERVAL, None);
        let fake = Arc::clone(&harness.fake);
        let (records_tx, records) = mpsc::sync_channel(DISPLAY_RECORD_QUEUE);
        let session = two_functions();
        let mut display = DisplayThread::spawn(
            harness.display,
            session.clone(),
            Arc::new(AtomicBool::new(false)),
            records_tx,
        )
        .unwrap();
        display.request(&session, "software_mode");
        thread::sleep(Duration::from_millis(1400));
        assert!(display.finish(Duration::from_secs(2)));
        let writes = dial_writes(&fake);
        assert!(writes.len() >= 4, "{} writes", writes.len());
        // Frames cycle: neighbours differ, and the loop comes back to frame 0.
        assert!(writes.windows(2).all(|pair| pair[0] != pair[1]));
        assert!(writes.iter().skip(3).any(|write| *write == writes[0]));
        let triggers: Vec<String> = records
            .try_iter()
            .map(|record| record["trigger"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(triggers[0], "software_mode");
        assert!(triggers[1..].iter().all(|trigger| trigger == "animation"));
    }

    #[test]
    fn cpu_sampling_and_writes_stay_rate_limited_by_default() {
        assert_eq!(
            crate::dial_artwork::CPU_SAMPLE_INTERVAL,
            Duration::from_secs(1)
        );
        // A new percentage on every sample, with the production intervals.
        let (artwork, calls) = cpu_artwork(
            (0..100).map(Some),
            crate::dial_artwork::CPU_SAMPLE_INTERVAL,
            None,
        );
        let harness = dial_harness(artwork, DIAL_MIN_INTERVAL, None);
        let (records_tx, _records) = mpsc::sync_channel(DISPLAY_RECORD_QUEUE);
        let session = two_functions();
        let mut display = DisplayThread::spawn(
            harness.display,
            session.clone(),
            Arc::new(AtomicBool::new(false)),
            records_tx,
        )
        .unwrap();
        display.request(&session, "software_mode");
        thread::sleep(Duration::from_millis(1500));
        assert!(display.finish(Duration::from_secs(2)));
        // 100 ms dial ticks over 1.5 s: at most two samples and two writes.
        let samples = calls.load(Ordering::SeqCst);
        assert!((1..=2).contains(&samples), "{samples} samples");
        let times = harness.times.lock().unwrap().clone();
        assert!((1..=2).contains(&times.len()), "{} writes", times.len());
        if let [first, second] = times[..] {
            assert!(second - first >= DIAL_MIN_INTERVAL - Duration::from_millis(50));
        }
    }

    #[test]
    fn cpu_samples_and_writes_happen_only_between_enabling_and_restoring_hardware_mode() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let (artwork, _) = cpu_artwork(
            (0..=100).cycle().take(10_000).map(Some),
            Duration::ZERO,
            Some(Arc::clone(&log)),
        );
        let harness = dial_harness(artwork, Duration::from_millis(10), Some(Arc::clone(&log)));
        let fake = Arc::clone(&harness.fake);
        let mut transport = Transport::new([wait_for_writes(&fake, 3), dial(3, 0, 10)]);
        transport.log = Some(Arc::clone(&log));
        let run = run_dial(
            transport,
            profile(),
            LiveOptions {
                software_mode: true,
                max_reports: Some(1),
                ..options()
            },
            Some(harness.display),
            Arc::default(),
        );
        run.result.unwrap();
        // Faces kept changing; give a detached thread time to misbehave.
        thread::sleep(Duration::from_millis(200));
        let log = log.lock().unwrap().clone();
        assert_eq!(log.first(), Some(&"on"), "{log:?}");
        assert_eq!(log.last(), Some(&"off"), "{log:?}");
        assert_eq!(log.iter().filter(|&&event| event == "off").count(), 1);
        assert!(log.iter().filter(|&&event| event == "dial").count() >= 3);
        assert!(log.contains(&"sample"));
        assert_eq!(run.mode_writes, modes());
        assert_eq!(run.rows.last().unwrap()["display_stopped"], true);
        assert_eq!(
            run.rows[0]["details"]["display_sync"]["dial_content"],
            "cpu"
        );
    }

    /// A clock that panics whenever it is read.
    struct PanickingClock;
    impl Clock for PanickingClock {
        fn now(&self) -> WallTime {
            panic!("clock failure")
        }
    }

    /// A CPU sampler that panics on every call.
    struct PanickingCpu(Arc<AtomicU64>);
    impl crate::dial_artwork::StatSource for PanickingCpu {
        fn sample(&mut self) -> Option<crate::dial_artwork::StatReading> {
            self.0.fetch_add(1, Ordering::SeqCst);
            panic!("sampler failure")
        }
    }

    fn settle(duration: Duration) -> Step {
        Step::Run(Box::new(move || thread::sleep(duration)))
    }

    #[test]
    fn a_panic_while_choosing_the_dial_face_is_contained_and_logged() {
        let harness = dial_harness(
            clock_artwork(DialContent::Clock, PanickingClock),
            Duration::from_millis(10),
            None,
        );
        let fake = Arc::clone(&harness.fake);
        let transport = Transport::new([settle(Duration::from_millis(300)), dial(3, 0, 10)]);
        let run = run_dial(
            transport,
            two_functions(),
            LiveOptions {
                software_mode: true,
                max_reports: Some(1),
                ..options()
            },
            Some(harness.display),
            Arc::default(),
        );
        let summary = run.result.unwrap();
        // Input is unaffected and hardware mode is restored.
        assert_eq!(run.actions, vec![Action::Volume { delta: 5 }]);
        assert_eq!(run.mode_writes, modes());
        assert!(fake.lock().unwrap().writes.is_empty());
        let records = display_rows(&run.rows);
        assert_eq!(records.len(), 1, "{records:?}");
        assert_eq!(records[0]["target"], "dial");
        assert_eq!(records[0]["error"], DISPLAY_PANIC);
        assert_eq!(records[0]["face"], Value::Null);
        assert_eq!(summary.display_failure_count, 1);
        assert_eq!(run.rows.last().unwrap()["display_stopped"], true);
    }

    #[test]
    fn a_panicking_cpu_sampler_shows_dashes_and_the_run_continues() {
        let calls = Arc::new(AtomicU64::new(0));
        let artwork = clock_artwork(DialContent::Cpu, ManualClock::new(WallTime::new(9, 0)))
            .with_stat_source(Box::new(PanickingCpu(Arc::clone(&calls))))
            .with_cpu_interval(Duration::from_millis(20));
        let harness = dial_harness(artwork, Duration::from_millis(10), None);
        let fake = Arc::clone(&harness.fake);
        let transport = Transport::new([
            wait_for_writes(&fake, 1),
            settle(Duration::from_millis(100)),
            dial(3, 0, 10),
        ]);
        let run = run_dial(
            transport,
            two_functions(),
            LiveOptions {
                software_mode: true,
                max_reports: Some(1),
                ..options()
            },
            Some(harness.display),
            Arc::default(),
        );
        let summary = run.result.unwrap();
        assert_eq!(run.actions, vec![Action::Volume { delta: 5 }]);
        assert_eq!(run.mode_writes, modes());
        let records = display_rows(&run.rows);
        assert_eq!(records.len(), 1, "{records:?}");
        assert_eq!(records[0]["face"], "cpu:--");
        assert_eq!(records[0]["written"], true);
        assert_eq!(records[0]["error"], Value::Null);
        assert_eq!(summary.display_failure_count, 0);
        // A sampler that panicked once is never called again.
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn the_first_cpu_face_waits_for_a_real_sample() {
        // The baseline sample (no value) is taken once software mode is on;
        // the dial is first drawn with the next sample's percentage.
        let (artwork, calls) = cpu_artwork([None, Some(40)], Duration::from_millis(50), None);
        let harness = dial_harness(artwork, Duration::from_millis(10), None);
        let (records_tx, records_rx) = mpsc::sync_channel(DISPLAY_RECORD_QUEUE);
        let session = two_functions();
        let mut display = DisplayThread::spawn(
            harness.display,
            session.clone(),
            Arc::new(AtomicBool::new(false)),
            records_tx,
        )
        .unwrap();
        thread::sleep(Duration::from_millis(150));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        display.request(&session, "software_mode");
        let first = records_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(display.finish(Duration::from_secs(2)));
        assert_eq!(first["face"], "cpu:40");
        assert_eq!(first["trigger"], "software_mode");
        assert_eq!(dial_writes(&harness.fake).len(), 1);
    }

    #[test]
    fn no_cpu_sample_starts_once_the_write_gate_is_closed() {
        let (artwork, calls) = cpu_artwork([Some(1), Some(2)], Duration::ZERO, None);
        let mut state = DialState {
            started: true,
            ..DialState::default()
        };
        assert!(state.due(&artwork, Duration::ZERO, &|| true).is_none());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(state.due(&artwork, Duration::ZERO, &|| false).is_some());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn cpu_dial_display_fails_closed_without_software_mode() {
        let (artwork, calls) = cpu_artwork([Some(1)], Duration::ZERO, None);
        let harness = dial_harness(artwork, Duration::from_millis(10), None);
        let transport = Transport::new([]);
        let writes = Arc::clone(&transport.writes);
        let error = run_inner(
            &mut InputReader::new(transport),
            two_functions(),
            device(),
            false,
            json!({}),
            options(),
            Arc::new(AtomicBool::new(false)),
            Some(harness.display),
            |_| -> Result<(Actions, Output), String> { panic!("must not construct worker") },
            || 0,
        )
        .unwrap_err();
        assert!(error.contains("software mode"), "{error}");
        assert!(harness.fake.lock().unwrap().writes.is_empty());
        assert!(writes.lock().unwrap().is_empty());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
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

    /// Runs a keypad (or, with `dial`, an M131E dial clock) session whose
    /// first display report blocks; `release` decides when the controller
    /// opens the gate (given the shared transport).
    fn run_gated(
        lock_timeout: Duration,
        dial: bool,
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
        let (display, session, device) = if dial {
            let display = crate::display_sink::live_dial_display(
                &shared,
                Some(ModuleKind::M131E),
                clock_artwork(DialContent::Clock, ManualClock::new(WallTime::new(9, 0))),
            )
            .unwrap();
            (display, profile(), device())
        } else {
            let display = crate::display_sink::live_display(
                &shared,
                Some(ModuleKind::M4315),
                PreparedKeyArtwork::prepare().unwrap(),
            )
            .unwrap();
            (display, keypad_profile(), keypad_device())
        };
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
            session,
            device,
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
        // The same holds for a key page and for a dial clock face.
        for dial in [false, true] {
            let run = run_gated(Duration::from_secs(5), dial, |shared, gate| {
                // Release the stuck write only once the restore is waiting for it.
                assert!(wait_until(|| shared.priority_waiters() > 0));
                *gate.0.lock().unwrap() = true;
                gate.1.notify_all();
            });
            let summary = run.result.unwrap();
            // One report went out before cleanup; the closed gate refused the rest.
            assert_eq!(*run.log.lock().unwrap(), vec!["on", "out", "off"], "{dial}");
            assert_eq!(summary.report_count, 0);
            let footer = run.rows.last().unwrap();
            assert_eq!(footer["reason"], "interrupted");
            assert_eq!(footer["display_stopped"], false);
            let refused = run.shared.write_output(&[2, 0]).unwrap_err().to_string();
            assert!(refused.contains(SHARED_WRITES_CLOSED), "{refused}");
        }
    }

    #[test]
    fn a_restore_that_never_gets_the_device_retries_once_and_the_run_still_ends() {
        for dial in [false, true] {
            let run = run_gated(Duration::from_millis(30), dial, |_, _| {});
            let error = run.result.unwrap_err();
            assert!(error.contains("first attempt"), "{error}");
            assert!(error.contains("retry"), "{error}");
            assert_eq!(error.matches(SHARED_BUSY).count(), 2, "{error}");
            assert_eq!(*run.log.lock().unwrap(), vec!["on", "out"], "{dial}");
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
            assert_eq!(*run.log.lock().unwrap(), vec!["on", "out"], "{dial}");
        }
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

    /// Arms a coordinator panic point on this thread until dropped.
    struct ArmedPanic;
    impl ArmedPanic {
        fn at(point: &'static str) -> Self {
            PANIC_AT.with(|armed| armed.set(Some(point)));
            Self
        }
    }
    impl Drop for ArmedPanic {
        fn drop(&mut self) {
            PANIC_AT.with(|armed| armed.set(None));
        }
    }

    #[test]
    fn a_coordinator_panic_at_any_point_after_enabling_still_restores_hardware_mode() {
        for (point, expected) in [
            ("software_mode_enabled", INPUT_PANIC),
            ("display_stopped", COORDINATOR_PANIC),
            ("queue_closed", COORDINATOR_PANIC),
            ("before_restore", COORDINATOR_PANIC),
        ] {
            let _armed = ArmedPanic::at(point);
            let transport = Transport::new([dial(0, 1, 10)]);
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
            assert!(error.contains(expected), "{point}: {error}");
            assert!(
                error.contains(&format!("injected panic at {point}")),
                "{point}: {error}"
            );
            assert_eq!(*writes.lock().unwrap(), modes(), "{point}");
        }
    }

    #[test]
    fn a_coordinator_panic_keeps_the_input_loops_own_error_or_panic() {
        for (step, original) in [
            (Step::Unplug, "synthetic unplug"),
            (
                Step::Run(Box::new(|| panic!("synthetic read panic"))),
                "synthetic read panic",
            ),
        ] {
            let _armed = ArmedPanic::at("display_stopped");
            let transport = Transport::new([step]);
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
            assert!(error.contains(original), "{error}");
            assert!(error.contains(COORDINATOR_PANIC), "{error}");
            assert!(
                error.contains("injected panic at display_stopped"),
                "{error}"
            );
            assert_eq!(*writes.lock().unwrap(), modes());
        }
    }

    #[test]
    fn a_coordinator_panic_keeps_restore_failures_and_the_single_retry() {
        let _armed = ArmedPanic::at("queue_closed");
        let mut transport = Transport::new([dial(0, 1, 10)]);
        transport.fail_cleanup = true;
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
        assert!(error.contains(COORDINATOR_PANIC), "{error}");
        assert!(error.contains("cleanup failed"), "{error}");
        // A non-busy failure is not retried, exactly as on the normal path.
        assert_eq!(*writes.lock().unwrap(), modes());
    }

    #[test]
    fn a_coordinator_panic_with_a_stuck_display_write_still_yields_to_the_restore() {
        let _armed = ArmedPanic::at("display_stopped");
        for dial in [false, true] {
            let run = run_gated(Duration::from_secs(5), dial, |shared, gate| {
                assert!(wait_until(|| shared.priority_waiters() > 0));
                *gate.0.lock().unwrap() = true;
                gate.1.notify_all();
            });
            let error = run.result.unwrap_err();
            assert!(error.contains(COORDINATOR_PANIC), "{error}");
            assert!(
                error.contains("injected panic at display_stopped"),
                "{error}"
            );
            assert_eq!(*run.log.lock().unwrap(), vec!["on", "out", "off"], "{dial}");
            // No further image report can start after the restore.
            let refused = run.shared.write_output(&[2, 0]).unwrap_err().to_string();
            assert!(refused.contains(SHARED_WRITES_CLOSED), "{refused}");
        }
    }
}
