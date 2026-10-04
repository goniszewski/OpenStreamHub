//! Explicit M131E dial commands recovered from the official Windows SDK.
//!
//! These operations are software-derived and hardware-unverified. They do not
//! initialize a device, select an interface, implement reconnection, or claim a
//! device acknowledgement. Display report lengths must come from the selected
//! interface's HID descriptor; no fixed output length is assumed.

use hidapi::{HidError, HidResult};

use crate::ReportTransport;

/// Length of the dial's ordinary feature requests, including the report ID.
pub const FEATURE_REPORT_LENGTH: usize = 32;
/// Image width prepared by the current official application.
pub const IMAGE_WIDTH: u16 = 240;
/// Image height prepared by the current official application.
pub const IMAGE_HEIGHT: u16 = 240;
/// Defensive upper limit for an encoded image accepted by this implementation.
///
/// This is an implementation limit, not a recovered hardware limit.
pub const MAX_JPEG_LENGTH: usize = crate::jpeg::MAX_JPEG_LENGTH;

fn invalid(message: impl Into<String>) -> HidError {
    HidError::HidApiError {
        message: message.into(),
    }
}

/// Builds a zero-padded backlight feature report for a value from 0 through 100.
///
/// # Errors
/// Returns an error for a value greater than 100, before any transport operation.
pub fn backlight_report(value: u8) -> HidResult<[u8; FEATURE_REPORT_LENGTH]> {
    if value > 100 {
        return Err(invalid("Dial backlight must be between 0 and 100"));
    }
    let mut report = [0; FEATURE_REPORT_LENGTH];
    report[..3].copy_from_slice(&[0x03, 0x44, value]);
    Ok(report)
}

/// Builds the solid-color command with its three raw channel bytes in SDK order.
///
/// Channel color semantics and physical display output remain unverified.
pub fn color_report(channels: [u8; 3]) -> [u8; FEATURE_REPORT_LENGTH] {
    let mut report = [0; FEATURE_REPORT_LENGTH];
    report[..2].copy_from_slice(&[0x03, 0x46]);
    report[2..5].copy_from_slice(&channels);
    report
}

/// Builds the command to enter (`true`) or leave (`false`) software mode.
pub fn software_mode_report(enabled: bool) -> [u8; FEATURE_REPORT_LENGTH] {
    let mut report = [0; FEATURE_REPORT_LENGTH];
    report[..3].copy_from_slice(&[0x03, 0x49, u8::from(enabled)]);
    report
}

/// Explicitly sets dial backlight through a physical or test transport.
///
/// # Errors
/// Returns an error for an invalid value or a transport failure.
pub fn set_backlight<T: ReportTransport + ?Sized>(transport: &T, value: u8) -> HidResult<()> {
    transport.send_feature_report(&backlight_report(value)?)
}

/// Explicitly sends the dial's raw three-channel solid-color command.
///
/// # Errors
/// Returns a transport failure without performing any other operation.
pub fn set_color<T: ReportTransport + ?Sized>(transport: &T, channels: [u8; 3]) -> HidResult<()> {
    transport.send_feature_report(&color_report(channels))
}

/// Explicitly enters or leaves software mode without extra initialization.
///
/// # Errors
/// Returns a transport failure without performing any other operation.
pub fn set_software_mode<T: ReportTransport + ?Sized>(
    transport: &T,
    enabled: bool,
) -> HidResult<()> {
    transport.send_feature_report(&software_mode_report(enabled))
}

fn feature_body(report: &[u8], expected_id: u8) -> HidResult<&[u8]> {
    if !(2..=FEATURE_REPORT_LENGTH).contains(&report.len()) {
        return Err(invalid(
            "Dial feature reply must contain a header and fit 32 bytes",
        ));
    }
    if report[0] != expected_id {
        return Err(invalid("Dial feature reply has an unexpected report ID"));
    }
    // The native backlight reader loads byte 1 but does not use it, and the
    // firmware reader skips it. Its meaning is unresolved for these queries.
    Ok(&report[2..])
}

fn query_feature<T: ReportTransport + ?Sized>(
    transport: &T,
    report_id: u8,
) -> HidResult<([u8; FEATURE_REPORT_LENGTH], usize)> {
    let mut report = [0; FEATURE_REPORT_LENGTH];
    report[0] = report_id;
    let received = transport.get_feature_report(&mut report)?;
    if received > report.len() {
        return Err(invalid(
            "Dial feature transport returned more than 32 bytes",
        ));
    }
    Ok((report, received))
}

/// Parses the returned portion of a backlight feature report (`4c`).
///
/// The value at offset 2 must fit in the received bytes. Byte 1 has unresolved
/// meaning and is not interpreted as a length. A value outside the setter's
/// 0–100 range is rejected rather than silently clamped.
///
/// # Errors
/// Returns an error for malformed framing, missing payload, or an invalid value.
pub fn parse_backlight_report(report: &[u8]) -> HidResult<u8> {
    let value = *feature_body(report, 0x4c)?
        .first()
        .ok_or_else(|| invalid("Dial backlight reply has no value"))?;
    if value > 100 {
        return Err(invalid("Dial backlight reply is outside the 0–100 range"));
    }
    Ok(value)
}

/// Explicitly queries backlight and parses only bytes actually returned.
///
/// HIDAPI's returned count is preserved; the Windows SDK's added byte is not
/// copied into this transport-independent implementation.
///
/// # Errors
/// Returns an error for a transport failure or malformed backlight response.
pub fn read_backlight<T: ReportTransport + ?Sized>(transport: &T) -> HidResult<u8> {
    let (report, received) = query_feature(transport, 0x4c)?;
    parse_backlight_report(&report[..received])
}

/// Firmware query selectors recovered from `Get_FW_Infos_by_type_CM`.
///
/// Only the ordinary application's selector is named semantically. The meanings
/// of the other two selectors have not been established.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FirmwareInfoKind {
    /// SDK selector 0, feature report ID `47`; meaning unresolved.
    Type0,
    /// SDK selector 1, feature report ID `48`, used by the ordinary dial wrapper.
    Current,
    /// SDK selector 2, feature report ID `49`; meaning unresolved.
    Type2,
}

impl FirmwareInfoKind {
    /// Returns the feature report ID for this selector.
    pub const fn report_id(self) -> u8 {
        match self {
            Self::Type0 => 0x47,
            Self::Current => 0x48,
            Self::Type2 => 0x49,
        }
    }
}

/// Firmware fields copied by the native dial SDK, without inventing semantics.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FirmwareInfo {
    /// Four opaque bytes at reply offsets 2 through 5.
    pub raw_version: [u8; 4],
    /// Eleven bytes at reply offsets 6 through 16, preserved exactly.
    pub version_text: [u8; 11],
}

impl FirmwareInfo {
    /// Decodes text up to the first NUL, replacing invalid UTF-8 lossily.
    ///
    /// The original bytes remain available in [`Self::version_text`].
    pub fn version_string_lossy(&self) -> String {
        let length = self
            .version_text
            .iter()
            .position(|&byte| byte == 0)
            .unwrap_or(self.version_text.len());
        String::from_utf8_lossy(&self.version_text[..length]).into_owned()
    }
}

/// Parses firmware fields after validating the ID and actual received length.
///
/// Byte 1 has unresolved meaning and is not interpreted as a length.
///
/// # Errors
/// Returns an error for malformed framing or fewer than 17 received bytes.
pub fn parse_firmware_info_report(
    report: &[u8],
    kind: FirmwareInfoKind,
) -> HidResult<FirmwareInfo> {
    let payload = feature_body(report, kind.report_id())?;
    if payload.len() < 15 {
        return Err(invalid(
            "Dial firmware reply has fewer than 15 payload bytes",
        ));
    }
    let mut raw_version = [0; 4];
    raw_version.copy_from_slice(&payload[..4]);
    let mut version_text = [0; 11];
    version_text.copy_from_slice(&payload[4..15]);
    Ok(FirmwareInfo {
        raw_version,
        version_text,
    })
}

/// Explicitly fetches firmware fields with the requested query selector.
///
/// # Errors
/// Returns an error for transport failure or malformed/truncated returned bytes.
pub fn read_firmware_info<T: ReportTransport + ?Sized>(
    transport: &T,
    kind: FirmwareInfoKind,
) -> HidResult<FirmwareInfo> {
    let (report, received) = query_feature(transport, kind.report_id())?;
    parse_firmware_info_report(&report[..received], kind)
}

/// Prevalidated iterator over descriptor-sized dial JPEG output reports.
///
/// It accepts the conservative first implementation format: 240×240, 8-bit,
/// three-component baseline JFIF JPEG with one interleaved scan and an EOI marker.
/// Marker lengths and dimensions are checked; entropy-coded image data is not
/// decoded or proved valid. Use a real JPEG encoder to prepare the image.
/// Progressive JPEG, BMP, animation, and image conversion are not implemented.
///
/// Every report is newly zero-filled. Byte 6 and final-chunk padding are set to
/// zero by deliberate policy; the native SDK reuses a buffer and does not assign
/// these fields. Their device-side requirements remain unknown.
#[derive(Clone, Debug)]
pub struct JpegPackets<'a> {
    jpeg: &'a [u8],
    report_length: usize,
    capacity: usize,
    offset: usize,
    sequence: u32,
}

impl<'a> JpegPackets<'a> {
    /// Validates the complete image and output length before constructing packets.
    ///
    /// `output_report_length` includes the report ID byte and must be obtained
    /// from the selected interface's descriptor. This function cannot establish
    /// that a caller-supplied number corresponds to the actual device.
    ///
    /// # Errors
    /// Rejects unsupported JPEG framing/dimensions, an image exceeding
    /// [`MAX_JPEG_LENGTH`], output lengths without payload capacity,
    /// report lengths exceeding `u16`, or more than 65,536 chunks.
    pub fn new(jpeg: &'a [u8], output_report_length: usize) -> HidResult<Self> {
        if output_report_length > usize::from(u16::MAX) {
            return Err(invalid(
                "Dial output report length exceeds the 16-bit HID descriptor length",
            ));
        }
        let capacity = output_report_length
            .checked_sub(8)
            .filter(|&length| length > 0)
            .ok_or_else(|| {
                invalid("Dial output length must contain payload capacity after its 8-byte header")
            })?;
        validate_jpeg(jpeg)?;
        if jpeg.len().div_ceil(capacity) > usize::from(u16::MAX) + 1 {
            return Err(invalid(
                "Dial image would overflow the 16-bit chunk sequence",
            ));
        }
        Ok(Self {
            jpeg,
            report_length: output_report_length,
            capacity,
            offset: 0,
            sequence: 0,
        })
    }

    /// Returns the complete output report length, including its report ID.
    pub fn output_report_length(&self) -> usize {
        self.report_length
    }
}

impl Iterator for JpegPackets<'_> {
    type Item = Vec<u8>;

    fn next(&mut self) -> Option<Self::Item> {
        let remaining = self.jpeg.len() - self.offset;
        if remaining == 0 {
            return None;
        }
        let payload_length = remaining.min(self.capacity);
        let mut report = vec![0; self.report_length];
        report[..2].copy_from_slice(&[0x02, 0x45]);
        report[2..4].copy_from_slice(&(self.sequence as u16).to_le_bytes());
        report[4..6].copy_from_slice(&(payload_length as u16).to_le_bytes());
        report[7] = u8::from(remaining <= self.capacity);
        report[8..8 + payload_length]
            .copy_from_slice(&self.jpeg[self.offset..self.offset + payload_length]);
        self.offset += payload_length;
        self.sequence += 1;
        Some(report)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = (self.jpeg.len() - self.offset).div_ceil(self.capacity);
        (remaining, Some(remaining))
    }
}

impl ExactSizeIterator for JpegPackets<'_> {}
impl std::iter::FusedIterator for JpegPackets<'_> {}

/// Explicitly uploads an encoded JPEG using complete output reports.
///
/// Validates every input before the first write and returns the report count on
/// success. A short write or error stops immediately without retrying or reading
/// an acknowledgement. Earlier reports may have reached the device on failure;
/// the caller must decide when and how to recover. The operation does not enter
/// software mode or adjust backlight.
///
/// # Errors
/// Returns validation errors, transport failures, or incomplete write counts.
pub fn display_jpeg<T: ReportTransport + ?Sized>(
    transport: &T,
    jpeg: &[u8],
    output_report_length: usize,
) -> HidResult<usize> {
    let packets = JpegPackets::new(jpeg, output_report_length)?;
    let count = packets.len();
    for report in packets {
        let written = transport.write_output(&report)?;
        if written != report.len() {
            return Err(invalid(format!(
                "Dial image output write returned {written} bytes for a {}-byte report",
                report.len()
            )));
        }
    }
    Ok(count)
}

fn validate_jpeg(jpeg: &[u8]) -> HidResult<()> {
    crate::jpeg::validate(jpeg, "Dial", IMAGE_WIDTH, IMAGE_HEIGHT)
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, collections::VecDeque};

    use super::*;

    #[derive(Default)]
    struct Fake {
        feature_writes: RefCell<Vec<Vec<u8>>>,
        feature_queries: RefCell<Vec<Vec<u8>>>,
        output_writes: RefCell<Vec<Vec<u8>>>,
        response: Vec<u8>,
        received: usize,
        output_results: RefCell<VecDeque<HidResult<usize>>>,
        fail_features: bool,
    }

    impl ReportTransport for Fake {
        fn read_timeout(&self, _: &mut [u8], _: i32) -> HidResult<usize> {
            panic!("dial commands do not poll input or acknowledgements")
        }

        fn get_feature_report(&self, report: &mut [u8]) -> HidResult<usize> {
            self.feature_queries.borrow_mut().push(report.to_vec());
            if self.fail_features {
                return Err(invalid("disconnected"));
            }
            report[..self.response.len()].copy_from_slice(&self.response);
            Ok(self.received)
        }

        fn send_feature_report(&self, report: &[u8]) -> HidResult<()> {
            self.feature_writes.borrow_mut().push(report.to_vec());
            if self.fail_features {
                return Err(invalid("disconnected"));
            }
            Ok(())
        }

        fn write_output(&self, report: &[u8]) -> HidResult<usize> {
            self.output_writes.borrow_mut().push(report.to_vec());
            self.output_results
                .borrow_mut()
                .pop_front()
                .unwrap_or(Ok(report.len()))
        }
    }

    #[test]
    fn feature_commands_have_exact_fields_and_zero_padding() {
        let fake = Fake::default();
        set_backlight(&fake, 0).unwrap();
        set_backlight(&fake, 100).unwrap();
        set_color(&fake, [0x12, 0x34, 0x56]).unwrap();
        set_software_mode(&fake, true).unwrap();
        set_software_mode(&fake, false).unwrap();
        let writes = fake.feature_writes.borrow();
        let expected = [
            &[3, 0x44, 0][..],
            &[3, 0x44, 100][..],
            &[3, 0x46, 0x12, 0x34, 0x56][..],
            &[3, 0x49, 1][..],
            &[3, 0x49, 0][..],
        ];
        assert_eq!(writes.len(), expected.len());
        for (report, prefix) in writes.iter().zip(expected) {
            assert_eq!(report.len(), 32);
            assert_eq!(&report[..prefix.len()], prefix);
            assert!(report[prefix.len()..].iter().all(|&byte| byte == 0));
        }
        assert!(fake.feature_queries.borrow().is_empty());
        assert!(fake.output_writes.borrow().is_empty());
    }

    #[test]
    fn invalid_backlight_never_writes_and_feature_errors_propagate() {
        let fake = Fake::default();
        for value in 101..=u8::MAX {
            assert!(set_backlight(&fake, value).is_err());
        }
        assert!(fake.feature_writes.borrow().is_empty());
        let fake = Fake {
            fail_features: true,
            ..Fake::default()
        };
        assert!(set_backlight(&fake, 50).is_err());
        assert!(set_color(&fake, [0; 3]).is_err());
        assert!(set_software_mode(&fake, false).is_err());
        assert!(read_backlight(&fake).is_err());
        assert!(read_firmware_info(&fake, FirmwareInfoKind::Current).is_err());
    }

    #[test]
    fn backlight_query_respects_actual_length_and_report_id() {
        let fake = Fake {
            response: vec![0x4c, 1, 73, 200],
            received: 3,
            ..Fake::default()
        };
        assert_eq!(read_backlight(&fake).unwrap(), 73);
        let mut expected = vec![0; 32];
        expected[0] = 0x4c;
        assert_eq!(fake.feature_queries.borrow().as_slice(), &[expected]);
        assert!(fake.feature_writes.borrow().is_empty());
        let short = Fake {
            received: 2,
            ..fake
        };
        assert!(read_backlight(&short).is_err());
        let impossible = Fake {
            received: 33,
            ..short
        };
        assert!(read_backlight(&impossible).is_err());
        for report in [
            &[][..],
            &[0x4c][..],
            &[0x4c, 0][..],
            &[0x48, 1, 30][..],
            &[0x4c, 1, 101][..],
        ] {
            assert!(parse_backlight_report(report).is_err());
        }
        for opaque_byte in 0..=u8::MAX {
            assert_eq!(
                parse_backlight_report(&[0x4c, opaque_byte, 30]).unwrap(),
                30
            );
        }
    }

    fn firmware_reply(kind: FirmwareInfoKind) -> Vec<u8> {
        let mut report = vec![0; 32];
        report[0] = kind.report_id();
        report[1] = 15;
        report[2..6].copy_from_slice(&[9, 8, 7, 6]);
        report[6..17].copy_from_slice(b"1.2.3.4\0XYZ");
        report[17..].fill(0xff);
        report
    }

    #[test]
    fn firmware_query_preserves_opaque_fields_for_all_selectors() {
        for kind in [
            FirmwareInfoKind::Type0,
            FirmwareInfoKind::Current,
            FirmwareInfoKind::Type2,
        ] {
            let fake = Fake {
                response: firmware_reply(kind),
                received: 17,
                ..Fake::default()
            };
            let firmware = read_firmware_info(&fake, kind).unwrap();
            assert_eq!(firmware.raw_version, [9, 8, 7, 6]);
            assert_eq!(firmware.version_text, *b"1.2.3.4\0XYZ");
            assert_eq!(firmware.version_string_lossy(), "1.2.3.4");
            let queries = fake.feature_queries.borrow();
            assert_eq!(queries.len(), 1);
            assert_eq!(queries[0].len(), 32);
            assert_eq!(queries[0][0], kind.report_id());
            assert!(queries[0][1..].iter().all(|&byte| byte == 0));
            assert!(fake.feature_writes.borrow().is_empty());
        }
    }

    #[test]
    fn firmware_parser_bounds_actual_length_and_ignores_opaque_header_byte() {
        let kind = FirmwareInfoKind::Current;
        let mut report = firmware_reply(kind);
        for received in 0..=32 {
            for opaque_byte in 0..=u8::MAX {
                report[1] = opaque_byte;
                let valid = received >= 17;
                assert_eq!(
                    parse_firmware_info_report(&report[..received], kind).is_ok(),
                    valid
                );
            }
        }
        report[1] = 15;
        report[0] = 0x47;
        assert!(parse_firmware_info_report(&report, kind).is_err());
        assert!(parse_firmware_info_report(&[0; 33], kind).is_err());
        let fake = Fake {
            response: firmware_reply(kind),
            received: 16,
            ..Fake::default()
        };
        assert!(read_firmware_info(&fake, kind).is_err());
        let fake = Fake {
            received: 33,
            ..fake
        };
        assert!(read_firmware_info(&fake, kind).is_err());
        let info = FirmwareInfo {
            raw_version: [0; 4],
            version_text: [0xff; 11],
        };
        assert_eq!(info.version_string_lossy(), "\u{fffd}".repeat(11));
    }

    #[test]
    fn generated_jpeg_is_split_into_exact_headers_and_reassembles_losslessly() {
        let jpeg = real_jpeg();
        let mut packets = JpegPackets::new(&jpeg, 72).unwrap();
        assert_eq!(packets.output_report_length(), 72);
        let count = jpeg.len().div_ceil(64);
        assert_eq!(packets.len(), count);
        let reports: Vec<_> = packets.by_ref().collect();
        assert_eq!(packets.len(), 0);
        assert!(packets.next().is_none());
        assert!(packets.next().is_none());
        let mut reconstructed = Vec::new();
        for (sequence, report) in reports.iter().enumerate() {
            assert_eq!(report.len(), 72);
            assert_eq!(&report[..2], &[0x02, 0x45]);
            assert_eq!(u16::from_le_bytes([report[2], report[3]]), sequence as u16);
            let payload_length = usize::from(u16::from_le_bytes([report[4], report[5]]));
            assert_eq!(payload_length, (jpeg.len() - sequence * 64).min(64));
            assert_eq!(report[6], 0);
            assert_eq!(report[7], u8::from(sequence + 1 == count));
            reconstructed.extend_from_slice(&report[8..8 + payload_length]);
            assert!(report[8 + payload_length..].iter().all(|&byte| byte == 0));
        }
        assert_eq!(reconstructed, jpeg);
        let fake = Fake::default();
        assert_eq!(display_jpeg(&fake, &jpeg, 72).unwrap(), count);
        assert_eq!(fake.output_writes.borrow().as_slice(), reports);
        assert!(fake.feature_queries.borrow().is_empty());
        assert!(fake.feature_writes.borrow().is_empty());
    }

    #[test]
    fn exact_multiple_has_no_extra_empty_chunk_and_sets_last_flag() {
        let jpeg = real_jpeg();
        let reports: Vec<_> = JpegPackets::new(&jpeg, jpeg.len() + 8).unwrap().collect();
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0][7], 1);
        assert_eq!(&reports[0][8..], jpeg);
        let jpeg = padded_jpeg(2000);
        let reports: Vec<_> = JpegPackets::new(&jpeg, 108).unwrap().collect();
        assert_eq!(reports.len(), 20);
        assert_eq!(reports[18][7], 0);
        assert_eq!(reports[19][7], 1);
        assert_eq!(&reports[19][4..6], &100u16.to_le_bytes());
    }

    #[test]
    fn sequence_limit_allows_last_u16_value_and_rejects_overflow_before_writing() {
        let jpeg = padded_jpeg(65_536);
        let packets = JpegPackets::new(&jpeg, 9).unwrap();
        assert_eq!(packets.len(), 65_536);
        let last = packets.last().unwrap();
        assert_eq!(&last[2..4], &u16::MAX.to_le_bytes());
        assert_eq!(last[7], 1);
        let fake = Fake::default();
        let overflowing = padded_jpeg(65_537);
        assert!(display_jpeg(&fake, &overflowing, 9).is_err());
        assert!(fake.output_writes.borrow().is_empty());
    }

    #[test]
    fn sequence_and_payload_lengths_use_both_little_endian_bytes() {
        let jpeg = padded_jpeg(65_536);
        let report = JpegPackets::new(&jpeg, 9).unwrap().nth(256).unwrap();
        assert_eq!(&report[2..4], &[0, 1]);
        let mut packets = JpegPackets::new(&jpeg, 65_535).unwrap();
        let first = packets.next().unwrap();
        assert_eq!(&first[4..6], &[0xf7, 0xff]);
        assert_eq!(first[7], 0);
        let last = packets.next().unwrap();
        assert_eq!(&last[4..6], &[9, 0]);
        assert_eq!(last[7], 1);
        assert!(last[17..].iter().all(|&byte| byte == 0));
        assert!(packets.next().is_none());
        for report_length in [65_536, 65_543] {
            assert!(JpegPackets::new(&jpeg, report_length).is_err());
        }
    }

    #[test]
    fn invalid_lengths_and_images_are_rejected_before_output() {
        let fake = Fake::default();
        let jpeg = real_jpeg();
        for length in [0, 1, 7, 8, 65_544, usize::MAX] {
            assert!(display_jpeg(&fake, &jpeg, length).is_err());
        }
        for image in [
            Vec::new(),
            vec![0; 20],
            jpeg[..jpeg.len() - 1].to_vec(),
            vec![0; MAX_JPEG_LENGTH + 1],
        ] {
            assert!(display_jpeg(&fake, &image, 72).is_err());
        }
        let sof = jpeg
            .windows(2)
            .position(|bytes| bytes == [0xff, 0xc0])
            .unwrap();
        let mut wrong_dimensions = jpeg.clone();
        wrong_dimensions[sof + 8] = 239;
        assert!(display_jpeg(&fake, &wrong_dimensions, 72).is_err());
        let mut progressive = jpeg.clone();
        progressive[sof + 1] = 0xc2;
        assert!(display_jpeg(&fake, &progressive, 72).is_err());
        let mut non_jfif = jpeg.clone();
        non_jfif[6] = b'X';
        assert!(display_jpeg(&fake, &non_jfif, 72).is_err());
        let mut bad_segment = jpeg.clone();
        bad_segment[4..6].copy_from_slice(&1u16.to_be_bytes());
        assert!(display_jpeg(&fake, &bad_segment, 72).is_err());
        let mut trailing = jpeg.clone();
        trailing.push(0);
        assert!(display_jpeg(&fake, &trailing, 72).is_err());
        for length in 0..jpeg.len() {
            assert!(JpegPackets::new(&jpeg[..length], 72).is_err());
        }
        assert!(fake.output_writes.borrow().is_empty());
    }

    #[test]
    fn short_or_failed_output_stops_without_retry_or_acknowledgement() {
        let jpeg = real_jpeg();
        for result in [Ok(71), Ok(73), Err(invalid("disconnected"))] {
            let fake = Fake {
                output_results: RefCell::new(VecDeque::from([Ok(72), result])),
                ..Fake::default()
            };
            assert!(display_jpeg(&fake, &jpeg, 72).is_err());
            assert_eq!(fake.output_writes.borrow().len(), 2);
            assert!(fake.feature_queries.borrow().is_empty());
            assert!(fake.feature_writes.borrow().is_empty());
        }
    }

    // Synthetic expansion exercises framing/sequence bounds. Added entropy bytes
    // are not claimed to form a decodable image; this parser does not entropy-decode.
    fn padded_jpeg(length: usize) -> Vec<u8> {
        let mut jpeg = real_jpeg();
        assert!(length >= jpeg.len());
        jpeg.truncate(jpeg.len() - 2);
        jpeg.resize(length - 2, 0);
        jpeg.extend_from_slice(&[0xff, 0xd9]);
        jpeg
    }

    fn real_jpeg() -> Vec<u8> {
        GENERATED_JPEG_HEX
            .as_bytes()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| {
                let digit = |byte| match byte {
                    b'0'..=b'9' => byte - b'0',
                    b'a'..=b'f' => byte - b'a' + 10,
                    _ => panic!("invalid generated JPEG fixture hex"),
                };
                (digit(pair[0]) << 4) | digit(pair[1])
            })
            .collect()
    }

    // A real 240×240 black baseline JFIF image encoded locally by macOS sips from
    // an original PPM on 3 October 2026. It is image data, never a device capture.
    const GENERATED_JPEG_HEX: &str = concat!(
        "ffd8ffe000104a46494600010100004800480000ffe1004c4578696600004d4d002a0000000800018769000400000001",
        "0000001a000000000003a00100030000000100010000a002000400000001000000f0a003000400000001000000f00000",
        "0000ffed003850686f746f73686f7020332e30003842494d04040000000000003842494d0425000000000010d41d8cd9",
        "8f00b204e9800998ecf8427effc000110800f000f003012200021101031101ffc4001f00000105010101010101000000",
        "00000000000102030405060708090a0bffc400b5100002010303020403050504040000017d0102030004110512213141",
        "0613516107227114328191a1082342b1c11552d1f02433627282090a161718191a25262728292a3435363738393a4344",
        "45464748494a535455565758595a636465666768696a737475767778797a838485868788898a92939495969798999aa2",
        "a3a4a5a6a7a8a9aab2b3b4b5b6b7b8b9bac2c3c4c5c6c7c8c9cad2d3d4d5d6d7d8d9dae1e2e3e4e5e6e7e8e9eaf1f2f3",
        "f4f5f6f7f8f9faffc4001f0100030101010101010101010000000000000102030405060708090a0bffc400b511000201",
        "02040403040705040400010277000102031104052131061241510761711322328108144291a1b1c109233352f0156272",
        "d10a162434e125f11718191a262728292a35363738393a434445464748494a535455565758595a636465666768696a73",
        "7475767778797a82838485868788898a92939495969798999aa2a3a4a5a6a7a8a9aab2b3b4b5b6b7b8b9bac2c3c4c5c6",
        "c7c8c9cad2d3d4d5d6d7d8d9dae2e3e4e5e6e7e8e9eaf2f3f4f5f6f7f8f9faffdb004300020202020202030202030503",
        "030305060505050506080606060606080a0808080808080a0a0a0a0a0a0a0a0c0c0c0c0c0c0e0e0e0e0e0f0f0f0f0f0f",
        "0f0f0f0fffdb00430102020204040407040407100b090b10101010101010101010101010101010101010101010101010",
        "10101010101010101010101010101010101010101010101010ffdd0004000fffda000c03010002110311003f00fe7fe8",
        "a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028",
        "a28a0028a28a0028a28a00ffd0fe7fe8a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028",
        "a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028a28a00ffd1fe7fe8a28a0028a28a0028a28a0028a28a0028",
        "a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028a28a00ffd2fe7fe8",
        "a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028",
        "a28a0028a28a0028a28a00ffd3fe7fe8a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028",
        "a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028a28a00ffd4fe7fe8a28a0028a28a0028a28a0028a28a0028",
        "a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028a28a00ffd5fe7fe8",
        "a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028",
        "a28a0028a28a0028a28a00ffd6fe7fe8a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028",
        "a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028a28a00ffd7fe7fe8a28a0028a28a0028a28a0028a28a0028",
        "a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028a28a00ffd0fe7fe8",
        "a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028",
        "a28a0028a28a0028a28a00ffd1fe7fe8a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028",
        "a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028a28a00ffd2fe7fe8a28a0028a28a0028a28a0028a28a0028",
        "a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028a28a00ffd3fe7fe8",
        "a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028",
        "a28a0028a28a0028a28a00ffd4fe7fe8a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028",
        "a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028a28a00ffd5fe7fe8a28a0028a28a0028a28a0028a28a0028",
        "a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028a28a0028a28a00ffd9",
    );
}
