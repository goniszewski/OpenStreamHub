//! Pure M131E IPS dial input parsing derived from MasterHUB 1.0.0.113.
//!
//! The native SDK supplies the outer framing; the application's renderer supplies
//! the control interpretation. These formats have not been verified on hardware.
//! The application's RIGHT/LEFT labels do not establish physical clockwise or
//! counter-clockwise direction. Complete payload lengths and the meaning of byte
//! 1 remain unknown, so parsers retain all declared payload bytes.
//!
//! Source provenance is recorded in the repository's
//! `docs/research/masterhub-1.0.0.113-static-analysis.md`, under "Dial input".

use std::fmt;

/// A malformed M131E input report whose known fields are incomplete.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum M131EInputError {
    /// An outer report header is shorter than its recognized layout requires.
    ShortHeader {
        /// Minimum header length required.
        required: usize,
        /// Number of bytes actually received.
        received: usize,
    },
    /// The outer report declares more payload bytes than were received.
    TruncatedPayload {
        /// Payload length declared by the report.
        declared: usize,
        /// Number of payload bytes actually available.
        available: usize,
    },
    /// A known logical control selector lacks fields read by the application.
    ShortControlPayload {
        /// Minimum payload length required for this interpretation.
        required: usize,
        /// Number of declared payload bytes actually present.
        received: usize,
    },
}

impl fmt::Display for M131EInputError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ShortHeader { required, received } => write!(
                formatter,
                "M131E input header needs {required} bytes, but received {received}"
            ),
            Self::TruncatedPayload {
                declared,
                available,
            } => write!(
                formatter,
                "M131E input payload declares {declared} bytes, but only {available} are available"
            ),
            Self::ShortControlPayload { required, received } => write!(
                formatter,
                "M131E control payload needs {required} bytes, but received {received}"
            ),
        }
    }
}

impl std::error::Error for M131EInputError {}

/// A recognized M131E outer frame or an uninterpreted raw HID report.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum M131EReport<'a> {
    /// A length-prefixed report belonging to queue 0, 1, or 3.
    Known {
        /// SDK queue index derived from class `0x44`, `0x45`, or `0x47`.
        queue: u8,
        /// All declared payload bytes, excluding the header and HID padding.
        payload: &'a [u8],
    },
    /// An unsupported report ID, class, or layout.
    Unknown {
        /// Entire received report retained without interpretation.
        raw: &'a [u8],
    },
}

/// Parses the software-derived M131E outer HID framing.
///
/// Report ID `0x01` uses a class at offset 1. Classes `0x44`, `0x45`, and
/// `0x47` contain a little-endian payload length at offsets 2–3 and payload at
/// offset 4. Class `0x46` has a different layout and is retained as unknown.
/// Padding after the declared payload is ignored. These bytes do not identify
/// the physical module; the caller must establish the source independently.
///
/// # Errors
///
/// Returns [`M131EInputError`] for incomplete known headers and payloads.
pub fn parse_m131e_report(raw: &[u8]) -> Result<M131EReport<'_>, M131EInputError> {
    let Some(&report_id) = raw.first() else {
        return Err(M131EInputError::ShortHeader {
            required: 1,
            received: 0,
        });
    };
    if report_id != 0x01 {
        return Ok(M131EReport::Unknown { raw });
    }
    let Some(&class) = raw.get(1) else {
        return Err(M131EInputError::ShortHeader {
            required: 2,
            received: raw.len(),
        });
    };
    let queue = match class {
        0x44 | 0x45 | 0x47 => class - 0x44,
        _ => return Ok(M131EReport::Unknown { raw }),
    };
    if raw.len() < 4 {
        return Err(M131EInputError::ShortHeader {
            required: 4,
            received: raw.len(),
        });
    }
    let declared = u16::from_le_bytes([raw[2], raw[3]]) as usize;
    let available = raw.len() - 4;
    if declared > available {
        return Err(M131EInputError::TruncatedPayload {
            declared,
            available,
        });
    }
    Ok(M131EReport::Known {
        queue,
        payload: &raw[4..4 + declared],
    })
}

/// A rotation label used by the official application for the M131E dial.
///
/// Physical clockwise/counter-clockwise mapping requires hardware observation.
/// The M003K knob uses a different direction-bit convention.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DialDirection {
    /// Application RIGHT action (`0x00` at payload offset 2).
    Right,
    /// Application LEFT action (`0x01` at payload offset 2).
    Left,
}

/// A logical queue-0 M131E payload interpreted by the official application.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum M131EPayload<'a> {
    /// One rotation update, retaining ignored and extra payload bytes.
    Rotation {
        /// Application direction label, independent of physical orientation.
        direction: DialDirection,
        /// The complete declared logical payload.
        payload: &'a [u8],
    },
    /// A button state update, retaining ignored and extra payload bytes.
    Button {
        /// Whether the update reports the dial pressed.
        pressed: bool,
        /// The complete declared logical payload.
        payload: &'a [u8],
    },
    /// A selector or value not established by the software trace.
    Unknown(&'a [u8]),
}

/// Interprets the known fields of a logical M131E queue-0 payload.
///
/// Selectors `0x03` and legacy `0x00` are rotations; `0x04` is button state.
/// Legacy selector `0x01` is accepted only for release (`payload[2] == 0`).
/// Byte 1 and bytes after byte 2 are preserved without imposing a meaning or
/// exact payload length. Unknown selectors and values are retained. Do not
/// pass an outer HID frame to this function.
///
/// # Errors
///
/// Returns [`M131EInputError`] for an empty payload or a known selector whose
/// payload is shorter than the three fields accessed by the application.
pub fn parse_m131e_payload(payload: &[u8]) -> Result<M131EPayload<'_>, M131EInputError> {
    let Some(&selector) = payload.first() else {
        return Err(M131EInputError::ShortControlPayload {
            required: 1,
            received: 0,
        });
    };
    if !matches!(selector, 0x00 | 0x01 | 0x03 | 0x04) {
        return Ok(M131EPayload::Unknown(payload));
    }
    if payload.len() < 3 {
        return Err(M131EInputError::ShortControlPayload {
            required: 3,
            received: payload.len(),
        });
    }
    let value = payload[2];
    match (selector, value) {
        (0x00 | 0x03, 0x00) => Ok(M131EPayload::Rotation {
            direction: DialDirection::Right,
            payload,
        }),
        (0x00 | 0x03, 0x01) => Ok(M131EPayload::Rotation {
            direction: DialDirection::Left,
            payload,
        }),
        (0x04, 0x01) => Ok(M131EPayload::Button {
            pressed: true,
            payload,
        }),
        (0x01 | 0x04, 0x00) => Ok(M131EPayload::Button {
            pressed: false,
            payload,
        }),
        _ => Ok(M131EPayload::Unknown(payload)),
    }
}

/// A normalized M131E dial input event.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum M131EEvent {
    /// One application rotation update; no distance or count is established.
    Rotated(DialDirection),
    /// The dial changed from released to pressed.
    Pressed,
    /// The dial changed from pressed to released.
    Released,
}

/// The result of decoding a raw M131E HID report.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DecodedM131E<'a> {
    /// Zero or one normalized event; callers can retain the original raw report.
    Events(Vec<M131EEvent>),
    /// Unsupported report retained in full, including its outer header.
    Unknown(&'a [u8]),
}

/// Stateful button-transition decoder for one connected M131E module.
///
/// A new decoder assumes released. Rotation updates are always emitted,
/// including identical consecutive updates. Unknown and malformed reports do
/// not change state. Reset or replace the decoder after reconnecting.
///
/// The official application's 600 ms long-press decision is host policy; this
/// decoder emits only the transitions established by the input bytes.
#[derive(Clone, Debug, Default)]
pub struct M131EDecoder {
    pressed: bool,
}

impl M131EDecoder {
    /// Creates a decoder whose dial button is assumed released.
    pub fn new() -> Self {
        Self::default()
    }

    /// Forgets button state without emitting a synthetic release event.
    ///
    /// Call after disconnect/reconnect. Consumers should clear held actions
    /// separately when their device disconnects.
    pub fn reset(&mut self) {
        self.pressed = false;
    }

    /// Returns the last successfully decoded button state.
    pub fn is_pressed(&self) -> bool {
        self.pressed
    }

    /// Decodes a raw report from a caller-identified M131E module.
    ///
    /// Only queue 0 (`0x44`) produces input events. Other queue layouts are
    /// preserved without applying queue-0 assumptions. Device identity must be
    /// established independently of these bytes.
    ///
    /// # Errors
    ///
    /// Returns [`M131EInputError`] for malformed known queue-0 reports without
    /// modifying button state.
    pub fn decode_report<'a>(
        &mut self,
        raw: &'a [u8],
    ) -> Result<DecodedM131E<'a>, M131EInputError> {
        if raw.first() == Some(&0x01) && raw.get(1).is_some_and(|&class| class != 0x44) {
            return Ok(DecodedM131E::Unknown(raw));
        }
        let payload = match parse_m131e_report(raw)? {
            M131EReport::Known { queue: 0, payload } => payload,
            _ => return Ok(DecodedM131E::Unknown(raw)),
        };
        let event = match parse_m131e_payload(payload)? {
            M131EPayload::Rotation { direction, .. } => Some(M131EEvent::Rotated(direction)),
            M131EPayload::Button { pressed, .. } => {
                let event = (pressed != self.pressed).then_some(if pressed {
                    M131EEvent::Pressed
                } else {
                    M131EEvent::Released
                });
                self.pressed = pressed;
                event
            }
            M131EPayload::Unknown(_) => return Ok(DecodedM131E::Unknown(raw)),
        };
        Ok(DecodedM131E::Events(event.into_iter().collect()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // These are synthetic field examples, not hardware captures. The full actual
    // payload length and meaning of byte 1 have not been established.
    fn frame(class: u8, payload: &[u8]) -> Vec<u8> {
        let length = u16::try_from(payload.len()).unwrap().to_le_bytes();
        let mut raw = vec![0x01, class, length[0], length[1]];
        raw.extend_from_slice(payload);
        raw
    }

    fn control(payload: &[u8]) -> Vec<u8> {
        frame(0x44, payload)
    }

    #[test]
    fn known_outer_queues_preserve_declared_bytes_and_ignore_padding() {
        for (class, queue) in [(0x44, 0), (0x45, 1), (0x47, 3)] {
            let mut raw = frame(class, &[0x03, 0xaa, 0, 0xbb, 0xcc]);
            raw.extend_from_slice(&[0x04, 0, 1]);
            assert_eq!(
                parse_m131e_report(&raw),
                Ok(M131EReport::Known {
                    queue,
                    payload: &[0x03, 0xaa, 0, 0xbb, 0xcc],
                })
            );
        }
    }

    #[test]
    fn length_is_little_endian_and_can_exceed_one_byte() {
        let raw = frame(0x44, &[0xaa; 256]);
        assert!(matches!(
            parse_m131e_report(&raw),
            Ok(M131EReport::Known { payload, .. }) if payload.len() == 256
        ));
    }

    #[test]
    fn incomplete_outer_headers_and_declared_payloads_are_rejected() {
        for raw in [&[][..], &[1][..], &[1, 0x44][..], &[1, 0x44, 3][..]] {
            assert!(matches!(
                parse_m131e_report(raw),
                Err(M131EInputError::ShortHeader { .. })
            ));
        }
        assert_eq!(
            parse_m131e_report(&[1, 0x44, 3, 0, 0x04, 0]),
            Err(M131EInputError::TruncatedPayload {
                declared: 3,
                available: 2,
            })
        );
        assert_eq!(
            parse_m131e_report(&[1, 0x44, 0xff, 0xff]),
            Err(M131EInputError::TruncatedPayload {
                declared: 65535,
                available: 0,
            })
        );
    }

    #[test]
    fn unknown_outer_reports_do_not_apply_length_framing() {
        for raw in [
            &[2][..],
            &[1, 0x46][..],
            &[1, 0x46, 0xff, 0xff][..],
            &[1, 0x77][..],
        ] {
            assert_eq!(parse_m131e_report(raw), Ok(M131EReport::Unknown { raw }));
        }
    }

    #[test]
    fn current_and_legacy_rotation_selectors_use_application_direction_labels() {
        for selector in [0x03, 0x00] {
            for (value, direction) in [(0, DialDirection::Right), (1, DialDirection::Left)] {
                let payload = [selector, 0xfa, value, 0xbb];
                assert_eq!(
                    parse_m131e_payload(&payload),
                    Ok(M131EPayload::Rotation {
                        direction,
                        payload: &payload,
                    })
                );
            }
        }
    }

    #[test]
    fn button_selector_and_legacy_release_preserve_payload() {
        for (selector, value, pressed) in [(4, 1, true), (4, 0, false), (1, 0, false)] {
            let payload = [selector, 0xfe, value, 0xaa, 0xbb];
            assert_eq!(
                parse_m131e_payload(&payload),
                Ok(M131EPayload::Button {
                    pressed,
                    payload: &payload,
                })
            );
        }
        let legacy_press = [1, 0, 1];
        assert_eq!(
            parse_m131e_payload(&legacy_press),
            Ok(M131EPayload::Unknown(&legacy_press))
        );
    }

    #[test]
    fn known_control_selectors_require_only_fields_actually_read() {
        for selector in [0x00, 0x01, 0x03, 0x04] {
            for payload in [&[selector][..], &[selector, 0xff][..]] {
                assert_eq!(
                    parse_m131e_payload(payload),
                    Err(M131EInputError::ShortControlPayload {
                        required: 3,
                        received: payload.len(),
                    })
                );
            }
        }
        assert_eq!(
            parse_m131e_payload(&[]),
            Err(M131EInputError::ShortControlPayload {
                required: 1,
                received: 0,
            })
        );
    }

    #[test]
    fn unknown_selectors_and_values_are_retained() {
        for payload in [
            &[0xaa][..],
            &[0x03, 0, 2][..],
            &[0x00, 0, 0xff][..],
            &[0x04, 0, 0xff][..],
            &[0x01, 0, 1][..],
        ] {
            assert_eq!(
                parse_m131e_payload(payload),
                Ok(M131EPayload::Unknown(payload))
            );
        }
    }

    #[test]
    fn repeated_rotation_reports_always_emit() {
        let mut decoder = M131EDecoder::new();
        for (value, direction) in [(0, DialDirection::Right), (1, DialDirection::Left)] {
            let raw = control(&[3, 0, value]);
            let expected = DecodedM131E::Events(vec![M131EEvent::Rotated(direction)]);
            assert_eq!(decoder.decode_report(&raw), Ok(expected.clone()));
            assert_eq!(decoder.decode_report(&raw), Ok(expected));
        }
        assert!(!decoder.is_pressed());
    }

    #[test]
    fn button_updates_emit_only_state_transitions_including_legacy_release() {
        let mut decoder = M131EDecoder::new();
        let press = control(&[4, 0, 1]);
        let release = control(&[4, 0, 0]);
        let legacy_release = control(&[1, 0, 0]);
        assert_eq!(
            decoder.decode_report(&release),
            Ok(DecodedM131E::Events(vec![]))
        );
        assert_eq!(
            decoder.decode_report(&press),
            Ok(DecodedM131E::Events(vec![M131EEvent::Pressed]))
        );
        assert!(decoder.is_pressed());
        assert_eq!(
            decoder.decode_report(&press),
            Ok(DecodedM131E::Events(vec![]))
        );
        assert_eq!(
            decoder.decode_report(&legacy_release),
            Ok(DecodedM131E::Events(vec![M131EEvent::Released]))
        );
        assert!(!decoder.is_pressed());
        assert_eq!(
            decoder.decode_report(&release),
            Ok(DecodedM131E::Events(vec![]))
        );
    }

    #[test]
    fn unknown_reports_and_malformed_payloads_do_not_mutate_state() {
        let mut decoder = M131EDecoder::new();
        decoder.decode_report(&control(&[4, 0, 1])).unwrap();
        for raw in [
            control(&[4, 0, 2]),
            control(&[1, 0, 1]),
            control(&[0xaa]),
            vec![2],
            vec![1, 0x45, 0xff, 0xff],
            vec![1, 0x46],
            vec![1, 0x47],
        ] {
            assert_eq!(decoder.decode_report(&raw), Ok(DecodedM131E::Unknown(&raw)));
            assert!(decoder.is_pressed());
        }
        for raw in [control(&[4, 0]), vec![1, 0x44, 3, 0, 4, 0]] {
            assert!(decoder.decode_report(&raw).is_err());
            assert!(decoder.is_pressed());
        }
        assert_eq!(
            decoder.decode_report(&control(&[4, 0, 0])),
            Ok(DecodedM131E::Events(vec![M131EEvent::Released]))
        );
    }

    #[test]
    fn reset_forgets_pressed_state_without_generating_release() {
        let mut decoder = M131EDecoder::new();
        decoder.decode_report(&control(&[4, 0, 1])).unwrap();
        decoder.reset();
        assert!(!decoder.is_pressed());
        assert_eq!(
            decoder.decode_report(&control(&[4, 0, 0])),
            Ok(DecodedM131E::Events(vec![]))
        );
        assert_eq!(
            decoder.decode_report(&control(&[4, 0, 1])),
            Ok(DecodedM131E::Events(vec![M131EEvent::Pressed]))
        );
    }
}
