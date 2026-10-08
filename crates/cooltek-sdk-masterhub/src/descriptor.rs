//! Pure HID descriptor inspection for deriving output and feature report lengths.

use hidapi::{HidError, HidResult};

const MAX_DESCRIPTOR_LENGTH: usize = 4096;
const MAX_REPORT_BITS: usize = (u16::MAX as usize - 1) * 8;
const MAX_GLOBAL_STACK_DEPTH: usize = 16;

#[derive(Clone, Copy, Default)]
struct Globals {
    report_size: Option<usize>,
    report_count: Option<usize>,
    report_id: u8,
}

/// Derives an output report's length from a complete HID report descriptor.
///
/// The returned length includes the report-ID byte required by HIDAPI, also for
/// an unnumbered report (`report_id == 0`). Every output field contributes its
/// bits, including constant padding, and the final bit count is rounded up to
/// bytes. Input and feature fields do not contribute to the output length.
///
/// Rejects malformed or unsupported descriptors rather than assuming a report
/// size. Long items, mixed numbered and unnumbered reports, unbalanced stacks or
/// collections, missing output IDs, and lengths beyond a Windows HID capability
/// length (`u16`) produce [`HidError::HidApiError`]. This function performs no HID
/// operations.
pub fn output_report_length(descriptor: &[u8], report_id: u8) -> HidResult<usize> {
    report_length(descriptor, report_id, OUTPUT, "output")?
        .ok_or_else(|| absent("output", report_id))
}

/// Derives a feature report's length from a complete HID report descriptor.
///
/// Identical rules to [`output_report_length`], counting only Feature fields.
/// The returned length includes the report-ID byte, matching the buffers passed
/// to HIDAPI's feature-report calls. Use it to check fixed-size feature commands
/// (such as the 32-byte software-mode report) against the selected interface.
///
/// # Errors
/// Returns [`HidError::HidApiError`] for malformed descriptors or an absent
/// feature Report ID. This function performs no HID operations.
pub fn feature_report_length(descriptor: &[u8], report_id: u8) -> HidResult<usize> {
    declared_feature_report_length(descriptor, report_id)?
        .ok_or_else(|| absent("feature", report_id))
}

/// Like [`feature_report_length`], but reports an undeclared ID as `Ok(None)`
/// so callers can tell it apart from a malformed descriptor.
pub(crate) fn declared_feature_report_length(
    descriptor: &[u8],
    report_id: u8,
) -> HidResult<Option<usize>> {
    report_length(descriptor, report_id, FEATURE, "feature")
}

fn absent(type_name: &str, report_id: u8) -> HidError {
    descriptor_error(&format!("{type_name} Report ID {report_id:#04x} is absent"))
}

const OUTPUT: usize = 1;
const FEATURE: usize = 2;

/// A Feature report declared by a HID report descriptor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeclaredFeatureReport {
    /// Report ID, or 0 for an unnumbered descriptor.
    pub report_id: u8,
    /// Sum of all Feature field bits for this ID, excluding the report-ID byte.
    pub bits: usize,
    /// HIDAPI buffer length: payload bits rounded up to bytes, plus the ID byte.
    pub length: usize,
}

/// Lists every Feature report the descriptor declares, in ascending ID order.
///
/// Uses the same validation and length rules as [`feature_report_length`]; an
/// ID appears only when at least one Feature field contributes bits to it, so
/// Input and Output report IDs are never listed. Performs no HID operations.
///
/// # Errors
/// Returns [`HidError::HidApiError`] for a malformed or unsupported descriptor.
pub fn declared_feature_reports(descriptor: &[u8]) -> HidResult<Vec<DeclaredFeatureReport>> {
    let report_bits = report_bits(descriptor)?;
    let mut reports = Vec::new();
    for (report_id, bits) in (0..=u8::MAX).zip(report_bits[FEATURE]) {
        if bits != 0 {
            reports.push(DeclaredFeatureReport {
                report_id,
                bits,
                length: byte_length(bits, "feature")?,
            });
        }
    }
    Ok(reports)
}

fn report_length(
    descriptor: &[u8],
    report_id: u8,
    report_type: usize,
    type_name: &str,
) -> HidResult<Option<usize>> {
    let bits = report_bits(descriptor)?[report_type][usize::from(report_id)];
    if bits == 0 {
        return Ok(None);
    }
    byte_length(bits, type_name).map(Some)
}

fn byte_length(bits: usize, type_name: &str) -> HidResult<usize> {
    bits.checked_add(7)
        .and_then(|bits| (bits / 8).checked_add(1))
        .filter(|length| *length <= usize::from(u16::MAX))
        .ok_or_else(|| descriptor_error(&format!("{type_name} report exceeds 65535 bytes")))
}

/// Validates a whole descriptor and returns per-type, per-ID field bit totals.
fn report_bits(descriptor: &[u8]) -> HidResult<Box<[[usize; 256]; 3]>> {
    if descriptor.len() > MAX_DESCRIPTOR_LENGTH {
        return Err(descriptor_error("descriptor exceeds 4096 bytes"));
    }

    let mut globals = Globals::default();
    let mut global_stack = Vec::new();
    let mut collection_depth = 0usize;
    // Separate input, output, and feature totals: IDs can be shared across types.
    let mut report_bits = Box::new([[0usize; 256]; 3]);
    let mut has_numbered_reports = false;
    let mut has_unnumbered_reports = false;
    let mut offset = 0usize;

    while offset < descriptor.len() {
        let prefix = descriptor[offset];
        offset += 1;
        if prefix == 0xfe {
            return Err(descriptor_error("long items are unsupported"));
        }
        let size = match prefix & 0x03 {
            0 => 0,
            1 => 1,
            2 => 2,
            _ => 4,
        };
        let end = offset
            .checked_add(size)
            .filter(|end| *end <= descriptor.len())
            .ok_or_else(|| descriptor_error("truncated short item"))?;
        let data = &descriptor[offset..end];
        offset = end;
        let item_type = (prefix >> 2) & 0x03;
        let tag = prefix >> 4;

        match (item_type, tag) {
            (1, 7) => globals.report_size = Some(positive_global(data, "Report Size")?),
            (1, 8) => {
                if data.len() != 1 || data[0] == 0 {
                    return Err(descriptor_error("Report ID must be one nonzero byte"));
                }
                globals.report_id = data[0];
            }
            (1, 9) => globals.report_count = Some(positive_global(data, "Report Count")?),
            (1, 10) => {
                if !data.is_empty() {
                    return Err(descriptor_error("Push must have no payload"));
                }
                if global_stack.len() == MAX_GLOBAL_STACK_DEPTH {
                    return Err(descriptor_error("global stack exceeds 16 entries"));
                }
                global_stack.push(globals);
            }
            (1, 11) => {
                if !data.is_empty() {
                    return Err(descriptor_error("Pop must have no payload"));
                }
                globals = global_stack
                    .pop()
                    .ok_or_else(|| descriptor_error("Pop has no matching Push"))?;
            }
            // A zero-length payload is valid and means flags = 0; flags never
            // affect length, so the payload is not inspected.
            (0, 8 | 9 | 11) => {
                let report_size = globals
                    .report_size
                    .ok_or_else(|| descriptor_error("data field has no Report Size"))?;
                let report_count = globals
                    .report_count
                    .ok_or_else(|| descriptor_error("data field has no Report Count"))?;
                if globals.report_id == 0 {
                    has_unnumbered_reports = true;
                } else {
                    has_numbered_reports = true;
                }
                if has_numbered_reports && has_unnumbered_reports {
                    return Err(descriptor_error(
                        "numbered and unnumbered reports are mixed",
                    ));
                }
                let bits = report_size
                    .checked_mul(report_count)
                    .filter(|bits| *bits <= MAX_REPORT_BITS)
                    .ok_or_else(|| {
                        descriptor_error("data field exceeds the report length limit")
                    })?;
                let report_type = match tag {
                    8 => 0,
                    9 => 1,
                    _ => 2,
                };
                let total = &mut report_bits[report_type][usize::from(globals.report_id)];
                *total = total
                    .checked_add(bits)
                    .filter(|bits| *bits <= MAX_REPORT_BITS)
                    .ok_or_else(|| {
                        descriptor_error("combined fields exceed the report length limit")
                    })?;
            }
            (0, 10) => {
                if data.len() != 1 {
                    return Err(descriptor_error("Collection must have a one-byte payload"));
                }
                collection_depth = collection_depth
                    .checked_add(1)
                    .ok_or_else(|| descriptor_error("collection depth overflow"))?;
            }
            (0, 12) => {
                if !data.is_empty() {
                    return Err(descriptor_error("End Collection must have no payload"));
                }
                collection_depth = collection_depth
                    .checked_sub(1)
                    .ok_or_else(|| descriptor_error("End Collection has no matching Collection"))?;
            }
            // Usage/local metadata and unrelated globals do not affect lengths.
            _ => {}
        }
    }

    if !global_stack.is_empty() {
        return Err(descriptor_error("Push has no matching Pop"));
    }
    if collection_depth != 0 {
        return Err(descriptor_error(
            "Collection has no matching End Collection",
        ));
    }
    Ok(report_bits)
}

fn positive_global(data: &[u8], name: &str) -> HidResult<usize> {
    if data.is_empty() {
        return Err(descriptor_error(&format!(
            "{name} needs an integer payload"
        )));
    }
    let value = data.iter().enumerate().fold(0u32, |value, (index, byte)| {
        value | (u32::from(*byte) << (index * 8))
    });
    let value = usize::try_from(value)
        .ok()
        .filter(|value| *value > 0 && *value <= MAX_REPORT_BITS)
        .ok_or_else(|| {
            descriptor_error(&format!(
                "{name} must be positive and within the report limit"
            ))
        })?;
    Ok(value)
}

fn descriptor_error(message: &str) -> HidError {
    HidError::HidApiError {
        message: format!("Invalid HID report descriptor: {message}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(descriptor: &[u8], report_id: u8) -> String {
        output_report_length(descriptor, report_id)
            .unwrap_err()
            .to_string()
    }

    #[test]
    fn separates_output_ids_from_input_and_feature_lengths() {
        let descriptor = [
            0x06, 0x00, 0xff, // Usage Page (vendor)
            0x09, 0x01, // Usage
            0xa1, 0x01, // Collection (application)
            0x75, 0x08, // Report Size (8 bits)
            0x85, 0x01, 0x95, 0x20, 0x81, 0x02, // ID 1: 32-byte input
            0x85, 0x02, 0x95, 0x3f, 0x91, 0x02, // ID 2: 63-byte output
            0x85, 0x03, 0x95, 0x07, 0xb1, 0x02, // ID 3: 7-byte feature
            0x85, 0x04, 0x96, 0xff, 0x01, 0x91, 0x02, // ID 4: 511-byte output
            0xc0,
        ];
        assert_eq!(output_report_length(&descriptor, 2).unwrap(), 64);
        assert_eq!(output_report_length(&descriptor, 4).unwrap(), 512);
        assert!(message(&descriptor, 1).contains("is absent"));
        assert!(message(&descriptor, 3).contains("is absent"));
        assert_eq!(feature_report_length(&descriptor, 3).unwrap(), 8);
        for id in [1, 2, 4] {
            assert!(
                feature_report_length(&descriptor, id)
                    .unwrap_err()
                    .to_string()
                    .contains("feature Report ID")
            );
        }
    }

    // Report descriptors captured from hardware by `masterhub-diag record`.
    const M032LE_DESCRIPTOR_HEX: &str = "050c0901a1010a00ff150026ff007508963f00850291020a00ff150026ff007508963f00850181020a00ff150026ff007508961f008503b1040a00ff150026ff007508961f00852db1040a00ff150026ff007508961f00852eb1040a00ff150026ff007508961f00852fb1040a00ff150026ff007508961f008530b1040a00ff150026ff007508961f008531b1040a00ff150026ff007508961f008536b104c0";
    const M003K_DESCRIPTOR_HEX: &str = "050c0901a1010a00ff150026ff007508963f00850291020a00ff150026ff007508963f00850181020a00ff150026ff007508961f008503b1040a00ff150026ff007508961f00852db1040a00ff150026ff007508961f00852eb1040a00ff150026ff007508961f00852fb1040a00ff150026ff007508961f008530b1040a00ff150026ff007508961f008531b1040a00ff150026ff007508961f008532b1040a00ff150026ff007508961f008534b1040a00ff150026ff007508961f008536b104c0";
    const M131E_DESCRIPTOR_HEX: &str = "050c0901a1010a00ff150026ff007508963f01850291020a00ff150026ff007508963f00850181020a00ff150026ff007508961f008503b1040a00ff150026ff007508961f008547b1040a00ff150026ff007508961f008548b1040a00ff150026ff007508961f008549b1040a00ff150026ff007508961f00854ab1040a00ff150026ff007508961f00854bb1040a00ff150026ff007508961f00854cb1040a00ff150026ff007508961f00854db1040a00ff150026ff007508961f00854eb104c0";

    fn bytes(hex: &str) -> Vec<u8> {
        (0..hex.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&hex[index..index + 2], 16).unwrap())
            .collect()
    }

    fn feature_ids(descriptor: &[u8]) -> Vec<u8> {
        declared_feature_reports(descriptor)
            .unwrap()
            .iter()
            .map(|report| report.report_id)
            .collect()
    }

    #[test]
    fn lists_declared_feature_reports_of_captured_descriptors() {
        let roller = bytes(M032LE_DESCRIPTOR_HEX);
        let knobs = bytes(M003K_DESCRIPTOR_HEX);
        let dial = bytes(M131E_DESCRIPTOR_HEX);
        assert_eq!((roller.len(), knobs.len(), dial.len()), (160, 194, 194));

        assert_eq!(
            feature_ids(&roller),
            [0x03, 0x2d, 0x2e, 0x2f, 0x30, 0x31, 0x36]
        );
        assert_eq!(
            feature_ids(&knobs),
            [0x03, 0x2d, 0x2e, 0x2f, 0x30, 0x31, 0x32, 0x34, 0x36]
        );
        assert_eq!(
            feature_ids(&dial),
            [0x03, 0x47, 0x48, 0x49, 0x4a, 0x4b, 0x4c, 0x4d, 0x4e]
        );
        for descriptor in [&roller, &knobs, &dial] {
            for report in declared_feature_reports(descriptor).unwrap() {
                // Report Size 8 x Report Count 31, plus the report-ID byte.
                assert_eq!((report.bits, report.length), (248, 32));
                assert_eq!(
                    feature_report_length(descriptor, report.report_id).unwrap(),
                    32
                );
            }
            // Input ID 1 and output ID 2 are not Feature reports.
            assert!(!feature_ids(descriptor).contains(&1));
            assert!(!feature_ids(descriptor).contains(&2));
        }
        assert_eq!(output_report_length(&roller, 2).unwrap(), 64);
        assert_eq!(output_report_length(&dial, 2).unwrap(), 320);
    }

    #[test]
    fn declared_feature_reports_keep_bit_sizes_and_reject_malformed_descriptors() {
        let descriptor = [
            0x85, 0x05, 0x75, 0x03, 0x95, 0x01, 0xb1, 0x02, // ID 5: 3 bits
            0x85, 0x04, 0x75, 0x08, 0x95, 0x02, 0xb1, 0x02, // ID 4: 16 bits
            0x85, 0x06, 0x81, 0x02, // ID 6: input only
        ];
        assert_eq!(
            declared_feature_reports(&descriptor).unwrap(),
            [
                DeclaredFeatureReport {
                    report_id: 4,
                    bits: 16,
                    length: 3
                },
                DeclaredFeatureReport {
                    report_id: 5,
                    bits: 3,
                    length: 2
                },
            ]
        );
        assert_eq!(
            declared_feature_reports(&[0x75, 8, 0x95, 2, 0xb1, 2]).unwrap(),
            [DeclaredFeatureReport {
                report_id: 0,
                bits: 16,
                length: 3
            }]
        );
        assert!(declared_feature_reports(&[]).unwrap().is_empty());
        let mut truncated = bytes(M032LE_DESCRIPTOR_HEX);
        truncated.pop();
        assert!(declared_feature_reports(&truncated).is_err());
        assert!(declared_feature_reports(&[0xb1, 2]).is_err());
    }

    #[test]
    fn arbitrary_descriptors_never_panic() {
        // Deterministic pseudo-random fuzzing; no external dependency.
        let mut state = 0x2545_f491_4f6c_dd1du64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        // Bias toward item prefixes that the parser interprets.
        const PREFIXES: [u8; 12] = [
            0x75, 0x76, 0x77, 0x85, 0x95, 0x96, 0x81, 0x91, 0xb1, 0xa1, 0xc0, 0xa4,
        ];
        for _ in 0..20_000 {
            let length = (next() % 64) as usize;
            let descriptor: Vec<u8> = (0..length)
                .map(|_| {
                    let value = next();
                    if value % 3 == 0 {
                        PREFIXES[(value >> 8) as usize % PREFIXES.len()]
                    } else {
                        (value >> 16) as u8
                    }
                })
                .collect();
            let id = (next() % 4) as u8;
            if let Ok(length) = output_report_length(&descriptor, id) {
                assert!((1..=65_535).contains(&length));
            }
            if let Ok(length) = feature_report_length(&descriptor, id) {
                assert!((1..=65_535).contains(&length));
            }
        }
    }

    #[test]
    fn sums_split_fields_and_constant_padding_before_rounding() {
        let descriptor = [
            0x85, 0x02, 0x75, 0x03, 0x95, 0x01, 0x91, 0x02, // 3 data bits
            0x75, 0x05, 0x91, 0x01, // 5 constant padding bits
            0x75, 0x01, 0x91, 0x02, // 1 further data bit
        ];
        assert_eq!(output_report_length(&descriptor, 2).unwrap(), 3);
    }

    #[test]
    fn restores_size_count_and_id_after_push_pop() {
        let descriptor = [
            0x85, 0x02, 0x75, 0x08, 0x95, 0x03, 0x91, 0x02, // ID 2: 3 bytes
            0xa4, // Push
            0x85, 0x03, 0x75, 0x04, 0x95, 0x01, 0x91, 0x02, // ID 3: 4 bits
            0xb4, // Pop
            0x91, 0x01, // ID 2: another 3 bytes, constant
        ];
        assert_eq!(output_report_length(&descriptor, 2).unwrap(), 7);
        assert_eq!(output_report_length(&descriptor, 3).unwrap(), 2);
    }

    #[test]
    fn includes_hidapi_zero_id_for_unnumbered_reports() {
        assert_eq!(
            output_report_length(&[0x75, 8, 0x95, 2, 0x91, 2], 0).unwrap(),
            3
        );
    }

    #[test]
    fn reads_four_byte_globals_as_little_endian_unsigned_values() {
        let descriptor = [
            0x85, 2, 0x77, 32, 0, 0, 0, // Report Size (32 bits, four-byte integer)
            0x97, 32, 0, 0, 0, // Report Count (32, four-byte integer)
            0x91, 2,
        ];
        assert_eq!(output_report_length(&descriptor, 2).unwrap(), 129);
    }

    #[test]
    fn validates_input_and_feature_globals_and_report_id_mixing() {
        for main in [0x81, 0x91, 0xb1] {
            assert!(message(&[main, 2], 2).contains("no Report Size"));
            assert!(message(&[0x75, 8, main, 2], 2).contains("no Report Count"));
            let descriptor = [
                0x75, 8, 0x95, 1, main, 2, // unnumbered field
                0x85, 2, 0x91, 2, // numbered output
            ];
            assert!(message(&descriptor, 2).contains("are mixed"));
        }
    }

    #[test]
    fn rejects_malformed_integer_fields_and_ids() {
        for descriptor in [
            vec![0x74], // Report Size without integer
            vec![0x94], // Report Count without integer
            vec![0x75, 0],
            vec![0x95, 0],
            vec![0x85, 0],
            vec![0x84],       // Report ID without integer
            vec![0x86, 2, 0], // Report ID with two bytes
        ] {
            assert!(output_report_length(&descriptor, 2).is_err());
        }
    }

    #[test]
    fn zero_length_main_item_payload_is_flags_zero() {
        // Output (bSize 0): a valid short item whose flags default to 0.
        assert_eq!(
            output_report_length(&[0x75, 8, 0x95, 1, 0x90], 0).unwrap(),
            2
        );
        assert_eq!(
            feature_report_length(&[0x85, 3, 0x75, 8, 0x95, 31, 0xb0], 3).unwrap(),
            32
        );
    }

    #[test]
    fn declared_feature_length_separates_absent_from_malformed() {
        assert_eq!(
            declared_feature_report_length(&[0x85, 3, 0x75, 8, 0x95, 1, 0x91, 2], 3).unwrap(),
            None
        );
        assert!(declared_feature_report_length(&[0x75], 3).is_err());
    }

    #[test]
    fn rejects_truncation_in_all_short_item_sizes_and_long_items() {
        for descriptor in [vec![0x75], vec![0x76, 1], vec![0x77, 1, 2, 3]] {
            assert!(message(&descriptor, 2).contains("truncated"));
        }
        assert!(message(&[0xfe, 0, 0], 2).contains("long items"));
        assert!(message(&[0xfe], 2).contains("long items"));
    }

    #[test]
    fn requires_balanced_globals_with_a_bounded_stack() {
        assert!(message(&[0xb4], 2).contains("matching Push"));
        assert!(message(&[0xa4], 2).contains("matching Pop"));
        assert!(message(&[0xa5, 0], 2).contains("Push must have no payload"));
        assert!(message(&[0xb5, 0], 2).contains("Pop must have no payload"));
        assert!(message(&[0xa4; 17], 2).contains("exceeds 16"));

        let mut descriptor = vec![0x85, 2, 0x75, 8, 0x95, 1];
        descriptor.extend_from_slice(&[0xa4; 16]);
        descriptor.extend_from_slice(&[0xb4; 16]);
        descriptor.extend_from_slice(&[0x91, 2]);
        assert_eq!(output_report_length(&descriptor, 2).unwrap(), 2);
    }

    #[test]
    fn requires_balanced_collections_with_correct_item_sizes() {
        assert!(message(&[0xc0], 2).contains("matching Collection"));
        assert!(message(&[0xa1, 1], 2).contains("matching End Collection"));
        assert!(message(&[0xa0], 2).contains("one-byte payload"));
        assert!(message(&[0xc1, 0], 2).contains("no payload"));
    }

    #[test]
    fn rejects_missing_outputs_and_oversized_descriptors() {
        assert!(message(&[], 2).contains("is absent"));
        assert!(message(&[0x85, 2, 0x75, 8, 0x95, 1, 0x81, 2], 2).contains("is absent"));
        assert!(message(&vec![0; 4097], 2).contains("exceeds 4096"));
    }

    #[test]
    fn checks_the_entire_descriptor_after_finding_the_requested_output() {
        let mut descriptor = vec![0x85, 2, 0x75, 8, 0x95, 1, 0x91, 2];
        descriptor.push(0x76);
        assert!(message(&descriptor, 2).contains("truncated"));
    }

    #[test]
    fn accepts_the_largest_report_and_rejects_combined_overflow() {
        let mut descriptor = vec![
            0x85, 2, 0x75, 8, 0x96, 0xfe, 0xff, 0x91, 2, // 65534 payload bytes
        ];
        assert_eq!(output_report_length(&descriptor, 2).unwrap(), 65535);
        descriptor.extend_from_slice(&[0x95, 1, 0x91, 1]);
        assert!(message(&descriptor, 2).contains("combined fields"));
    }

    #[test]
    fn rejects_oversized_fields_before_arithmetic_or_allocation() {
        let descriptor = [0x85, 2, 0x76, 0xff, 0xff, 0x96, 0xff, 0xff, 0x91, 2];
        assert!(message(&descriptor, 2).contains("data field exceeds"));
        assert!(message(&[0x77, 0xff, 0xff, 0xff, 0xff], 2).contains("Report Size"));
        assert!(message(&[0x97, 0xff, 0xff, 0xff, 0xff], 2).contains("Report Count"));
    }
}
