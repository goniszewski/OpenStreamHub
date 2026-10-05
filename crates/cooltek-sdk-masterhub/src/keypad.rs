//! Explicit M4315 keypad commands recovered from native SDK 03.02.00.
//!
//! Commands are software-derived and hardware-unverified. They do not open,
//! initialize, reconnect, or implicitly configure a device. Source: the pinned
//! official MasterHUB 1.0.0.113 DLL's `Set_Sw_Mode_CM` implementation at RVA 5070.
//! Its installed and PE original filename is `CoolTEK_SDK_CM_M4315_V03.02.00.dll`;
//! the MSI/CAB file identifier misleadingly contains `M4315_V02.00.00`.
//! Per-key JPEG packets follow its `Display_Picture_On_Button_CM` sender at
//! RVA 5bb0. Packet generation is pure and does not upload an image. Image size
//! comes from the pinned application's 72×72 JPEG preparation, not a proved
//! firmware restriction. Pacing, orientation, and image acceptance are unverified.

use hidapi::{HidError, HidResult};

use crate::ReportTransport;

/// Complete software-mode feature length, including its report ID.
pub const FEATURE_REPORT_LENGTH: usize = 32;
/// Per-key image width prepared by the pinned official application.
pub const IMAGE_WIDTH: u16 = 72;
/// Per-key image height prepared by the pinned official application.
pub const IMAGE_HEIGHT: u16 = 72;
/// Defensive encoded-image limit for this implementation, not a hardware limit.
pub const MAX_JPEG_LENGTH: usize = crate::jpeg::MAX_JPEG_LENGTH;

fn invalid(message: impl Into<String>) -> HidError {
    HidError::HidApiError {
        message: message.into(),
    }
}

/// Checks the conservative JPEG format used by offline keypad image plans.
///
/// Requires 72×72, 8-bit, three-component baseline JFIF with one interleaved
/// scan and a final EOI marker. Marker framing and component parameters are
/// checked, but entropy-coded pixels are not decoded or proved valid. Prepare
/// images with a real JPEG encoder. This is an SDK policy, not proof of the
/// firmware's complete set of accepted image formats.
///
/// # Errors
/// Rejects oversized images, unsupported dimensions or JPEG framing, including
/// progressive frames, additional scans, incomplete segments, and trailing data.
pub fn validate_jpeg(jpeg: &[u8]) -> HidResult<()> {
    crate::jpeg::validate(jpeg, "Keypad", IMAGE_WIDTH, IMAGE_HEIGHT)
}

/// Prevalidated iterator over descriptor-sized JPEG reports for one M4315 key.
///
/// The index uses wire order, from 0 through 14; physical placement is unverified.
/// Every report is freshly zero-filled, including unused final-chunk bytes.
/// The native sender reuses a buffer; zero padding is a deliberate SDK policy.
/// This iterator opens no device, changes no software mode, and establishes no
/// transfer pacing or device acknowledgement.
#[derive(Clone, Debug)]
pub struct JpegPackets<'a> {
    chunks: crate::jpeg::JpegChunks<'a>,
    key_index: u8,
}

impl<'a> JpegPackets<'a> {
    /// Validates the key, complete JPEG, and report size before generating bytes.
    ///
    /// `output_report_length` includes the report ID and must come from the
    /// selected interface's HID descriptor. The SDK cannot establish that a
    /// caller-supplied size belongs to the actual M4315 device. Payload capacity
    /// is that size minus the 8-byte header; no fixed 320-byte report is assumed.
    ///
    /// # Errors
    /// Rejects key indexes above 14, unsupported JPEG framing/dimensions, images
    /// exceeding [`MAX_JPEG_LENGTH`], report lengths outside 9 through 65,535,
    /// and image plans needing more than 65,536 chunks.
    pub fn new(key_index: u8, jpeg: &'a [u8], output_report_length: usize) -> HidResult<Self> {
        if key_index > 14 {
            return Err(invalid("Keypad key index must be between 0 and 14"));
        }
        let chunks =
            crate::jpeg::JpegChunks::new(jpeg, 0x36, key_index, output_report_length, "Keypad")?;
        validate_jpeg(jpeg)?;
        chunks.check_sequence_limit("Keypad")?;
        Ok(Self { chunks, key_index })
    }

    /// Returns the complete output report length, including the report ID.
    pub fn output_report_length(&self) -> usize {
        self.chunks.output_report_length()
    }

    /// Returns the targeted wire key index, from 0 through 14.
    pub fn key_index(&self) -> u8 {
        self.key_index
    }

    /// Returns the complete encoded JPEG length, unaffected by iteration.
    pub fn jpeg_length(&self) -> usize {
        self.chunks.jpeg_length()
    }
}

impl Iterator for JpegPackets<'_> {
    type Item = Vec<u8>;

    fn next(&mut self) -> Option<Self::Item> {
        self.chunks.next()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.chunks.size_hint()
    }
}

impl ExactSizeIterator for JpegPackets<'_> {}
impl std::iter::FusedIterator for JpegPackets<'_> {}

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

    // A real 72×72 black baseline JFIF, generated from an original RGB PPM by
    // macOS sips at best JPEG quality on 4 October 2026. This is artwork, not a
    // device capture. SHA-256: e19318753dcd5fd7fa5c808f6d8db8de51bc2a5249f11c220d29662cf1ebe968.
    const REAL_JPEG: &[u8] = include_bytes!("../tests/fixtures/keypad-72x72-baseline.jpg");

    #[test]
    fn descriptor_sized_packets_reconstruct_each_endpoint_key_and_clear_padding() {
        for report_length in [65u16, 320] {
            let count = (report_length - 1).to_le_bytes();
            let descriptor = [
                0x85, 0x02, // Report ID 2.
                0x75, 0x08, // Eight-bit fields.
                0x96, count[0], count[1], // Count excludes the report ID.
                0x91, 0x02, // Output.
            ];
            let length = crate::descriptor::output_report_length(&descriptor, 2).unwrap();
            assert_eq!(length, usize::from(report_length));
            let capacity = length - 8;
            for key_index in [0, 14] {
                let mut packets = JpegPackets::new(key_index, REAL_JPEG, length).unwrap();
                let total = REAL_JPEG.len().div_ceil(capacity);
                assert_eq!(packets.output_report_length(), length);
                assert_eq!(packets.key_index(), key_index);
                assert_eq!(packets.jpeg_length(), REAL_JPEG.len());
                assert_eq!(packets.len(), total);
                let mut reconstructed = Vec::new();
                for sequence in 0..total {
                    let report = packets.next().unwrap();
                    assert_eq!(packets.len(), total - sequence - 1);
                    assert_eq!(report.len(), length);
                    assert_eq!(&report[..2], &[2, 0x36]);
                    assert_eq!(u16::from_le_bytes([report[2], report[3]]), sequence as u16);
                    let payload_length = usize::from(u16::from_le_bytes([report[4], report[5]]));
                    assert_eq!(
                        payload_length,
                        (REAL_JPEG.len() - sequence * capacity).min(capacity)
                    );
                    assert_eq!(report[6], key_index);
                    assert_eq!(report[7], u8::from(sequence + 1 == total));
                    reconstructed.extend_from_slice(&report[8..8 + payload_length]);
                    assert!(report[8 + payload_length..].iter().all(|&byte| byte == 0));
                }
                assert_eq!(reconstructed, REAL_JPEG);
                assert!(packets.next().is_none());
                assert!(packets.next().is_none());
                assert_eq!(packets.jpeg_length(), REAL_JPEG.len());
                assert_eq!(packets.key_index(), key_index);
            }
        }
    }

    #[test]
    fn every_wire_key_is_supported_and_invalid_key_or_report_sizes_fail() {
        for key_index in 0..=14 {
            assert_eq!(
                JpegPackets::new(key_index, REAL_JPEG, 320)
                    .unwrap()
                    .key_index(),
                key_index
            );
        }
        for key_index in 15..=u8::MAX {
            assert!(
                JpegPackets::new(key_index, REAL_JPEG, 320)
                    .unwrap_err()
                    .to_string()
                    .contains("between 0 and 14")
            );
        }
        for report_length in [0, 1, 7, 8, 65_536, usize::MAX] {
            assert!(JpegPackets::new(0, REAL_JPEG, report_length).is_err());
        }
    }

    #[test]
    fn exact_payload_multiple_has_one_final_flag_and_no_empty_packet() {
        let reports: Vec<_> = JpegPackets::new(8, REAL_JPEG, REAL_JPEG.len() + 8)
            .unwrap()
            .collect();
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0][7], 1);
        assert_eq!(&reports[0][8..], REAL_JPEG);
        let padded = padded_jpeg(2000);
        let reports: Vec<_> = JpegPackets::new(8, &padded, 108).unwrap().collect();
        assert_eq!(reports.len(), 20);
        assert!(reports[..19].iter().all(|report| report[7] == 0));
        assert_eq!(reports[19][7], 1);
        assert_eq!(&reports[19][4..6], &100u16.to_le_bytes());
    }

    #[test]
    fn full_sequence_range_is_allowed_but_wrapping_is_rejected() {
        let padded = padded_jpeg(65_536);
        let packets = JpegPackets::new(14, &padded, 9).unwrap();
        assert_eq!(packets.len(), 65_536);
        let last = packets.last().unwrap();
        assert_eq!(&last[2..4], &u16::MAX.to_le_bytes());
        assert_eq!(last[7], 1);
        let overflowing = padded_jpeg(65_537);
        assert!(
            JpegPackets::new(14, &overflowing, 9)
                .unwrap_err()
                .to_string()
                .contains("overflow the 16-bit chunk sequence")
        );
    }

    #[test]
    fn sequence_and_payload_length_use_both_little_endian_bytes() {
        let padded = padded_jpeg(65_536);
        let report = JpegPackets::new(1, &padded, 9).unwrap().nth(256).unwrap();
        assert_eq!(&report[2..4], &[0, 1]);
        let mut packets = JpegPackets::new(1, &padded, 65_535).unwrap();
        let first = packets.next().unwrap();
        assert_eq!(&first[4..6], &[0xf7, 0xff]);
        assert_eq!(first[7], 0);
        let last = packets.next().unwrap();
        assert_eq!(&last[4..6], &[9, 0]);
        assert_eq!(last[7], 1);
        assert!(last[17..].iter().all(|&byte| byte == 0));
        assert!(packets.next().is_none());
    }

    #[test]
    fn malformed_or_unsupported_jpegs_fail_complete_validation() {
        validate_jpeg(REAL_JPEG).unwrap();
        for end in 0..REAL_JPEG.len() {
            assert!(validate_jpeg(&REAL_JPEG[..end]).is_err());
        }
        let mut trailing = REAL_JPEG.to_vec();
        trailing.push(0);
        assert!(validate_jpeg(&trailing).is_err());
        let mut additional_scan = REAL_JPEG[..REAL_JPEG.len() - 2].to_vec();
        additional_scan.extend_from_slice(&[0xff, 0xda, 0, 12]);
        assert!(validate_jpeg(&additional_scan).is_err());
        let mut non_jfif = REAL_JPEG.to_vec();
        non_jfif[6] = b'X';
        assert!(validate_jpeg(&non_jfif).is_err());
        let mut bad_segment = REAL_JPEG.to_vec();
        bad_segment[4..6].copy_from_slice(&1u16.to_be_bytes());
        assert!(validate_jpeg(&bad_segment).is_err());
        let frame = REAL_JPEG
            .windows(2)
            .position(|bytes| bytes == [0xff, 0xc0])
            .unwrap();
        let mut progressive = REAL_JPEG.to_vec();
        progressive[frame + 1] = 0xc2;
        assert!(validate_jpeg(&progressive).is_err());
        let mut wrong_dimensions = REAL_JPEG.to_vec();
        wrong_dimensions[frame + 8] = 71;
        assert!(
            validate_jpeg(&wrong_dimensions)
                .unwrap_err()
                .to_string()
                .contains("Keypad JPEG dimensions must be 72×72")
        );
        let mut grayscale = REAL_JPEG.to_vec();
        grayscale[frame + 9] = 1;
        assert!(validate_jpeg(&grayscale).is_err());
        let mut duplicate_component = REAL_JPEG.to_vec();
        duplicate_component[frame + 13] = duplicate_component[frame + 10];
        assert!(validate_jpeg(&duplicate_component).is_err());
        let oversized = vec![0; MAX_JPEG_LENGTH + 1];
        assert!(
            validate_jpeg(&oversized)
                .unwrap_err()
                .to_string()
                .contains("16 MiB implementation limit")
        );
    }

    #[test]
    fn shared_validator_keeps_dial_dimensions_and_diagnostics() {
        assert!(
            crate::dial::JpegPackets::new(REAL_JPEG, 320)
                .unwrap_err()
                .to_string()
                .contains("Dial JPEG dimensions must be 240×240")
        );
    }

    // Synthetic entropy expansion tests framing and sequence bounds only; the
    // expanded images are not claimed to have valid entropy-coded pixels.
    fn padded_jpeg(length: usize) -> Vec<u8> {
        assert!(length >= REAL_JPEG.len());
        let mut jpeg = REAL_JPEG[..REAL_JPEG.len() - 2].to_vec();
        jpeg.resize(length - 2, 0);
        jpeg.extend_from_slice(&[0xff, 0xd9]);
        jpeg
    }
}
