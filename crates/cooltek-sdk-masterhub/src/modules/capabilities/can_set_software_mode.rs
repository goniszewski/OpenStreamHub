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
        let mut raw = [0u8; 0x20];
        raw[0] = Self::REPORT_ID;
        raw[1] = Self::COMMAND;
        raw[2] = 0x01; // 0x00 is hardware mode (default), 0x01 is software mode

        device.send_feature_report(&raw)?;

        Ok(())
    }
}
