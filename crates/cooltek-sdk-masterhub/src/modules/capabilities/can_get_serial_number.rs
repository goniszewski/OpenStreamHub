use hidapi::{HidDevice, HidError};

/// Represents the capability to read the serial number of the device.
pub trait CanGetSerialNumber: crate::Module + crate::private::Sealed {
    /// The feature report ID to read the serial number of the device.
    const REPORT_ID: u8;

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
    /// * [`HidError`] when the underlying [`HidDevice`] returns an error or the response is malformed.
    ///
    /// # Remarks
    ///
    /// The USB HID specification does not require devices to actually implement a serial number.
    /// Therefore, we return an [`Option<String>`] (empty string is coalesced to `None`).
    fn get_serial_number(device: &HidDevice) -> Result<Option<String>, HidError> {
        Self::get_serial_number_with_transport(device)
    }

    /// Reads the serial number through a report transport.
    ///
    /// This is also useful for replaying recorded responses without a physical device.
    /// Only the bytes actually returned by the transport are parsed.
    ///
    /// # Errors
    ///
    /// Returns an error if the transport fails, reports more bytes than the supplied buffer,
    /// or returns a malformed serial-number report.
    fn get_serial_number_with_transport<T: crate::ReportTransport + ?Sized>(
        transport: &T,
    ) -> Result<Option<String>, HidError> {
        let mut raw = [0u8; 0x20];
        raw[0] = Self::REPORT_ID;

        // Our raw buffer is overwritten with the response.
        let received = transport.get_feature_report(&mut raw)?;
        let report = raw.get(..received).ok_or_else(|| HidError::HidApiError {
            message: "Serial-number transport returned more than 32 bytes".into(),
        })?;
        Self::parse_serial_number_report(report)
    }

    /// Parses a serial-number feature report, including its report ID and payload length.
    ///
    /// A report must contain its two-byte header, use this module's [`Self::REPORT_ID`],
    /// and fit the 32-byte report buffer. Its declared payload must fit both that buffer
    /// and the received bytes. Bytes after the declared payload are ignored.
    ///
    /// Invalid UTF-8 is replaced lossily, preserving the existing SDK behavior. Leading
    /// and trailing NUL padding is removed for C-style responses; embedded NULs are
    /// preserved. An empty serial number is returned as `None`.
    ///
    /// # Errors
    ///
    /// Returns an error if the header, report ID, or payload length is invalid.
    fn parse_serial_number_report(raw: &[u8]) -> Result<Option<String>, HidError> {
        const REPORT_SIZE: usize = 0x20;
        const HEADER_SIZE: usize = 2;

        let invalid_report = |message: &str| HidError::HidApiError {
            message: message.into(),
        };
        if raw.len() < HEADER_SIZE {
            return Err(invalid_report("Serial-number report is missing its header"));
        }
        if raw.len() > REPORT_SIZE {
            return Err(invalid_report("Serial-number report exceeds 32 bytes"));
        }
        if raw[0] != Self::REPORT_ID {
            return Err(invalid_report(
                "Serial-number report has an unexpected report ID",
            ));
        }

        let payload_len = usize::from(raw[1]);
        if payload_len > REPORT_SIZE - HEADER_SIZE {
            return Err(invalid_report("Serial-number payload exceeds 30 bytes"));
        }
        let payload = raw
            .get(HEADER_SIZE..HEADER_SIZE + payload_len)
            .ok_or_else(|| invalid_report("Serial-number payload is truncated"))?;
        let raw_serial_number = String::from_utf8_lossy(payload);
        let serial_number = raw_serial_number.trim_matches('\0');

        Ok((!serial_number.is_empty()).then(|| serial_number.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::CanGetSerialNumber;
    use crate::{ReportTransport, modules::M003K};
    use hidapi::{HidError, HidResult};

    fn parse(raw: &[u8]) -> Result<Option<String>, HidError> {
        M003K::parse_serial_number_report(raw)
    }

    #[test]
    fn rejects_short_headers_and_wrong_report_id() {
        for raw in [
            &[][..],
            &[M003K::REPORT_ID][..],
            &[0, 0][..],
            &[0x31, 0][..],
        ] {
            assert!(parse(raw).is_err(), "accepted {raw:?}");
        }
    }

    #[test]
    fn rejects_truncated_and_oversized_payloads() {
        assert!(parse(&[M003K::REPORT_ID, 1]).is_err());
        assert!(parse(&[M003K::REPORT_ID, 3, b'A', b'B']).is_err());
        for payload_len in [31, 32, 255] {
            let mut raw = [0; 32];
            raw[0] = M003K::REPORT_ID;
            raw[1] = payload_len;
            assert!(parse(&raw).is_err());
        }
        let mut raw = [0; 33];
        raw[0] = M003K::REPORT_ID;
        assert!(parse(&raw).is_err());
    }

    #[test]
    fn coalesces_empty_and_null_padded_serials() {
        assert_eq!(parse(&[M003K::REPORT_ID, 0]).unwrap(), None);
        assert_eq!(parse(&[M003K::REPORT_ID, 3, 0, 0, 0]).unwrap(), None);
        assert_eq!(
            parse(&[M003K::REPORT_ID, 5, 0, b'A', b'B', 0, 0]).unwrap(),
            Some("AB".into())
        );
    }

    #[test]
    fn accepts_valid_and_maximum_serials_and_ignores_padding() {
        assert_eq!(
            parse(&[M003K::REPORT_ID, 3, b'A', b'B', b'C', b'X']).unwrap(),
            Some("ABC".into())
        );
        let mut raw = [b'A'; 32];
        raw[0] = M003K::REPORT_ID;
        raw[1] = 30;
        assert_eq!(parse(&raw).unwrap(), Some("A".repeat(30)));
    }

    #[test]
    fn preserves_lossy_utf8_and_embedded_null_behavior() {
        assert_eq!(
            parse(&[M003K::REPORT_ID, 3, b'A', 0, 0xff]).unwrap(),
            Some("A\0\u{fffd}".into())
        );
    }

    #[test]
    fn every_declared_length_is_safe_for_short_and_full_reports() {
        for received in 0..=32 {
            for declared in 0..=u8::MAX {
                let mut raw = [b'A'; 32];
                raw[0] = M003K::REPORT_ID;
                raw[1] = declared;
                let result = parse(&raw[..received]);
                let is_valid = received >= 2
                    && usize::from(declared) <= 30
                    && usize::from(declared) <= received - 2;
                assert_eq!(
                    result.is_ok(),
                    is_valid,
                    "received={received}, declared={declared}"
                );
            }
        }
    }

    struct FakeTransport {
        response: Vec<u8>,
        received: usize,
    }

    impl ReportTransport for FakeTransport {
        fn read_timeout(&self, _buf: &mut [u8], _timeout_ms: i32) -> HidResult<usize> {
            unreachable!("serial queries use feature reports")
        }

        fn get_feature_report(&self, buf: &mut [u8]) -> HidResult<usize> {
            assert_eq!(buf.len(), 32);
            assert_eq!(buf[0], M003K::REPORT_ID);
            buf[..self.response.len()].copy_from_slice(&self.response);
            Ok(self.received)
        }

        fn send_feature_report(&self, _data: &[u8]) -> HidResult<()> {
            unreachable!("serial queries do not send feature reports")
        }
    }

    #[test]
    fn parses_only_the_transport_received_length() {
        let transport = FakeTransport {
            response: vec![M003K::REPORT_ID, 3, b'A', b'B', b'C'],
            received: 4,
        };
        assert!(M003K::get_serial_number_with_transport(&transport).is_err());
        let transport = FakeTransport {
            received: 5,
            ..transport
        };
        assert_eq!(
            M003K::get_serial_number_with_transport(&transport).unwrap(),
            Some("ABC".into())
        );
    }

    #[test]
    fn rejects_impossible_transport_length() {
        let transport = FakeTransport {
            response: vec![M003K::REPORT_ID, 0],
            received: 33,
        };
        assert!(M003K::get_serial_number_with_transport(&transport).is_err());
    }
}
