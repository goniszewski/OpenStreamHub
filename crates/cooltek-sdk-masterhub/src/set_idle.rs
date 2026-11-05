//! Best-effort HID SET_IDLE for the M032LE roller module.
//!
//! The roller's firmware sends no input reports until it receives the HID class request
//! SET_IDLE(0). Linux and Windows send it when they bind any HID device; macOS only does so
//! for keyboards and mice. The effect is lost whenever the module resets (replug, base power
//! loss, sleep), so it must be sent every time the roller is opened. Credit: KamilKobylarek,
//! GitHub issue #4. Verified on one unit on 5 November 2026, including recovery after a replug.
//!
//! On macOS the request goes to the `IOUSBHostDevice` without capturing it, so the system HID
//! driver keeps the interface. No root or entitlement is needed. Other platforms already send
//! the request themselves, so [`send_set_idle`] reports them as skipped.

use std::ffi::CStr;

/// `bmRequestType` of SET_IDLE: host to device, class, interface.
pub const SET_IDLE_REQUEST_TYPE: u8 = 0x21;
/// `bRequest` of SET_IDLE.
pub const SET_IDLE_REQUEST: u8 = 0x0A;
/// The full control request: SET_IDLE, duration 0 (report only on change), all report IDs,
/// interface 0, no data stage. Returned as `(bmRequestType, bRequest, wValue, wIndex, wLength)`.
pub const SET_IDLE_SETUP: (u8, u8, u16, u16, u16) =
    (SET_IDLE_REQUEST_TYPE, SET_IDLE_REQUEST, 0, 0, 0);

/// Result of one SET_IDLE attempt. Never an error: a failure must not stop a session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetIdleOutcome {
    /// The module accepted the request.
    Accepted,
    /// The request was attempted and failed.
    Failed(String),
    /// The request was not attempted, with the reason.
    Skipped(String),
}

impl SetIdleOutcome {
    /// Short label for diagnostics: `accepted`, `failed` or `skipped`.
    pub fn status(&self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Failed(_) => "failed",
            Self::Skipped(_) => "skipped",
        }
    }

    /// The failure or skip reason, if any.
    pub fn detail(&self) -> Option<&str> {
        match self {
            Self::Accepted => None,
            Self::Failed(reason) | Self::Skipped(reason) => Some(reason),
        }
    }
}

/// Extracts the registry entry ID from a hidapi macOS path (`DevSrvsID:<decimal id>`).
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
fn parse_dev_srvs_id(path: &CStr) -> Option<u64> {
    path.to_str().ok()?.strip_prefix("DevSrvsID:")?.parse().ok()
}

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn cooltek_roller_set_idle(
        registry_id: u64,
        error: *mut std::ffi::c_char,
        error_length: usize,
    ) -> std::ffi::c_int;
}

/// Sends SET_IDLE(0) to the roller behind a hidapi device path. Best effort; never panics.
#[cfg(target_os = "macos")]
pub fn send_set_idle(path: &CStr) -> SetIdleOutcome {
    let Some(registry_id) = parse_dev_srvs_id(path) else {
        return SetIdleOutcome::Skipped("the HID path is not a DevSrvsID path".into());
    };
    let mut buffer = [0 as std::ffi::c_char; 256];
    // SAFETY: the buffer outlives the call and its length is passed; the shim only writes a
    // NUL-terminated message within it and sends one fixed control request.
    let status = unsafe { cooltek_roller_set_idle(registry_id, buffer.as_mut_ptr(), buffer.len()) };
    if status == 0 {
        SetIdleOutcome::Accepted
    } else {
        // SAFETY: the shim always NUL-terminates within the buffer (snprintf).
        let message = unsafe { CStr::from_ptr(buffer.as_ptr()) };
        SetIdleOutcome::Failed(message.to_string_lossy().into_owned())
    }
}

/// Sends SET_IDLE(0) to the roller behind a hidapi device path. Non-macOS stub.
#[cfg(not(target_os = "macos"))]
pub fn send_set_idle(_path: &CStr) -> SetIdleOutcome {
    SetIdleOutcome::Skipped("the operating system sends SET_IDLE itself".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    #[test]
    fn request_bytes_match_the_hid_spec() {
        assert_eq!(SET_IDLE_SETUP, (0x21, 0x0A, 0, 0, 0));
    }

    #[test]
    fn parses_dev_srvs_id_paths() {
        let id = |text: &str| parse_dev_srvs_id(&CString::new(text).unwrap());
        assert_eq!(id("DevSrvsID:4294970373"), Some(4_294_970_373));
        assert_eq!(id("DevSrvsID:0"), Some(0));
        assert_eq!(id("DevSrvsID:"), None);
        assert_eq!(id("DevSrvsID:12ab"), None);
        assert_eq!(id("DevSrvsID:-1"), None);
        assert_eq!(id("/dev/hidraw3"), None);
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn other_platforms_skip() {
        let outcome = send_set_idle(&CString::new("/dev/hidraw3").unwrap());
        assert_eq!(outcome.status(), "skipped");
        assert!(outcome.detail().is_some());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn non_devsrvs_paths_are_skipped_on_macos() {
        let outcome = send_set_idle(&CString::new("/dev/hidraw3").unwrap());
        assert_eq!(outcome.status(), "skipped");
    }

    #[test]
    fn outcome_labels() {
        assert_eq!(SetIdleOutcome::Accepted.status(), "accepted");
        assert_eq!(SetIdleOutcome::Accepted.detail(), None);
        assert_eq!(SetIdleOutcome::Failed("x".into()).detail(), Some("x"));
    }
}
