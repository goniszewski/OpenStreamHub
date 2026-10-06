use std::{fmt, time::Duration};

use hidapi::{HidError, HidResult};

use crate::{
    InputReader, ReportTransport,
    input::{DecodedM131E, M131EDecoder, M131EEvent, M131EInputError},
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

/// Failure from [`DialSession::read_input`], separating transport and decode errors.
///
/// Only [`Self::Transport`] clears the session's lifecycle and button state.
/// Malformed reports preserve state so the caller can continue reading.
#[derive(Debug)]
#[non_exhaustive]
pub enum DialReadError {
    /// The session is not [`DialSessionState::Active`]; no read was attempted.
    Inactive,
    /// The timeout was zero or negative; no read was attempted.
    InvalidTimeout,
    /// The HID read failed; the session is now [`DialSessionState::Inactive`].
    Transport(HidError),
    /// A report was received but its known M131E framing was malformed.
    Malformed(M131EInputError),
}

impl fmt::Display for DialReadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Inactive => formatter.write_str("Dial input requires an active session"),
            Self::InvalidTimeout => formatter.write_str("Input timeout must be positive"),
            Self::Transport(error) => write!(formatter, "Dial input transport failed: {error}"),
            Self::Malformed(error) => write!(formatter, "Malformed dial input: {error}"),
        }
    }
}

impl std::error::Error for DialReadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Transport(error) => Some(error),
            Self::Malformed(error) => Some(error),
            Self::Inactive | Self::InvalidTimeout => None,
        }
    }
}

/// Compatibility path for callers that still propagate [`HidError`].
///
/// Transport errors are returned unchanged; other variants become
/// [`HidError::HidApiError`] carrying the [`fmt::Display`] message.
impl From<DialReadError> for HidError {
    fn from(error: DialReadError) -> Self {
        match error {
            DialReadError::Transport(error) => error,
            other => HidError::HidApiError {
                message: other.to_string(),
            },
        }
    }
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
    /// Whether the device may be in software mode, so shutdown must restore
    /// hardware mode. True whenever the state is Active, after a failed or
    /// interrupted session, and for transports taken from an initialized handle.
    restore_hardware_mode: bool,
}

impl<T: ReportTransport> DialSession<T> {
    /// Resume delay observed in the official application, not a firmware requirement.
    pub const RESUME_DELAY: Duration = Duration::from_secs(2);

    /// Takes ownership of a transport without reading or initializing it.
    ///
    /// The session assumes the device is in hardware mode; [`Self::shutdown`]
    /// sends nothing unless the session was activated.
    pub fn new(transport: T) -> Self {
        Self {
            reader: InputReader::new(transport),
            decoder: M131EDecoder::new(),
            state: DialSessionState::Inactive,
            restore_hardware_mode: false,
        }
    }

    /// Takes ownership of a transport that may already be in software mode.
    ///
    /// Performs no I/O. The state starts [`DialSessionState::Inactive`], but
    /// [`Self::shutdown`] restores hardware mode unless the session is suspended.
    pub(crate) fn from_possibly_software_mode(transport: T) -> Self {
        Self {
            restore_hardware_mode: true,
            ..Self::new(transport)
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
    ///
    /// # Errors
    /// Fails without I/O while [`DialSessionState::Suspended`] or
    /// [`DialSessionState::ResumePending`]: resumption must go through
    /// [`Self::resume_at`] and [`Self::advance`] so the resume delay is honored.
    /// Call [`Self::shutdown`] first to start over without the delay.
    pub fn activate(&mut self) -> HidResult<()> {
        match self.state {
            DialSessionState::Active => Ok(()),
            DialSessionState::Inactive => self.enter_software_mode(),
            DialSessionState::Suspended | DialSessionState::ResumePending { .. } => Err(error(
                "Suspended dial session must resume through resume_at and advance",
            )),
        }
    }

    fn enter_software_mode(&mut self) -> HidResult<()> {
        self.decoder.reset();
        self.state = DialSessionState::Inactive;
        // A failed write may still have reached the device.
        self.restore_hardware_mode = true;
        crate::dial::set_software_mode(self.reader.transport(), true)?;
        self.state = DialSessionState::Active;
        Ok(())
    }

    /// Requests hardware mode and clears button state before suspension.
    ///
    /// From [`DialSessionState::ResumePending`] this cancels the pending
    /// resume without I/O; hardware mode was already requested. From
    /// [`DialSessionState::Inactive`] it succeeds only when the device may be in
    /// software mode (e.g. after a failed session or a handle initialized by
    /// `connect()`), in which case hardware mode is requested first.
    pub fn suspend(&mut self) -> HidResult<()> {
        match self.state {
            DialSessionState::Inactive if !self.restore_hardware_mode => {
                return Err(error("Activate the dial session before suspending"));
            }
            DialSessionState::Inactive | DialSessionState::Active => {
                self.decoder.reset();
                self.state = DialSessionState::Inactive;
                crate::dial::set_software_mode(self.reader.transport(), false)?;
                self.restore_hardware_mode = false;
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
            self.enter_software_mode()?;
            return Ok(true);
        }
        Ok(false)
    }

    /// Reads and decodes one bounded input update only while active.
    ///
    /// Idle returns `None`. Transport failures clear lifecycle/button state;
    /// malformed or unknown reports preserve state. Action consumers must reset
    /// any held actions and gesture trackers when the session becomes inactive.
    ///
    /// # Errors
    /// See [`DialReadError`]; it converts into [`HidError`] for `?` callers.
    pub fn read_input(
        &mut self,
        timeout_ms: i32,
    ) -> Result<Option<DecodedM131E<'_>>, DialReadError> {
        if self.state != DialSessionState::Active {
            return Err(DialReadError::Inactive);
        }
        if timeout_ms <= 0 {
            return Err(DialReadError::InvalidTimeout);
        }
        let raw = match self.reader.read_input(timeout_ms) {
            Ok(Some(raw)) => raw,
            Ok(None) => return Ok(None),
            Err(error) => {
                self.decoder.reset();
                self.state = DialSessionState::Inactive;
                return Err(DialReadError::Transport(error));
            }
        };
        self.decoder
            .decode_report(raw)
            .map(Some)
            .map_err(DialReadError::Malformed)
    }

    /// Restores hardware mode when needed, then clears state even on failure.
    ///
    /// Hardware mode is requested unless it is already known to be restored:
    /// no command is sent when suspended, resume-pending, or after a previous
    /// successful shutdown. A failed restore keeps the restore obligation, so a
    /// later shutdown retries it. This covers a session created from a handle initialized into
    /// software mode by `connect()`, even if it was never activated. Call before
    /// releasing the transport; cleanup does not run implicitly on drop.
    pub fn shutdown(&mut self) -> HidResult<()> {
        self.state = DialSessionState::Inactive;
        self.decoder.reset();
        if self.restore_hardware_mode {
            crate::dial::set_software_mode(self.reader.transport(), false)?;
            self.restore_hardware_mode = false;
        }
        Ok(())
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
    fn initialized_handle_session_restores_hardware_mode_on_shutdown_without_activation() {
        // Mirrors ModuleHandle::<M131E>::connect() -> into_dial_session() -> shutdown().
        let mut session = DialSession::from_possibly_software_mode(Fake::default());
        assert_eq!(session.state(), DialSessionState::Inactive);
        assert!(session.transport().modes.borrow().is_empty());
        session.shutdown().unwrap();
        session.shutdown().unwrap();
        assert_eq!(*session.transport().modes.borrow(), [false]);

        let mut session = DialSession::from_possibly_software_mode(Fake::default());
        session.activate().unwrap();
        session.shutdown().unwrap();
        assert_eq!(*session.transport().modes.borrow(), [true, false]);

        let mut session = DialSession::from_possibly_software_mode(Fake::default());
        session.activate().unwrap();
        session.suspend().unwrap();
        session.shutdown().unwrap();
        assert_eq!(*session.transport().modes.borrow(), [true, false]);
    }

    #[test]
    fn transport_failure_still_attempts_hardware_restore_on_shutdown() {
        let mut session = DialSession::new(Fake::default());
        session.activate().unwrap();
        session
            .transport()
            .reads
            .borrow_mut()
            .push_back(Err(error("Unplugged")));
        assert!(matches!(
            session.read_input(10),
            Err(DialReadError::Transport(_))
        ));
        assert_eq!(session.state(), DialSessionState::Inactive);
        session.shutdown().unwrap();
        assert_eq!(*session.transport().modes.borrow(), [true, false]);
    }

    #[test]
    fn failed_shutdown_keeps_restore_obligation_for_retry() {
        let mut session = DialSession::new(Fake::default());
        session.activate().unwrap();
        session.transport().fail.set(true);
        assert!(session.shutdown().is_err());
        assert_eq!(session.state(), DialSessionState::Inactive);
        session.transport().fail.set(false);
        session.shutdown().unwrap();
        session.shutdown().unwrap();
        assert_eq!(*session.transport().modes.borrow(), [true, false, false]);
    }

    #[test]
    fn suspend_from_inactive_restores_hardware_mode_only_when_needed() {
        let mut session = DialSession::new(Fake::default());
        assert!(session.suspend().is_err());
        assert!(session.transport().modes.borrow().is_empty());

        let mut session = DialSession::from_possibly_software_mode(Fake::default());
        session.suspend().unwrap();
        assert_eq!(session.state(), DialSessionState::Suspended);
        assert_eq!(*session.transport().modes.borrow(), [false]);
        // Hardware mode is restored, so shutdown sends nothing further.
        session.shutdown().unwrap();
        assert_eq!(*session.transport().modes.borrow(), [false]);

        let mut session = DialSession::from_possibly_software_mode(Fake::default());
        session.transport().fail.set(true);
        assert!(session.suspend().is_err());
        assert_eq!(session.state(), DialSessionState::Inactive);
        session.transport().fail.set(false);
        session.shutdown().unwrap();
        assert_eq!(*session.transport().modes.borrow(), [false, false]);
    }

    #[test]
    fn suspend_from_resume_pending_cancels_resume_without_io() {
        let mut session = DialSession::new(Fake::default());
        session.activate().unwrap();
        session.suspend().unwrap();
        session.resume_at(Duration::ZERO).unwrap();
        session.suspend().unwrap();
        assert_eq!(session.state(), DialSessionState::Suspended);
        assert!(!session.advance(Duration::from_secs(5)).unwrap());
        assert_eq!(*session.transport().modes.borrow(), [true, false]);
        session.resume_at(Duration::from_secs(10)).unwrap();
        assert!(session.advance(Duration::from_secs(12)).unwrap());
        assert_eq!(session.state(), DialSessionState::Active);
        assert_eq!(*session.transport().modes.borrow(), [true, false, true]);
    }

    #[test]
    fn activate_rejects_suspended_and_resume_pending_without_bypassing_delay() {
        let mut session = DialSession::new(Fake::default());
        session.activate().unwrap();
        session.suspend().unwrap();
        assert!(session.activate().is_err());
        assert_eq!(session.state(), DialSessionState::Suspended);
        session.resume_at(Duration::ZERO).unwrap();
        assert!(session.activate().is_err());
        assert_eq!(
            session.state(),
            DialSessionState::ResumePending {
                ready_at: DialSession::<Fake>::RESUME_DELAY
            }
        );
        assert_eq!(*session.transport().modes.borrow(), [true, false]);
        session.shutdown().unwrap();
        assert_eq!(*session.transport().modes.borrow(), [true, false]);
        session.activate().unwrap();
        assert_eq!(*session.transport().modes.borrow(), [true, false, true]);
    }

    #[test]
    fn read_errors_are_typed_and_convert_to_hid_errors() {
        let mut session = DialSession::new(Fake::default());
        assert!(matches!(
            session.read_input(10),
            Err(DialReadError::Inactive)
        ));
        session.activate().unwrap();
        assert!(matches!(
            session.read_input(0),
            Err(DialReadError::InvalidTimeout)
        ));
        session
            .transport()
            .reads
            .borrow_mut()
            .push_back(Ok(vec![1, 0x44, 3, 0, 4]));
        let malformed = session.read_input(10).unwrap_err();
        assert!(matches!(
            malformed,
            DialReadError::Malformed(M131EInputError::TruncatedPayload { .. })
        ));
        assert_eq!(session.state(), DialSessionState::Active);
        let converted = HidError::from(malformed);
        assert!(converted.to_string().contains("Malformed dial input"));
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
