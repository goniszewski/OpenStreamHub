//! System sleep/wake events for the session manager.
//!
//! macOS uses the native `NSWorkspaceWillSleepNotification` /
//! `NSWorkspaceDidWakeNotification` observers. `objc2`, `objc2-foundation`,
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
    sync::Arc,
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
    use super::{PowerEvent, PowerSource, Sink};
    use block2::RcBlock;
    use objc2_app_kit::{
        NSWorkspace, NSWorkspaceDidWakeNotification, NSWorkspaceWillSleepNotification,
    };
    use objc2_foundation::NSNotification;
    use std::{ptr::NonNull, sync::Arc};

    /// Native observers on the shared workspace's notification center. The
    /// notifications are posted on the main thread, which runs the blocks.
    pub struct Workspace;

    impl PowerSource for Workspace {
        fn start(self: Box<Self>, sink: Sink) -> Result<(), String> {
            let center = NSWorkspace::sharedWorkspace().notificationCenter();
            // SAFETY: AppKit's immutable notification-name constants.
            let names = unsafe {
                [
                    (NSWorkspaceWillSleepNotification, PowerEvent::Suspend),
                    (NSWorkspaceDidWakeNotification, PowerEvent::Resume),
                ]
            };
            for (name, event) in names {
                let sink = Arc::clone(&sink);
                let block = RcBlock::new(move |_: NonNull<NSNotification>| sink(event));
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
}
