//! Ctrl-C/SIGTERM handling shared by recording and live sessions.
//!
//! The first signal sets a cooperative flag so the caller can write its footer
//! and restore device mode. A second signal exits immediately with status 130,
//! skipping any cleanup that is still pending. Signals are counted separately
//! from the flag, so a caller that sets the flag for internal cancellation does
//! not turn the next real signal into a forced exit.

use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

/// Exit status used when a repeated interrupt forces termination.
pub const FORCED_EXIT_CODE: i32 = 130;

/// Installs the process-wide handler and returns its cooperative stop flag.
pub fn install() -> Result<Arc<AtomicBool>, String> {
    let stopped = Arc::new(AtomicBool::new(false));
    let signal = Arc::clone(&stopped);
    let signals = AtomicUsize::new(0);
    ctrlc::set_handler(move || {
        if signal_received(&signals, &signal) {
            eprintln!("second interrupt received; exiting without cleanup");
            std::process::exit(FORCED_EXIT_CODE);
        }
    })
    .map_err(|error| format!("could not install interrupt handler: {error}"))?;
    Ok(stopped)
}

/// Records one signal and sets the stop flag; returns true when a previous
/// signal (not merely a set flag) was already received.
pub fn signal_received(signals: &AtomicUsize, flag: &AtomicBool) -> bool {
    flag.store(true, Ordering::SeqCst);
    signals.fetch_add(1, Ordering::SeqCst) > 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_signal_requests_cleanup_and_second_forces_exit() {
        let signals = AtomicUsize::new(0);
        let flag = AtomicBool::new(false);
        assert!(!signal_received(&signals, &flag));
        assert!(flag.load(Ordering::SeqCst));
        assert!(signal_received(&signals, &flag));
    }

    #[test]
    fn internal_cancellation_does_not_count_as_a_signal() {
        let signals = AtomicUsize::new(0);
        // A live session sets the shared flag for drain/worker cancellation.
        let flag = AtomicBool::new(true);
        assert!(!signal_received(&signals, &flag));
        assert!(signal_received(&signals, &flag));
    }
}
