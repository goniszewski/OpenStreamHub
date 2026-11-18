//! macOS virtual key codes for the platform-free key names of `shortcut` and
//! `macro`. Validation never reads these tables; only the macOS host does, to
//! turn a validated key into the integer it sends.

/// The macOS virtual key code of every name in
/// [`KEY_NAMES`](crate::profile::KEY_NAMES), in the same order; a named key is
/// sent by this integer, never as text.
///
/// Codes are the Carbon `kVK_*` values from `HIToolbox/Events.h`; hardware
/// behaviour is unverified. F13–F20 are the last function keys macOS assigns
/// virtual key codes (there are no `kVK_F21`–`kVK_F24`).
pub(super) const NAMED_KEYS: &[(&str, u8)] = &[
    ("return", 36),
    ("tab", 48),
    ("space", 49),
    ("delete", 51),
    ("escape", 53),
    ("forward_delete", 117),
    ("home", 115),
    ("end", 119),
    ("page_up", 116),
    ("page_down", 121),
    ("arrow_left", 123),
    ("arrow_right", 124),
    ("arrow_down", 125),
    ("arrow_up", 126),
    ("f1", 122),
    ("f2", 120),
    ("f3", 99),
    ("f4", 118),
    ("f5", 96),
    ("f6", 97),
    ("f7", 98),
    ("f8", 100),
    ("f9", 101),
    ("f10", 109),
    ("f11", 103),
    ("f12", 111),
    ("f13", 105),
    ("f14", 107),
    ("f15", 113),
    ("f16", 106),
    ("f17", 64),
    ("f18", 79),
    ("f19", 80),
    ("f20", 90),
    ("keypad_0", 0x52),
    ("keypad_1", 0x53),
    ("keypad_2", 0x54),
    ("keypad_3", 0x55),
    ("keypad_4", 0x56),
    ("keypad_5", 0x57),
    ("keypad_6", 0x58),
    ("keypad_7", 0x59),
    ("keypad_8", 0x5B),
    ("keypad_9", 0x5C),
    ("keypad_decimal", 0x41),
    ("keypad_multiply", 0x43),
    ("keypad_plus", 0x45),
    ("keypad_clear", 0x47),
    ("keypad_divide", 0x4B),
    ("keypad_enter", 0x4C),
    ("keypad_minus", 0x4E),
    ("keypad_equals", 0x51),
];

/// The fixed ANSI virtual key codes of the literal keys (`a`–`z`, `0`–`9`,
/// space), from `HIToolbox/Events.h`. A Dvorak or other alternative layout
/// prints different characters at these codes, like every key-code table here.
const LITERAL_CODES: &[(u8, u8)] = &[
    (b'a', 0x00),
    (b'b', 0x0B),
    (b'c', 0x08),
    (b'd', 0x02),
    (b'e', 0x0E),
    (b'f', 0x03),
    (b'g', 0x05),
    (b'h', 0x04),
    (b'i', 0x22),
    (b'j', 0x26),
    (b'k', 0x28),
    (b'l', 0x25),
    (b'm', 0x2E),
    (b'n', 0x2D),
    (b'o', 0x1F),
    (b'p', 0x23),
    (b'q', 0x0C),
    (b'r', 0x0F),
    (b's', 0x01),
    (b't', 0x11),
    (b'u', 0x20),
    (b'v', 0x09),
    (b'w', 0x0D),
    (b'x', 0x07),
    (b'y', 0x10),
    (b'z', 0x06),
    (b'0', 0x1D),
    (b'1', 0x12),
    (b'2', 0x13),
    (b'3', 0x14),
    (b'4', 0x15),
    (b'5', 0x17),
    (b'6', 0x16),
    (b'7', 0x1A),
    (b'8', 0x1C),
    (b'9', 0x19),
    (b' ', 0x31),
];

/// Returns the macOS virtual key code of an allow-listed named key.
pub(super) fn named_key_code(name: &str) -> Option<u8> {
    NAMED_KEYS
        .iter()
        .find(|(candidate, _)| *candidate == name)
        .map(|(_, code)| *code)
}

/// Returns the macOS virtual key code of any `shortcut` or `macro` key: the
/// fixed ANSI code of a literal (an upper-case letter is the same physical
/// key), or the code of an allow-listed named key. Media keys have no virtual
/// key code and return `None`. The macro replay and the recorder both key off
/// this table, so a recording and its replay name the same physical keys.
pub fn virtual_key_code(key: &str) -> Option<u8> {
    if let [byte] = key.as_bytes() {
        let lowered = byte.to_ascii_lowercase();
        if let Some((_, code)) = LITERAL_CODES.iter().find(|(letter, _)| *letter == lowered) {
            return Some(*code);
        }
    }
    named_key_code(key)
}

/// Returns the key name of a macOS virtual key code: the inverse of
/// [`virtual_key_code`] for the keys a recording can hold. Letters, digits and
/// space come back as the literal character, so a recorded macro reads like
/// the keys it replays; media keys and unknown codes return `None`.
pub fn key_for_virtual_code(code: u8) -> Option<String> {
    if let Some((letter, _)) = LITERAL_CODES
        .iter()
        .find(|(_, candidate)| *candidate == code)
    {
        return Some((*letter as char).to_string());
    }
    NAMED_KEYS
        .iter()
        .find(|(_, candidate)| *candidate == code)
        .map(|(name, _)| (*name).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::KEY_NAMES;

    #[test]
    fn the_code_table_covers_exactly_the_key_names() {
        let names: Vec<_> = NAMED_KEYS.iter().map(|(name, _)| *name).collect();
        assert_eq!(names, KEY_NAMES);
        let mut codes: Vec<_> = NAMED_KEYS.iter().map(|(_, code)| *code).collect();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), NAMED_KEYS.len(), "key codes must be distinct");
        // Media keys are posted as media-key events and have no key code.
        for name in ["play_pause", "next_track", "previous_track"] {
            assert!(named_key_code(name).is_none(), "{name} has no key code");
        }
    }

    #[test]
    fn virtual_key_codes_round_trip_and_cover_every_macro_key() {
        // Every literal and named key has a code, and the code reads back as
        // the same physical key (upper case is the same key as lower case).
        for name in ["a", "z", "A", "0", "9", " "] {
            let code = virtual_key_code(name).unwrap();
            assert_eq!(
                key_for_virtual_code(code).as_deref(),
                Some(name.to_lowercase().as_str()),
                "{name}"
            );
        }
        for name in KEY_NAMES {
            let code = virtual_key_code(name).unwrap();
            // Code 49 is both the literal space and the named `space` key; the
            // inverse reads it back as the literal, which validates the same.
            if *name == "space" {
                assert_eq!(code, virtual_key_code(" ").unwrap());
                assert_eq!(key_for_virtual_code(code).as_deref(), Some(" "));
            } else {
                assert_eq!(key_for_virtual_code(code).as_deref(), Some(*name), "{name}");
            }
        }
        assert!(virtual_key_code("play_pause").is_none());
        // Codes outside the table (the = and ` keys are not macro keys) read
        // back as nothing.
        assert!(key_for_virtual_code(0x18).is_none());
        assert!(key_for_virtual_code(0x32).is_none());
        // A few fixed anchor codes from HIToolbox/Events.h.
        assert_eq!(virtual_key_code("c"), Some(0x08));
        assert_eq!(virtual_key_code("5"), Some(0x17));
        assert_eq!(virtual_key_code("keypad_enter"), Some(0x4C));
    }
}
