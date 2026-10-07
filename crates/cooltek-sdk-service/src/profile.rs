use serde::{Deserialize, Serialize};
use std::collections::HashSet;

use crate::{DialFunction, KeypadBinding, KeypadLayout, KeypadPage, KeypadTile};

/// Modules whose input is supported by the binding service.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ServiceModule {
    /// Three knobs using wire control indexes 0, 1, and 2.
    M003K,
    /// IPS encoder dial using control index 0.
    M131E,
    /// Fifteen keys using wire control indexes 0 through 14.
    M4315,
    /// Five absolute faders using wire control indexes 0 through 4.
    M005S,
    /// Two rollers using wire control indexes 0 and 1.
    M032LE,
}

/// A normalized wire transition or host-derived dial gesture.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Gesture {
    /// M003K clockwise rotation update.
    Clockwise,
    /// M003K counter-clockwise rotation update.
    CounterClockwise,
    /// Button changed from released to pressed.
    Pressed,
    /// Button changed from pressed to released.
    Released,
    /// M131E application RIGHT label; physical orientation is unverified.
    Right,
    /// M131E application LEFT label; physical orientation is unverified.
    Left,
    /// M131E release after a known press shorter than 600 ms.
    ShortPress,
    /// M131E release after a known press lasting at least 600 ms.
    LongPress,
    /// Absolute fader position update; its numeric value accompanies the event.
    PositionChanged,
    /// M032LE application's UP roller label; physical orientation is unverified.
    Up,
    /// M032LE application's DOWN roller label; physical orientation is unverified.
    Down,
}

impl ServiceModule {
    /// Checks a module's supported control indexes and gesture labels.
    pub fn validate_selector(self, control: u8, gesture: Gesture) -> Result<(), String> {
        let supported = match self {
            Self::M003K => {
                control <= 2
                    && matches!(
                        gesture,
                        Gesture::Clockwise
                            | Gesture::CounterClockwise
                            | Gesture::Pressed
                            | Gesture::Released
                    )
            }
            Self::M131E => {
                control == 0
                    && matches!(
                        gesture,
                        Gesture::Right
                            | Gesture::Left
                            | Gesture::Pressed
                            | Gesture::Released
                            | Gesture::ShortPress
                            | Gesture::LongPress
                    )
            }
            Self::M4315 => control < 15 && matches!(gesture, Gesture::Pressed | Gesture::Released),
            Self::M005S => control < 5 && gesture == Gesture::PositionChanged,
            Self::M032LE => control < 2 && matches!(gesture, Gesture::Up | Gesture::Down),
        };
        if supported {
            Ok(())
        } else {
            Err("control or gesture is unsupported for this module".into())
        }
    }
}

/// A modifier held for the duration of one keyboard shortcut.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Modifier {
    /// macOS Command modifier.
    Command,
    /// Control modifier.
    Control,
    /// macOS Option modifier.
    Option,
    /// Shift modifier.
    Shift,
}

/// One bounded operation performed by an action dispatcher.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    /// Adjust the default output volume by signed percentage points.
    Volume {
        /// Nonzero delta between -100 and 100 inclusive.
        delta: i16,
    },
    /// Set the default output volume to an absolute percentage.
    SetVolume {
        /// Absolute output percentage, between 0 and 100 inclusive.
        percent: u8,
    },
    /// Resolve a fader's absolute position into an absolute volume percentage.
    ///
    /// This is a binding template. It must be resolved using the current input
    /// value before it reaches an action dispatcher.
    VolumeFromPosition,
    /// Invert the default output's current mute state.
    ToggleMute,
    /// Send one ASCII alphanumeric key or space with explicit modifiers.
    Shortcut {
        /// Exactly one ASCII alphanumeric byte or ASCII space.
        key: String,
        /// Distinct modifiers; order does not change the action.
        modifiers: Vec<Modifier>,
    },
    /// Ask macOS to open an installed application by its bundle identifier.
    LaunchApp {
        /// Dotted ASCII identifier, such as `com.apple.calculator`.
        bundle_id: String,
    },
}

impl<'de> Deserialize<'de> for Action {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        // Serde's internally tagged unit variants ignore trailing fields even
        // with deny_unknown_fields. A zero-field struct variant enforces the
        // same strict JSON contract as the action variants that carry values.
        #[derive(Deserialize)]
        #[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
        enum Shape {
            Volume {
                delta: i16,
            },
            SetVolume {
                percent: u8,
            },
            VolumeFromPosition {},
            ToggleMute {},
            Shortcut {
                key: String,
                modifiers: Vec<Modifier>,
            },
            LaunchApp {
                bundle_id: String,
            },
        }
        Ok(match Shape::deserialize(deserializer)? {
            Shape::Volume { delta } => Self::Volume { delta },
            Shape::SetVolume { percent } => Self::SetVolume { percent },
            Shape::VolumeFromPosition {} => Self::VolumeFromPosition,
            Shape::ToggleMute {} => Self::ToggleMute,
            Shape::Shortcut { key, modifiers } => Self::Shortcut { key, modifiers },
            Shape::LaunchApp { bundle_id } => Self::LaunchApp { bundle_id },
        })
    }
}

impl Action {
    /// Validates bounds before an action can reach an operating-system adapter.
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Volume { delta } if *delta == 0 || !(-100..=100).contains(delta) => {
                Err("volume delta must be nonzero and between -100 and 100".into())
            }
            Self::SetVolume { percent } if *percent > 100 => {
                Err("absolute volume must be between 0 and 100".into())
            }
            Self::Shortcut { key, modifiers } => {
                if key.len() != 1
                    || !key.as_bytes()[0].is_ascii_alphanumeric() && key.as_bytes()[0] != b' '
                {
                    return Err("shortcut key must be one ASCII alphanumeric key or space".into());
                }
                if modifiers.len() > 4 {
                    return Err("shortcut must contain at most four modifiers".into());
                }
                let unique: HashSet<_> = modifiers.iter().collect();
                if unique.len() != modifiers.len() {
                    return Err("shortcut modifiers must not contain duplicates".into());
                }
                Ok(())
            }
            Self::LaunchApp { bundle_id } => {
                if bundle_id.len() > 255
                    || !bundle_id.contains('.')
                    || bundle_id.split('.').any(|component| {
                        !component
                            .as_bytes()
                            .first()
                            .is_some_and(u8::is_ascii_alphanumeric)
                            || !component
                                .bytes()
                                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                    })
                {
                    return Err("application bundle ID must be a dotted ASCII identifier of at most 255 bytes".into());
                }
                Ok(())
            }
            Self::Volume { .. }
            | Self::SetVolume { .. }
            | Self::VolumeFromPosition
            | Self::ToggleMute => Ok(()),
        }
    }

    /// Validates an operation and rejects templates lacking a resolved input value.
    pub fn validate_executable(&self) -> Result<(), String> {
        self.validate()?;
        if matches!(self, Self::VolumeFromPosition) {
            Err("volume_from_position must be resolved with a fader event before dispatch".into())
        } else {
            Ok(())
        }
    }
}

/// One module/control/gesture selector and the action it invokes.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    /// Source module, independently established from device identity.
    pub module: ServiceModule,
    /// Zero-based wire control index.
    pub control: u8,
    /// Event or host gesture to match.
    pub gesture: Gesture,
    /// Exact serial selector; absent matches any serial for this module.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serial_number: Option<String>,
    /// Bounded host operation.
    pub action: Action,
}

/// Strict, versioned JSON configuration for one binding set.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    /// Configuration schema: 1 for flat bindings or 2 for a keypad page tree.
    pub schema_version: u32,
    /// Human-readable name of at most 128 bytes.
    pub name: String,
    /// At most 256 bindings with unique complete selectors.
    pub bindings: Vec<Binding>,
    /// Schema-2 keypad pages; omitted from existing schema-1 profiles.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keypad: Option<KeypadLayout>,
    /// Optional ordered M131E functions cycled by a short click (schema 1 or 2).
    ///
    /// When present, every M131E binding lives in these functions and flat
    /// `bindings` must not select M131E. Omitted when absent, so existing
    /// profiles serialize unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dial_functions: Option<Vec<DialFunction>>,
}

impl Profile {
    /// Maximum accepted JSON document size before parsing.
    pub const MAX_JSON_BYTES: usize = 1024 * 1024;
    /// Maximum number of bindings in one profile.
    pub const MAX_BINDINGS: usize = 256;

    /// Parses bounded JSON and checks its complete semantic constraints.
    pub fn from_json(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > Self::MAX_JSON_BYTES {
            return Err(format!("profile exceeds {} bytes", Self::MAX_JSON_BYTES));
        }
        let profile: Self = serde_json::from_slice(bytes)
            .map_err(|error| format!("invalid profile JSON: {error}"))?;
        profile.validate()?;
        Ok(profile)
    }

    /// Validates a profile constructed in Rust or decoded using another path.
    pub fn validate(&self) -> Result<(), String> {
        match (self.schema_version, self.keypad.as_ref()) {
            (1, None) => {}
            (2, Some(keypad)) => keypad.validate()?,
            (1, Some(_)) => return Err("schema 1 profiles must omit keypad pages".into()),
            (2, None) => return Err("schema 2 profiles require keypad pages".into()),
            (version, _) => {
                return Err(format!(
                    "unsupported profile schema {version} (expected 1 or 2)"
                ));
            }
        }
        if self.name.trim().is_empty()
            || self.name.len() > 128
            || self.name.chars().any(char::is_control)
        {
            return Err(
                "profile name must be nonempty, free of control characters, and at most 128 bytes"
                    .into(),
            );
        }
        if let Some(functions) = &self.dial_functions {
            crate::dial::validate(functions)?;
        }
        if self.binding_count() > Self::MAX_BINDINGS {
            return Err(format!("profile exceeds {} bindings", Self::MAX_BINDINGS));
        }
        let mut selectors = HashSet::new();
        for (index, binding) in self.bindings.iter().enumerate() {
            if self.schema_version == 2 && binding.module == ServiceModule::M4315 {
                return Err(format!(
                    "binding {index}: schema 2 keypad bindings belong in keypad action tiles"
                ));
            }
            if self.dial_functions.is_some() && binding.module == ServiceModule::M131E {
                return Err(format!(
                    "binding {index}: m131e bindings belong in dial_functions when dial functions are defined"
                ));
            }
            binding
                .module
                .validate_selector(binding.control, binding.gesture)
                .map_err(|error| format!("binding {index}: {error}"))?;
            if (binding.module == ServiceModule::M005S)
                != matches!(binding.action, Action::VolumeFromPosition)
            {
                return Err(format!(
                    "binding {index}: fader bindings require volume_from_position, which is supported only for faders"
                ));
            }
            validate_serial_number(binding.serial_number.as_deref())
                .map_err(|error| format!("binding {index}: {error}"))?;
            if !selectors.insert((
                binding.module,
                binding.control,
                binding.gesture,
                binding.serial_number.as_deref(),
            )) {
                return Err(format!("binding {index}: duplicate selector"));
            }
            binding
                .action
                .validate()
                .map_err(|error| format!("binding {index}: {error}"))?;
        }
        Ok(())
    }

    /// Counts flat bindings, nested keypad action tile bindings, and dial function bindings.
    pub fn binding_count(&self) -> usize {
        self.bindings.len()
            + self.keypad.as_ref().map_or(0, KeypadLayout::binding_count)
            + crate::dial::binding_count(self)
    }

    /// Converts a valid flat profile into schema 2 without writing to storage.
    ///
    /// Every M4315 selector becomes a binding on its original root key, retaining
    /// gesture, serial selector, and action. Other module bindings keep their
    /// original order. A valid schema-2 profile is returned unchanged.
    pub fn into_v2(mut self) -> Result<Self, String> {
        self.validate()?;
        if self.schema_version == 2 {
            return Ok(self);
        }
        let mut slots = vec![None; KeypadLayout::KEY_COUNT];
        let mut flat = Vec::new();
        for binding in self.bindings {
            if binding.module == ServiceModule::M4315 {
                let tile =
                    slots[binding.control as usize].get_or_insert_with(|| KeypadTile::Action {
                        label: format!("Key {}", binding.control + 1),
                        bindings: Vec::new(),
                    });
                let KeypadTile::Action { bindings, .. } = tile else {
                    unreachable!("migration creates only action tiles")
                };
                bindings.push(KeypadBinding {
                    gesture: binding.gesture,
                    serial_number: binding.serial_number,
                    action: binding.action,
                });
            } else {
                flat.push(binding);
            }
        }
        self.bindings = flat;
        self.schema_version = 2;
        self.keypad = Some(KeypadLayout {
            pages: vec![KeypadPage {
                id: KeypadLayout::ROOT_PAGE_ID.into(),
                name: "Root".into(),
                slots,
            }],
        });
        self.validate()?;
        Ok(self)
    }
}

pub(crate) fn validate_serial_number(serial: Option<&str>) -> Result<(), String> {
    if serial.is_some_and(|serial| {
        serial.is_empty() || serial.len() > 256 || serial.chars().any(char::is_control)
    }) {
        Err(
            "serial number must be nonempty, free of control characters, and at most 256 bytes"
                .into(),
        )
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Profile {
        Profile {
            schema_version: 1,
            name: "Desk".into(),
            bindings: vec![Binding {
                module: ServiceModule::M003K,
                control: 0,
                gesture: Gesture::Clockwise,
                serial_number: None,
                action: Action::Volume { delta: 5 },
            }],
            keypad: None,
            dial_functions: None,
        }
    }

    #[test]
    fn json_roundtrip_is_strict_and_optional_serial_defaults() {
        let json = br#"{"schema_version":1,"name":"Desk","bindings":[{"module":"m003k","control":0,"gesture":"clockwise","action":{"type":"volume","delta":5}}]}"#;
        assert_eq!(Profile::from_json(json).unwrap(), sample());
        let encoded = serde_json::to_vec(&sample()).unwrap();
        assert_eq!(Profile::from_json(&encoded).unwrap(), sample());
        for json in [
            br#"{"schema_version":1,"name":"Desk","bindings":[],"extra":true}"#.as_slice(),
            br#"{"schema_version":1,"name":"Desk","bindings":[{"module":"m003k","control":0,"gesture":"clockwise","extra":true,"action":{"type":"toggle_mute"}}]}"#.as_slice(),
            br#"{"schema_version":1,"name":"Desk","bindings":[{"module":"m003k","control":0,"gesture":"clockwise","action":{"type":"toggle_mute","extra":true}}]}"#.as_slice(),
            br#"{"schema_version":1,"name":"Desk","bindings":[]} trailing"#.as_slice(),
        ] {
            assert!(Profile::from_json(json).is_err());
        }
    }

    #[test]
    fn profile_bounds_and_versions_are_checked() {
        let mut profile = sample();
        profile.schema_version = 2;
        assert!(profile.validate().is_err());
        profile.schema_version = 1;
        for name in ["".to_string(), "   ".into(), "x\n".into(), "x".repeat(129)] {
            profile.name = name;
            assert!(profile.validate().is_err());
        }
        assert!(Profile::from_json(&vec![b' '; Profile::MAX_JSON_BYTES + 1]).is_err());
        profile.name = "Desk".into();
        profile.bindings = vec![profile.bindings[0].clone(); Profile::MAX_BINDINGS + 1];
        assert!(profile.validate().unwrap_err().contains("exceeds"));
    }

    #[test]
    fn serial_selector_bounds_and_exact_duplicates_are_checked() {
        let mut profile = sample();
        profile.bindings.push(profile.bindings[0].clone());
        assert!(profile.validate().unwrap_err().contains("duplicate"));
        profile.bindings[1].serial_number = Some("unit-1".into());
        assert!(profile.validate().is_ok());
        for serial in ["".to_string(), "x\0".into(), "x".repeat(257)] {
            profile.bindings[1].serial_number = Some(serial);
            assert!(profile.validate().is_err());
        }
    }

    #[test]
    fn module_controls_and_direction_labels_are_not_interchangeable() {
        let mut profile = sample();
        profile.bindings[0].control = 3;
        assert!(profile.validate().is_err());
        profile.bindings[0].control = 0;
        profile.bindings[0].gesture = Gesture::Right;
        assert!(profile.validate().is_err());
        profile.bindings[0].module = ServiceModule::M131E;
        assert!(profile.validate().is_ok());
        profile.bindings[0].control = 1;
        assert!(profile.validate().is_err());
        profile.bindings[0].control = 0;
        profile.bindings[0].gesture = Gesture::Clockwise;
        assert!(profile.validate().is_err());
        profile.bindings[0].gesture = Gesture::LongPress;
        assert!(profile.validate().is_ok());
    }

    #[test]
    fn keys_faders_and_rollers_enforce_their_control_and_action_contracts() {
        let mut profile = sample();
        profile.bindings[0].module = ServiceModule::M4315;
        profile.bindings[0].control = 14;
        profile.bindings[0].gesture = Gesture::Pressed;
        assert!(profile.validate().is_ok());
        profile.bindings[0].control = 15;
        assert!(profile.validate().is_err());
        profile.bindings[0].control = 0;
        profile.bindings[0].gesture = Gesture::LongPress;
        assert!(profile.validate().is_err());

        profile.bindings[0].module = ServiceModule::M005S;
        profile.bindings[0].gesture = Gesture::PositionChanged;
        assert!(profile.validate().is_err());
        profile.bindings[0].action = Action::VolumeFromPosition;
        profile.bindings[0].control = 4;
        assert!(profile.validate().is_ok());
        profile.bindings[0].control = 5;
        assert!(profile.validate().is_err());
        profile.bindings[0].module = ServiceModule::M003K;
        profile.bindings[0].control = 0;
        profile.bindings[0].gesture = Gesture::Clockwise;
        assert!(profile.validate().is_err());

        profile.bindings[0].module = ServiceModule::M032LE;
        profile.bindings[0].control = 1;
        profile.bindings[0].gesture = Gesture::Up;
        profile.bindings[0].action = Action::Volume { delta: 5 };
        assert!(profile.validate().is_ok());
        profile.bindings[0].control = 2;
        assert!(profile.validate().is_err());
        profile.bindings[0].control = 0;
        profile.bindings[0].gesture = Gesture::Pressed;
        assert!(profile.validate().is_err());
    }

    #[test]
    fn position_templates_and_absolute_actions_have_strict_additive_json_shapes() {
        for action in [
            Action::VolumeFromPosition,
            Action::SetVolume { percent: 0 },
            Action::SetVolume { percent: 100 },
        ] {
            let json = serde_json::to_vec(&action).unwrap();
            assert_eq!(serde_json::from_slice::<Action>(&json).unwrap(), action);
            assert!(action.validate().is_ok());
        }
        assert!(Action::VolumeFromPosition.validate_executable().is_err());
        assert!(
            Action::SetVolume { percent: 100 }
                .validate_executable()
                .is_ok()
        );
        assert!(Action::SetVolume { percent: 101 }.validate().is_err());
        for json in [
            br#"{"type":"volume_from_position","delta":5}"#.as_slice(),
            br#"{"type":"set_volume","percent":5,"delta":5}"#.as_slice(),
            br#"{"type":"set_volume","percent":-1}"#.as_slice(),
        ] {
            assert!(serde_json::from_slice::<Action>(json).is_err());
        }
    }

    #[test]
    fn actions_reject_ambiguous_or_unbounded_parameters() {
        for delta in [-101, 0, 101] {
            assert!(Action::Volume { delta }.validate().is_err());
        }
        for delta in [-100, -1, 1, 100] {
            assert!(Action::Volume { delta }.validate().is_ok());
        }
        for key in ["", "aa", "é", "\"", "\\", "\n"] {
            assert!(
                Action::Shortcut {
                    key: key.into(),
                    modifiers: vec![]
                }
                .validate()
                .is_err()
            );
        }
        assert!(
            Action::Shortcut {
                key: " ".into(),
                modifiers: vec![Modifier::Command, Modifier::Command]
            }
            .validate()
            .is_err()
        );
        for key in [" ", "A", "z", "4"] {
            assert!(
                Action::Shortcut {
                    key: key.into(),
                    modifiers: vec![Modifier::Command, Modifier::Shift]
                }
                .validate()
                .is_ok()
            );
        }
        for bundle_id in [
            "",
            "Calculator",
            "com..app",
            "com.-app",
            "com.app;do shell script",
            "com.é",
            "com.app\n",
        ] {
            assert!(
                Action::LaunchApp {
                    bundle_id: bundle_id.into()
                }
                .validate()
                .is_err()
            );
        }
        assert!(
            Action::LaunchApp {
                bundle_id: "com.apple.calculator".into()
            }
            .validate()
            .is_ok()
        );
    }
}
