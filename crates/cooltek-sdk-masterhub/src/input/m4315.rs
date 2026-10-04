//! Fifteen-key input from the official renderer and pinned public probe notes.
//!
//! Native SDK 03.02.00 frames channel zero as `01 34 <u16 LE length> payload`.
//! Probe selector `02` and the renderer's legacy example selector `00` both
//! describe fifteen boolean states. Physical key layout needs hardware checks.
//!
//! Probe notes: <https://github.com/projectzero7/cooltek-sdk-probe/blob/0e72f87547537193309c3a5f2eae9ef483d80d7f/docs/MasterHUB/modules/m4315/input_reports.md>.

use super::{InputError, channel_zero_payload, control_bytes};

/// A logical fifteen-key snapshot or an unsupported payload.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum M4315Report<'a> {
    /// Complete press state in wire-index order.
    PressSnapshot([bool; 15]),
    /// Unsupported selector retained without interpretation.
    Unknown(&'a [u8]),
}

/// Parses an M4315 logical payload, excluding outer framing and HID padding.
///
/// Selectors `02` (probe) and `00` (official renderer example) require count
/// fifteen and exactly fifteen values, each zero or one. This does not establish
/// a physical row/column mapping or identify the source device.
///
/// # Errors
/// Returns an error for incomplete or malformed recognized snapshots.
pub fn parse_m4315_payload(payload: &[u8]) -> Result<M4315Report<'_>, InputError> {
    let Some(&selector) = payload.first() else {
        return Err(InputError::ShortHeader {
            required: 1,
            received: 0,
        });
    };
    if !matches!(selector, 0x00 | 0x02) {
        return Ok(M4315Report::Unknown(payload));
    }
    let bytes = control_bytes(payload, "M4315", 15)?;
    let mut pressed = [false; 15];
    for (index, &value) in bytes.iter().enumerate() {
        pressed[index] = match value {
            0 => false,
            1 => true,
            _ => {
                return Err(InputError::InvalidControlValue {
                    module: "M4315",
                    report_id: selector,
                    control_index: index as u8,
                    value,
                });
            }
        };
    }
    Ok(M4315Report::PressSnapshot(pressed))
}

/// One key transition, independent of any binding or operating-system action.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyAction {
    /// Changed from released to pressed.
    Pressed,
    /// Changed from pressed to released.
    Released,
}

/// Transition for an M4315 key in raw wire order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KeyEvent {
    /// Zero-based wire index from zero through fourteen.
    /// Physical matrix placement remains hardware-unverified.
    pub key_index: u8,
    /// Press or release transition.
    pub action: KeyAction,
}

/// Events or preserved raw bytes from a complete M4315 HID report.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DecodedM4315<'a> {
    /// Transitions in ascending key-index order; duplicate snapshots emit none.
    Events(Vec<KeyEvent>),
    /// Unsupported report retained with its outer header.
    Unknown(&'a [u8]),
}

/// Stateful decoder for one M4315 module, initially assuming released keys.
///
/// First held keys emit press events. Invalid and unknown reports never change
/// state. Use one decoder per device and reset it after disconnecting.
#[derive(Clone, Debug, Default)]
pub struct M4315Decoder {
    pressed: [bool; 15],
}

impl M4315Decoder {
    /// Creates a decoder with all keys assumed released.
    pub fn new() -> Self {
        Self::default()
    }

    /// Clears key state without emitting artificial releases.
    pub fn reset(&mut self) {
        self.pressed = [false; 15];
    }

    /// Returns the last successfully decoded state in wire order.
    pub fn pressed_states(&self) -> [bool; 15] {
        self.pressed
    }

    /// Decodes channel-zero framing and a recognized fifteen-key snapshot.
    ///
    /// The caller must independently establish that the source is M4315.
    ///
    /// # Errors
    /// Returns an error for a malformed known report without changing state.
    pub fn decode_report<'a>(&mut self, raw: &'a [u8]) -> Result<DecodedM4315<'a>, InputError> {
        let Some(payload) = channel_zero_payload(raw, 0x34)? else {
            return Ok(DecodedM4315::Unknown(raw));
        };
        let M4315Report::PressSnapshot(pressed) = parse_m4315_payload(payload)? else {
            return Ok(DecodedM4315::Unknown(raw));
        };
        let mut events = Vec::with_capacity(15);
        for (index, is_pressed) in pressed.iter().copied().enumerate() {
            if is_pressed != self.pressed[index] {
                events.push(KeyEvent {
                    key_index: index as u8,
                    action: if is_pressed {
                        KeyAction::Pressed
                    } else {
                        KeyAction::Released
                    },
                });
            }
        }
        self.pressed = pressed;
        Ok(DecodedM4315::Events(events))
    }
}
