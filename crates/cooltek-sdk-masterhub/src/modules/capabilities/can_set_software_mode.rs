use hidapi::{HidDevice, HidError};

/// Represents the capability to enter software mode on the device.
///
/// In software mode, the host is responsible driving state rather than the onboard
/// microcontroller.
pub trait CanSetSoftwareMode: crate::Module + crate::private::Sealed {
    /// The feature report ID to enter software mode on the device.
    const REPORT_ID: u8 = 0x03;

    /// The command byte to enter software mode on the device.
    const COMMAND: u8;

    /// Attempts to enter software mode on the device.
    ///
    /// This allows the program to drive device state.
    ///
    /// # Parameters
    ///
    /// * `device` - The [`HidDevice`] handle.
    ///
    /// # Errors
    ///
    /// * [`HidError`] when the underlying [`HidDevice`] returns an error.
    fn set_software_mode(device: &HidDevice) -> Result<(), HidError> {
        Self::set_software_mode_with_transport(device)
    }

    /// Builds the software-mode feature report without communicating with hardware.
    fn software_mode_report() -> [u8; 0x20] {
        let mut raw = [0u8; 0x20];
        raw[0] = Self::REPORT_ID;
        raw[1] = Self::COMMAND;
        raw[2] = 0x01; // 0x00 is hardware mode (default), 0x01 is software mode

        raw
    }

    /// Enters software mode through a physical or test transport.
    ///
    /// Before writing, the HID descriptor is consulted (read-only): if it
    /// declares this feature report with a length other than the fixed 32
    /// bytes, an error is returned and nothing is sent. An unreadable
    /// descriptor or undeclared report does not block the command.
    fn set_software_mode_with_transport<T: crate::ReportTransport + ?Sized>(
        transport: &T,
    ) -> Result<(), HidError> {
        let report = Self::software_mode_report();
        crate::transport::check_feature_report_length(transport, Self::REPORT_ID, report.len())?;
        transport.send_feature_report(&report)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use crate::{ReportTransport, modules::M131E};

    use super::*;

    struct FeatureWriter(RefCell<Vec<Vec<u8>>>, Option<Vec<u8>>);

    impl ReportTransport for FeatureWriter {
        fn read_timeout(&self, _: &mut [u8], _: i32) -> Result<usize, HidError> {
            panic!("software-mode command must not read input")
        }

        fn get_feature_report(&self, _: &mut [u8]) -> Result<usize, HidError> {
            panic!("software-mode command must not fetch a feature report")
        }

        fn send_feature_report(&self, data: &[u8]) -> Result<(), HidError> {
            self.0.borrow_mut().push(data.to_vec());
            Ok(())
        }

        fn get_report_descriptor(&self, buffer: &mut [u8]) -> Result<usize, HidError> {
            let descriptor = self.1.as_ref().ok_or_else(|| HidError::HidApiError {
                message: "no descriptor".into(),
            })?;
            buffer[..descriptor.len()].copy_from_slice(descriptor);
            Ok(descriptor.len())
        }
    }

    fn feature_descriptor(payload_bytes: u8) -> Vec<u8> {
        vec![0x75, 0x08, 0x85, 0x03, 0x95, payload_bytes, 0xb1, 0x02]
    }

    #[test]
    fn mismatched_declared_feature_length_is_rejected_before_writing() {
        let transport = FeatureWriter(RefCell::new(Vec::new()), Some(feature_descriptor(63)));
        let error = M131E::set_software_mode_with_transport(&transport)
            .unwrap_err()
            .to_string();
        assert!(error.contains("declared as 64 bytes"), "{error}");
        assert!(transport.0.borrow().is_empty());

        let transport = FeatureWriter(RefCell::new(Vec::new()), Some(feature_descriptor(31)));
        M131E::set_software_mode_with_transport(&transport).unwrap();
        assert_eq!(transport.0.borrow().len(), 1);
    }

    #[test]
    fn dial_initialization_is_exactly_one_padded_feature_report() {
        let transport = FeatureWriter(RefCell::new(Vec::new()), None);
        M131E::set_software_mode_with_transport(&transport).unwrap();
        let reports = transport.0.borrow();
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].len(), 32);
        assert_eq!(&reports[0][..3], &[0x03, 0x49, 0x01]);
        assert!(reports[0][3..].iter().all(|&byte| byte == 0));
    }
}
