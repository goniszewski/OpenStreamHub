//! Two-roller updates from pinned probe notes and official renderer code.
//!
//! Direction updates have no recovered magnitude or tick count. The app calls
//! wire index zero its right roller and index one its left roller; physical
//! orientation and direction need hardware verification.
//!
//! Probe notes: <https://github.com/projectzero7/cooltek-sdk-probe/blob/0e72f87547537193309c3a5f2eae9ef483d80d7f/docs/MasterHUB/modules/m032le/input_reports.md>.

use super::{InputError, channel_zero_payload, control_bytes};

/// Application direction label for one roller update.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RollerDirection {
    /// Application up action, raw value zero.
    Up,
    /// Application down action, raw value one.
    Down,
}

/// A logical roller update or an unsupported payload.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum M032LEReport<'a> {
    /// Direction updates in wire order; `None` corresponds to `ff` (no update).
    Rotation([Option<RollerDirection>; 2]),
    /// Unsupported selector retained without interpretation.
    Unknown(&'a [u8]),
}

/// Parses a logical roller payload, excluding outer HID framing and padding.
///
/// Selector one, count two, and direction values zero/one/`ff` are supported.
/// No distance, continuous position, or press gesture is inferred.
///
/// # Errors
/// Returns an error for an incomplete or malformed recognized update.
pub fn parse_m032le_payload(payload: &[u8]) -> Result<M032LEReport<'_>, InputError> {
    let Some(&selector) = payload.first() else {
        return Err(InputError::ShortHeader {
            required: 1,
            received: 0,
        });
    };
    if selector != 0x01 {
        return Ok(M032LEReport::Unknown(payload));
    }
    let bytes = control_bytes(payload, "M032LE", 2)?;
    let mut directions = [None; 2];
    for (index, &value) in bytes.iter().enumerate() {
        directions[index] = match value {
            0 => Some(RollerDirection::Up),
            1 => Some(RollerDirection::Down),
            0xff => None,
            _ => {
                return Err(InputError::InvalidControlValue {
                    module: "M032LE",
                    report_id: selector,
                    control_index: index as u8,
                    value,
                });
            }
        };
    }
    Ok(M032LEReport::Rotation(directions))
}

/// One direction update for an M032LE roller.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RollerEvent {
    /// Zero-based wire index zero or one. The app labels these right and left.
    pub roller_index: u8,
    /// Discrete application direction without an inferred magnitude.
    pub direction: RollerDirection,
}

/// Events or preserved raw bytes from a complete M032LE HID report.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DecodedM032LE<'a> {
    /// Direction updates in ascending index order, including repeated updates.
    Events(Vec<RollerEvent>),
    /// Unsupported report retained with its outer header.
    Unknown(&'a [u8]),
}

/// Stateless decoder for one independently identified M032LE module.
///
/// Identical consecutive direction reports represent separate updates and are
/// emitted each time. No absolute position is stored.
#[derive(Clone, Debug, Default)]
pub struct M032LEDecoder;

impl M032LEDecoder {
    /// Creates a roller decoder.
    pub fn new() -> Self {
        Self
    }

    /// Resets the decoder; no position state is stored.
    pub fn reset(&mut self) {}

    /// Decodes an M0 channel-zero roller update.
    ///
    /// # Errors
    /// Returns an error for a malformed known report; no partial events escape.
    pub fn decode_report<'a>(&mut self, raw: &'a [u8]) -> Result<DecodedM032LE<'a>, InputError> {
        let Some(payload) = channel_zero_payload(raw, 0x2a)? else {
            return Ok(DecodedM032LE::Unknown(raw));
        };
        let M032LEReport::Rotation(directions) = parse_m032le_payload(payload)? else {
            return Ok(DecodedM032LE::Unknown(raw));
        };
        let events = directions
            .into_iter()
            .enumerate()
            .filter_map(|(index, direction)| {
                direction.map(|direction| RollerEvent {
                    roller_index: index as u8,
                    direction,
                })
            })
            .collect();
        Ok(DecodedM032LE::Events(events))
    }
}
