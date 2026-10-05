//! Ctrl-C/SIGTERM handling shared by recording and live sessions.
//!
//! The first signal sets a cooperative flag so the caller can write its footer
//! and restore device mode. A second signal exits immediately with status 130,
//! skipping any cleanup that is still pending.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

/// Exit status used when a repeated interrupt forces termination.
pub const FORCED_EXIT_CODE: i32 = 130;

/// Installs the process-wide handler and returns its cooperative stop flag.
pub fn install() -> Result<Arc<AtomicBool>, String> {
    let stopped = Arc::new(AtomicBool::new(false));
    let signal = Arc::clone(&stopped);
    ctrlc::set_handler(move || {
        if signal_received(&signal) {
            eprintln!("second interrupt received; exiting without cleanup");
            std::process::exit(FORCED_EXIT_CODE);
        }
    })
    .map_err(|error| format!("could not install interrupt handler: {error}"))?;
    Ok(stopped)
}

/// Records one signal; returns true when a previous signal was already pending.
pub fn signal_received(flag: &AtomicBool) -> bool {
    flag.swap(true, Ordering::SeqCst)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_signal_requests_cleanup_and_second_forces_exit() {
        let flag = AtomicBool::new(false);
        assert!(!signal_received(&flag));
        assert!(flag.load(Ordering::SeqCst));
        assert!(signal_received(&flag));
    }
}
