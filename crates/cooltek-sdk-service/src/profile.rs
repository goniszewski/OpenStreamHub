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

/// Named non-character keys accepted by `shortcut` and their macOS virtual key codes.
///
/// The table is the allow-list: a `shortcut` key is either one ASCII
/// alphanumeric byte or space (sent as a literal keystroke) or exactly one of
/// these lowercase names (sent by integer key code). Codes are the Carbon
/// `kVK_*` values from `HIToolbox/Events.h`; hardware behaviour is unverified.
pub(crate) const NAMED_KEYS: &[(&str, u8)] = &[
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
];

/// Returns the macOS virtual key code of an allow-listed named key.
pub(crate) fn named_key_code(name: &str) -> Option<u8> {
    NAMED_KEYS
        .iter()
        .find(|(candidate, _)| *candidate == name)
        .map(|(_, code)| *code)
}

/// A media transport command, matching the official `media-track-control` options.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaCommand {
    /// Toggle playback.
    PlayPause,
    /// Skip to the next track.
    NextTrack,
    /// Return to the previous track.
    PreviousTrack,
    /// Stop playback.
    Stop,
}

/// A fixed system command, a subset of the official app's system actions.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SystemCommand {
    /// Command-Z in the focused application.
    Undo,
    /// Shift-Command-Z in the focused application.
    Redo,
    /// Capture the whole screen to the clipboard.
    Screenshot,
}

/// Which neighbouring saved profile a `profile` action asks for.
///
/// The service does not own the profile list: it only reports the request.
/// A supervising session manager resolves it against its saved profiles,
/// wrapping around at either end.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileDirection {
    /// The next saved profile.
    Next,
    /// The previous saved profile.
    Previous,
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
    /// Send one key with explicit modifiers.
    Shortcut {
        /// One ASCII alphanumeric byte or ASCII space, or one allow-listed
        /// named key such as `arrow_left`, `return`, or `f5`.
        key: String,
        /// Distinct modifiers; order does not change the action.
        modifiers: Vec<Modifier>,
    },
    /// Ask macOS to open an installed application by its bundle identifier.
    LaunchApp {
        /// Dotted ASCII identifier, such as `com.apple.calculator`.
        bundle_id: String,
    },
    /// Open an `http` or `https` URL with the default handler.
    OpenUrl {
        /// Printable ASCII URL with a host and no user information.
        url: String,
    },
    /// Send a media transport command. Plan-only on the macOS backend.
    Media {
        /// Transport command.
        command: MediaCommand,
    },
    /// Run a fixed system command.
    System {
        /// System command.
        command: SystemCommand,
    },
    /// Paste a bounded text snippet into the focused application.
    Text {
        /// Nonempty text without control characters other than newline and
        /// tab, and without invisible format (`Cf`) or line/paragraph
        /// separator (`Zl`/`Zp`) characters.
        text: String,
        /// Press Return after pasting.
        press_enter: bool,
    },
    /// Run 2 to 8 executable, non-nested actions in order with fixed spacing.
    MultiAction {
        /// Ordered steps; a failure stops the remaining steps.
        actions: Vec<Action>,
    },
    /// Ask the supervising session manager to switch to a neighbouring saved
    /// profile. Never executed on the host; not allowed as a multi-action step.
    Profile {
        /// Next or previous saved profile.
        direction: ProfileDirection,
    },
}

/// Characters a `text` snippet must not contain beyond the control-character
/// rule: Unicode `Cf` (format) characters such as zero-width spaces, bidi
/// embeddings/overrides/isolates and the BOM, plus the `Zl`/`Zp` line and
/// paragraph separators. They are invisible in an editor, so a profile could
/// otherwise paste text that differs from what the user reviewed. Importers
/// use the same rule to clean names and labels.
pub fn is_invisible_text_character(c: char) -> bool {
    // U+2065 is unassigned but sits inside the reserved invisible-operator
    // block, so the whole U+2060..=U+206F range is rejected.
    crate::dial::is_format_character(c) || matches!(c, '\u{2028}' | '\u{2029}' | '\u{2065}')
}

fn validate_url(url: &str) -> Result<(), String> {
    const ALLOWED: &[u8] = b"-._~:/?#[]@!$&'()*+,;=%";
    if url.len() > Action::MAX_URL_BYTES {
        return Err(format!(
            "URL must be at most {} bytes",
            Action::MAX_URL_BYTES
        ));
    }
    if !url
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || ALLOWED.contains(&byte))
    {
        return Err(
            "URL must contain only unreserved, reserved, or percent-encoded ASCII characters"
                .into(),
        );
    }
    let bytes = url.as_bytes();
    for (index, byte) in bytes.iter().enumerate() {
        if *byte == b'%'
            && !(bytes.get(index + 1).is_some_and(u8::is_ascii_hexdigit)
                && bytes.get(index + 2).is_some_and(u8::is_ascii_hexdigit))
        {
            return Err("URL percent escapes must use two hexadecimal digits".into());
        }
    }
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .ok_or("URL must start with lowercase http:// or https://")?;
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(authority_end);
    if authority.contains('@') {
        return Err("URL must not contain user information".into());
    }
    if tail.contains(['[', ']']) {
        return Err("URL brackets are allowed only around an IPv6 host".into());
    }
    let (_host, port) = if let Some(bracketed) = authority.strip_prefix('[') {
        let (host, after) = bracketed
            .split_once(']')
            .ok_or("URL IPv6 host must be closed by ]")?;
        if host.is_empty()
            || !host.contains(':')
            || !host
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() || byte == b':' || byte == b'.')
        {
            return Err("URL IPv6 host is malformed".into());
        }
        match after {
            "" => (host, None),
            _ => (
                host,
                Some(
                    after
                        .strip_prefix(':')
                        .ok_or("URL IPv6 host must be followed by a port or path")?,
                ),
            ),
        }
    } else {
        if authority.contains(['[', ']']) {
            return Err("URL brackets are allowed only around an IPv6 host".into());
        }
        let (host, port) = match authority.split_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (authority, None),
        };
        if host.is_empty()
            || host.split('.').any(|label| {
                label.is_empty()
                    || label.len() > 63
                    || label.starts_with('-')
                    || label.ends_with('-')
                    || !label
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            })
        {
            return Err(
                "URL must have a host of dot-separated letters, digits, and hyphens".into(),
            );
        }
        (host, port)
    };
    if let Some(port) = port
        && (port.is_empty()
            || port.len() > 5
            || !port.bytes().all(|byte| byte.is_ascii_digit())
            || !port.parse::<u16>().is_ok_and(|port| port != 0))
    {
        return Err("URL port must be a number between 1 and 65535".into());
    }
    Ok(())
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
            OpenUrl {
                url: String,
            },
            Media {
                command: MediaCommand,
            },
            System {
                command: SystemCommand,
            },
            Text {
                text: String,
                #[serde(default)]
                press_enter: bool,
            },
            MultiAction {
                actions: Vec<Action>,
            },
            Profile {
                direction: ProfileDirection,
            },
        }
        Ok(match Shape::deserialize(deserializer)? {
            Shape::Volume { delta } => Self::Volume { delta },
            Shape::SetVolume { percent } => Self::SetVolume { percent },
            Shape::VolumeFromPosition {} => Self::VolumeFromPosition,
            Shape::ToggleMute {} => Self::ToggleMute,
            Shape::Shortcut { key, modifiers } => Self::Shortcut { key, modifiers },
            Shape::LaunchApp { bundle_id } => Self::LaunchApp { bundle_id },
            Shape::OpenUrl { url } => Self::OpenUrl { url },
            Shape::Media { command } => Self::Media { command },
            Shape::System { command } => Self::System { command },
            Shape::Text { text, press_enter } => Self::Text { text, press_enter },
            Shape::MultiAction { actions } => Self::MultiAction { actions },
            Shape::Profile { direction } => Self::Profile { direction },
        })
    }
}

impl Action {
    /// Maximum `open_url` URL length in bytes.
    pub const MAX_URL_BYTES: usize = 2048;
    /// Maximum `text` snippet length in UTF-8 bytes.
    pub const MAX_TEXT_BYTES: usize = 2048;
    /// Minimum number of `multi_action` steps.
    pub const MIN_MULTI_ACTION_STEPS: usize = 2;
    /// Maximum number of `multi_action` steps.
    pub const MAX_MULTI_ACTION_STEPS: usize = 8;

    /// The JSON `type` tag of this action, for diagnostics.
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Volume { .. } => "volume",
            Self::SetVolume { .. } => "set_volume",
            Self::VolumeFromPosition => "volume_from_position",
            Self::ToggleMute => "toggle_mute",
            Self::Shortcut { .. } => "shortcut",
            Self::LaunchApp { .. } => "launch_app",
            Self::OpenUrl { .. } => "open_url",
            Self::Media { .. } => "media",
            Self::System { .. } => "system",
            Self::Text { .. } => "text",
            Self::MultiAction { .. } => "multi_action",
            Self::Profile { .. } => "profile",
        }
    }

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
                let literal = key.len() == 1
                    && (key.as_bytes()[0].is_ascii_alphanumeric() || key.as_bytes()[0] == b' ');
                if !literal && named_key_code(key).is_none() {
                    return Err(
                        "shortcut key must be one ASCII alphanumeric key, space, or a supported named key"
                            .into(),
                    );
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
            Self::OpenUrl { url } => validate_url(url),
            Self::Text { text, .. } => {
                if text.is_empty() || text.len() > Self::MAX_TEXT_BYTES {
                    return Err(format!(
                        "text must be nonempty and at most {} bytes",
                        Self::MAX_TEXT_BYTES
                    ));
                }
                if text
                    .chars()
                    .any(|character| character.is_control() && !matches!(character, '\n' | '\t'))
                {
                    return Err(
                        "text must not contain control characters other than newline and tab"
                            .into(),
                    );
                }
                if text.chars().any(is_invisible_text_character) {
                    return Err(
                        "text must not contain invisible format or line/paragraph separator characters"
                            .into(),
                    );
                }
                Ok(())
            }
            Self::MultiAction { actions } => {
                if !(Self::MIN_MULTI_ACTION_STEPS..=Self::MAX_MULTI_ACTION_STEPS)
                    .contains(&actions.len())
                {
                    return Err(format!(
                        "multi_action must contain {} to {} steps",
                        Self::MIN_MULTI_ACTION_STEPS,
                        Self::MAX_MULTI_ACTION_STEPS
                    ));
                }
                for (index, step) in actions.iter().enumerate() {
                    // A profile switch stops the running service, so it
                    // could never be followed by the remaining steps.
                    if matches!(
                        step,
                        Self::MultiAction { .. } | Self::VolumeFromPosition | Self::Profile { .. }
                    ) {
                        return Err(format!(
                            "multi_action step {}: {} cannot be a multi_action step",
                            index + 1,
                            step.type_name()
                        ));
                    }
                    step.validate()
                        .map_err(|error| format!("multi_action step {}: {error}", index + 1))?;
                }
                Ok(())
            }
            Self::Volume { .. }
            | Self::SetVolume { .. }
            | Self::VolumeFromPosition
            | Self::ToggleMute
            | Self::Media { .. }
            | Self::System { .. }
            | Self::Profile { .. } => Ok(()),
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

    fn url(url: &str) -> Action {
        Action::OpenUrl { url: url.into() }
    }

    fn key(key: &str, modifiers: Vec<Modifier>) -> Action {
        Action::Shortcut {
            key: key.into(),
            modifiers,
        }
    }

    fn text(text: &str) -> Action {
        Action::Text {
            text: text.into(),
            press_enter: false,
        }
    }

    #[test]
    fn new_actions_have_exact_strict_json_round_trips() {
        for (json, action) in [
            (
                r#"{"type":"open_url","url":"https://example.com/a?b=1#c"}"#,
                url("https://example.com/a?b=1#c"),
            ),
            (
                r#"{"type":"shortcut","key":"arrow_left","modifiers":["command","shift"]}"#,
                key("arrow_left", vec![Modifier::Command, Modifier::Shift]),
            ),
            (
                r#"{"type":"media","command":"play_pause"}"#,
                Action::Media {
                    command: MediaCommand::PlayPause,
                },
            ),
            (
                r#"{"type":"media","command":"previous_track"}"#,
                Action::Media {
                    command: MediaCommand::PreviousTrack,
                },
            ),
            (
                r#"{"type":"system","command":"screenshot"}"#,
                Action::System {
                    command: SystemCommand::Screenshot,
                },
            ),
            (
                r#"{"type":"text","text":"hi\nthere","press_enter":true}"#,
                Action::Text {
                    text: "hi\nthere".into(),
                    press_enter: true,
                },
            ),
            (
                r#"{"type":"multi_action","actions":[{"type":"toggle_mute"},{"type":"system","command":"undo"}]}"#,
                Action::MultiAction {
                    actions: vec![
                        Action::ToggleMute,
                        Action::System {
                            command: SystemCommand::Undo,
                        },
                    ],
                },
            ),
        ] {
            let decoded: Action = serde_json::from_str(json).unwrap();
            assert_eq!(decoded, action);
            assert_eq!(serde_json::to_string(&decoded).unwrap(), json);
            assert!(decoded.validate_executable().is_ok(), "{json}");
        }
        assert_eq!(
            serde_json::from_str::<Action>(r#"{"type":"text","text":"x"}"#).unwrap(),
            text("x")
        );
        for json in [
            r#"{"type":"open_url","url":"https://a.b","extra":1}"#,
            r#"{"type":"open_url"}"#,
            r#"{"type":"media","command":"fast_forward"}"#,
            r#"{"type":"media","command":"play_pause","app":"Music"}"#,
            r#"{"type":"system","command":"shutdown"}"#,
            r#"{"type":"system","command":"undo","modifiers":[]}"#,
            r#"{"type":"text","text":"x","press_enter":"yes"}"#,
            r#"{"type":"text","text":"x","after_enter":true}"#,
            r#"{"type":"multi_action","actions":[{"type":"toggle_mute","x":1},{"type":"toggle_mute"}]}"#,
            r#"{"type":"multi_action","actions":[],"spacing_ms":0}"#,
            r#"{"type":"multi_action","actions":{"type":"toggle_mute"}}"#,
            r#"{"type":"run_script","script":"do shell script \"x\""}"#,
        ] {
            assert!(serde_json::from_str::<Action>(json).is_err(), "{json}");
        }
    }

    #[test]
    fn profile_actions_are_a_closed_strict_set_and_never_multi_action_steps() {
        for (json, direction) in [
            (
                r#"{"type":"profile","direction":"next"}"#,
                ProfileDirection::Next,
            ),
            (
                r#"{"type":"profile","direction":"previous"}"#,
                ProfileDirection::Previous,
            ),
        ] {
            let action: Action = serde_json::from_str(json).unwrap();
            assert_eq!(action, Action::Profile { direction });
            assert_eq!(serde_json::to_string(&action).unwrap(), json);
            assert_eq!(action.type_name(), "profile");
            action.validate_executable().unwrap();
        }
        for json in [
            r#"{"type":"profile"}"#,
            r#"{"type":"profile","direction":"first"}"#,
            r#"{"type":"profile","direction":"NEXT"}"#,
            r#"{"type":"profile","direction":1}"#,
            r#"{"type":"profile","direction":"next","name":"Desk"}"#,
            r#"{"type":"profile","name":"Desk"}"#,
        ] {
            assert!(serde_json::from_str::<Action>(json).is_err(), "{json}");
        }
        let error = Action::MultiAction {
            actions: vec![
                Action::ToggleMute,
                Action::Profile {
                    direction: ProfileDirection::Next,
                },
            ],
        }
        .validate()
        .unwrap_err();
        assert!(
            error.contains("step 2: profile cannot be a multi_action step"),
            "{error}"
        );
        let mut profile = sample();
        profile.bindings[0].action = Action::Profile {
            direction: ProfileDirection::Previous,
        };
        assert!(profile.validate().is_ok());
        profile.bindings[0].module = ServiceModule::M005S;
        profile.bindings[0].gesture = Gesture::PositionChanged;
        assert!(profile.validate().is_err());
    }

    #[test]
    fn open_url_accepts_only_bounded_http_urls_with_a_host() {
        for valid in [
            "https://example.com",
            "http://localhost:8080/path?q=1&r=2#frag",
            "https://[::1]:443/",
            "https://[2001:db8::1]",
            "https://192.168.0.1/x",
            "https://example.com/a%20b",
            "https://example.com:65535/it's",
            "https://sub.EXAMPLE.com/(x)*+,;=!$~",
        ] {
            assert!(url(valid).validate().is_ok(), "{valid}");
        }
        let long = format!("https://example.com/{}", "a".repeat(Action::MAX_URL_BYTES));
        for invalid in [
            "",
            "example.com",
            "ftp://example.com",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "data:text/html,x",
            "HTTP://example.com",
            "http:/example.com",
            "https://",
            "https:///path",
            "https://?q",
            "https://user:pw@example.com",
            "https://example.com@evil.com/",
            "https://@example.com",
            "https://ex ample.com",
            " https://example.com",
            "-u https://example.com",
            "https://example.com/\"; do shell script \"rm -rf ~\"",
            "https://example.com/\" & do shell script \"x\" & \"",
            "https://example.com/a\\b",
            "https://example.com\\@evil.com",
            "https://example.com/\n",
            "https://example.com/\r\nx",
            "https://example.com/\t",
            "https://example.com/\0",
            "https://example.com/<script>",
            "https://example.com/`x`",
            "https://example.com/%zz",
            "https://example.com/%4",
            "https://example.com:0",
            "https://example.com:65536",
            "https://example.com:",
            "https://example.com:8a",
            "https://-example.com",
            "https://example-.com",
            "https://a..com",
            "https://.example.com",
            "https://exa_mple.com",
            "https://é.com",
            "https://[::1",
            "https://[zz]",
            "https://[]",
            "https://[::1]x",
            "https://example.com/[x]",
            "https://ex[a]mple.com",
            &long,
        ] {
            assert!(url(invalid).validate().is_err(), "{invalid:?}");
        }
    }

    #[test]
    fn shortcut_named_keys_are_an_exact_allow_list() {
        for (name, _) in NAMED_KEYS {
            assert!(
                key(
                    name,
                    vec![
                        Modifier::Command,
                        Modifier::Control,
                        Modifier::Option,
                        Modifier::Shift
                    ]
                )
                .validate()
                .is_ok(),
                "{name}"
            );
        }
        assert_eq!(NAMED_KEYS.len(), 34);
        let mut codes: Vec<_> = NAMED_KEYS.iter().map(|(_, code)| *code).collect();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), NAMED_KEYS.len(), "key codes must be distinct");
        for invalid in [
            "F1",
            "f0",
            "f21",
            "Return",
            "enter",
            "esc",
            "left",
            "arrow",
            "return ",
            "return;",
            "\"return\"",
            "key code 36",
            "return\n",
            "36",
            "\" & do shell script \"x",
            ";",
            "-",
        ] {
            assert!(key(invalid, vec![]).validate().is_err(), "{invalid:?}");
        }
    }

    #[test]
    fn text_is_bounded_and_free_of_unsafe_control_characters() {
        for valid in [
            "hello",
            "line one\nline two\tend",
            "zażółć gęślą jaźń",
            "\"; do shell script \"rm -rf ~\" --",
            "\\\" & quit",
        ] {
            assert!(text(valid).validate().is_ok(), "{valid:?}");
        }
        assert!(text(&"x".repeat(Action::MAX_TEXT_BYTES)).validate().is_ok());
        for invalid in [
            String::new(),
            "x".repeat(Action::MAX_TEXT_BYTES + 1),
            "é".repeat(Action::MAX_TEXT_BYTES / 2 + 1),
            "a\rb".into(),
            "\u{7}".into(),
            "\u{1b}[31m".into(),
            "nul\0".into(),
            "\u{85}".into(),
        ] {
            assert!(text(&invalid).validate().is_err(), "{invalid:?}");
        }
    }

    #[test]
    fn text_rejects_invisible_format_and_separator_characters() {
        let mut invisible: Vec<char> = Vec::new();
        invisible.extend('\u{200B}'..='\u{200F}');
        invisible.extend('\u{202A}'..='\u{202E}');
        invisible.extend('\u{2060}'..='\u{2069}');
        invisible.extend([
            '\u{2028}',
            '\u{2029}',
            '\u{FEFF}',
            '\u{00AD}',
            '\u{061C}',
            '\u{180E}',
            '\u{206F}',
            '\u{FFF9}',
            '\u{E0001}',
            '\u{E0041}',
        ]);
        for character in invisible {
            let snippet = format!("safe{character}text");
            assert!(
                text(&snippet).validate().is_err(),
                "U+{:04X} must be rejected",
                character as u32
            );
        }
        for valid in [
            "Zażółć gęślą jaźń ĄĆĘŁŃÓŚŹŻ",
            "Thanks! 🎉👍",
            "heart ❤️ (with variation selector)",
            "flag 🇵🇱",
            "日本語 ελληνικά русский",
            "tab\tand\nnewline",
        ] {
            assert!(text(valid).validate().is_ok(), "{valid:?}");
        }
        // Emoji ZWJ sequences contain U+200D, so they are rejected too.
        assert!(text("family 👨\u{200D}👩\u{200D}👧").validate().is_err());
    }

    #[test]
    fn multi_action_is_bounded_flat_and_validates_every_step() {
        let step = Action::ToggleMute;
        for count in [2, 8] {
            assert!(
                Action::MultiAction {
                    actions: vec![step.clone(); count]
                }
                .validate_executable()
                .is_ok()
            );
        }
        for count in [0, 1, 9] {
            assert!(
                Action::MultiAction {
                    actions: vec![step.clone(); count]
                }
                .validate()
                .is_err()
            );
        }
        let nested = Action::MultiAction {
            actions: vec![step.clone(), step.clone()],
        };
        let error = Action::MultiAction {
            actions: vec![step.clone(), nested],
        }
        .validate()
        .unwrap_err();
        assert!(error.contains("step 2: multi_action cannot be a multi_action step"));
        assert!(
            Action::MultiAction {
                actions: vec![Action::VolumeFromPosition, step.clone()]
            }
            .validate()
            .unwrap_err()
            .contains("step 1: volume_from_position")
        );
        assert!(
            Action::MultiAction {
                actions: vec![step.clone(), url("ftp://example.com")]
            }
            .validate()
            .unwrap_err()
            .starts_with("multi_action step 2: URL")
        );
        let json = r#"{"type":"multi_action","actions":[{"type":"toggle_mute"},{"type":"multi_action","actions":[{"type":"toggle_mute"},{"type":"toggle_mute"}]}]}"#;
        let decoded: Action = serde_json::from_str(json).unwrap();
        assert!(decoded.validate().is_err());
    }

    #[test]
    fn dry_run_plans_new_actions_through_executable_validation() {
        use crate::{ActionDispatcher, DryRunDispatcher};
        let mut dispatcher = DryRunDispatcher::default();
        let plan_only = Action::Media {
            command: MediaCommand::NextTrack,
        };
        let multi = Action::MultiAction {
            actions: vec![url("https://example.com"), plan_only.clone(), text("x")],
        };
        dispatcher.dispatch(&plan_only).unwrap();
        dispatcher.dispatch(&multi).unwrap();
        for invalid in [
            url("file:///etc/passwd"),
            text(""),
            Action::MultiAction {
                actions: vec![Action::ToggleMute, multi.clone()],
            },
        ] {
            assert!(dispatcher.dispatch(&invalid).is_err());
        }
        assert_eq!(dispatcher.actions, [plan_only, multi]);
    }

    #[test]
    fn profiles_accept_new_actions_on_discrete_controls_only() {
        let mut profile = sample();
        profile.bindings[0].action = Action::MultiAction {
            actions: vec![url("https://example.com"), key("f5", vec![])],
        };
        assert!(profile.validate().is_ok());
        let encoded = serde_json::to_vec(&profile).unwrap();
        assert_eq!(Profile::from_json(&encoded).unwrap(), profile);
        profile.bindings[0].module = ServiceModule::M005S;
        profile.bindings[0].gesture = Gesture::PositionChanged;
        assert!(profile.validate().is_err());
        let mut profile = sample();
        profile.bindings[0].action = text("\u{7}");
        assert!(
            profile
                .validate()
                .unwrap_err()
                .starts_with("binding 0: text")
        );
    }

    #[test]
    fn existing_profile_files_round_trip_unchanged() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut paths: Vec<_> = std::fs::read_dir(root.join("../../examples/profiles"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
            .collect();
        paths.push(root.join("tests/fixtures/dial-functions.json"));
        assert!(paths.len() > 1);
        for path in paths {
            let bytes = std::fs::read(&path).unwrap();
            let profile = Profile::from_json(&bytes)
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            assert_eq!(
                serde_json::to_value(&profile).unwrap(),
                serde_json::from_slice::<serde_json::Value>(&bytes).unwrap(),
                "{}",
                path.display()
            );
        }
    }

    #[test]
    fn shared_action_fixture_matches_the_rust_contract() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/actions.json")).unwrap();
        let names: Vec<_> = fixture["named_keys"]
            .as_array()
            .unwrap()
            .iter()
            .map(|name| name.as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            NAMED_KEYS.iter().map(|(name, _)| *name).collect::<Vec<_>>()
        );
        for json in fixture["valid"].as_array().unwrap() {
            let action: Action = serde_json::from_value(json.clone()).unwrap();
            action.validate_executable().unwrap();
            assert_eq!(&serde_json::to_value(&action).unwrap(), json);
        }
        for json in fixture["invalid"].as_array().unwrap() {
            let rejected = serde_json::from_value::<Action>(json.clone())
                .map_err(|error| error.to_string())
                .and_then(|action| action.validate_executable());
            assert!(rejected.is_err(), "{json}");
        }
    }
}
