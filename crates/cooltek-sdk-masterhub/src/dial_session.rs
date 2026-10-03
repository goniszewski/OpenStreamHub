use std::time::Duration;

use hidapi::{HidError, HidResult};

use crate::{
    InputReader, ReportTransport,
    input::{DecodedM131E, M131EDecoder, M131EEvent},
};

fn error(message: &str) -> HidError {
    HidError::HidApiError {
        message: message.into(),
    }
}

/// Explicit lifecycle state; it does not establish that hardware remains attached.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialSessionState {
    /// No active session, or a transport operation failed.
    Inactive,
    /// Software mode was requested successfully and input reads are enabled.
    Active,
    /// Hardware mode was requested for suspension.
    Suspended,
    /// Waiting for the application-derived resume delay, without sleeping.
    ResumePending {
        /// Earliest activation time in the caller's monotonic clock domain.
        ready_at: Duration,
    },
}

/// Owns dial input and provides explicit start, suspend, resume, and shutdown.
///
/// Construction performs no I/O. The caller delivers OS lifecycle notifications
/// and monotonic times; this type does not watch macOS or reconnect devices.
/// Firmware queries and display refresh are explicit caller operations after
/// activation. No commands run from `Drop`.
pub struct DialSession<T: ReportTransport> {
    reader: InputReader<T>,
    decoder: M131EDecoder,
    state: DialSessionState,
}

impl<T: ReportTransport> DialSession<T> {
    /// Resume delay observed in the official application, not a firmware requirement.
    pub const RESUME_DELAY: Duration = Duration::from_secs(2);

    /// Takes ownership of a transport without reading or initializing it.
    pub fn new(transport: T) -> Self {
        Self {
            reader: InputReader::new(transport),
            decoder: M131EDecoder::new(),
            state: DialSessionState::Inactive,
        }
    }

    /// Returns the last successful lifecycle state.
    pub fn state(&self) -> DialSessionState {
        self.state
    }

    /// Explicitly enters software mode and clears old button state.
    ///
    /// Repeated calls while active are idempotent. Errors leave the session
    /// inactive; the actual device mode is unknown after a failed write.
    pub fn activate(&mut self) -> HidResult<()> {
        if self.state == DialSessionState::Active {
            return Ok(());
        }
        self.decoder.reset();
        self.state = DialSessionState::Inactive;
        crate::dial::set_software_mode(self.reader.transport(), true)?;
        self.state = DialSessionState::Active;
        Ok(())
    }

    /// Requests hardware mode and clears button state before suspension.
    pub fn suspend(&mut self) -> HidResult<()> {
        match self.state {
            DialSessionState::Inactive => {
                return Err(error("Activate the dial session before suspending"));
            }
            DialSessionState::Active => {
                self.decoder.reset();
                self.state = DialSessionState::Inactive;
                crate::dial::set_software_mode(self.reader.transport(), false)?;
            }
            _ => self.decoder.reset(),
        }
        self.state = DialSessionState::Suspended;
        Ok(())
    }

    /// Schedules resume using a caller-provided monotonic time, without I/O.
    ///
    /// Duplicate resume notifications preserve the first deadline. Call
    /// [`Self::advance`] from the owning event loop when time advances.
    pub fn resume_at(&mut self, now: Duration) -> HidResult<()> {
        match self.state {
            DialSessionState::Suspended => {
                let ready_at = now
                    .checked_add(Self::RESUME_DELAY)
                    .ok_or_else(|| error("Dial resume deadline overflow"))?;
                self.state = DialSessionState::ResumePending { ready_at };
                Ok(())
            }
            DialSessionState::ResumePending { .. } => Ok(()),
            _ => Err(error("Only a suspended dial session can schedule resume")),
        }
    }

    /// Activates when a pending resume deadline is reached; returns true once.
    ///
    /// A true result lets the caller query firmware and refresh the display.
    /// An activation failure clears the pending state and propagates the error.
    pub fn advance(&mut self, now: Duration) -> HidResult<bool> {
        if let DialSessionState::ResumePending { ready_at } = self.state
            && now >= ready_at
        {
            self.activate()?;
            return Ok(true);
        }
        Ok(false)
    }

    /// Reads and decodes one bounded input update only while active.
    ///
    /// Idle returns `None`. Transport failures clear lifecycle/button state;
    /// malformed or unknown reports preserve state. Action consumers must reset
    /// any held actions and gesture trackers when the session becomes inactive.
    pub fn read_input(&mut self, timeout_ms: i32) -> HidResult<Option<DecodedM131E<'_>>> {
        if self.state != DialSessionState::Active {
            return Err(error("Dial input requires an active session"));
        }
        if timeout_ms <= 0 {
            return Err(error("Input timeout must be positive"));
        }
        let raw = match self.reader.read_input(timeout_ms) {
            Ok(Some(raw)) => raw,
            Ok(None) => return Ok(None),
            Err(error) => {
                self.decoder.reset();
                self.state = DialSessionState::Inactive;
                return Err(error);
            }
        };
        self.decoder
            .decode_report(raw)
            .map(Some)
            .map_err(|e| error(&e.to_string()))
    }

    /// Returns hardware mode when active, then clears state even on failure.
    ///
    /// No commands are sent when already suspended or inactive. Call before
    /// releasing the transport; cleanup does not run implicitly on drop.
    pub fn shutdown(&mut self) -> HidResult<()> {
        let active = self.state == DialSessionState::Active;
        self.state = DialSessionState::Inactive;
        self.decoder.reset();
        if active {
            crate::dial::set_software_mode(self.reader.transport(), false)
        } else {
            Ok(())
        }
    }

    /// Borrows the transport for coordinated explicit queries or display writes.
    pub fn transport(&self) -> &T {
        self.reader.transport()
    }

    /// Releases ownership without implicit commands; call shutdown first if needed.
    pub fn into_transport(self) -> T {
        self.reader.into_transport()
    }
}

/// A host-derived gesture, emitted on release after a known press.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialGesture {
    /// Press duration was below the long-press threshold.
    ShortPress {
        /// Duration measured using caller-supplied monotonic timestamps.
        held: Duration,
    },
    /// Press duration met the host long-press threshold.
    LongPress {
        /// Duration measured using caller-supplied monotonic timestamps.
        held: Duration,
    },
}

/// Optional host gesture policy matching the app's 600 ms threshold.
///
/// Hardware reports contain button state, not these gestures. Reset after a
/// transport failure, reconnect, or suspension; an unmatched release emits none.
#[derive(Clone, Debug, Default)]
pub struct DialGestureTracker {
    pressed_at: Option<Duration>,
    last_time: Option<Duration>,
}

impl DialGestureTracker {
    /// Threshold observed in the official application.
    pub const LONG_PRESS_THRESHOLD: Duration = Duration::from_millis(600);

    /// Starts with no held button or prior timestamp.
    pub fn new() -> Self {
        Self::default()
    }

    /// Forgets pending gestures without emitting an artificial action.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Consumes a decoded event at a nondecreasing monotonic timestamp.
    ///
    /// Repeated presses preserve the first timestamp. Backward timestamps fail
    /// without modifying state. Rotations do not complete or cancel a press.
    pub fn observe(&mut self, event: M131EEvent, now: Duration) -> HidResult<Option<DialGesture>> {
        if self.last_time.is_some_and(|last| now < last) {
            return Err(error("Dial gesture timestamps must be nondecreasing"));
        }
        self.last_time = Some(now);
        match event {
            M131EEvent::Pressed => {
                self.pressed_at.get_or_insert(now);
                Ok(None)
            }
            M131EEvent::Released => Ok(self.pressed_at.take().map(|pressed_at| {
                let held = now - pressed_at;
                if held >= Self::LONG_PRESS_THRESHOLD {
                    DialGesture::LongPress { held }
                } else {
                    DialGesture::ShortPress { held }
                }
            })),
            M131EEvent::Rotated(_) => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        cell::{Cell, RefCell},
        collections::VecDeque,
    };

    #[derive(Default)]
    struct Fake {
        modes: RefCell<Vec<bool>>,
        fail: Cell<bool>,
        reads: RefCell<VecDeque<HidResult<Vec<u8>>>>,
    }
    impl ReportTransport for Fake {
        fn read_timeout(&self, buffer: &mut [u8], _: i32) -> HidResult<usize> {
            let bytes = self.reads.borrow_mut().pop_front().unwrap_or(Ok(vec![]))?;
            buffer[..bytes.len()].copy_from_slice(&bytes);
            Ok(bytes.len())
        }
        fn get_feature_report(&self, _: &mut [u8]) -> HidResult<usize> {
            panic!("Lifecycle must not make implicit firmware queries")
        }
        fn send_feature_report(&self, bytes: &[u8]) -> HidResult<()> {
            assert_eq!(&bytes[..2], &[3, 0x49]);
            self.modes.borrow_mut().push(bytes[2] == 1);
            if self.fail.get() {
                Err(error("Disconnected"))
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn neutral_construction_and_explicit_idempotent_cleanup() {
        let mut session = DialSession::new(Fake::default());
        assert!(session.transport().modes.borrow().is_empty());
        assert!(session.read_input(10).is_err());
        session.activate().unwrap();
        session.activate().unwrap();
        session.shutdown().unwrap();
        session.shutdown().unwrap();
        assert_eq!(*session.transport().modes.borrow(), [true, false]);
    }

    #[test]
    fn resume_waits_without_io_and_does_not_extend_on_duplicate_notification() {
        let mut session = DialSession::new(Fake::default());
        session.activate().unwrap();
        session.suspend().unwrap();
        session.resume_at(Duration::from_secs(10)).unwrap();
        session.resume_at(Duration::from_secs(11)).unwrap();
        assert!(!session.advance(Duration::from_millis(11_999)).unwrap());
        assert_eq!(*session.transport().modes.borrow(), [true, false]);
        assert!(session.advance(Duration::from_secs(12)).unwrap());
        assert!(!session.advance(Duration::from_secs(13)).unwrap());
        assert_eq!(*session.transport().modes.borrow(), [true, false, true]);
    }

    #[test]
    fn failures_leave_no_stale_active_or_pending_state() {
        let mut session = DialSession::new(Fake::default());
        session.activate().unwrap();
        session.suspend().unwrap();
        session.resume_at(Duration::ZERO).unwrap();
        session.transport().fail.set(true);
        assert!(session.advance(Duration::from_secs(2)).is_err());
        assert_eq!(session.state(), DialSessionState::Inactive);
        assert!(session.read_input(10).is_err());
        session.transport().fail.set(false);
        session.activate().unwrap();
        session.transport().fail.set(true);
        assert!(session.shutdown().is_err());
        assert_eq!(session.state(), DialSessionState::Inactive);
    }

    #[test]
    fn input_failure_resets_button_state_and_malformed_input_does_not() {
        let mut session = DialSession::new(Fake::default());
        session.activate().unwrap();
        session.transport().reads.borrow_mut().extend([
            Ok(vec![1, 0x44, 3, 0, 4, 0, 1]),
            Ok(vec![1, 0x44, 3, 0, 4]),
            Ok(vec![1, 0x44, 3, 0, 4, 0, 1]),
            Err(error("Unplugged")),
        ]);
        assert_eq!(
            session.read_input(10).unwrap(),
            Some(DecodedM131E::Events(vec![M131EEvent::Pressed]))
        );
        assert!(session.read_input(10).is_err());
        assert_eq!(session.state(), DialSessionState::Active);
        assert_eq!(
            session.read_input(10).unwrap(),
            Some(DecodedM131E::Events(vec![]))
        );
        assert!(session.read_input(10).is_err());
        assert_eq!(session.state(), DialSessionState::Inactive);
        session.activate().unwrap();
        session
            .transport()
            .reads
            .borrow_mut()
            .push_back(Ok(vec![1, 0x44, 3, 0, 4, 0, 1]));
        assert_eq!(
            session.read_input(10).unwrap(),
            Some(DecodedM131E::Events(vec![M131EEvent::Pressed]))
        );
    }

    #[test]
    fn gestures_handle_threshold_duplicates_backwards_time_and_reset() {
        let mut tracker = DialGestureTracker::new();
        assert_eq!(
            tracker
                .observe(M131EEvent::Released, Duration::ZERO)
                .unwrap(),
            None
        );
        tracker
            .observe(M131EEvent::Pressed, Duration::from_millis(1))
            .unwrap();
        tracker
            .observe(M131EEvent::Pressed, Duration::from_millis(100))
            .unwrap();
        assert!(
            tracker
                .observe(M131EEvent::Released, Duration::ZERO)
                .is_err()
        );
        assert_eq!(
            tracker
                .observe(M131EEvent::Released, Duration::from_millis(601))
                .unwrap(),
            Some(DialGesture::LongPress {
                held: Duration::from_millis(600)
            })
        );
        tracker
            .observe(M131EEvent::Pressed, Duration::from_millis(700))
            .unwrap();
        assert_eq!(
            tracker
                .observe(M131EEvent::Released, Duration::from_millis(1299))
                .unwrap(),
            Some(DialGesture::ShortPress {
                held: Duration::from_millis(599)
            })
        );
        tracker
            .observe(M131EEvent::Pressed, Duration::from_millis(1300))
            .unwrap();
        tracker.reset();
        assert_eq!(
            tracker
                .observe(M131EEvent::Released, Duration::ZERO)
                .unwrap(),
            None
        );
    }
}
