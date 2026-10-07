//! A HID-backed [`DisplaySink`] for M4315 key images and the M131E dial image.
//!
//! [`HidDisplaySink`] is generic over [`ReportTransport`], so every behaviour is
//! tested with fake transports. [`SharedTransport`] lets the live input reader
//! and a display thread share one opened device; `masterhub-service run
//! --display-sync` is the only caller over a physical device. [`open_hardware`]
//! (a second handle) is kept for platforms that allow it and is not called.
//! Physical key order, orientation, pacing, and firmware image acceptance are
//! unverified; see `docs/masterhub-display-sync.md`.

use std::{
    sync::{
        Arc, Mutex, MutexGuard, TryLockError,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use cooltek_sdk_masterhub::{
    DeviceRegistry, ModuleIdentity, ModuleKind, ReportTransport, ShortWrite, dial,
    hidapi::{HidDevice, HidError, HidResult},
    keypad,
};
use cooltek_sdk_service::display_sync::{
    DisplayCapabilities, DisplaySink, DisplayTarget, KEY_COUNT, KeyImageProvider, SinkError,
};

use crate::live_runtime::LiveDisplay;

/// Output report ID that carries image chunks on both IPS modules.
pub const DISPLAY_REPORT_ID: u8 = 0x02;

/// Writes complete images through the SDK's descriptor-sized display writers.
///
/// Each write validates its image before the first report, writes reports in
/// order, forwards the pacing hook between reports, and stops at the first
/// error. A write count that differs from the report length becomes
/// [`SinkError::ShortWrite`], which `DisplaySync` never retries. Every other
/// failure becomes [`SinkError::Transport`]. The sink sends no mode or
/// configuration commands: the caller enters software mode first.
#[derive(Debug)]
pub struct HidDisplaySink<T: ReportTransport> {
    transport: T,
    module: ModuleKind,
    output_report_length: usize,
}

impl<T: ReportTransport> HidDisplaySink<T> {
    /// Creates a sink for an M4315 keypad or an M131E dial over `transport`.
    ///
    /// Reads the transport's HID report descriptor once to establish the output
    /// report length of report `0x02`. Sends no reports.
    ///
    /// # Errors
    /// Fails for any other module, an unreadable descriptor, a descriptor that
    /// does not declare output report `0x02`, or a length outside 9 through
    /// 65,535 bytes (no payload capacity, or beyond the packet header's range).
    pub fn new(transport: T, module: Option<ModuleKind>) -> Result<Self, String> {
        let module = match module {
            Some(module @ (ModuleKind::M4315 | ModuleKind::M131E)) => module,
            _ => return Err("display images are supported only for M4315 and M131E".into()),
        };
        let output_report_length =
            cooltek_sdk_masterhub::output_report_length(&transport, DISPLAY_REPORT_ID).map_err(
                |error| {
                    format!(
                        "could not establish the display output report length from the HID \
                         descriptor: {error}"
                    )
                },
            )?;
        if !(9..=usize::from(u16::MAX)).contains(&output_report_length) {
            return Err(format!(
                "display output report length {output_report_length} is outside 9 through 65535"
            ));
        }
        Ok(Self {
            transport,
            module,
            output_report_length,
        })
    }

    /// Returns the module this sink writes.
    pub fn module(&self) -> ModuleKind {
        self.module
    }

    /// Returns the descriptor-derived output report length, including the ID.
    pub fn output_report_length(&self) -> usize {
        self.output_report_length
    }

    /// Borrows the transport.
    pub fn transport(&self) -> &T {
        &self.transport
    }

    /// Releases the transport without sending anything.
    pub fn into_transport(self) -> T {
        self.transport
    }
}

/// Maps an SDK display error: a typed short write stays a short write.
fn sink_error(error: HidError) -> SinkError {
    match ShortWrite::from_error(&error) {
        Some(short) => SinkError::ShortWrite {
            written: short.written,
            expected: short.expected,
        },
        None => SinkError::Transport(error.to_string()),
    }
}

impl<T: ReportTransport> DisplaySink for HidDisplaySink<T> {
    fn capabilities(&self) -> DisplayCapabilities {
        match self.module {
            ModuleKind::M4315 => DisplayCapabilities {
                key_count: KEY_COUNT as u8,
                dial: false,
            },
            _ => DisplayCapabilities {
                key_count: 0,
                dial: true,
            },
        }
    }

    fn write_key_image(
        &mut self,
        key_index: u8,
        jpeg: &[u8],
        between_chunks: &mut dyn FnMut(usize),
    ) -> Result<(), SinkError> {
        if self.module != ModuleKind::M4315 || usize::from(key_index) >= KEY_COUNT {
            return Err(SinkError::Unsupported(DisplayTarget::Key(key_index)));
        }
        keypad::display_jpeg_with_pacing(
            &self.transport,
            key_index,
            jpeg,
            self.output_report_length,
            between_chunks,
        )
        .map(drop)
        .map_err(sink_error)
    }

    fn write_dial_image(
        &mut self,
        jpeg: &[u8],
        between_chunks: &mut dyn FnMut(usize),
    ) -> Result<(), SinkError> {
        if self.module != ModuleKind::M131E {
            return Err(SinkError::Unsupported(DisplayTarget::Dial));
        }
        dial::display_jpeg_with_pacing(
            &self.transport,
            jpeg,
            self.output_report_length,
            between_chunks,
        )
        .map(drop)
        .map_err(sink_error)
    }
}

/// **Hardware only.** Opens a second handle to a selected IPS interface and
/// wraps it in a sink. Not called from anywhere yet, and never from tests.
///
/// Opening sends no reports; the descriptor is read once. The caller must have
/// entered software mode on the module before writing. Where the platform does
/// not allow a second handle (macOS opens HID devices exclusively by default),
/// share one device instead: wrap it in [`SharedTransport`] and pass clones to
/// both `InputReader::new` and [`HidDisplaySink::new`], as the live service does.
///
/// # Errors
/// Fails when the interface cannot be opened or [`HidDisplaySink::new`] fails.
pub fn open_hardware(
    registry: &DeviceRegistry,
    identity: &ModuleIdentity,
) -> Result<HidDisplaySink<HidDevice>, String> {
    let device = registry
        .open(identity)
        .map_err(|error| format!("could not open the display interface: {error}"))?;
    HidDisplaySink::new(device, identity.module)
}

/// Longest single inner read while a [`SharedTransport`] holds its lock.
const SHARED_READ_SLICE_MS: i32 = 5;
/// Pause after an empty read slice so a waiting writer can take the lock.
const SHARED_READ_PAUSE: Duration = Duration::from_millis(1);
/// Poll interval for writers and priority operations waiting for their turn.
const SHARED_POLL: Duration = Duration::from_micros(200);
/// Default bound on waiting for the lock for any non-read operation.
pub const SHARED_LOCK_TIMEOUT: Duration = Duration::from_secs(2);
/// Output reports a writer may send while a reader waits before it yields.
pub const SHARED_WRITE_BURST: usize = 16;
/// Error text of a lock wait that exceeded its bound; callers may retry once.
pub const SHARED_BUSY: &str = "shared device is busy; lock timeout exceeded";
/// Error text of an output write refused after [`SharedTransport::close_writes`].
pub const SHARED_WRITES_CLOSED: &str = "display writes are closed";

struct SharedState<T> {
    device: Mutex<T>,
    writes_closed: Arc<AtomicBool>,
    readers_waiting: AtomicUsize,
    reader_turns: AtomicU64,
    priority_waiting: AtomicUsize,
    writes_since_turn: AtomicUsize,
    seen_turn: AtomicU64,
}

/// Decrements a waiter count on every exit path.
struct Waiting<'a>(&'a AtomicUsize);

impl<'a> Waiting<'a> {
    fn enter(count: &'a AtomicUsize) -> Self {
        count.fetch_add(1, Ordering::SeqCst);
        Self(count)
    }
}

impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// One opened transport shared between the input thread and a display thread.
///
/// `HidDevice` is `Send` but not `Sync`, so it cannot be shared across threads
/// through `Arc` alone, and macOS opens HID devices exclusively by default, so
/// a second handle ([`open_hardware`]) is not expected to work there. This
/// wrapper serializes every operation through one mutex with bounded waits:
///
/// - `read_timeout` reads in slices of at most 5 ms and never waits for the
///   lock beyond its own timeout. While another holder keeps the lock, it
///   returns `Ok(0)`, a normal timeout, and input reports queue in the OS.
/// - Output writes (`write_output`) yield to a waiting reader after
///   [`SHARED_WRITE_BURST`] reports since the reader's last turn, and to any
///   waiting priority operation before every report.
/// - Feature reports and descriptor reads are priority operations: writers do
///   not take the lock while one waits. They wait at most the lock timeout and
///   then fail with [`SHARED_BUSY`].
/// - [`Self::close_writes`] makes every later output write fail at once with
///   [`SHARED_WRITES_CLOSED`], checked before and after taking the lock, so an
///   in-flight image stops at its next report.
///
/// A backend call that blocks inside HIDAPI keeps the lock and is not
/// interruptible; priority operations then fail after their bound. Hardware
/// behaviour is unverified.
pub struct SharedTransport<T> {
    shared: Arc<SharedState<T>>,
    lock_timeout: Duration,
}

impl<T> Clone for SharedTransport<T> {
    fn clone(&self) -> Self {
        Self {
            shared: Arc::clone(&self.shared),
            lock_timeout: self.lock_timeout,
        }
    }
}

impl<T: ReportTransport> SharedTransport<T> {
    /// Wraps an opened transport with the default lock timeout.
    pub fn new(transport: T) -> Self {
        Self::with_lock_timeout(transport, SHARED_LOCK_TIMEOUT)
    }

    /// Wraps an opened transport with a positive lock timeout for non-read operations.
    pub fn with_lock_timeout(transport: T, lock_timeout: Duration) -> Self {
        Self {
            shared: Arc::new(SharedState {
                device: Mutex::new(transport),
                writes_closed: Arc::new(AtomicBool::new(false)),
                readers_waiting: AtomicUsize::new(0),
                reader_turns: AtomicU64::new(0),
                priority_waiting: AtomicUsize::new(0),
                writes_since_turn: AtomicUsize::new(0),
                seen_turn: AtomicU64::new(0),
            }),
            lock_timeout,
        }
    }

    /// The output write gate: storing `true` closes writes for good.
    ///
    /// The live runtime closes it when the input loop ends, before it waits
    /// for the display thread and restores hardware mode.
    pub fn write_gate(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.shared.writes_closed)
    }

    /// Makes every later output write fail at once.
    pub fn close_writes(&self) {
        self.shared.writes_closed.store(true, Ordering::SeqCst);
    }

    /// Priority operations currently waiting for the lock.
    pub fn priority_waiters(&self) -> usize {
        self.shared.priority_waiting.load(Ordering::SeqCst)
    }

    /// Takes the lock, recovering it after a panic in another holder.
    fn try_acquire(&self) -> Option<MutexGuard<'_, T>> {
        match self.shared.device.try_lock() {
            Ok(guard) => Some(guard),
            Err(TryLockError::Poisoned(poisoned)) => Some(poisoned.into_inner()),
            Err(TryLockError::WouldBlock) => None,
        }
    }

    fn busy() -> HidError {
        HidError::HidApiError {
            message: SHARED_BUSY.into(),
        }
    }

    fn closed() -> HidError {
        HidError::HidApiError {
            message: SHARED_WRITES_CLOSED.into(),
        }
    }

    /// Acquires ahead of writers, waiting at most the lock timeout.
    fn acquire_priority(&self) -> HidResult<MutexGuard<'_, T>> {
        let _waiting = Waiting::enter(&self.shared.priority_waiting);
        let deadline = Instant::now() + self.lock_timeout;
        loop {
            if let Some(guard) = self.try_acquire() {
                return Ok(guard);
            }
            if Instant::now() >= deadline {
                return Err(Self::busy());
            }
            thread::sleep(SHARED_POLL);
        }
    }

    /// Acquires for one output report, yielding to readers and priority waiters.
    fn acquire_write(&self) -> HidResult<MutexGuard<'_, T>> {
        let state = &*self.shared;
        let mut deadline = Instant::now() + self.lock_timeout;
        loop {
            if state.writes_closed.load(Ordering::SeqCst) {
                return Err(Self::closed());
            }
            let turn = state.reader_turns.load(Ordering::SeqCst);
            if state.seen_turn.swap(turn, Ordering::SeqCst) != turn {
                state.writes_since_turn.store(0, Ordering::SeqCst);
            }
            let reader_due = state.readers_waiting.load(Ordering::SeqCst) > 0
                && state.writes_since_turn.load(Ordering::SeqCst) >= SHARED_WRITE_BURST;
            let yield_now = state.priority_waiting.load(Ordering::SeqCst) > 0 || reader_due;
            if !yield_now && let Some(guard) = self.try_acquire() {
                if state.writes_closed.load(Ordering::SeqCst) {
                    return Err(Self::closed());
                }
                if state.priority_waiting.load(Ordering::SeqCst) == 0 {
                    // Exact while held: no reader turn can start until release.
                    let turn = state.reader_turns.load(Ordering::SeqCst);
                    if state.seen_turn.swap(turn, Ordering::SeqCst) != turn {
                        state.writes_since_turn.store(0, Ordering::SeqCst);
                    }
                    state.writes_since_turn.fetch_add(1, Ordering::SeqCst);
                    return Ok(guard);
                }
            }
            if Instant::now() >= deadline {
                // A reader that never takes its turn cannot stall writes forever.
                if reader_due && state.priority_waiting.load(Ordering::SeqCst) == 0 {
                    state.writes_since_turn.store(0, Ordering::SeqCst);
                    deadline = Instant::now() + self.lock_timeout;
                    continue;
                }
                return Err(Self::busy());
            }
            thread::sleep(SHARED_POLL);
        }
    }
}

impl<T: ReportTransport> ReportTransport for SharedTransport<T> {
    fn read_timeout(&self, buffer: &mut [u8], timeout_ms: i32) -> HidResult<usize> {
        let state = &*self.shared;
        // Counted for the whole call, so a writer yields once per burst.
        let _waiting = Waiting::enter(&state.readers_waiting);
        let deadline =
            (timeout_ms >= 0).then(|| Instant::now() + Duration::from_millis(timeout_ms as u64));
        loop {
            let slice = match deadline {
                Some(deadline) => {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    (remaining.as_millis() as i32).min(SHARED_READ_SLICE_MS)
                }
                None => SHARED_READ_SLICE_MS,
            };
            if let Some(guard) = self.try_acquire() {
                state.reader_turns.fetch_add(1, Ordering::SeqCst);
                let read = guard.read_timeout(buffer, slice)?;
                drop(guard);
                if read > 0 {
                    return Ok(read);
                }
            }
            if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                return Ok(0);
            }
            thread::sleep(SHARED_READ_PAUSE);
        }
    }

    fn get_feature_report(&self, buffer: &mut [u8]) -> HidResult<usize> {
        self.acquire_priority()?.get_feature_report(buffer)
    }

    fn send_feature_report(&self, data: &[u8]) -> HidResult<()> {
        self.acquire_priority()?.send_feature_report(data)
    }

    fn write_output(&self, data: &[u8]) -> HidResult<usize> {
        self.acquire_write()?.write_output(data)
    }

    fn get_report_descriptor(&self, buffer: &mut [u8]) -> HidResult<usize> {
        self.acquire_priority()?.get_report_descriptor(buffer)
    }
}

/// Builds the live display over a shared device, wired to its write gate.
///
/// Fails closed: an error here must end the run before software mode. Reads
/// the descriptor once and sends no reports. Used by `masterhub-service run
/// --display-sync`.
///
/// # Errors
/// Fails when [`HidDisplaySink::new`] fails, prefixed with
/// `--display-sync setup failed`.
pub fn live_display<T: ReportTransport + Send + 'static>(
    shared: &SharedTransport<T>,
    module: Option<ModuleKind>,
    artwork: impl KeyImageProvider + Send + 'static,
) -> Result<LiveDisplay, String> {
    let sink = HidDisplaySink::new(shared.clone(), module)
        .map_err(|error| format!("--display-sync setup failed: {error}"))?;
    Ok(LiveDisplay::new(sink, artwork, shared.write_gate()))
}

#[cfg(test)]
mod tests {
    use std::{
        cell::{Cell, RefCell},
        collections::VecDeque,
        rc::Rc,
    };

    use cooltek_sdk_masterhub::hidapi::HidResult;
    use cooltek_sdk_service::{
        Profile, ProfileSession,
        display_sync::{DisplayError, DisplaySync, KeyImageProvider, KeyImageRequest, RetryPolicy},
    };

    use super::*;

    const KEY_JPEG: &[u8] =
        include_bytes!("../../cooltek-sdk-masterhub/tests/fixtures/keypad-72x72-baseline.jpg");
    /// Report ID 2, 64 eight-bit output fields: a 65-byte output report.
    const DESCRIPTOR: [u8; 8] = [0x85, 0x02, 0x75, 0x08, 0x95, 64, 0x91, 0x02];
    const REPORT: usize = 65;

    /// The key fixture with its frame rewritten to 240×240. Framing only; the
    /// entropy data is not valid for that size and is never sent to hardware.
    fn dial_jpeg() -> Vec<u8> {
        let mut jpeg = KEY_JPEG.to_vec();
        let frame = jpeg.windows(2).position(|b| b == [0xff, 0xc0]).unwrap();
        jpeg[frame + 5..frame + 9].copy_from_slice(&[0, 240, 0, 240]);
        jpeg
    }

    #[derive(Debug, Default)]
    struct Fake {
        descriptor: Option<Vec<u8>>,
        outputs: RefCell<Vec<Vec<u8>>>,
        results: RefCell<VecDeque<HidResult<usize>>>,
    }

    impl Fake {
        fn new() -> Self {
            Self {
                descriptor: Some(DESCRIPTOR.to_vec()),
                ..Self::default()
            }
        }

        fn script(self, results: impl IntoIterator<Item = HidResult<usize>>) -> Self {
            *self.results.borrow_mut() = results.into_iter().collect();
            self
        }
    }

    impl ReportTransport for Fake {
        fn read_timeout(&self, _: &mut [u8], _: i32) -> HidResult<usize> {
            panic!("a display sink never reads input")
        }
        fn get_feature_report(&self, _: &mut [u8]) -> HidResult<usize> {
            panic!("a display sink never queries features")
        }
        fn send_feature_report(&self, _: &[u8]) -> HidResult<()> {
            panic!("a display sink never sends mode or configuration commands")
        }
        fn write_output(&self, data: &[u8]) -> HidResult<usize> {
            self.outputs.borrow_mut().push(data.to_vec());
            self.results
                .borrow_mut()
                .pop_front()
                .unwrap_or(Ok(data.len()))
        }
        fn get_report_descriptor(&self, buffer: &mut [u8]) -> HidResult<usize> {
            let descriptor = self.descriptor.as_ref().ok_or(HidError::HidApiError {
                message: "no descriptor".into(),
            })?;
            buffer[..descriptor.len()].copy_from_slice(descriptor);
            Ok(descriptor.len())
        }
    }

    fn keypad_sink(fake: Fake) -> HidDisplaySink<Fake> {
        HidDisplaySink::new(fake, Some(ModuleKind::M4315)).unwrap()
    }

    fn disconnected() -> HidError {
        HidError::HidApiError {
            message: "disconnected".into(),
        }
    }

    struct Fixture;

    impl KeyImageProvider for Fixture {
        fn key_image(&self, _: &KeyImageRequest) -> Option<Vec<u8>> {
            Some(KEY_JPEG.to_vec())
        }
    }

    fn session() -> ProfileSession {
        ProfileSession::new(
            Profile::from_json(include_bytes!(
                "../../../examples/profiles/keypad-folders.json"
            ))
            .unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn construction_requires_an_ips_module_and_a_descriptor_length() {
        let sink = keypad_sink(Fake::new());
        assert_eq!(sink.output_report_length(), REPORT);
        assert_eq!(sink.module(), ModuleKind::M4315);
        assert_eq!(
            sink.capabilities(),
            DisplayCapabilities {
                key_count: 15,
                dial: false
            }
        );
        let dial = HidDisplaySink::new(Fake::new(), Some(ModuleKind::M131E)).unwrap();
        assert_eq!(
            dial.capabilities(),
            DisplayCapabilities {
                key_count: 0,
                dial: true
            }
        );
        for module in [None, Some(ModuleKind::M003K), Some(ModuleKind::M0064)] {
            assert!(HidDisplaySink::new(Fake::new(), module).is_err());
        }
        let missing = HidDisplaySink::new(Fake::default(), Some(ModuleKind::M4315));
        assert!(missing.unwrap_err().contains("HID descriptor"));
        // Report 2 declared only as a feature: no output length can be established.
        let feature_only = Fake {
            descriptor: Some(vec![0x85, 0x02, 0x75, 0x08, 0x95, 64, 0xb1, 0x02]),
            ..Fake::default()
        };
        assert!(HidDisplaySink::new(feature_only, Some(ModuleKind::M4315)).is_err());
        // An 8-byte report has no payload capacity.
        let tiny = Fake {
            descriptor: Some(vec![0x85, 0x02, 0x75, 0x08, 0x95, 7, 0x91, 0x02]),
            ..Fake::default()
        };
        assert!(HidDisplaySink::new(tiny, Some(ModuleKind::M4315)).is_err());
    }

    #[test]
    fn key_and_dial_writes_send_exact_packets_with_pacing_between_reports() {
        let mut sink = keypad_sink(Fake::new());
        let mut calls = Vec::new();
        sink.write_key_image(9, KEY_JPEG, &mut |sent| calls.push(sent))
            .unwrap();
        let expected: Vec<_> = keypad::JpegPackets::new(9, KEY_JPEG, REPORT)
            .unwrap()
            .collect();
        assert!(expected.len() > 1);
        assert_eq!(sink.transport().outputs.borrow().as_slice(), expected);
        assert_eq!(calls, (1..expected.len()).collect::<Vec<_>>());

        let image = dial_jpeg();
        let mut sink = HidDisplaySink::new(Fake::new(), Some(ModuleKind::M131E)).unwrap();
        let mut calls = 0;
        sink.write_dial_image(&image, &mut |_| calls += 1).unwrap();
        let expected: Vec<_> = dial::JpegPackets::new(&image, REPORT).unwrap().collect();
        assert_eq!(sink.transport().outputs.borrow().as_slice(), expected);
        assert_eq!(calls, expected.len() - 1);
    }

    #[test]
    fn short_writes_and_transport_failures_are_typed_and_stop_the_image() {
        for (result, error) in [
            (
                Ok(64),
                SinkError::ShortWrite {
                    written: 64,
                    expected: REPORT,
                },
            ),
            (
                Ok(66),
                SinkError::ShortWrite {
                    written: 66,
                    expected: REPORT,
                },
            ),
            (
                Err(disconnected()),
                SinkError::Transport(disconnected().to_string()),
            ),
        ] {
            let mut sink = keypad_sink(Fake::new().script([Ok(REPORT), result]));
            assert_eq!(sink.write_key_image(0, KEY_JPEG, &mut |_| {}), Err(error));
            assert_eq!(sink.transport().outputs.borrow().len(), 2);
        }
        // A transport that itself reports HIDAPI's incomplete send is a short write.
        let mut sink = keypad_sink(Fake::new().script([Err(HidError::IncompleteSendError {
            sent: 3,
            all: REPORT,
        })]));
        assert_eq!(
            sink.write_key_image(0, KEY_JPEG, &mut |_| {}),
            Err(SinkError::ShortWrite {
                written: 3,
                expected: REPORT
            })
        );
    }

    #[test]
    fn wrong_module_or_key_is_unsupported_and_writes_nothing() {
        let mut keys = keypad_sink(Fake::new());
        assert_eq!(
            keys.write_dial_image(&dial_jpeg(), &mut |_| {}),
            Err(SinkError::Unsupported(DisplayTarget::Dial))
        );
        assert_eq!(
            keys.write_key_image(15, KEY_JPEG, &mut |_| {}),
            Err(SinkError::Unsupported(DisplayTarget::Key(15)))
        );
        let mut dial = HidDisplaySink::new(Fake::new(), Some(ModuleKind::M131E)).unwrap();
        assert_eq!(
            dial.write_key_image(0, KEY_JPEG, &mut |_| {}),
            Err(SinkError::Unsupported(DisplayTarget::Key(0)))
        );
        assert!(keys.transport().outputs.borrow().is_empty());
        assert!(dial.transport().outputs.borrow().is_empty());

        // DisplaySync rejects a keypad page for a dial sink before any write.
        let mut sync = DisplaySync::new(dial);
        assert_eq!(
            sync.sync_page(&session(), &Fixture).unwrap_err(),
            DisplayError::Unsupported(DisplayTarget::Key(0))
        );
        assert!(sync.sink().transport().outputs.borrow().is_empty());
    }

    #[test]
    fn display_sync_never_retries_a_short_write_but_retries_transport_failures() {
        let retry = RetryPolicy {
            transport_retries: 3,
        };
        let short = Fake::new().script([Ok(REPORT), Ok(10)]);
        let mut sync = DisplaySync::new(keypad_sink(short)).with_retry(retry);
        let report = sync.sync_page(&session(), &Fixture).unwrap();
        assert!(report.written.is_empty());
        assert_eq!(report.failures.len(), 1);
        assert_eq!(report.failures[0].target, DisplayTarget::Key(0));
        assert_eq!(report.failures[0].attempts, 1);
        assert_eq!(
            report.failures[0].error,
            SinkError::ShortWrite {
                written: 10,
                expected: REPORT
            }
        );
        assert_eq!(
            report.not_attempted,
            (1..15).map(DisplayTarget::Key).collect::<Vec<_>>()
        );
        // Exactly the two reports of the failed attempt: no retry, no later key.
        assert_eq!(sync.sink().transport().outputs.borrow().len(), 2);
        assert_eq!(sync.drawn().key(0), None);

        let failing = Fake::new().script([Err(disconnected())]);
        let mut sync = DisplaySync::new(keypad_sink(failing)).with_retry(retry);
        let report = sync.sync_page(&session(), &Fixture).unwrap();
        assert!(report.failures.is_empty());
        assert_eq!(report.written.len(), 15);
        let packets = keypad::JpegPackets::new(0, KEY_JPEG, REPORT).unwrap().len();
        assert_eq!(
            sync.sink().transport().outputs.borrow().len(),
            1 + 15 * packets
        );
    }

    #[test]
    fn display_sync_chunk_pacing_reaches_the_hid_writer() {
        let calls = Rc::new(Cell::new(0));
        let counter = Rc::clone(&calls);
        let mut sync = DisplaySync::new(keypad_sink(Fake::new()))
            .with_chunk_pacing(move |_| counter.set(counter.get() + 1));
        let report = sync.sync_page(&session(), &Fixture).unwrap();
        assert!(report.is_complete());
        let packets = keypad::JpegPackets::new(0, KEY_JPEG, REPORT).unwrap().len();
        assert_eq!(calls.get(), 15 * (packets - 1));
        assert_eq!(sync.sink().transport().outputs.borrow().len(), 15 * packets);
    }

    #[test]
    fn a_shared_transport_can_back_the_sink() {
        let shared = Rc::new(Fake::new());
        let mut sink = HidDisplaySink::new(Rc::clone(&shared), Some(ModuleKind::M4315)).unwrap();
        sink.write_key_image(1, KEY_JPEG, &mut |_| {}).unwrap();
        assert!(!shared.outputs.borrow().is_empty());
    }

    /// A thread-safe fake: reads time out after their slice, writes are recorded.
    #[derive(Default)]
    struct Threaded {
        reads: Mutex<Vec<i32>>,
        outputs: Mutex<Vec<Vec<u8>>>,
    }

    impl ReportTransport for Threaded {
        fn read_timeout(&self, _: &mut [u8], timeout: i32) -> HidResult<usize> {
            self.reads.lock().unwrap().push(timeout);
            thread::sleep(Duration::from_millis(timeout.max(0) as u64));
            Ok(0)
        }
        fn get_feature_report(&self, _: &mut [u8]) -> HidResult<usize> {
            Ok(0)
        }
        fn send_feature_report(&self, _: &[u8]) -> HidResult<()> {
            Ok(())
        }
        fn write_output(&self, data: &[u8]) -> HidResult<usize> {
            self.outputs.lock().unwrap().push(data.to_vec());
            Ok(data.len())
        }
    }

    #[test]
    fn shared_reads_are_sliced_and_never_wait_past_their_timeout_for_a_held_lock() {
        let shared =
            SharedTransport::with_lock_timeout(Threaded::default(), Duration::from_millis(20));
        assert_eq!(shared.read_timeout(&mut [0; 8], 12).unwrap(), 0);
        let device = shared.shared.device.lock().unwrap();
        assert!(
            device
                .reads
                .lock()
                .unwrap()
                .iter()
                .all(|t| (0..=5).contains(t))
        );
        drop(device);

        // Another holder (a stalled display write) keeps the lock: the read
        // reports a normal timeout and a priority operation fails after its bound.
        let guard = shared.shared.device.lock().unwrap();
        let reader = shared.clone();
        let blocked = thread::spawn(move || {
            let read = reader.read_timeout(&mut [0; 8], 15).unwrap();
            let feature = reader.send_feature_report(&[0, 1]).unwrap_err();
            (read, feature.to_string())
        });
        let (read, feature) = blocked.join().unwrap();
        drop(guard);
        assert_eq!(read, 0);
        assert!(feature.contains(SHARED_BUSY), "{feature}");
    }

    #[test]
    fn closed_writes_fail_before_the_lock_while_feature_reports_still_go_out() {
        let shared = SharedTransport::new(Threaded::default());
        assert_eq!(shared.write_output(&[2, 1]).unwrap(), 2);
        let gate = shared.write_gate();
        gate.store(true, Ordering::SeqCst);
        // Even with the lock held elsewhere, a closed write fails at once.
        let guard = shared.shared.device.lock().unwrap();
        let error = shared.write_output(&[2, 2]).unwrap_err().to_string();
        drop(guard);
        assert!(error.contains(SHARED_WRITES_CLOSED), "{error}");
        shared.send_feature_report(&[0, 1]).unwrap();
        let device = shared.shared.device.lock().unwrap();
        assert_eq!(*device.outputs.lock().unwrap(), vec![vec![2, 1]]);
    }

    /// Logs reads (`R`) and writes (`W`) in the order they reach the device.
    #[derive(Default)]
    struct Ordered(Mutex<Vec<char>>);

    impl ReportTransport for Ordered {
        fn read_timeout(&self, _: &mut [u8], _: i32) -> HidResult<usize> {
            self.0.lock().unwrap().push('R');
            Ok(0)
        }
        fn get_feature_report(&self, _: &mut [u8]) -> HidResult<usize> {
            Ok(0)
        }
        fn send_feature_report(&self, _: &[u8]) -> HidResult<()> {
            self.0.lock().unwrap().push('F');
            Ok(())
        }
        fn write_output(&self, data: &[u8]) -> HidResult<usize> {
            self.0.lock().unwrap().push('W');
            Ok(data.len())
        }
    }

    #[test]
    fn a_waiting_reader_gets_the_lock_within_one_write_burst_and_writes_still_progress() {
        let shared = SharedTransport::new(Ordered::default());
        let stop = Arc::new(AtomicBool::new(false));
        let (writer, writing) = (shared.clone(), Arc::clone(&stop));
        let display = thread::spawn(move || {
            while !writing.load(Ordering::SeqCst) {
                writer.write_output(&[2, 0]).unwrap();
            }
        });
        let log = |shared: &SharedTransport<Ordered>| {
            shared
                .shared
                .device
                .lock()
                .unwrap()
                .0
                .lock()
                .unwrap()
                .clone()
        };
        let started = Instant::now() + Duration::from_secs(5);
        while !log(&shared).contains(&'W') {
            assert!(Instant::now() < started, "the writer never started");
            thread::yield_now();
        }
        // Each read call keeps the reader waiting throughout, slice after slice,
        // so the burst bound applies between consecutive reads of one call.
        // Repeat calls until the writer progressed inside one; how long that
        // takes depends only on scheduling, so the deadline is generous.
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut progressed = false;
        while !progressed && Instant::now() < deadline {
            let start = log(&shared).len();
            assert_eq!(shared.read_timeout(&mut [0; 8], 20).unwrap(), 0);
            let call = log(&shared)[start..].to_vec();
            let (Some(first), Some(last)) = (
                call.iter().position(|&event| event == 'R'),
                call.iter().rposition(|&event| event == 'R'),
            ) else {
                continue;
            };
            let during: String = call[first..=last].iter().collect();
            let longest = during.split('R').map(str::len).max().unwrap();
            assert!(
                longest <= SHARED_WRITE_BURST,
                "{longest} writes between reads"
            );
            progressed = during.contains('W');
        }
        stop.store(true, Ordering::SeqCst);
        display.join().unwrap();
        assert!(progressed, "the writer never wrote between two reads");
    }

    #[test]
    fn a_priority_feature_report_goes_out_before_further_writes() {
        let shared = SharedTransport::new(Ordered::default());
        let guard = shared.shared.device.lock().unwrap();
        let (restore, writer) = (shared.clone(), shared.clone());
        let feature = thread::spawn(move || restore.send_feature_report(&[0, 0]));
        let deadline = Instant::now() + Duration::from_secs(5);
        while shared.priority_waiters() == 0 {
            assert!(Instant::now() < deadline, "the restore never waited");
            thread::yield_now();
        }
        // A write queued behind the waiting priority operation goes out after it.
        let write = thread::spawn(move || writer.write_output(&[2, 0]));
        drop(guard);
        feature.join().unwrap().unwrap();
        write.join().unwrap().unwrap();
        assert_eq!(
            *shared.shared.device.lock().unwrap().0.lock().unwrap(),
            vec!['F', 'W']
        );
    }

    #[test]
    fn live_display_setup_fails_closed_without_touching_the_device() {
        let shared = SharedTransport::new(Fake::default());
        let error = live_display(&shared, Some(ModuleKind::M4315), Fixture)
            .err()
            .unwrap();
        assert!(error.starts_with("--display-sync setup failed"), "{error}");
        // `Fake` panics on any feature report, so only output writes need checking.
        assert!(
            shared
                .shared
                .device
                .lock()
                .unwrap()
                .outputs
                .borrow()
                .is_empty()
        );
        let unsupported = SharedTransport::new(Fake::new());
        let error = live_display(&unsupported, None, Fixture).err().unwrap();
        assert!(error.starts_with("--display-sync setup failed"), "{error}");
        assert!(error.contains("M4315 and M131E"), "{error}");
        // As above: no output report, and `Fake` would panic on a feature report.
        assert!(
            unsupported
                .shared
                .device
                .lock()
                .unwrap()
                .outputs
                .borrow()
                .is_empty()
        );
    }

    #[test]
    fn live_display_closing_its_gate_refuses_the_next_report() {
        let shared = SharedTransport::new(Fake::new());
        let mut display = live_display(&shared, Some(ModuleKind::M4315), Fixture).unwrap();
        display
            .sink
            .write_key_image(1, KEY_JPEG, &mut |_| {})
            .unwrap();
        let sent = shared.shared.device.lock().unwrap().outputs.borrow().len();
        assert!(sent > 0);
        // The runtime closes this gate; it must be the transport's own.
        display.write_gate.store(true, Ordering::SeqCst);
        let error = display
            .sink
            .write_key_image(2, KEY_JPEG, &mut |_| {})
            .unwrap_err();
        assert!(
            matches!(&error, SinkError::Transport(message) if message.contains(SHARED_WRITES_CLOSED)),
            "{error:?}"
        );
        assert_eq!(
            shared.shared.device.lock().unwrap().outputs.borrow().len(),
            sent
        );
    }
}
