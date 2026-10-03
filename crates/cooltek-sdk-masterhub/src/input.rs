//! Pure input parsing for M003K knobs and the M131E IPS dial.
//!
//! These formats come from the public `cooltek-sdk-probe` protocol notes and
//! have not yet been verified with this SDK on real hardware. The M131E formats
//! come from static analysis of official software 1.0.0.113 and SDK 03.02.00.
//! Pass only the bytes actually
//! received from HIDAPI, including the outer report ID, to [`parse_m0_report`]
//! or [`M003KDecoder::decode_report`].
//!
//! Protocol references:
//! - <https://github.com/projectzero7/cooltek-sdk-probe/blob/main/src/m0.rs>
//! - <https://github.com/projectzero7/cooltek-sdk-probe/blob/main/docs/MasterHUB/modules/m003k/input_reports.md>

use std::fmt;

mod m131e;
pub use m131e::*;

/// A malformed report whose known fields could not be safely decoded.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InputError {
    /// A known report header is shorter than required.
    ShortHeader {
        /// Minimum number of header bytes required.
        required: usize,
        /// Number of bytes actually received.
        received: usize,
    },
    /// A framed payload claims more bytes than were actually received.
    TruncatedPayload {
        /// Payload length declared by the report.
        declared: usize,
        /// Number of payload bytes actually available.
        available: usize,
    },
    /// A known M003K payload has an unexpected control count or byte length.
    InvalidKnobLength {
        /// Number of controls declared by the logical payload.
        declared: u8,
        /// Number of bytes actually present after its two-byte header.
        actual: usize,
    },
    /// A known control report contains an undocumented value.
    InvalidKnobValue {
        /// Logical report ID: rotation (`0x00`) or press (`0x03`).
        report_id: u8,
        /// Zero-based index of the control containing the invalid value.
        knob_index: u8,
        /// Unrecognized raw value.
        value: u8,
    },
}

impl fmt::Display for InputError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ShortHeader { required, received } => write!(
                formatter,
                "input header needs {required} bytes, but received {received}"
            ),
            Self::TruncatedPayload {
                declared,
                available,
            } => write!(
                formatter,
                "input payload declares {declared} bytes, but only {available} are available"
            ),
            Self::InvalidKnobLength { declared, actual } => write!(
                formatter,
                "M003K payload must declare and contain 3 control bytes (declared {declared}, actual {actual})"
            ),
            Self::InvalidKnobValue {
                report_id,
                knob_index,
                value,
            } => write!(
                formatter,
                "M003K report 0x{report_id:02x}, knob {knob_index}: unknown value 0x{value:02x}"
            ),
        }
    }
}

impl std::error::Error for InputError {}

/// A recognized outer M0 frame or an uninterpreted raw HID report.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum M0Report<'a> {
    /// A report with the documented length-prefixed channel layout.
    Known {
        /// Zero-based channel index (0, 1, 2, or 4).
        channel: u8,
        /// Logical payload without the outer header or trailing HID padding.
        payload: &'a [u8],
    },
    /// An unsupported report ID, channel tag, or channel layout.
    Unknown {
        /// Entire received report, retained without interpretation.
        raw: &'a [u8],
    },
}

/// Parses a raw HID report using the documented M0 channel framing.
///
/// Report `0x01` uses a channel tag at byte 1 and, for supported channels,
/// a little-endian payload length at bytes 2 and 3. Padding after the declared
/// payload is ignored. Channel 3 (`0x2d`) has a different layout in the probe
/// and is retained as unknown rather than interpreted as length-prefixed.
///
/// This does not identify which physical module produced a report.
///
/// # Errors
///
/// Returns [`InputError`] for truncated headers or payloads of known layouts.
pub fn parse_m0_report(raw: &[u8]) -> Result<M0Report<'_>, InputError> {
    let Some(&report_id) = raw.first() else {
        return Err(InputError::ShortHeader {
            required: 1,
            received: 0,
        });
    };
    if report_id != 0x01 {
        return Ok(M0Report::Unknown { raw });
    }

    let Some(&tag) = raw.get(1) else {
        return Err(InputError::ShortHeader {
            required: 2,
            received: raw.len(),
        });
    };
    let channel = match tag {
        0x2a..=0x2c | 0x2e => tag - 0x2a,
        _ => return Ok(M0Report::Unknown { raw }),
    };
    if raw.len() < 4 {
        return Err(InputError::ShortHeader {
            required: 4,
            received: raw.len(),
        });
    }

    let declared = u16::from_le_bytes([raw[2], raw[3]]) as usize;
    let available = raw.len() - 4;
    if declared > available {
        return Err(InputError::TruncatedPayload {
            declared,
            available,
        });
    }

    Ok(M0Report::Known {
        channel,
        payload: &raw[4..4 + declared],
    })
}

/// Direction of one documented M003K rotation update.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RotationDirection {
    /// Clockwise (`0x01` in the logical payload).
    Clockwise,
    /// Counter-clockwise (`0x00` in the logical payload).
    CounterClockwise,
}

/// A decoded logical M003K channel-0 payload.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum M003KReport<'a> {
    /// Directions for the three knobs; `None` denotes `0xff` (no change).
    Rotation([Option<RotationDirection>; 3]),
    /// Complete pressed/released state of all three knobs.
    PressSnapshot([bool; 3]),
    /// A logical report kind not described in the protocol notes.
    Unknown(&'a [u8]),
}

/// Parses a logical M003K payload after removing the outer M0 frame.
///
/// Rotation (`0x00`) and press (`0x03`) reports contain a count of three,
/// followed by exactly three control bytes. Unknown report IDs are preserved.
/// Do not pass an outer raw HID report to this function.
///
/// # Errors
///
/// Returns [`InputError`] for incomplete or invalid known payloads. No state is
/// modified, including when an invalid value appears after valid values.
pub fn parse_m003k_payload(payload: &[u8]) -> Result<M003KReport<'_>, InputError> {
    let Some(&report_id) = payload.first() else {
        return Err(InputError::ShortHeader {
            required: 1,
            received: 0,
        });
    };
    if !matches!(report_id, 0x00 | 0x03) {
        return Ok(M003KReport::Unknown(payload));
    }
    if payload.len() < 2 {
        return Err(InputError::ShortHeader {
            required: 2,
            received: payload.len(),
        });
    }
    if payload[1] != 3 || payload.len() != 5 {
        return Err(InputError::InvalidKnobLength {
            declared: payload[1],
            actual: payload.len() - 2,
        });
    }

    let invalid_value = |knob_index: usize, value: u8| InputError::InvalidKnobValue {
        report_id,
        knob_index: knob_index as u8,
        value,
    };
    if report_id == 0x00 {
        let mut directions = [None; 3];
        for (index, &value) in payload[2..].iter().enumerate() {
            directions[index] = match value {
                0x00 => Some(RotationDirection::CounterClockwise),
                0x01 => Some(RotationDirection::Clockwise),
                0xff => None,
                _ => return Err(invalid_value(index, value)),
            };
        }
        Ok(M003KReport::Rotation(directions))
    } else {
        let mut pressed = [false; 3];
        for (index, &value) in payload[2..].iter().enumerate() {
            pressed[index] = match value {
                0x00 => false,
                0x01 => true,
                _ => return Err(invalid_value(index, value)),
            };
        }
        Ok(M003KReport::PressSnapshot(pressed))
    }
}

/// An action reported by one M003K knob.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KnobEventKind {
    /// One rotation update; the protocol does not include a distance or count.
    Rotated(RotationDirection),
    /// The knob changed from released to pressed.
    Pressed,
    /// The knob changed from pressed to released.
    Released,
}

/// An input event for one zero-based M003K control index.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KnobEvent {
    /// Index 0, 1, or 2, matching the order of control bytes on the wire.
    ///
    /// Physical left/right placement still needs verification on the device.
    pub knob_index: u8,
    /// Rotation update or button transition.
    pub kind: KnobEventKind,
}

/// The result of decoding a raw M003K HID report.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DecodedM003K<'a> {
    /// Zero or more events in ascending control-index order.
    Events(Vec<KnobEvent>),
    /// An unsupported report, preserved in full including its outer header.
    Unknown(&'a [u8]),
}

/// Stateful press-transition decoder for one connected M003K module.
///
/// A new decoder assumes all controls are released. Its first pressed snapshot
/// therefore produces press events. Use a separate decoder per physical module
/// and reset or replace it after reconnecting. Invalid and unknown reports do
/// not modify the press state. Rotation reports are always emitted, including
/// identical consecutive reports.
#[derive(Clone, Debug, Default)]
pub struct M003KDecoder {
    pressed: [bool; 3],
}

impl M003KDecoder {
    /// Creates a decoder with all controls assumed released.
    pub fn new() -> Self {
        Self::default()
    }

    /// Forgets button state without emitting artificial release events.
    ///
    /// Call on disconnect/reconnect; action consumers should clear any held
    /// actions separately when a device disconnects.
    pub fn reset(&mut self) {
        self.pressed = [false; 3];
    }

    /// Returns the last successfully decoded press snapshot.
    pub fn pressed_states(&self) -> [bool; 3] {
        self.pressed
    }

    /// Decodes a complete raw report from the M003K module.
    ///
    /// Only the documented channel-0 formats generate knob events. Reports
    /// from other channels or unknown report kinds are retained unchanged.
    /// The caller must establish that the source is an M003K, for example by
    /// matching product ID `0x0202`; the bytes alone do not identify the module.
    ///
    /// # Errors
    ///
    /// Returns [`InputError`] for malformed known reports without changing state.
    pub fn decode_report<'a>(&mut self, raw: &'a [u8]) -> Result<DecodedM003K<'a>, InputError> {
        // This decoder only knows the channel-0 payload. Retain other channels
        // without imposing channel-0 assumptions on their contents or length.
        if raw.first() == Some(&0x01) && raw.get(1).is_some_and(|&tag| tag != 0x2a) {
            return Ok(DecodedM003K::Unknown(raw));
        }
        let payload = match parse_m0_report(raw)? {
            M0Report::Known {
                channel: 0,
                payload,
            } => payload,
            _ => return Ok(DecodedM003K::Unknown(raw)),
        };

        let mut events = Vec::with_capacity(3);
        match parse_m003k_payload(payload)? {
            M003KReport::Rotation(directions) => {
                for (index, direction) in directions.into_iter().enumerate() {
                    if let Some(direction) = direction {
                        events.push(KnobEvent {
                            knob_index: index as u8,
                            kind: KnobEventKind::Rotated(direction),
                        });
                    }
                }
            }
            M003KReport::PressSnapshot(pressed) => {
                for (index, is_pressed) in pressed.into_iter().enumerate() {
                    if is_pressed != self.pressed[index] {
                        events.push(KnobEvent {
                            knob_index: index as u8,
                            kind: if is_pressed {
                                KnobEventKind::Pressed
                            } else {
                                KnobEventKind::Released
                            },
                        });
                    }
                }
                self.pressed = pressed;
            }
            M003KReport::Unknown(_) => return Ok(DecodedM003K::Unknown(raw)),
        }
        Ok(DecodedM003K::Events(events))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // All packets below are synthetic protocol examples, not hardware captures.
    fn frame(payload: &[u8]) -> Vec<u8> {
        let length = u16::try_from(payload.len()).unwrap().to_le_bytes();
        let mut raw = vec![0x01, 0x2a, length[0], length[1]];
        raw.extend_from_slice(payload);
        raw
    }

    fn event(knob_index: u8, kind: KnobEventKind) -> KnobEvent {
        KnobEvent { knob_index, kind }
    }

    #[test]
    fn outer_frame_respects_received_length_and_ignores_padding() {
        let mut raw = frame(&[0x00, 3, 1, 0xff, 0xff]);
        raw.extend_from_slice(&[0x03, 3, 1, 1, 1]);
        assert_eq!(
            parse_m0_report(&raw),
            Ok(M0Report::Known {
                channel: 0,
                payload: &[0x00, 3, 1, 0xff, 0xff],
            })
        );
        assert_eq!(
            parse_m0_report(&raw[..8]),
            Err(InputError::TruncatedPayload {
                declared: 5,
                available: 4,
            })
        );
        assert!(matches!(
            parse_m0_report(&[1, 0x2a, 0xff, 0xff]),
            Err(InputError::TruncatedPayload {
                declared: 65535,
                ..
            })
        ));
    }

    #[test]
    fn outer_length_is_little_endian_and_not_limited_to_one_byte() {
        let raw = frame(&[0xaa; 256]);
        assert!(matches!(
            parse_m0_report(&raw),
            Ok(M0Report::Known { payload, .. }) if payload.len() == 256
        ));
    }

    #[test]
    fn short_known_headers_return_errors() {
        for raw in [&[][..], &[1][..], &[1, 0x2a][..], &[1, 0x2a, 5][..]] {
            assert!(matches!(
                parse_m0_report(raw),
                Err(InputError::ShortHeader { .. })
            ));
        }
        for payload in [&[][..], &[0][..], &[3][..]] {
            assert!(matches!(
                parse_m003k_payload(payload),
                Err(InputError::ShortHeader { .. })
            ));
        }
    }

    #[test]
    fn unknown_outer_reports_and_channel_three_are_retained_without_framing() {
        for raw in [&[2][..], &[1, 0x7f][..], &[1, 0x2d, 0xff, 0xff][..]] {
            assert_eq!(parse_m0_report(raw), Ok(M0Report::Unknown { raw }));
        }
    }

    #[test]
    fn logical_decoder_is_separate_from_raw_hid_framing() {
        let payload = [0x00, 3, 0, 1, 0xff];
        assert_eq!(
            parse_m003k_payload(&payload),
            Ok(M003KReport::Rotation([
                Some(RotationDirection::CounterClockwise),
                Some(RotationDirection::Clockwise),
                None,
            ]))
        );
        let raw = frame(&payload);
        assert_eq!(parse_m003k_payload(&raw), Ok(M003KReport::Unknown(&raw)));
    }

    #[test]
    fn known_logical_lengths_are_strict() {
        for payload in [
            &[0, 2, 0, 1][..],
            &[0, 3, 0, 1][..],
            &[3, 3, 0, 1, 0, 0][..],
            &[3, 255, 0, 1, 0][..],
        ] {
            assert!(matches!(
                parse_m003k_payload(payload),
                Err(InputError::InvalidKnobLength { .. })
            ));
        }
    }

    #[test]
    fn all_rotation_controls_and_repeated_reports_generate_events() {
        let mut decoder = M003KDecoder::new();
        let raw = frame(&[0, 3, 1, 0, 1]);
        let expected = DecodedM003K::Events(vec![
            event(0, KnobEventKind::Rotated(RotationDirection::Clockwise)),
            event(
                1,
                KnobEventKind::Rotated(RotationDirection::CounterClockwise),
            ),
            event(2, KnobEventKind::Rotated(RotationDirection::Clockwise)),
        ]);
        assert_eq!(decoder.decode_report(&raw), Ok(expected.clone()));
        assert_eq!(decoder.decode_report(&raw), Ok(expected));
        assert_eq!(
            decoder.decode_report(&frame(&[0, 3, 0xff, 0xff, 0xff])),
            Ok(DecodedM003K::Events(Vec::new()))
        );
    }

    #[test]
    fn press_snapshots_emit_only_transitions_for_every_control() {
        let mut decoder = M003KDecoder::new();
        assert_eq!(
            decoder.decode_report(&frame(&[3, 3, 1, 1, 1])),
            Ok(DecodedM003K::Events(vec![
                event(0, KnobEventKind::Pressed),
                event(1, KnobEventKind::Pressed),
                event(2, KnobEventKind::Pressed),
            ]))
        );
        assert_eq!(
            decoder.decode_report(&frame(&[3, 3, 1, 1, 1])),
            Ok(DecodedM003K::Events(Vec::new()))
        );
        assert_eq!(
            decoder.decode_report(&frame(&[3, 3, 0, 0, 0])),
            Ok(DecodedM003K::Events(vec![
                event(0, KnobEventKind::Released),
                event(1, KnobEventKind::Released),
                event(2, KnobEventKind::Released),
            ]))
        );
    }

    #[test]
    fn malformed_snapshot_never_partially_changes_state() {
        let mut decoder = M003KDecoder::new();
        decoder.decode_report(&frame(&[3, 3, 1, 0, 0])).unwrap();
        assert_eq!(
            decoder.decode_report(&frame(&[3, 3, 0, 1, 2])),
            Err(InputError::InvalidKnobValue {
                report_id: 3,
                knob_index: 2,
                value: 2,
            })
        );
        assert_eq!(decoder.pressed_states(), [true, false, false]);
        assert_eq!(
            decoder.decode_report(&frame(&[3, 3, 0, 0, 0])),
            Ok(DecodedM003K::Events(vec![event(
                0,
                KnobEventKind::Released
            )]))
        );
    }

    #[test]
    fn unsupported_channels_and_kinds_retain_raw_and_do_not_change_state() {
        let mut decoder = M003KDecoder::new();
        decoder.decode_report(&frame(&[3, 3, 1, 0, 0])).unwrap();
        let mut raw = frame(&[3, 3, 0, 1, 1]);
        for tag in [0x2b, 0x2c, 0x2d, 0x2e, 0x99] {
            raw[1] = tag;
            assert_eq!(decoder.decode_report(&raw), Ok(DecodedM003K::Unknown(&raw)));
            let short_unknown = [1, tag, 0xff, 0xff];
            assert_eq!(
                decoder.decode_report(&short_unknown),
                Ok(DecodedM003K::Unknown(&short_unknown))
            );
        }
        let unknown_kind = frame(&[0x77, 3, 0, 1, 1]);
        assert_eq!(
            decoder.decode_report(&unknown_kind),
            Ok(DecodedM003K::Unknown(&unknown_kind))
        );
        assert_eq!(decoder.pressed_states(), [true, false, false]);
    }

    #[test]
    fn invalid_rotation_is_rejected_and_reset_is_local() {
        assert!(matches!(
            parse_m003k_payload(&[0, 3, 1, 2, 0xff]),
            Err(InputError::InvalidKnobValue {
                report_id: 0,
                knob_index: 1,
                value: 2
            })
        ));
        let mut first = M003KDecoder::new();
        let second = M003KDecoder::new();
        first.decode_report(&frame(&[3, 3, 1, 1, 1])).unwrap();
        assert_eq!(second.pressed_states(), [false; 3]);
        first.reset();
        assert_eq!(first.pressed_states(), [false; 3]);
    }
}
