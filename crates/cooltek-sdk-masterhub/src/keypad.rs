//! Explicit M4315 keypad commands recovered from native SDK 03.02.00.
//!
//! Commands are software-derived and hardware-unverified. They do not open,
//! initialize, reconnect, or implicitly configure a device. Source: the pinned
//! official MasterHUB 1.0.0.113 DLL's `Set_Sw_Mode_CM` implementation at RVA 5070.
//! Its installed and PE original filename is `CoolTEK_SDK_CM_M4315_V03.02.00.dll`;
//! the MSI/CAB file identifier misleadingly contains `M4315_V02.00.00`.

use hidapi::HidResult;

use crate::ReportTransport;

/// Complete software-mode feature length, including its report ID.
pub const FEATURE_REPORT_LENGTH: usize = 32;

/// Builds the explicit command to enter (`true`) or leave (`false`) software mode.
///
/// The native setter sends `03 3a <bool>` in a 32-byte feature report. Both
/// enabled and disabled calls also appear in the app's resume/suspend paths.
pub fn software_mode_report(enabled: bool) -> [u8; FEATURE_REPORT_LENGTH] {
    let mut report = [0; FEATURE_REPORT_LENGTH];
    report[..3].copy_from_slice(&[0x03, 0x3a, u8::from(enabled)]);
    report
}

/// Explicitly changes keypad software mode through a physical or test transport.
///
/// Leaving software mode restores the native mode bit; no assumption is made
/// about the device's display, stored actions, or firmware configuration.
///
/// # Errors
/// Propagates a feature-report transport failure without retrying.
pub fn set_software_mode<T: ReportTransport + ?Sized>(
    transport: &T,
    enabled: bool,
) -> HidResult<()> {
    transport.send_feature_report(&software_mode_report(enabled))
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use hidapi::HidError;

    use super::*;

    #[derive(Default)]
    struct Fake {
        reports: RefCell<Vec<Vec<u8>>>,
        fail: bool,
    }

    impl ReportTransport for Fake {
        fn read_timeout(&self, _: &mut [u8], _: i32) -> HidResult<usize> {
            panic!("unexpected read")
        }
        fn get_feature_report(&self, _: &mut [u8]) -> HidResult<usize> {
            panic!("unexpected query")
        }
        fn send_feature_report(&self, data: &[u8]) -> HidResult<()> {
            self.reports.borrow_mut().push(data.to_vec());
            if self.fail {
                Err(HidError::HidApiError {
                    message: "disconnected".into(),
                })
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn explicit_mode_reports_have_native_header_length_and_padding() {
        let fake = Fake::default();
        for enabled in [true, false] {
            set_software_mode(&fake, enabled).unwrap();
        }
        let reports = fake.reports.borrow();
        assert_eq!(reports.len(), 2);
        for (report, enabled) in reports.iter().zip([1, 0]) {
            assert_eq!(report.len(), 32);
            assert_eq!(&report[..3], &[3, 0x3a, enabled]);
            assert!(report[3..].iter().all(|&value| value == 0));
        }
    }

    #[test]
    fn mode_transport_failure_is_not_retried() {
        let fake = Fake {
            fail: true,
            ..Fake::default()
        };
        assert!(set_software_mode(&fake, false).is_err());
        assert_eq!(fake.reports.borrow().len(), 1);
    }
}
