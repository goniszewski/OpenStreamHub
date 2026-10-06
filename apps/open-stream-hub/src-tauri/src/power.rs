//! System sleep/wake events for the session manager.
//!
//! macOS uses the native `NSWorkspaceWillSleepNotification` /
//! `NSWorkspaceDidWakeNotification` observers, plus
//! `NSWorkspaceScreensDidWakeNotification` as a second source of `Resume`
//! (screens only wake on a full wake, never in a Power Nap dark wake). It is
//! forwarded only while a `WillSleep` has not been answered by a wake yet, so
//! a display waking from display sleep alone never restarts anything. A
//! `DidWake` after a forwarded `ScreensDidWake` is not forwarded again (a
//! second `Resume` would restart the settle delay); see `WakeGate`.
//! `objc2`, `objc2-foundation`,
//! `objc2-app-kit` and `block2` were already locked as Tauri dependencies, so
//! depending on them directly downloads nothing; only the features
//! `objc2-foundation/NSOperation` and `objc2-app-kit/NSWorkspace` are newly
//! enabled on those already-downloaded crates.
//!
//! Every other platform uses the clock-gap fallback: a 1 s poll compares the
//! monotonic clock (which does not advance while the machine sleeps) with the
//! wall clock. It can only report `Resume` after the fact; there is no clean
//! pre-sleep stop, so the OS will have interrupted the service without a
//! hardware-mode restore. A forward wall-clock adjustment of at least
//! `GAP_THRESHOLD` (e.g. NTP) also reads as a wake.

use std::{
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant, SystemTime},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PowerEvent {
    Suspend,
    Resume,
}

/// Receives power events. A sink may block briefly (bounded) so that the
/// pre-sleep stop can finish before the observer returns.
pub type Sink = Arc<dyn Fn(PowerEvent) + Send + Sync>;

pub trait PowerSource {
    fn start(self: Box<Self>, sink: Sink) -> Result<(), String>;
}

// The fallback is compiled and tested everywhere, but only used off macOS.
/// Wall-clock time beyond the monotonic delta that counts as a sleep.
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub const GAP_THRESHOLD: Duration = Duration::from_secs(10);
#[cfg_attr(target_os = "macos", allow(dead_code))]
const GAP_POLL: Duration = Duration::from_secs(1);

/// True when the wall clock ran at least `GAP_THRESHOLD` further than the
/// monotonic clock. `wall_delta` is `None` when the wall clock went backwards.
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub fn slept(monotonic_delta: Duration, wall_delta: Option<Duration>) -> bool {
    wall_delta.is_some_and(|wall| wall >= monotonic_delta.saturating_add(GAP_THRESHOLD))
}

/// Fallback detector: emits `Resume` after a sleep-sized clock gap.
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub struct ClockGap;

impl PowerSource for ClockGap {
    fn start(self: Box<Self>, sink: Sink) -> Result<(), String> {
        thread::Builder::new()
            .name("sleep detector".into())
            .spawn(move || {
                let mut previous = (Instant::now(), SystemTime::now());
                loop {
                    thread::sleep(GAP_POLL);
                    let now = (Instant::now(), SystemTime::now());
                    let monotonic = now.0.duration_since(previous.0);
                    let wall = now.1.duration_since(previous.1).ok();
                    previous = now;
                    if slept(monotonic, wall) {
                        sink(PowerEvent::Resume);
                    }
                }
            })
            .map(drop)
            .map_err(|error| format!("could not start the sleep detector: {error}"))
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use super::{PowerEvent, PowerSource, Sink, WakeGate};
    use block2::RcBlock;
    use objc2_app_kit::{
        NSWorkspace, NSWorkspaceDidWakeNotification, NSWorkspaceScreensDidWakeNotification,
        NSWorkspaceWillSleepNotification,
    };
    use objc2_foundation::NSNotification;
    use std::{ptr::NonNull, sync::Arc};

    /// Which notification a block observes.
    #[derive(Clone, Copy)]
    enum Kind {
        WillSleep,
        DidWake,
        ScreensDidWake,
    }

    /// Native observers on the shared workspace's notification center. The
    /// notifications are posted on the main thread, which runs the blocks.
    pub struct Workspace;

    impl PowerSource for Workspace {
        fn start(self: Box<Self>, sink: Sink) -> Result<(), String> {
            let center = NSWorkspace::sharedWorkspace().notificationCenter();
            // Shared by the three observers; see `WakeGate`.
            let gate = Arc::new(WakeGate::new());
            // SAFETY: AppKit's immutable notification-name constants.
            let names = unsafe {
                [
                    (NSWorkspaceWillSleepNotification, Kind::WillSleep),
                    (NSWorkspaceDidWakeNotification, Kind::DidWake),
                    (NSWorkspaceScreensDidWakeNotification, Kind::ScreensDidWake),
                ]
            };
            for (name, kind) in names {
                let sink = Arc::clone(&sink);
                let gate = Arc::clone(&gate);
                let block = RcBlock::new(move |_: NonNull<NSNotification>| {
                    let event = match kind {
                        Kind::WillSleep => gate.will_sleep().then_some(PowerEvent::Suspend),
                        // Unless `ScreensDidWake` already resumed this wake.
                        Kind::DidWake => gate.did_wake().then_some(PowerEvent::Resume),
                        // Only a wake whose `DidWake` has not arrived (yet).
                        Kind::ScreensDidWake => {
                            gate.screens_did_wake().then_some(PowerEvent::Resume)
                        }
                    };
                    if let Some(event) = event {
                        sink(event);
                    }
                });
                // SAFETY: no object filter and no queue (the block runs on the
                // posting thread); the block only calls a Send + Sync sink.
                let observer = unsafe {
                    center.addObserverForName_object_queue_usingBlock(
                        Some(name),
                        None,
                        None,
                        &block,
                    )
                };
                // Observed for the whole process lifetime.
                std::mem::forget(observer);
            }
            Ok(())
        }
    }
}

/// Which wake notifications become `Resume` (macOS only; pure for tests).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
enum WakeState {
    Awake,
    /// A `WillSleep` that no wake notification answered yet.
    Sleeping,
    /// `ScreensDidWake` already forwarded the `Resume` for this sleep.
    ScreensWoke,
}

/// Decides which of the sleep/wake notifications are forwarded:
/// - `WillSleep` always is.
/// - `ScreensDidWake` only answers a pending `WillSleep`, so display sleep
///   alone (or a second screens wake) restarts nothing.
/// - `DidWake` is forwarded unless `ScreensDidWake` already forwarded this
///   wake: a second `Resume` would restart the settle delay and could stop a
///   process the first one just started. A `DidWake` without a prior
///   `WillSleep` (a missed sleep notification) is still forwarded.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub struct WakeGate(Mutex<WakeState>);

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
impl WakeGate {
    pub fn new() -> Self {
        Self(Mutex::new(WakeState::Awake))
    }

    fn set(&self, change: impl FnOnce(WakeState) -> (WakeState, bool)) -> bool {
        let mut state = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (next, forward) = change(*state);
        *state = next;
        forward
    }

    /// True: forward `Suspend`.
    pub fn will_sleep(&self) -> bool {
        self.set(|_| (WakeState::Sleeping, true))
    }

    /// True: forward `Resume`.
    pub fn did_wake(&self) -> bool {
        self.set(|state| (WakeState::Awake, state != WakeState::ScreensWoke))
    }

    /// True: forward `Resume`.
    pub fn screens_did_wake(&self) -> bool {
        self.set(|state| match state {
            WakeState::Sleeping => (WakeState::ScreensWoke, true),
            other => (other, false),
        })
    }
}

/// The platform's power source; see the module docs for the choice.
pub fn system_source() -> Box<dyn PowerSource> {
    #[cfg(target_os = "macos")]
    return Box::new(macos::Workspace);
    #[cfg(not(target_os = "macos"))]
    Box::new(ClockGap)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(value: u64) -> Duration {
        Duration::from_secs(value)
    }

    #[test]
    fn a_wall_clock_jump_beyond_the_monotonic_delta_reads_as_a_wake() {
        // A normal poll: both clocks advance together.
        assert!(!slept(secs(1), Some(secs(1))));
        // Jitter and a 9 s gap stay below the threshold.
        assert!(!slept(secs(1), Some(Duration::from_millis(1_200))));
        assert!(!slept(secs(1), Some(secs(10))));
        // 10 s beyond the monotonic delta is a sleep.
        assert!(slept(secs(1), Some(secs(11))));
        assert!(slept(secs(1), Some(secs(3_600))));
        // A stalled poll advances both clocks: no sleep.
        assert!(!slept(secs(30), Some(secs(31))));
        // A wall clock set backwards never reads as a wake.
        assert!(!slept(secs(1), None));
    }

    #[test]
    fn each_wake_is_forwarded_once() {
        let gate = WakeGate::new();
        // DidWake alone (its sleep notification was missed) still resumes.
        assert!(gate.did_wake());
        // Display sleep alone: the screens wake without a sleep.
        assert!(!gate.screens_did_wake());
        // A normal wake: DidWake, then ScreensDidWake.
        assert!(gate.will_sleep());
        assert!(gate.did_wake());
        assert!(!gate.screens_did_wake());
        // Screens first: the later DidWake is not a second Resume.
        assert!(gate.will_sleep());
        assert!(gate.screens_did_wake());
        assert!(!gate.screens_did_wake());
        assert!(!gate.did_wake());
        // The next DidWake without a sleep resumes again.
        assert!(gate.did_wake());
        // A lost wake: the next sleep starts over.
        assert!(gate.will_sleep());
        assert!(gate.screens_did_wake());
        assert!(gate.will_sleep());
        assert!(gate.did_wake());
    }
}
