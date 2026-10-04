//! Five-fader input from pinned probe notes and official renderer/store code.
//!
//! Native M0 SDK 02.05.00 supplies framing. App.vue consumes payload bytes 2–6
//! directly; slider.js stores them unchanged as values from zero through 100.
//!
//! Probe notes: <https://github.com/projectzero7/cooltek-sdk-probe/blob/0e72f87547537193309c3a5f2eae9ef483d80d7f/docs/MasterHUB/modules/m005s/input_reports.md>.

use super::{InputError, channel_zero_payload, control_bytes};

/// Maximum absolute position accepted by the documented M005S payload.
pub const FADER_POSITION_MAXIMUM: u8 = 100;

/// A logical five-fader position snapshot or an unsupported payload.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum M005SReport<'a> {
    /// All five absolute positions in wire-index order, each zero through 100.
    PositionSnapshot([u8; 5]),
    /// Unsupported selector retained without interpretation.
    Unknown(&'a [u8]),
}

/// Parses the documented logical fader payload, excluding outer HID framing.
///
/// Selector `02`, count five, and five positions from zero through 100 are
/// supported. `ff` has no established unchanged meaning for faders and fails
/// validation. The probe's table lists `00`, conflicting with its `02` heading
/// and example; that unresolved selector is retained as unknown.
///
/// # Errors
/// Returns an error for incomplete or malformed recognized snapshots.
pub fn parse_m005s_payload(payload: &[u8]) -> Result<M005SReport<'_>, InputError> {
    let Some(&selector) = payload.first() else {
        return Err(InputError::ShortHeader {
            required: 1,
            received: 0,
        });
    };
    if selector != 0x02 {
        return Ok(M005SReport::Unknown(payload));
    }
    let bytes = control_bytes(payload, "M005S", 5)?;
    let mut positions = [0; 5];
    for (index, &value) in bytes.iter().enumerate() {
        if value > FADER_POSITION_MAXIMUM {
            return Err(InputError::InvalidControlValue {
                module: "M005S",
                report_id: selector,
                control_index: index as u8,
                value,
            });
        }
        positions[index] = value;
    }
    Ok(M005SReport::PositionSnapshot(positions))
}

/// A reported absolute position for one M005S fader.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FaderEvent {
    /// Zero-based wire index from zero through four; physical order is unverified.
    pub fader_index: u8,
    /// Raw absolute position, in the supported range zero through 100.
    pub position: u8,
}

/// Events or preserved raw bytes from a complete M005S HID report.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DecodedM005S<'a> {
    /// Positions in ascending index order; duplicate snapshots emit none.
    Events(Vec<FaderEvent>),
    /// Unsupported report retained with its outer header.
    Unknown(&'a [u8]),
}

/// Stateful absolute-position decoder for one M005S module.
///
/// The first valid snapshot emits all five positions, because initial positions
/// are unknown. Subsequent snapshots emit only changed positions. Invalid and
/// unknown reports do not change state. Reset after reconnecting.
#[derive(Clone, Debug, Default)]
pub struct M005SDecoder {
    positions: Option<[u8; 5]>,
}

impl M005SDecoder {
    /// Creates a decoder with no known positions.
    pub fn new() -> Self {
        Self::default()
    }

    /// Forgets previous positions without emitting events.
    pub fn reset(&mut self) {
        self.positions = None;
    }

    /// Returns the last complete valid snapshot, if one has been received.
    pub fn positions(&self) -> Option<[u8; 5]> {
        self.positions
    }

    /// Decodes an M0 channel-zero report from an independently identified M005S.
    ///
    /// # Errors
    /// Returns an error for a malformed known report without changing state.
    pub fn decode_report<'a>(&mut self, raw: &'a [u8]) -> Result<DecodedM005S<'a>, InputError> {
        let Some(payload) = channel_zero_payload(raw, 0x2a)? else {
            return Ok(DecodedM005S::Unknown(raw));
        };
        let M005SReport::PositionSnapshot(positions) = parse_m005s_payload(payload)? else {
            return Ok(DecodedM005S::Unknown(raw));
        };
        let mut events = Vec::with_capacity(5);
        for (index, position) in positions.iter().copied().enumerate() {
            if self
                .positions
                .is_none_or(|previous| previous[index] != position)
            {
                events.push(FaderEvent {
                    fader_index: index as u8,
                    position,
                });
            }
        }
        self.positions = Some(positions);
        Ok(DecodedM005S::Events(events))
    }
}
