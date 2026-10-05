//! Read-only device feed: normalized input from bundled captures or one HID
//! interface, streamed to the webview for live control state.
//!
//! A feed never enables software mode, sends feature/output reports, loads a
//! profile, or plans/dispatches actions. It only reads and decodes input.

use crate::replay::{self, DemoKind};
use cooltek_sdk_cli::{
    capture::encode_hex,
    device::{parse_path, select_device},
};
use cooltek_sdk_masterhub::{DeviceRegistry, InputReader, ModuleIdentity, ReportTransport};
use cooltek_sdk_service::{InputEvent, InputSession, InputUpdate, ServiceModule};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, hash_map::Entry},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

pub const INPUT_EVENT: &str = "device://input";
pub const STATUS_EVENT: &str = "device://status";
pub const INVENTORY_EVENT: &str = "device://inventory";

/// hidapi has no hot-plug callbacks, so the inventory is polled.
const INVENTORY_POLL: Duration = Duration::from_millis(1000);

/// Live read timeout; bounds how long a stop request waits for the reader.
const READ_TIMEOUT_MS: i32 = 100;
/// Longest pause between demo events, so sparse captures stay watchable.
const MAX_DEMO_GAP: Duration = Duration::from_millis(500);
/// Shortest pause between demo events. The bundled captures span only a few
/// milliseconds, so without it a demo would finish before it could be seen.
const MIN_DEMO_GAP: Duration = Duration::from_millis(350);
/// Demo pauses are sliced so a stop request is observed promptly.
const STOP_POLL: Duration = Duration::from_millis(50);

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FeedSource {
    Demo { demo: DemoKind },
    Live { path_hex: String },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Demo,
    Live,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FeedState {
    Running,
    Finished,
    Stopped,
    Failed,
}

#[derive(Clone, Debug, Serialize)]
pub struct InputPayload {
    pub feed_id: String,
    pub module: ServiceModule,
    pub serial_number: Option<String>,
    pub event: InputEvent,
}

#[derive(Clone, Debug, Serialize)]
pub struct StatusPayload {
    pub feed_id: String,
    pub state: FeedState,
    pub source: SourceKind,
    pub module: Option<ServiceModule>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Everything a feed thread reports; the Tauri glue maps it to events.
#[derive(Clone, Debug)]
pub enum FeedMessage {
    Input(InputPayload),
    Status(StatusPayload),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LiveDevice {
    pub path_hex: String,
    pub module: ServiceModule,
    pub serial_number: Option<String>,
}

/// Payload of `device://inventory`: every input-capable device, sorted.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct InventoryPayload {
    pub devices: Vec<LiveDevice>,
}

/// First HID usage page reserved for vendor-defined usages.
const VENDOR_USAGE_PAGE: u16 = 0xff00;

/// Selects the interfaces that can feed input, one entry per HID path.
///
/// Only modules with service input support are listed (the M0064 base is
/// not). A module may enumerate several interfaces or top-level collections;
/// the SDK's `ModuleHandle::connect` prefers the vendor-defined usage page
/// (`0xff00..`), which carries the vendor reports the input decoders parse
/// (every bundled capture identity is interface 0, usage page 0xff00). So for
/// each physical module (VID/PID/serial) only its vendor-page paths are kept,
/// falling back to all of its paths when none is vendor-defined. Paths that
/// enumerate more than once collapse to one entry; selecting such a path is
/// then rejected as ambiguous by `select_device` with an explanatory error.
/// This is inferred from the SDK and synthetic captures, not yet verified on
/// every module's hardware.
pub fn live_devices(devices: &[ModuleIdentity]) -> Vec<LiveDevice> {
    let supported: Vec<_> = devices
        .iter()
        .filter_map(|device| replay::service_module(device.module).map(|module| (device, module)))
        .collect();
    let has_vendor_page = |candidate: &ModuleIdentity| {
        supported.iter().any(|(device, _)| {
            device.vendor_id == candidate.vendor_id
                && device.product_id == candidate.product_id
                && device.serial_number == candidate.serial_number
                && device.usage_page >= VENDOR_USAGE_PAGE
        })
    };
    let mut listed: Vec<LiveDevice> = Vec::new();
    for (device, module) in &supported {
        if device.usage_page < VENDOR_USAGE_PAGE && has_vendor_page(device) {
            continue;
        }
        let path_hex = encode_hex(device.path.as_bytes());
        if listed.iter().any(|entry| entry.path_hex == path_hex) {
            continue;
        }
        listed.push(LiveDevice {
            path_hex,
            module: *module,
            serial_number: device.serial_number.clone(),
        });
    }
    sort_devices(&mut listed);
    listed
}

/// Orders devices by wire module id, then serial number (absent first), then
/// path, so equal inventories always compare and serialize identically.
pub fn sort_devices(devices: &mut [LiveDevice]) {
    devices.sort_by_cached_key(|device| {
        let module = serde_json::to_value(device.module)
            .ok()
            .and_then(|value| value.as_str().map(str::to_owned))
            .unwrap_or_default();
        (
            module,
            device.serial_number.clone(),
            device.path_hex.clone(),
        )
    });
}

/// Sorts a fresh snapshot and returns it only when it differs from the last
/// emitted one (`None` before anything was emitted, so the first always is).
pub fn inventory_change(
    last: Option<&[LiveDevice]>,
    mut snapshot: Vec<LiveDevice>,
) -> Option<Vec<LiveDevice>> {
    sort_devices(&mut snapshot);
    (last != Some(snapshot.as_slice())).then_some(snapshot)
}

/// Polls `snapshot` every `INVENTORY_POLL` until stopped, emitting the
/// inventory once at start (even when empty) and afterwards only when it
/// changes. A failed snapshot keeps the previous one, so errors emit nothing;
/// a failure before any success counts as an empty inventory. `wait` returns
/// false once stopped.
pub fn watch_inventory(
    stop: &AtomicBool,
    mut snapshot: impl FnMut() -> Result<Vec<LiveDevice>, String>,
    mut wait: impl FnMut(&AtomicBool, Duration) -> bool,
    mut emit: impl FnMut(InventoryPayload),
) {
    let mut current = Vec::new();
    let mut emitted: Option<Vec<LiveDevice>> = None;
    while !stop.load(Ordering::SeqCst) {
        if let Ok(devices) = snapshot() {
            current = devices;
        }
        if let Some(changed) = inventory_change(emitted.as_deref(), current.clone()) {
            emit(InventoryPayload {
                devices: changed.clone(),
            });
            emitted = Some(changed);
        }
        if !wait(stop, INVENTORY_POLL) {
            return;
        }
    }
}

/// Snapshot provider for the watcher thread. The registry is created lazily on
/// the first call, i.e. on the calling (watcher) thread, then refreshed on
/// every call; a failed creation or refresh drops it so the next tick starts
/// over.
///
/// Enumerating here does not disturb running feeds (checked against hidapi
/// 2.6.7 and `DeviceRegistry`): `HidApi::new` calls `hid_init` once per
/// process and never `hid_exit`, so several contexts may coexist; each keeps
/// its own device-list cache, and `refresh_devices` only rebuilds that cache
/// via `hid_enumerate`, which never touches open handles. A feed opens
/// through its own short-lived registry (`open_live`), and the SDK enables
/// `macos-shared-device`, so opens are non-exclusive on macOS. Enumeration
/// sends no feature or output reports. Not yet verified on hardware.
pub fn hid_inventory() -> impl FnMut() -> Result<Vec<LiveDevice>, String> {
    let mut registry: Option<DeviceRegistry> = None;
    move || {
        let refreshed = match registry.as_mut() {
            Some(existing) => existing.refresh(),
            None => DeviceRegistry::new().map(|created| {
                registry = Some(created);
            }),
        };
        if let Err(error) = refreshed {
            registry = None;
            return Err(format!("could not enumerate HID devices: {error}"));
        }
        Ok(registry
            .as_ref()
            .map(|registry| live_devices(&registry.devices()))
            .unwrap_or_default())
    }
}

/// The background inventory watcher; started once in setup, signaled on exit.
#[derive(Default)]
pub struct InventoryWatcher {
    stop: Arc<AtomicBool>,
    join: Mutex<Option<JoinHandle<()>>>,
}

impl InventoryWatcher {
    /// Starts `run` on its own thread; a second start is rejected.
    pub fn start(&self, run: impl FnOnce(&AtomicBool) + Send + 'static) -> Result<(), String> {
        let mut join = self
            .join
            .lock()
            .map_err(|_| "inventory watcher lock is unavailable".to_string())?;
        if join.is_some() {
            return Err("the inventory watcher is already running".into());
        }
        let stop = Arc::clone(&self.stop);
        *join = Some(
            thread::Builder::new()
                .name("device inventory".into())
                .spawn(move || run(&stop))
                .map_err(|error| format!("could not start the inventory watcher: {error}"))?,
        );
        Ok(())
    }

    /// Signals the watcher to stop without waiting, e.g. on app exit.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

/// Validates a source and returns its stable feed id without touching hardware.
pub fn feed_id(source: &FeedSource) -> Result<String, String> {
    Ok(match source {
        FeedSource::Demo { demo } => format!("demo:{}", demo_name(*demo)),
        FeedSource::Live { path_hex } => format!("live:{}", encode_hex(&parse_path(path_hex)?)),
    })
}

fn demo_name(kind: DemoKind) -> &'static str {
    match kind {
        DemoKind::Knobs => "knobs",
        DemoKind::Dial => "dial",
        DemoKind::Keys => "keys",
        DemoKind::Faders => "faders",
        DemoKind::Rollers => "rollers",
    }
}

fn blank_to_none(serial: Option<String>) -> Option<String> {
    serial.filter(|serial| !serial.trim().is_empty())
}

/// Decodes a bundled capture into input events; malformed reports are skipped.
pub fn decode_capture_events(
    capture: &str,
) -> Result<(ServiceModule, Option<String>, Vec<InputEvent>), String> {
    let decoded = replay::decode_capture(capture)?;
    let events = decoded
        .reports
        .into_iter()
        .filter_map(|(_, update)| match update {
            Ok(InputUpdate::Events(events)) => Some(events),
            Ok(InputUpdate::Unknown) | Err(_) => None,
        })
        .flatten()
        .collect();
    Ok((
        decoded.context.module,
        blank_to_none(decoded.context.serial_number),
        events,
    ))
}

/// Why a feed loop ended without an error.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Ending {
    Finished,
    Stopped,
}

/// Sleeps up to `duration`, returning false as soon as `stop` is set.
pub fn wait(stop: &AtomicBool, duration: Duration) -> bool {
    let deadline = Instant::now() + duration;
    loop {
        if stop.load(Ordering::SeqCst) {
            return false;
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return true;
        }
        thread::sleep(remaining.min(STOP_POLL));
    }
}

/// Emits events at capture pace; `wait` returns false once stopped.
pub fn play_events(
    events: &[InputEvent],
    stop: &AtomicBool,
    mut wait: impl FnMut(&AtomicBool, Duration) -> bool,
    mut emit: impl FnMut(InputEvent),
) -> Ending {
    let first = events.first().map(|event| event.timestamp_micros);
    let mut previous = first;
    for (index, event) in events.iter().enumerate() {
        let gap = event
            .timestamp_micros
            .saturating_sub(previous.unwrap_or(event.timestamp_micros));
        previous = Some(event.timestamp_micros);
        let pause = if index == 0 {
            Duration::ZERO
        } else {
            Duration::from_micros(gap).clamp(MIN_DEMO_GAP, MAX_DEMO_GAP)
        };
        if !wait(stop, pause) {
            return Ending::Stopped;
        }
        emit(*event);
    }
    Ending::Finished
}

/// Reads and decodes input until stopped. Only `read_timeout` is ever called
/// on the transport. Single malformed or unknown reports are skipped; a read
/// failure ends the feed with an error.
pub fn read_live<T: ReportTransport>(
    reader: &mut InputReader<T>,
    module: ServiceModule,
    stop: &AtomicBool,
    mut clock: impl FnMut() -> u64,
    mut emit: impl FnMut(InputEvent),
) -> Result<Ending, String> {
    let mut input = InputSession::new(module);
    while !stop.load(Ordering::SeqCst) {
        let Some(raw) = reader
            .read_input(READ_TIMEOUT_MS)
            .map_err(|error| format!("input read failed: {error}"))?
        else {
            continue;
        };
        if let Ok(InputUpdate::Events(events)) = input.decode(raw, clock()) {
            events.into_iter().for_each(&mut emit);
        }
    }
    Ok(Ending::Stopped)
}

/// Opens one exact interface read-only: no initialization or software mode.
fn open_live(
    path_hex: &str,
) -> Result<
    (
        InputReader<impl ReportTransport>,
        ServiceModule,
        Option<String>,
    ),
    String,
> {
    let path = parse_path(path_hex)?;
    let registry = DeviceRegistry::new()
        .map_err(|error| format!("could not enumerate HID devices: {error}"))?;
    let devices = registry.devices();
    let selected = select_device(&devices, &path, false)?;
    let module = replay::service_module(selected.module)
        .ok_or("the selected interface is not a supported input module")?;
    let device = registry.open(selected).map_err(|error| {
        format!(
            "could not open the module: {error}. Another program (for example \
             masterhub-service) may be holding the device."
        )
    })?;
    Ok((
        InputReader::new(device),
        module,
        blank_to_none(selected.serial_number.clone()),
    ))
}

/// Runs one feed to completion on the calling thread, emitting `running`
/// once started and then exactly one terminal status. A failure before the
/// source starts emits only `failed`.
pub fn run_feed(
    feed_id: &str,
    source: &FeedSource,
    stop: &AtomicBool,
    mut emit: impl FnMut(FeedMessage),
) {
    let kind = match source {
        FeedSource::Demo { .. } => SourceKind::Demo,
        FeedSource::Live { .. } => SourceKind::Live,
    };
    let mut module = None;
    let status = |emit: &mut dyn FnMut(FeedMessage), module, state, error| {
        emit(FeedMessage::Status(StatusPayload {
            feed_id: feed_id.into(),
            state,
            source: kind,
            module,
            error,
        }))
    };
    let input = |emit: &mut dyn FnMut(FeedMessage), module, serial: &Option<String>, event| {
        emit(FeedMessage::Input(InputPayload {
            feed_id: feed_id.into(),
            module,
            serial_number: serial.clone(),
            event,
        }))
    };
    let result = match source {
        FeedSource::Demo { demo } => {
            decode_capture_events(replay::demo_capture(*demo).0).map(|(found, serial, events)| {
                module = Some(found);
                status(&mut emit, module, FeedState::Running, None);
                play_events(&events, stop, wait, |event| {
                    input(&mut emit, found, &serial, event)
                })
            })
        }
        FeedSource::Live { path_hex } => {
            open_live(path_hex).and_then(|(mut reader, found, serial)| {
                module = Some(found);
                status(&mut emit, module, FeedState::Running, None);
                let origin = Instant::now();
                let clock = || u64::try_from(origin.elapsed().as_micros()).unwrap_or(u64::MAX);
                read_live(&mut reader, found, stop, clock, |event| {
                    input(&mut emit, found, &serial, event)
                })
            })
        }
    };
    let (state, error) = match result {
        Ok(Ending::Finished) => (FeedState::Finished, None),
        Ok(Ending::Stopped) => (FeedState::Stopped, None),
        Err(error) => (FeedState::Failed, Some(error)),
    };
    status(&mut emit, module, state, error);
}

struct FeedHandle {
    stop: Arc<AtomicBool>,
    /// Set after the terminal status is emitted, so the id can be reused.
    done: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl FeedHandle {
    fn running(&self) -> bool {
        !self.done.load(Ordering::SeqCst)
            && self.join.as_ref().is_some_and(|join| !join.is_finished())
    }
}

/// Running feeds by id. Finished feeds stay until replaced or stopped.
#[derive(Default)]
pub struct Feeds {
    feeds: Mutex<HashMap<String, FeedHandle>>,
}

impl Feeds {
    /// Starts `run` on its own thread unless the same id is still running.
    pub fn start(
        &self,
        feed_id: String,
        run: impl FnOnce(&AtomicBool) + Send + 'static,
    ) -> Result<String, String> {
        let mut feeds = self.lock()?;
        if let Entry::Occupied(entry) = feeds.entry(feed_id.clone()) {
            // A panicked thread never sets `done` but has still finished.
            let handle = entry.get();
            let finished = handle.done.load(Ordering::SeqCst)
                || handle.join.as_ref().is_none_or(JoinHandle::is_finished);
            if !finished {
                return Err(format!("feed {feed_id} is already running"));
            }
            if let Some(join) = entry.remove().join {
                let _ = join.join();
            }
        }
        let stop = Arc::new(AtomicBool::new(false));
        let done = Arc::new(AtomicBool::new(false));
        let (thread_stop, thread_done) = (Arc::clone(&stop), Arc::clone(&done));
        let join = thread::Builder::new()
            .name(format!("feed {feed_id}"))
            .spawn(move || {
                run(&thread_stop);
                thread_done.store(true, Ordering::SeqCst);
            })
            .map_err(|error| format!("could not start the feed: {error}"))?;
        feeds.insert(
            feed_id.clone(),
            FeedHandle {
                stop,
                done,
                join: Some(join),
            },
        );
        Ok(feed_id)
    }

    /// Stops a feed and waits for its thread, which emits `stopped` (or its
    /// earlier terminal status). Unknown ids are ignored.
    pub fn stop(&self, feed_id: &str) -> Result<(), String> {
        let handle = self.lock()?.remove(feed_id);
        if let Some(mut handle) = handle {
            handle.stop.store(true, Ordering::SeqCst);
            if let Some(join) = handle.join.take() {
                let _ = join.join();
            }
        }
        Ok(())
    }

    /// True while the feed's thread has not finished.
    pub fn is_running(&self, feed_id: &str) -> bool {
        self.lock()
            .is_ok_and(|feeds| feeds.get(feed_id).is_some_and(FeedHandle::running))
    }

    /// Ids of feeds whose threads have not finished, sorted.
    pub fn running_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self
            .lock()
            .map(|feeds| {
                feeds
                    .iter()
                    .filter(|(_, handle)| handle.running())
                    .map(|(id, _)| id.clone())
                    .collect()
            })
            .unwrap_or_default();
        ids.sort();
        ids
    }

    /// Signals every feed to stop without waiting, e.g. on app exit.
    pub fn stop_all(&self) {
        if let Ok(feeds) = self.feeds.lock() {
            for handle in feeds.values() {
                handle.stop.store(true, Ordering::SeqCst);
            }
        }
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, HashMap<String, FeedHandle>>, String> {
        self.feeds
            .lock()
            .map_err(|_| "feed registry lock is unavailable".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cooltek_sdk_masterhub::ModuleKind;
    use cooltek_sdk_masterhub::hidapi::{HidError, HidResult};
    use cooltek_sdk_service::{Gesture, InputValue};
    use std::{cell::RefCell, collections::VecDeque, ffi::CString, sync::mpsc};

    fn demo(kind: DemoKind) -> (ServiceModule, Option<String>, Vec<InputEvent>) {
        decode_capture_events(replay::demo_capture(kind).0).unwrap()
    }

    #[test]
    fn every_demo_capture_decodes_to_its_module_events() {
        for (kind, module, count) in [
            (DemoKind::Knobs, ServiceModule::M003K, 6),
            (DemoKind::Dial, ServiceModule::M131E, 9),
            (DemoKind::Keys, ServiceModule::M4315, 6),
            (DemoKind::Faders, ServiceModule::M005S, 7),
            (DemoKind::Rollers, ServiceModule::M032LE, 5),
        ] {
            let (found, serial, events) = demo(kind);
            assert_eq!((found, serial, events.len()), (module, None, count));
            assert!(
                events
                    .iter()
                    .all(|event| event.validate_for(module).is_ok())
            );
            assert!(
                events
                    .windows(2)
                    .all(|pair| pair[0].timestamp_micros <= pair[1].timestamp_micros)
            );
        }
    }

    #[test]
    fn fader_demo_carries_absolute_positions_with_their_maximum() {
        let (_, _, events) = demo(DemoKind::Faders);
        assert!(events.iter().all(|event| {
            event.gesture == Gesture::PositionChanged
                && matches!(event.value, Some(InputValue::Position { maximum: 100, .. }))
        }));
        assert_eq!(
            events[0].value,
            Some(InputValue::Position {
                raw: 0,
                maximum: 100
            })
        );
    }

    #[test]
    fn malformed_capture_reports_are_skipped_but_broken_captures_fail() {
        // The knob capture contains unknown/malformed reports among 7 reports.
        let (_, _, events) = demo(DemoKind::Knobs);
        assert_eq!(events.len(), 6);
        let (capture, _) = replay::demo_capture(DemoKind::Knobs);
        assert!(decode_capture_events(capture.trim_end()).is_err());
    }

    #[test]
    fn demo_playback_paces_with_capped_gaps_and_honors_stop() {
        let (_, _, events) = demo(DemoKind::Dial);
        let stop = AtomicBool::new(false);
        let mut waits = Vec::new();
        let mut emitted = Vec::new();
        let ending = play_events(
            &events,
            &stop,
            |_, gap| {
                waits.push(gap);
                true
            },
            |event| emitted.push(event),
        );
        assert_eq!(ending, Ending::Finished);
        assert_eq!(emitted, events);
        assert_eq!(waits[0], Duration::ZERO);
        assert!(
            waits[1..]
                .iter()
                .all(|gap| (MIN_DEMO_GAP..=MAX_DEMO_GAP).contains(gap))
        );
        assert!(waits.contains(&MIN_DEMO_GAP));

        let mut emitted = 0;
        stop.store(true, Ordering::SeqCst);
        let ending = play_events(&events, &stop, wait, |_| emitted += 1);
        assert_eq!((ending, emitted), (Ending::Stopped, 0));
    }

    #[derive(Default)]
    struct FakeTransport {
        reads: RefCell<VecDeque<HidResult<Vec<u8>>>>,
        commands: RefCell<Vec<&'static str>>,
        stop_when_empty: Option<Arc<AtomicBool>>,
    }

    impl ReportTransport for FakeTransport {
        fn read_timeout(&self, buffer: &mut [u8], _timeout_ms: i32) -> HidResult<usize> {
            match self.reads.borrow_mut().pop_front() {
                Some(Ok(bytes)) => {
                    buffer[..bytes.len()].copy_from_slice(&bytes);
                    Ok(bytes.len())
                }
                Some(Err(error)) => Err(error),
                None => {
                    if let Some(stop) = &self.stop_when_empty {
                        stop.store(true, Ordering::SeqCst);
                    }
                    Ok(0)
                }
            }
        }

        fn get_feature_report(&self, _buffer: &mut [u8]) -> HidResult<usize> {
            self.commands.borrow_mut().push("get_feature_report");
            Ok(0)
        }

        fn send_feature_report(&self, _data: &[u8]) -> HidResult<()> {
            self.commands.borrow_mut().push("send_feature_report");
            Ok(())
        }

        fn write_output(&self, _data: &[u8]) -> HidResult<usize> {
            self.commands.borrow_mut().push("write_output");
            Ok(0)
        }

        fn get_report_descriptor(&self, _buffer: &mut [u8]) -> HidResult<usize> {
            self.commands.borrow_mut().push("get_report_descriptor");
            Ok(0)
        }
    }

    fn fader_report(positions: [u8; 5]) -> Vec<u8> {
        let mut raw = vec![0x01, 0x2a, 0x07, 0x00, 0x02, 0x05];
        raw.extend_from_slice(&positions);
        raw
    }

    #[test]
    fn live_loop_skips_malformed_reports_and_never_writes_to_the_device() {
        let stop = Arc::new(AtomicBool::new(false));
        let transport = FakeTransport {
            reads: RefCell::new(VecDeque::from([
                Ok(fader_report([0, 25, 50, 75, 100])),
                Ok(vec![0x01, 0x2a, 0x07, 0x00, 0x02, 0x05, 0x01]),
                Ok(fader_report([60, 25, 50, 75, 100])),
            ])),
            stop_when_empty: Some(Arc::clone(&stop)),
            ..FakeTransport::default()
        };
        let mut reader = InputReader::new(transport);
        let mut tick = 0;
        let mut events = Vec::new();
        let ending = read_live(
            &mut reader,
            ServiceModule::M005S,
            &stop,
            || {
                tick += 10;
                tick
            },
            |event| events.push(event),
        )
        .unwrap();
        assert_eq!(ending, Ending::Stopped);
        assert_eq!(events.len(), 6);
        assert_eq!(
            events.last().unwrap().value,
            Some(InputValue::Position {
                raw: 60,
                maximum: 100
            })
        );
        assert_eq!(events.last().unwrap().timestamp_micros, 30);
        assert!(reader.transport().commands.borrow().is_empty());
    }

    #[test]
    fn live_loop_fails_on_read_errors_and_exits_once_stopped() {
        let transport = FakeTransport {
            reads: RefCell::new(VecDeque::from([Err(HidError::HidApiError {
                message: "unplugged".into(),
            })])),
            ..FakeTransport::default()
        };
        let mut reader = InputReader::new(transport);
        let stop = AtomicBool::new(false);
        let error = read_live(&mut reader, ServiceModule::M003K, &stop, || 0, |_| {}).unwrap_err();
        assert!(error.contains("unplugged"));

        stop.store(true, Ordering::SeqCst);
        let mut reader = InputReader::new(FakeTransport::default());
        assert_eq!(
            read_live(&mut reader, ServiceModule::M003K, &stop, || 0, |_| {}),
            Ok(Ending::Stopped)
        );
        assert!(reader.transport().commands.borrow().is_empty());
    }

    fn statuses(messages: &[FeedMessage]) -> Vec<(FeedState, Option<ServiceModule>)> {
        messages
            .iter()
            .filter_map(|message| match message {
                FeedMessage::Status(status) => Some((status.state, status.module)),
                FeedMessage::Input(_) => None,
            })
            .collect()
    }

    #[test]
    fn demo_feed_reports_running_inputs_then_finished() {
        let mut messages = Vec::new();
        let source = FeedSource::Demo {
            demo: DemoKind::Faders,
        };
        let id = feed_id(&source).unwrap();
        run_feed(&id, &source, &AtomicBool::new(false), |message| {
            messages.push(message)
        });
        assert_eq!(id, "demo:faders");
        assert_eq!(
            statuses(&messages),
            [
                (FeedState::Running, Some(ServiceModule::M005S)),
                (FeedState::Finished, Some(ServiceModule::M005S))
            ]
        );
        assert_eq!(messages.len(), 9);
        assert!(matches!(&messages[1], FeedMessage::Input(input)
            if input.feed_id == "demo:faders" && input.module == ServiceModule::M005S));
    }

    #[test]
    fn stopped_demo_feed_reports_stopped_once() {
        let mut messages = Vec::new();
        let source = FeedSource::Demo {
            demo: DemoKind::Dial,
        };
        run_feed("demo:dial", &source, &AtomicBool::new(true), |message| {
            messages.push(message)
        });
        assert_eq!(
            statuses(&messages),
            [
                (FeedState::Running, Some(ServiceModule::M131E)),
                (FeedState::Stopped, Some(ServiceModule::M131E))
            ]
        );
    }

    #[test]
    fn live_ids_are_normalized_and_invalid_paths_rejected_without_hardware() {
        let source = FeedSource::Live {
            path_hex: "4142".into(),
        };
        assert_eq!(feed_id(&source).unwrap(), "live:4142");
        let upper = FeedSource::Live {
            path_hex: "4A".into(),
        };
        assert_eq!(feed_id(&upper).unwrap(), "live:4a");
        for bad in ["", "00", "zz", "123"] {
            assert!(
                feed_id(&FeedSource::Live {
                    path_hex: bad.into()
                })
                .is_err()
            );
        }
        let parsed: FeedSource = serde_json::from_str(r#"{"kind":"demo","demo":"keys"}"#).unwrap();
        assert!(matches!(
            parsed,
            FeedSource::Demo {
                demo: DemoKind::Keys
            }
        ));
        assert!(serde_json::from_str::<FeedSource>(r#"{"kind":"file","path":"/x"}"#).is_err());
    }

    #[test]
    fn feed_ids_collide_while_running_and_can_restart_once_done() {
        let feeds = Feeds::default();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let id = feeds
            .start("demo:keys".into(), move |_| {
                let _ = release_rx.recv();
            })
            .unwrap();
        assert!(feeds.start(id.clone(), |_| {}).is_err());
        release_tx.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while feeds.start(id.clone(), |_| {}).is_err() {
            assert!(
                Instant::now() < deadline,
                "finished feed never became restartable"
            );
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn stop_sets_the_flag_and_waits_for_the_thread() {
        let feeds = Feeds::default();
        let (exited_tx, exited_rx) = mpsc::channel();
        feeds
            .start("live:41".into(), move |stop| {
                while !stop.load(Ordering::SeqCst) {
                    thread::sleep(Duration::from_millis(5));
                }
                exited_tx.send(()).unwrap();
            })
            .unwrap();
        let started = Instant::now();
        feeds.stop("live:41").unwrap();
        assert!(exited_rx.try_recv().is_ok());
        assert!(started.elapsed() < Duration::from_millis(200));
        feeds.stop("unknown").unwrap();
        // The stopped id is free again.
        feeds.start("live:41".into(), |_| {}).unwrap();
        feeds.stop_all();
    }

    fn identity(
        path: &str,
        product_id: u16,
        serial: Option<&str>,
        usage_page: u16,
    ) -> ModuleIdentity {
        ModuleIdentity {
            path: CString::new(path).unwrap(),
            vendor_id: 0x2516,
            product_id,
            serial_number: serial.map(str::to_owned),
            manufacturer_string: None,
            product_string: None,
            release_number: 0,
            interface_number: 0,
            usage_page,
            usage: 1,
            module: ModuleKind::from_ids(0x2516, product_id),
        }
    }

    fn device(module: ServiceModule, serial: Option<&str>, path: &str) -> LiveDevice {
        LiveDevice {
            path_hex: encode_hex(path.as_bytes()),
            module,
            serial_number: serial.map(str::to_owned),
        }
    }

    /// Runs the watcher over scripted snapshots, one per tick, stopping once
    /// the script is exhausted. Returns every emitted device list.
    fn watch(script: Vec<Result<Vec<LiveDevice>, String>>) -> Vec<Vec<LiveDevice>> {
        let ticks = script.len();
        let script = RefCell::new(VecDeque::from(script));
        let mut waits = Vec::new();
        let mut emitted = Vec::new();
        watch_inventory(
            &AtomicBool::new(false),
            || script.borrow_mut().pop_front().unwrap(),
            |_, pause| {
                waits.push(pause);
                !script.borrow().is_empty()
            },
            |payload| emitted.push(payload.devices),
        );
        assert_eq!(waits, vec![INVENTORY_POLL; ticks]);
        emitted
    }

    #[test]
    fn inventory_is_emitted_at_start_even_when_empty_and_not_repeated() {
        assert_eq!(watch(vec![Ok(vec![]), Ok(vec![]), Ok(vec![])]), [vec![]]);
        let knobs = device(ServiceModule::M003K, Some("K1"), "a");
        assert_eq!(
            watch(vec![Ok(vec![knobs.clone()]), Ok(vec![knobs.clone()])]),
            [vec![knobs]]
        );
    }

    #[test]
    fn inventory_changes_on_add_remove_and_swap() {
        let knobs = device(ServiceModule::M003K, Some("K1"), "a");
        let faders = device(ServiceModule::M005S, None, "b");
        // Same module in the same slot with another serial is a swap.
        let other_knobs = device(ServiceModule::M003K, Some("K9"), "a");
        let emitted = watch(vec![
            Ok(vec![knobs.clone()]),
            Ok(vec![faders.clone(), knobs.clone()]),
            Ok(vec![knobs.clone(), faders.clone()]),
            Ok(vec![faders.clone()]),
            Ok(vec![faders.clone(), other_knobs.clone()]),
            Ok(vec![]),
        ]);
        assert_eq!(
            emitted,
            [
                vec![knobs.clone()],
                vec![knobs.clone(), faders.clone()],
                vec![faders.clone()],
                vec![other_knobs, faders],
                vec![],
            ]
        );
    }

    #[test]
    fn inventory_errors_keep_the_previous_snapshot_silently() {
        let keys = device(ServiceModule::M4315, Some("S"), "k");
        let emitted = watch(vec![
            Err("no hid".into()),
            Ok(vec![keys.clone()]),
            Err("refresh failed".into()),
            Err("refresh failed".into()),
            Ok(vec![keys.clone()]),
        ]);
        // A failure before any success counts as an empty inventory.
        assert_eq!(emitted, [vec![], vec![keys]]);
    }

    #[test]
    fn inventory_watch_ends_on_stop() {
        let stop = AtomicBool::new(true);
        let mut calls = 0;
        watch_inventory(&stop, || Ok(vec![]), wait, |_| calls += 1);
        assert_eq!(calls, 0);

        // A stop during the poll pause ends the loop after the current tick.
        let stop = AtomicBool::new(false);
        let mut snapshots = 0;
        watch_inventory(
            &stop,
            || {
                snapshots += 1;
                Ok(vec![])
            },
            |stop, pause| {
                stop.store(true, Ordering::SeqCst);
                wait(stop, pause)
            },
            |_| {},
        );
        assert_eq!(snapshots, 1);
    }

    #[test]
    fn inventory_order_is_deterministic() {
        let a = device(ServiceModule::M4315, Some("B"), "z");
        let b = device(ServiceModule::M4315, Some("A"), "y");
        let c = device(ServiceModule::M4315, None, "x");
        let d = device(ServiceModule::M003K, Some("Z"), "w");
        let e = device(ServiceModule::M4315, Some("A"), "v");
        let expected = vec![d.clone(), c.clone(), e.clone(), b.clone(), a.clone()];
        for order in [
            vec![a.clone(), b.clone(), c.clone(), d.clone(), e.clone()],
            vec![e.clone(), d.clone(), c.clone(), b.clone(), a.clone()],
            vec![c, a, e, b, d],
        ] {
            assert_eq!(
                inventory_change(None, order.clone()),
                Some(expected.clone())
            );
            assert_eq!(inventory_change(Some(&expected), order), None);
        }
    }

    #[test]
    fn inventory_watcher_runs_once_and_stops_promptly() {
        let watcher = InventoryWatcher::default();
        let (exited_tx, exited_rx) = mpsc::channel();
        watcher
            .start(move |stop| {
                watch_inventory(stop, || Ok(vec![]), wait, |_| {});
                exited_tx.send(()).unwrap();
            })
            .unwrap();
        assert!(watcher.start(|_| {}).is_err());
        thread::sleep(Duration::from_millis(20));
        watcher.stop();
        assert!(exited_rx.recv_timeout(Duration::from_millis(150)).is_ok());
    }

    #[test]
    fn live_device_listing_keeps_supported_vendor_interfaces_once() {
        let devices = [
            // Base hub: recognized but has no service input support.
            identity("hub", 0x01f6, None, 0xff00),
            // Knobs: a consumer collection and the vendor collection.
            identity("knobs-consumer", 0x0202, Some("K1"), 0x000c),
            identity("knobs-vendor", 0x0202, Some("K1"), 0xff00),
            // Faders without a vendor page fall back to their only interface.
            identity("faders", 0x0200, None, 0x0001),
            // The same path enumerated twice is listed once.
            identity("keys", 0x01fe, Some("K2"), 0xff00),
            identity("keys", 0x01fe, Some("K2"), 0xff01),
            // Unknown product.
            identity("unknown", 0x1234, None, 0xff00),
        ];
        let listed = live_devices(&devices);
        assert_eq!(
            listed,
            [
                LiveDevice {
                    path_hex: encode_hex(b"knobs-vendor"),
                    module: ServiceModule::M003K,
                    serial_number: Some("K1".into()),
                },
                LiveDevice {
                    path_hex: encode_hex(b"faders"),
                    module: ServiceModule::M005S,
                    serial_number: None,
                },
                LiveDevice {
                    path_hex: encode_hex(b"keys"),
                    module: ServiceModule::M4315,
                    serial_number: Some("K2".into()),
                },
            ]
        );
    }
}
