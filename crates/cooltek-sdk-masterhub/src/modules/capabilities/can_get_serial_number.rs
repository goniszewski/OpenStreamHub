use hidapi::{HidDevice, HidError};

/// Represents the capability to read the serial number of the device.
pub trait CanGetSerialNumber: crate::Module + crate::private::Sealed {
    /// The feature report ID to read the serial number of the device.
    const REPORT_ID: u8 = 0x30;

    /// Reads the serial number of the device.
    ///
    /// # Parameters
    ///
    /// * `device` - The [`HidDevice`] handle.
    ///
    /// # Returns
    ///
    /// A [`Result`] containing either an [`Option<String>`] or a [`HidError`].
    ///
    /// # Errors
    ///
    /// * [`HidError`] when the underlying [`HidDevice`] returns an error.
    ///
    /// # Remarks
    ///
    /// The USB HID specification does not require devices to actually implement a serial number.
    /// Therefore, we return an [`Option<String>`] (empty string is coalesced to `None`).
    fn get_serial_number(device: &HidDevice) -> Result<Option<String>, HidError> {
        let mut raw = [0u8; 0x20];
        raw[0] = Self::REPORT_ID;

        // Our raw buffer is overwritten with the response.
        let _ = device.get_feature_report(&mut raw)?;

        // The second byte is the report size (or length).
        Ok(match raw[1] as usize {
            0 => None,
            len => {
                // The report data starts at index 2, after the report ID and report size.
                let bytes = &raw[2..2 + len];

                // We want to coalesce an empty string to `None`. The conversion from raw UTF-8
                // bytes results in a `Cow`, which means we might not have allocated a string if
                // the bytes are already valid UTF-8. Since the Cooler Master device might return a
                // C-style string, we also need to remove the the null byte ('\0') at the end.
                //
                // To avoid allocating a string unnecessarily, we want to check the length of the
                // `Cow` to see if its `0` (i.e. empty). However, the `Cow` would be freed if we
                // chain `trim_matches` immediately, so we have to bind it to a `let` statement
                // first to prevent that.
                let raw_serial_number = String::from_utf8_lossy(bytes);
                let serial_number = raw_serial_number.trim_matches('\0');

                match serial_number.len() {
                    0 => None,
                    _ => Some(serial_number.to_string()),
                }
            }
        })
    }
}
