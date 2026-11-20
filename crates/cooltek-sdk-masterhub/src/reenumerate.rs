//! A software replug: re-enumerating the USB device of one module.
//!
//! An M4315 keypad or M131E dial that was sent a feature read while in hardware mode ignores every later
//! software-mode command until it is unplugged or loses power. On macOS, re-enumerating its USB device resets it the same way: measured on 19
//! November 2026 (docs/masterhub-hardware-results.md), the module dropped out, came back on a new HID path about
//! 0.2 s later, and took software mode on the first try. Only that module drops out. No root or entitlement is
//! needed. Other platforms report [`reenumerate`] as unavailable.

use std::ffi::CStr;

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn cooltek_reenumerate_usb_device(
        registry_id: u64,
        error: *mut std::ffi::c_char,
        error_length: usize,
    ) -> std::ffi::c_int;
}

/// Re-enumerates the USB device behind a hidapi device path (`DevSrvsID:<registry id>` on macOS). Returns once the
/// request was made; the module reappears on a new path shortly after. Never panics.
///
/// # Errors
/// A path that is not a macOS registry path, a device that cannot be found, opened or re-enumerated, or another
/// platform.
#[cfg(target_os = "macos")]
pub fn reenumerate(path: &CStr) -> Result<(), String> {
    let Some(registry_id) = crate::set_idle::parse_dev_srvs_id(path) else {
        return Err("the HID path is not a DevSrvsID path".into());
    };
    let mut buffer = [0 as std::ffi::c_char; 256];
    // SAFETY: the buffer outlives the call and its length is passed; the shim only writes a NUL-terminated message
    // within it (snprintf) and makes one re-enumeration request.
    let status =
        unsafe { cooltek_reenumerate_usb_device(registry_id, buffer.as_mut_ptr(), buffer.len()) };
    if status == 0 {
        Ok(())
    } else {
        // SAFETY: the shim always NUL-terminates within the buffer.
        let message = unsafe { CStr::from_ptr(buffer.as_ptr()) };
        Err(message.to_string_lossy().into_owned())
    }
}

/// Non-macOS stub: re-enumeration is not available.
///
/// # Errors
/// Always.
#[cfg(not(target_os = "macos"))]
pub fn reenumerate(_path: &CStr) -> Result<(), String> {
    Err(UNAVAILABLE.into())
}

/// The error of [`reenumerate`] on platforms without it.
pub const UNAVAILABLE: &str = "re-enumerating a module is available on macOS only";

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    #[cfg(target_os = "macos")]
    #[test]
    fn a_path_that_is_not_a_registry_path_is_refused_without_any_device_access() {
        let error = reenumerate(&CString::new("/dev/hidraw3").unwrap()).unwrap_err();
        assert!(error.contains("DevSrvsID"), "{error}");
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn other_platforms_report_it_unavailable() {
        assert_eq!(
            reenumerate(&CString::new("/dev/hidraw3").unwrap()).unwrap_err(),
            UNAVAILABLE
        );
    }
}
