use serde::{Deserialize, Serialize};
use std::collections::HashSet;

use crate::{DialDisplay, DialFunction, KeypadBinding, KeypadLayout, KeypadPage, KeypadTile};

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
    /// M131E application RIGHT label; verified on hardware as clockwise.
    Right,
    /// M131E application LEFT label; verified on hardware as counter-clockwise.
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

/// Named non-character keys accepted by `shortcut` and `macro` and their macOS
/// virtual key codes.
///
/// The table is the allow-list: a key is either one ASCII alphanumeric byte or
/// space (sent as a literal keystroke) or exactly one of these lowercase names
/// (sent by integer key code). Codes are the Carbon `kVK_*` values from
/// `HIToolbox/Events.h`; hardware behaviour is unverified. The keypad names
/// cover the whole Apple extended keypad; F13–F20 are the last function keys
/// macOS assigns virtual key codes (there are no `kVK_F21`–`kVK_F24`).
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

/// Hardware media keys a `shortcut` can send, and the transport command each
/// one posts. macOS has no virtual key codes for them: they are posted as
/// `NX_SYSDEFINED` media-key events (the fixed media-key script in `macos`),
/// so they can never combine with modifiers and cannot be recorded into a
/// `macro` (the system consumes them before a recording can see them; the
/// `media` action sends them on its own).
pub(crate) const MEDIA_KEYS: &[(&str, MediaCommand)] = &[
    ("play_pause", MediaCommand::PlayPause),
    ("next_track", MediaCommand::NextTrack),
    ("previous_track", MediaCommand::PreviousTrack),
];

/// Returns the transport command behind an allow-listed media key name.
pub(crate) fn media_key_command(name: &str) -> Option<MediaCommand> {
    MEDIA_KEYS
        .iter()
        .find(|(candidate, _)| *candidate == name)
        .map(|(_, command)| *command)
}

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

/// True for the keys that pass `shortcut` and `macro` validation as a single
/// literal: one ASCII alphanumeric byte or space.
pub(crate) fn is_literal_key(key: &str) -> bool {
    key.len() == 1 && (key.as_bytes()[0].is_ascii_alphanumeric() || key.as_bytes()[0] == b' ')
}

/// Returns the macOS virtual key code of an allow-listed named key.
pub(crate) fn named_key_code(name: &str) -> Option<u8> {
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
    /// Stop playback. macOS has no stop media key, so this stops Apple Music
    /// and pauses Spotify, each only when already running.
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

/// What an OBS output action does to an output that can be started and
/// stopped. `toggle` is the only choice the official app offers; `start` and
/// `stop` do not depend on the current state, which makes them safe inside a
/// `multi_action`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObsOutputOp {
    /// Start the output when it is stopped and stop it when it runs.
    Toggle,
    /// Start the output (OBS refuses when it already runs).
    Start,
    /// Stop the output (OBS refuses when it is not running).
    Stop,
}

/// A safeguard on an action that cannot be undone from the controller.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObsConfirm {
    /// The first press only arms the key; a second press of the same binding
    /// within [`crate::TimerTable::CONFIRM_WINDOW_MICROS`] runs the action.
    PressTwice,
}

/// What an `obs_record` action does to the OBS recording.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObsRecordOp {
    /// Start recording when it is stopped and stop it when it runs.
    Toggle,
    /// Start recording (OBS refuses when it already runs).
    Start,
    /// Stop recording (OBS refuses when it is not running).
    Stop,
    /// Pause a running recording (OBS refuses when it is not recording or already paused).
    Pause,
    /// Resume a paused recording (OBS refuses when it is not paused).
    Resume,
    /// Pause a running recording and resume a paused one.
    TogglePause,
}

/// What an `obs_input_mute` action does to an OBS input's mute state.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObsMuteOp {
    /// Mute the input when it is audible and unmute it when it is muted.
    Toggle,
    /// Mute the input.
    Mute,
    /// Unmute the input.
    Unmute,
}

/// What an `obs_media_source` action does to a media source (the official
/// app's four modes). Each reads the source's play state first.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObsMediaOp {
    /// Pause a playing source, play a paused one, restart a stopped or ended one.
    PlayPause,
    /// Stop a playing source, restart any other.
    PlayStop,
    /// Restart a playing, stopped or ended source, play a paused one.
    PlayRestart,
    /// Stop the source.
    Stop,
}

/// What an OBS action does to something that is either on or off (studio
/// mode, a filter, a source's visibility). `toggle` reads the current state
/// from OBS first; `on` and `off` do not depend on it, which makes them safe
/// inside a `multi_action`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObsSwitchOp {
    /// Turn it on when it is off and off when it is on.
    Toggle,
    /// Turn it on.
    On,
    /// Turn it off.
    Off,
}

/// What an `obs_replay_buffer` action does: the output operations plus saving
/// the buffer to a file.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObsReplayBufferOp {
    /// Start the buffer when it is stopped and stop it when it runs.
    Toggle,
    /// Start the buffer.
    Start,
    /// Stop the buffer.
    Stop,
    /// Save the last seconds of the running buffer (OBS refuses when it is not running).
    Save,
}

/// The end sound of a `timer`, from a closed set of macOS system sounds.
///
/// Each name maps to one fixed file under `/System/Library/Sounds`; a profile
/// can never name a path. `none` plays nothing.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimerSound {
    /// `Basso.aiff`.
    Basso,
    /// `Blow.aiff`.
    Blow,
    /// `Bottle.aiff`.
    Bottle,
    /// `Frog.aiff`.
    Frog,
    /// `Funk.aiff`.
    Funk,
    /// `Glass.aiff`, the default when a timer names no sound.
    Glass,
    /// `Hero.aiff`.
    Hero,
    /// `Morse.aiff`.
    Morse,
    /// `Ping.aiff`.
    Ping,
    /// `Pop.aiff`.
    Pop,
    /// `Purr.aiff`.
    Purr,
    /// `Sosumi.aiff`.
    Sosumi,
    /// `Submarine.aiff`.
    Submarine,
    /// `Tink.aiff`.
    Tink,
    /// No sound; only the notification is shown.
    None,
}

impl TimerSound {
    /// Every sound in declaration order, `none` last.
    pub const ALL: [Self; 15] = [
        Self::Basso,
        Self::Blow,
        Self::Bottle,
        Self::Frog,
        Self::Funk,
        Self::Glass,
        Self::Hero,
        Self::Morse,
        Self::Ping,
        Self::Pop,
        Self::Purr,
        Self::Sosumi,
        Self::Submarine,
        Self::Tink,
        Self::None,
    ];

    /// The fixed system sound file, or `None` for [`TimerSound::None`].
    pub fn path(self) -> Option<&'static str> {
        Some(match self {
            Self::Basso => "/System/Library/Sounds/Basso.aiff",
            Self::Blow => "/System/Library/Sounds/Blow.aiff",
            Self::Bottle => "/System/Library/Sounds/Bottle.aiff",
            Self::Frog => "/System/Library/Sounds/Frog.aiff",
            Self::Funk => "/System/Library/Sounds/Funk.aiff",
            Self::Glass => "/System/Library/Sounds/Glass.aiff",
            Self::Hero => "/System/Library/Sounds/Hero.aiff",
            Self::Morse => "/System/Library/Sounds/Morse.aiff",
            Self::Ping => "/System/Library/Sounds/Ping.aiff",
            Self::Pop => "/System/Library/Sounds/Pop.aiff",
            Self::Purr => "/System/Library/Sounds/Purr.aiff",
            Self::Sosumi => "/System/Library/Sounds/Sosumi.aiff",
            Self::Submarine => "/System/Library/Sounds/Submarine.aiff",
            Self::Tink => "/System/Library/Sounds/Tink.aiff",
            Self::None => return None,
        })
    }
}

/// One recorded keystroke of a `macro`: a key held down and released, with
/// the wait before it and how long it stays down.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MacroStep {
    /// One ASCII alphanumeric byte or ASCII space (an upper-case letter is
    /// the same physical key), or one allow-listed named key such as
    /// `arrow_left`, `return`, or `keypad_enter`. Media keys cannot be macro
    /// keys: the system consumes them before a recording sees them, and the
    /// `media` action sends them on its own.
    pub key: String,
    /// Distinct modifiers held while the key is down; order does not matter.
    pub modifiers: Vec<Modifier>,
    /// Wait before the key goes down, in whole milliseconds.
    pub delay_before_ms: u32,
    /// How long the key stays down, in whole milliseconds.
    pub hold_ms: u32,
}

/// The state a `conditional` action tests right before it picks a branch.
///
/// Written as a single-key object such as `{"obs_recording":true}`; a second
/// key or an unknown key is refused. Each variant asks OBS over the local
/// obs-websocket connection, so it needs the same endpoint as an OBS action.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Condition {
    /// OBS is streaming (`true`) or is not (`false`).
    #[serde(rename = "obs_streaming")]
    Streaming(bool),
    /// OBS is recording (a paused recording counts as recording).
    #[serde(rename = "obs_recording")]
    Recording(bool),
    /// The OBS virtual camera is running.
    #[serde(rename = "obs_virtual_camera")]
    VirtualCamera(bool),
    /// The OBS replay buffer is running.
    #[serde(rename = "obs_replay_buffer")]
    ReplayBuffer(bool),
    /// The default output is muted (`true`) or is not (`false`).
    OutputMuted(bool),
    /// The default output volume is at least this percentage, 0 to 100.
    OutputVolumeAtLeast(u8),
}

impl Condition {
    /// The JSON key of this condition, for diagnostics.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Streaming(_) => "obs_streaming",
            Self::Recording(_) => "obs_recording",
            Self::VirtualCamera(_) => "obs_virtual_camera",
            Self::ReplayBuffer(_) => "obs_replay_buffer",
            Self::OutputMuted(_) => "output_muted",
            Self::OutputVolumeAtLeast(_) => "output_volume_at_least",
        }
    }

    /// Whether the condition is read from OBS (and so needs an endpoint).
    pub fn uses_obs(&self) -> bool {
        matches!(
            self,
            Self::Streaming(_)
                | Self::Recording(_)
                | Self::VirtualCamera(_)
                | Self::ReplayBuffer(_)
        )
    }

    /// The value an OBS output condition compares the live state with;
    /// `None` for a condition that is not an OBS output.
    pub fn expected(&self) -> Option<bool> {
        match self {
            Self::Streaming(value)
            | Self::Recording(value)
            | Self::VirtualCamera(value)
            | Self::ReplayBuffer(value) => Some(*value),
            Self::OutputMuted(_) | Self::OutputVolumeAtLeast(_) => None,
        }
    }

    /// Checks the value ranges.
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::OutputVolumeAtLeast(percent) if *percent > 100 => {
                Err("output_volume_at_least must be between 0 and 100".into())
            }
            _ => Ok(()),
        }
    }
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
    /// Set the default input device's volume (the microphone gain) to an
    /// absolute percentage.
    SetMicInputVolume {
        /// Absolute input percentage, between 0 and 100 inclusive.
        percent: u8,
    },
    /// Resolve a fader's absolute position into an absolute microphone input
    /// volume percentage.
    ///
    /// A binding template like [`Action::VolumeFromPosition`]; it must be
    /// resolved using the current input value before it reaches a dispatcher.
    MicInputVolumeFromPosition,
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
    /// Send a media transport command: a hardware media key on macOS for
    /// play/pause, next and previous; `stop` reaches only an already running
    /// Apple Music or Spotify.
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
    /// Test one condition right before running, then run the `then` steps when
    /// it holds or the `otherwise` steps when it does not. A condition that
    /// cannot be read fails the press and runs neither branch. Steps follow
    /// the `multi_action` rules, and a `conditional` is not a step itself.
    Conditional {
        /// The state to test.
        when: Condition,
        /// 1 to 8 steps run when the condition holds.
        then: Vec<Action>,
        /// 0 to 8 steps run when it does not (JSON key `else`).
        #[serde(rename = "else", skip_serializing_if = "Vec::is_empty")]
        otherwise: Vec<Action>,
    },
    /// Run 2 to 8 executable, non-nested actions in order with fixed spacing.
    MultiAction {
        /// Ordered steps; a failure stops the remaining steps.
        actions: Vec<Action>,
    },
    /// Replay a recorded sequence of keystrokes, each with its own wait and
    /// hold time. Bounded: at most [`Action::MAX_MACRO_STEPS`] keystrokes and
    /// at most [`Action::MAX_MACRO_DURATION_MS`] of waits and holds in total.
    /// Not allowed as a multi-action step, so the sequence timing stays in one
    /// place and the whole replay keeps one deadline.
    Macro {
        /// Recorded keystrokes in the order they play.
        steps: Vec<MacroStep>,
    },
    /// Ask the supervising session manager to switch to a neighbouring saved
    /// profile. Never executed on the host; not allowed as a multi-action step.
    Profile {
        /// Next or previous saved profile.
        direction: ProfileDirection,
    },
    /// Start a countdown owned by the running service, or cancel it when the
    /// same binding is pressed again while it runs. At expiry the host shows a
    /// notification and plays the sound. Not allowed as a multi-action step.
    Timer {
        /// Countdown length in whole seconds, 1 to 86400 inclusive.
        seconds: u32,
        /// Optional notification text, validated like a dial function name.
        #[serde(skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        /// End sound; absent means [`TimerSound::Glass`].
        #[serde(skip_serializing_if = "Option::is_none")]
        sound: Option<TimerSound>,
    },
    /// Make a scene the current OBS Studio program scene over the local
    /// obs-websocket v5 server. The endpoint and password are never part of a
    /// profile; a run without them reports the action as unavailable.
    ObsScene {
        /// Exact OBS scene name: nonempty, at most 128 bytes, no control or
        /// invisible format/separator characters.
        scene: String,
    },
    /// Start, stop or toggle the OBS virtual camera over the local
    /// obs-websocket v5 server. Needs an OBS endpoint like `obs_scene`.
    ObsVirtualCamera {
        /// What to do with the virtual camera.
        op: ObsOutputOp,
    },
    /// Start, stop or toggle the OBS stream, which goes live or ends a
    /// broadcast. Needs an OBS endpoint like `obs_scene`.
    ObsStream {
        /// What to do with the stream.
        op: ObsOutputOp,
        /// Optional safeguard: with `press_twice` a key must be pressed twice
        /// within three seconds before going live or ending the broadcast.
        /// Only a key's own action can carry it, not a `multi_action` step.
        #[serde(skip_serializing_if = "Option::is_none")]
        confirm: Option<ObsConfirm>,
    },
    /// Start, stop, pause or resume the OBS recording. Needs an OBS endpoint
    /// like `obs_scene`.
    ObsRecord {
        /// What to do with the recording.
        op: ObsRecordOp,
    },
    /// Start, stop, toggle or save the OBS replay buffer over the local
    /// obs-websocket v5 server. Needs an OBS endpoint like `obs_scene`.
    ObsReplayBuffer {
        /// What to do with the replay buffer.
        op: ObsReplayBufferOp,
    },
    /// Turn OBS studio mode on or off. Needs an OBS endpoint like `obs_scene`.
    ObsStudioMode {
        /// What to do with studio mode.
        op: ObsSwitchOp,
    },
    /// Trigger the studio-mode transition (preview to program). OBS refuses
    /// it when studio mode is off. Needs an OBS endpoint like `obs_scene`.
    ObsTransition,
    /// Save a PNG of the current OBS program scene into the Pictures folder
    /// as `OBS Screenshot <date time>.png`. The location is fixed: a profile
    /// can never name a path. Needs an OBS endpoint like `obs_scene`.
    ObsScreenshot,
    /// Turn a source filter on or off. Both names are matched exactly by OBS.
    ObsFilter {
        /// Exact name of the source (or scene) that owns the filter.
        source: String,
        /// Exact name of the filter on that source.
        filter: String,
        /// What to do with the filter.
        op: ObsSwitchOp,
    },
    /// Show or hide a source inside a scene (a group name works as a scene).
    ObsSourceVisibility {
        /// Exact name of the scene (or group) that holds the source.
        scene: String,
        /// Exact name of the source in that scene.
        source: String,
        /// What to do with the source's visibility.
        op: ObsSwitchOp,
    },
    /// Switch the OBS scene collection by name. OBS reloads its scenes, which
    /// can interrupt a live output. Needs an OBS endpoint like `obs_scene`.
    ObsSceneCollection {
        /// Exact scene collection name.
        name: String,
    },
    /// Switch the OBS profile by name. OBS reloads its settings, which can
    /// interrupt a live output. Needs an OBS endpoint like `obs_scene`.
    ObsProfile {
        /// Exact profile name.
        name: String,
    },
    /// Set an OBS input's volume to an absolute percentage (0 is silent, 100
    /// is unity gain, 0 dB) on OBS's own logarithmic slider scale. Needs an OBS endpoint like `obs_scene`.
    ObsSetInputVolume {
        /// Exact input (source) name.
        input: String,
        /// Absolute volume percentage, between 0 and 100 inclusive.
        percent: u8,
    },
    /// Mute, unmute or toggle an OBS input. Needs an OBS endpoint like `obs_scene`.
    ObsInputMute {
        /// Exact input (source) name.
        input: String,
        /// What to do with the mute state.
        op: ObsMuteOp,
    },
    /// Change an OBS input's volume by a step in decibels, held between -100
    /// dB and 0 dB. Needs an OBS endpoint like `obs_scene`.
    ObsAdjustInputVolume {
        /// Exact input (source) name.
        input: String,
        /// Step in dB: nonzero, between -20 and 20 inclusive.
        step_db: i8,
    },
    /// Play, pause, restart or stop an OBS media source. Needs an OBS
    /// endpoint like `obs_scene`.
    ObsMediaSource {
        /// Exact input (source) name.
        input: String,
        /// What to do with the media source.
        op: ObsMediaOp,
    },
    /// Resolve a fader's absolute position into an absolute OBS input volume.
    ///
    /// Like [`Action::VolumeFromPosition`] this is a binding template that
    /// must be resolved into [`Action::ObsSetInputVolume`] before dispatch.
    ObsInputVolumeFromPosition {
        /// Exact input (source) name.
        input: String,
    },
}

/// Deserializes a present field as `Some`, so an explicit `null` is rejected
/// instead of being read as absent (which would not round-trip).
fn present<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

/// Checks an OBS name (scene, source, filter, scene collection or profile): the keypad label rules plus
/// the invisible-character rule of text snippets, so the name OBS receives is
/// exactly the one reviewed.
fn validate_obs_name(action: &str, field: &str, name: &str) -> Result<(), String> {
    if name.trim().is_empty()
        || name.len() > Action::MAX_OBS_NAME_BYTES
        || name.chars().any(char::is_control)
    {
        return Err(format!(
            "{action} {field} must be nonempty, free of control characters, and at most {} bytes",
            Action::MAX_OBS_NAME_BYTES
        ));
    }
    if name.chars().any(is_invisible_text_character) {
        return Err(format!(
            "{action} {field} must not contain invisible format or line/paragraph separator characters"
        ));
    }
    Ok(())
}

/// Checks a `timer` label: nonempty, no leading or trailing whitespace, at
/// most [`Action::MAX_TIMER_LABEL_BYTES`], and free of control and invisible
/// characters (newlines included), so the notification shows what was reviewed.
pub(crate) fn validate_timer_label(label: &str) -> Result<(), String> {
    if label.trim().is_empty()
        || label.trim() != label
        || label.len() > Action::MAX_TIMER_LABEL_BYTES
        || label
            .chars()
            .any(|c| c.is_control() || is_invisible_text_character(c))
    {
        return Err(format!(
            "timer label must be nonempty, without leading or trailing whitespace, free of control and invisible characters, and at most {} bytes",
            Action::MAX_TIMER_LABEL_BYTES
        ));
    }
    Ok(())
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
            SetMicInputVolume {
                percent: u8,
            },
            MicInputVolumeFromPosition {},
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
            Conditional {
                when: Condition,
                then: Vec<Action>,
                #[serde(default, rename = "else")]
                otherwise: Vec<Action>,
            },
            MultiAction {
                actions: Vec<Action>,
            },
            Macro {
                steps: Vec<MacroStep>,
            },
            Profile {
                direction: ProfileDirection,
            },
            Timer {
                seconds: u32,
                #[serde(default, deserialize_with = "present")]
                label: Option<String>,
                #[serde(default, deserialize_with = "present")]
                sound: Option<TimerSound>,
            },
            ObsScene {
                scene: String,
            },
            ObsVirtualCamera {
                op: ObsOutputOp,
            },
            ObsStream {
                op: ObsOutputOp,
                #[serde(default, deserialize_with = "present")]
                confirm: Option<ObsConfirm>,
            },
            ObsRecord {
                op: ObsRecordOp,
            },
            ObsReplayBuffer {
                op: ObsReplayBufferOp,
            },
            ObsStudioMode {
                op: ObsSwitchOp,
            },
            ObsTransition {},
            ObsScreenshot {},
            ObsFilter {
                source: String,
                filter: String,
                op: ObsSwitchOp,
            },
            ObsSourceVisibility {
                scene: String,
                source: String,
                op: ObsSwitchOp,
            },
            ObsSceneCollection {
                name: String,
            },
            ObsProfile {
                name: String,
            },
            ObsSetInputVolume {
                input: String,
                percent: u8,
            },
            ObsInputMute {
                input: String,
                op: ObsMuteOp,
            },
            ObsAdjustInputVolume {
                input: String,
                step_db: i8,
            },
            ObsMediaSource {
                input: String,
                op: ObsMediaOp,
            },
            ObsInputVolumeFromPosition {
                input: String,
            },
        }
        Ok(match Shape::deserialize(deserializer)? {
            Shape::Volume { delta } => Self::Volume { delta },
            Shape::SetVolume { percent } => Self::SetVolume { percent },
            Shape::VolumeFromPosition {} => Self::VolumeFromPosition,
            Shape::SetMicInputVolume { percent } => Self::SetMicInputVolume { percent },
            Shape::MicInputVolumeFromPosition {} => Self::MicInputVolumeFromPosition,
            Shape::ToggleMute {} => Self::ToggleMute,
            Shape::Shortcut { key, modifiers } => Self::Shortcut { key, modifiers },
            Shape::LaunchApp { bundle_id } => Self::LaunchApp { bundle_id },
            Shape::OpenUrl { url } => Self::OpenUrl { url },
            Shape::Media { command } => Self::Media { command },
            Shape::System { command } => Self::System { command },
            Shape::Text { text, press_enter } => Self::Text { text, press_enter },
            Shape::Conditional {
                when,
                then,
                otherwise,
            } => Self::Conditional {
                when,
                then,
                otherwise,
            },
            Shape::MultiAction { actions } => Self::MultiAction { actions },
            Shape::Macro { steps } => Self::Macro { steps },
            Shape::Profile { direction } => Self::Profile { direction },
            Shape::Timer {
                seconds,
                label,
                sound,
            } => Self::Timer {
                seconds,
                label,
                sound,
            },
            Shape::ObsScene { scene } => Self::ObsScene { scene },
            Shape::ObsVirtualCamera { op } => Self::ObsVirtualCamera { op },
            Shape::ObsStream { op, confirm } => Self::ObsStream { op, confirm },
            Shape::ObsRecord { op } => Self::ObsRecord { op },
            Shape::ObsReplayBuffer { op } => Self::ObsReplayBuffer { op },
            Shape::ObsStudioMode { op } => Self::ObsStudioMode { op },
            Shape::ObsTransition {} => Self::ObsTransition,
            Shape::ObsScreenshot {} => Self::ObsScreenshot,
            Shape::ObsFilter { source, filter, op } => Self::ObsFilter { source, filter, op },
            Shape::ObsSourceVisibility { scene, source, op } => {
                Self::ObsSourceVisibility { scene, source, op }
            }
            Shape::ObsSceneCollection { name } => Self::ObsSceneCollection { name },
            Shape::ObsProfile { name } => Self::ObsProfile { name },
            Shape::ObsSetInputVolume { input, percent } => {
                Self::ObsSetInputVolume { input, percent }
            }
            Shape::ObsInputMute { input, op } => Self::ObsInputMute { input, op },
            Shape::ObsAdjustInputVolume { input, step_db } => {
                Self::ObsAdjustInputVolume { input, step_db }
            }
            Shape::ObsMediaSource { input, op } => Self::ObsMediaSource { input, op },
            Shape::ObsInputVolumeFromPosition { input } => {
                Self::ObsInputVolumeFromPosition { input }
            }
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
    /// Shortest `timer` countdown in seconds.
    pub const MIN_TIMER_SECONDS: u32 = 1;
    /// Longest `timer` countdown in seconds (24 hours).
    pub const MAX_TIMER_SECONDS: u32 = 86_400;
    /// Maximum `timer` label length in UTF-8 bytes.
    pub const MAX_TIMER_LABEL_BYTES: usize = 64;
    /// Maximum length in UTF-8 bytes of any OBS name (scene, source, filter, scene collection, profile).
    pub const MAX_OBS_NAME_BYTES: usize = 128;
    /// Maximum `obs_scene` scene name length in UTF-8 bytes.
    pub const MAX_OBS_SCENE_BYTES: usize = Self::MAX_OBS_NAME_BYTES;
    /// Maximum number of keystrokes in a `macro`.
    pub const MAX_MACRO_STEPS: usize = 64;
    /// Longest wait before a macro keystroke, in milliseconds.
    pub const MAX_MACRO_DELAY_MS: u32 = 10_000;
    /// Longest hold of a macro keystroke, in milliseconds.
    pub const MAX_MACRO_HOLD_MS: u32 = 5_000;
    /// Longest wall-clock time of a whole `macro` (waits plus holds), in
    /// milliseconds. The replay timeout is derived from it in `macos`.
    pub const MAX_MACRO_DURATION_MS: u32 = 30_000;

    /// The JSON `type` tag of this action, for diagnostics.
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Volume { .. } => "volume",
            Self::SetVolume { .. } => "set_volume",
            Self::VolumeFromPosition => "volume_from_position",
            Self::SetMicInputVolume { .. } => "set_mic_input_volume",
            Self::MicInputVolumeFromPosition => "mic_input_volume_from_position",
            Self::ToggleMute => "toggle_mute",
            Self::Shortcut { .. } => "shortcut",
            Self::LaunchApp { .. } => "launch_app",
            Self::OpenUrl { .. } => "open_url",
            Self::Media { .. } => "media",
            Self::System { .. } => "system",
            Self::Text { .. } => "text",
            Self::Conditional { .. } => "conditional",
            Self::MultiAction { .. } => "multi_action",
            Self::Macro { .. } => "macro",
            Self::Profile { .. } => "profile",
            Self::Timer { .. } => "timer",
            Self::ObsScene { .. } => "obs_scene",
            Self::ObsVirtualCamera { .. } => "obs_virtual_camera",
            Self::ObsStream { .. } => "obs_stream",
            Self::ObsRecord { .. } => "obs_record",
            Self::ObsReplayBuffer { .. } => "obs_replay_buffer",
            Self::ObsStudioMode { .. } => "obs_studio_mode",
            Self::ObsTransition => "obs_transition",
            Self::ObsScreenshot => "obs_screenshot",
            Self::ObsFilter { .. } => "obs_filter",
            Self::ObsSourceVisibility { .. } => "obs_source_visibility",
            Self::ObsSceneCollection { .. } => "obs_scene_collection",
            Self::ObsProfile { .. } => "obs_profile",
            Self::ObsSetInputVolume { .. } => "obs_set_input_volume",
            Self::ObsInputMute { .. } => "obs_input_mute",
            Self::ObsAdjustInputVolume { .. } => "obs_adjust_input_volume",
            Self::ObsMediaSource { .. } => "obs_media_source",
            Self::ObsInputVolumeFromPosition { .. } => "obs_input_volume_from_position",
        }
    }

    /// True when a press only arms the action and a second press of the same
    /// binding runs it (see [`ObsConfirm`]).
    pub fn needs_confirmation(&self) -> bool {
        matches!(
            self,
            Self::ObsStream {
                confirm: Some(ObsConfirm::PressTwice),
                ..
            }
        )
    }

    /// True for an OBS action (`obs_*`) or a `multi_action` with an OBS step.
    pub fn uses_obs(&self) -> bool {
        match self {
            Self::ObsScene { .. }
            | Self::ObsVirtualCamera { .. }
            | Self::ObsStream { .. }
            | Self::ObsRecord { .. }
            | Self::ObsReplayBuffer { .. }
            | Self::ObsStudioMode { .. }
            | Self::ObsTransition
            | Self::ObsScreenshot
            | Self::ObsFilter { .. }
            | Self::ObsSourceVisibility { .. }
            | Self::ObsSceneCollection { .. }
            | Self::ObsProfile { .. }
            | Self::ObsSetInputVolume { .. }
            | Self::ObsInputMute { .. }
            | Self::ObsAdjustInputVolume { .. }
            | Self::ObsMediaSource { .. }
            | Self::ObsInputVolumeFromPosition { .. } => true,
            Self::Conditional {
                when,
                then,
                otherwise,
            } => when.uses_obs() || then.iter().chain(otherwise).any(Self::uses_obs),
            Self::MultiAction { actions } => actions.iter().any(Self::uses_obs),
            _ => false,
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
                if media_key_command(key).is_some() {
                    // The media-key event carries no modifier flags (see
                    // `macos::MEDIA_KEY_SCRIPT`), so a chord would be dropped.
                    if !modifiers.is_empty() {
                        return Err("a media key cannot combine with modifiers".into());
                    }
                    return Ok(());
                }
                if !is_literal_key(key) && named_key_code(key).is_none() {
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
            Self::Macro { steps } => {
                if steps.is_empty() || steps.len() > Self::MAX_MACRO_STEPS {
                    return Err(format!(
                        "macro must contain 1 to {} keystrokes",
                        Self::MAX_MACRO_STEPS
                    ));
                }
                let mut total_ms = 0u64;
                for (index, step) in steps.iter().enumerate() {
                    if !is_literal_key(&step.key) && named_key_code(&step.key).is_none() {
                        if media_key_command(&step.key).is_some() {
                            return Err(format!(
                                "macro step {}: media keys cannot be macro keys; the media action sends them",
                                index + 1
                            ));
                        }
                        return Err(format!(
                            "macro step {}: key must be one ASCII alphanumeric key, space, or a supported named key",
                            index + 1
                        ));
                    }
                    if step.modifiers.len() > 4 {
                        return Err(format!("macro step {}: at most four modifiers", index + 1));
                    }
                    let unique: HashSet<_> = step.modifiers.iter().collect();
                    if unique.len() != step.modifiers.len() {
                        return Err(format!(
                            "macro step {}: modifiers must not contain duplicates",
                            index + 1
                        ));
                    }
                    if step.delay_before_ms > Self::MAX_MACRO_DELAY_MS {
                        return Err(format!(
                            "macro step {}: the wait must be at most {} ms",
                            index + 1,
                            Self::MAX_MACRO_DELAY_MS
                        ));
                    }
                    if step.hold_ms > Self::MAX_MACRO_HOLD_MS {
                        return Err(format!(
                            "macro step {}: the hold must be at most {} ms",
                            index + 1,
                            Self::MAX_MACRO_HOLD_MS
                        ));
                    }
                    total_ms += u64::from(step.delay_before_ms) + u64::from(step.hold_ms);
                    if total_ms > u64::from(Self::MAX_MACRO_DURATION_MS) {
                        return Err(format!(
                            "macro waits and holds must total at most {} ms",
                            Self::MAX_MACRO_DURATION_MS
                        ));
                    }
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
            Self::ObsScene { scene } => validate_obs_name("obs_scene", "scene", scene),
            Self::ObsSceneCollection { name } => {
                validate_obs_name("obs_scene_collection", "name", name)
            }
            Self::ObsProfile { name } => validate_obs_name("obs_profile", "name", name),
            Self::ObsFilter { source, filter, .. } => {
                validate_obs_name("obs_filter", "source", source)?;
                validate_obs_name("obs_filter", "filter", filter)
            }
            Self::ObsSetInputVolume { input, percent } => {
                validate_obs_name("obs_set_input_volume", "input", input)?;
                if *percent > 100 {
                    return Err("absolute OBS input volume must be between 0 and 100".into());
                }
                Ok(())
            }
            Self::ObsInputVolumeFromPosition { input } => {
                validate_obs_name("obs_input_volume_from_position", "input", input)
            }
            Self::ObsInputMute { input, .. } => validate_obs_name("obs_input_mute", "input", input),
            Self::ObsMediaSource { input, .. } => {
                validate_obs_name("obs_media_source", "input", input)
            }
            Self::ObsAdjustInputVolume { input, step_db } => {
                validate_obs_name("obs_adjust_input_volume", "input", input)?;
                if *step_db == 0 || !(-20..=20).contains(step_db) {
                    return Err("OBS volume step must be nonzero and between -20 and 20 dB".into());
                }
                Ok(())
            }
            Self::ObsSourceVisibility { scene, source, .. } => {
                validate_obs_name("obs_source_visibility", "scene", scene)?;
                validate_obs_name("obs_source_visibility", "source", source)
            }
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
            Self::Conditional {
                when,
                then,
                otherwise,
            } => {
                when.validate()?;
                if then.is_empty() || then.len() > Self::MAX_MULTI_ACTION_STEPS {
                    return Err(format!(
                        "conditional must run 1 to {} steps when the condition holds",
                        Self::MAX_MULTI_ACTION_STEPS
                    ));
                }
                if otherwise.len() > Self::MAX_MULTI_ACTION_STEPS {
                    return Err(format!(
                        "conditional else must have at most {} steps",
                        Self::MAX_MULTI_ACTION_STEPS
                    ));
                }
                Self::validate_steps(then, "conditional then", "conditional")?;
                Self::validate_steps(otherwise, "conditional else", "conditional")
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
                Self::validate_steps(actions, "multi_action", "multi_action")?;
                Ok(())
            }
            Self::Timer { seconds, label, .. } => {
                if !(Self::MIN_TIMER_SECONDS..=Self::MAX_TIMER_SECONDS).contains(seconds) {
                    return Err(format!(
                        "timer seconds must be between {} and {}",
                        Self::MIN_TIMER_SECONDS,
                        Self::MAX_TIMER_SECONDS
                    ));
                }
                label.as_deref().map_or(Ok(()), validate_timer_label)
            }
            Self::SetMicInputVolume { percent } => {
                if *percent > 100 {
                    return Err("absolute microphone input volume must be between 0 and 100".into());
                }
                Ok(())
            }
            Self::Volume { .. }
            | Self::SetVolume { .. }
            | Self::VolumeFromPosition
            | Self::MicInputVolumeFromPosition
            | Self::ToggleMute
            | Self::Media { .. }
            | Self::System { .. }
            | Self::Profile { .. }
            | Self::ObsVirtualCamera { .. }
            | Self::ObsStream { .. }
            | Self::ObsRecord { .. }
            | Self::ObsReplayBuffer { .. }
            | Self::ObsStudioMode { .. }
            | Self::ObsTransition
            | Self::ObsScreenshot => Ok(()),
        }
    }

    /// Validates the steps of a `multi_action` or a `conditional` branch.
    ///
    /// A profile switch stops the running service, so it could never be
    /// followed by the remaining steps. A timer toggles per binding, which a
    /// step does not have; the official app refuses timers in multi-task lists
    /// too. A macro is already a timed sequence; nesting one would split the
    /// overall deadline across two clocks. A `conditional` is not a step, so a
    /// branch can never nest another test or loop.
    fn validate_steps(steps: &[Action], label: &str, noun: &str) -> Result<(), String> {
        for (index, step) in steps.iter().enumerate() {
            if matches!(
                step,
                Self::MultiAction { .. }
                    | Self::Conditional { .. }
                    | Self::Macro { .. }
                    | Self::VolumeFromPosition
                    | Self::MicInputVolumeFromPosition
                    | Self::ObsInputVolumeFromPosition { .. }
                    | Self::Profile { .. }
                    | Self::Timer { .. }
                    | Self::ObsStream {
                        confirm: Some(_),
                        ..
                    }
            ) {
                return Err(format!(
                    "{label} step {}: {} cannot be a {noun} step",
                    index + 1,
                    step.type_name()
                ));
            }
            step.validate()
                .map_err(|error| format!("{label} step {}: {error}", index + 1))?;
        }
        Ok(())
    }

    /// Validates an operation and rejects templates lacking a resolved input value.
    pub fn validate_executable(&self) -> Result<(), String> {
        self.validate()?;
        if self.is_fader_template() {
            Err(format!(
                "{} must be resolved with a fader event before dispatch",
                self.type_name()
            ))
        } else {
            Ok(())
        }
    }

    /// True for a binding template that follows a fader's absolute position
    /// and must be resolved into an absolute action before dispatch.
    pub fn is_fader_template(&self) -> bool {
        matches!(
            self,
            Self::VolumeFromPosition
                | Self::MicInputVolumeFromPosition
                | Self::ObsInputVolumeFromPosition { .. }
        )
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
    /// Configuration schema: 1 for flat bindings, 2 or 3 for a keypad page tree. Schema 3 is the current one: every
    /// profile this version saves carries it, and [`Profile::into_current`] upgrades the older two.
    pub schema_version: u32,
    /// Human-readable name of at most 128 bytes.
    pub name: String,
    /// At most 256 bindings with unique complete selectors.
    pub bindings: Vec<Binding>,
    /// Keypad pages (schema 2 and later); omitted from schema-1 profiles.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keypad: Option<KeypadLayout>,
    /// Optional ordered M131E functions cycled by a short click (schema 1 or 2).
    ///
    /// When present, every M131E binding lives in these functions and flat
    /// `bindings` must not select M131E. Omitted when absent, so existing
    /// profiles serialize unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dial_functions: Option<Vec<DialFunction>>,
    /// What the M131E dial screen shows while this profile runs (schema 1 or
    /// 2). Absent means the default: the active function's name, or the analog
    /// clock for a profile with nothing on the dial. Omitted when absent, so
    /// existing profiles serialize unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dial_display: Option<DialDisplay>,
    /// How bright the module screens are while this profile runs and the app
    /// drives them. Absent means full brightness. Omitted when absent, so
    /// existing profiles serialize unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screen_brightness: Option<ScreenBrightness>,
}

/// The brightness of the module screens, in percent. A screen left out shows at full brightness.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScreenBrightness {
    /// The 15 key screens: each key picture is drawn at this share of its brightness.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keys: Option<u8>,
    /// The dial screen's own backlight.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dial: Option<u8>,
}

impl ScreenBrightness {
    /// The dimmest accepted setting; a darker screen cannot be read.
    pub const MIN: u8 = 10;

    fn validate(&self) -> Result<(), String> {
        for (name, value) in [("keys", self.keys), ("dial", self.dial)] {
            if let Some(value) = value
                && !(Self::MIN..=100).contains(&value)
            {
                return Err(format!(
                    "screen_brightness: {name} must be between {} and 100",
                    Self::MIN
                ));
            }
        }
        Ok(())
    }
}

impl Profile {
    /// Maximum accepted JSON document size before parsing.
    pub const MAX_JSON_BYTES: usize = 1024 * 1024;
    /// Maximum number of bindings in one profile.
    pub const MAX_BINDINGS: usize = 256;
    /// The oldest schema this version still reads: flat bindings.
    pub const OLDEST_SCHEMA: u32 = 1;
    /// The schema this version writes and the newest it reads.
    ///
    /// History: 1 flat bindings; 2 keypad page tree, with dial functions, dial display and screen brightness added as
    /// optional fields; 3 the same layout under a defined upgrade path, so a newer file is recognised as newer rather
    /// than refused as malformed; 4 adds the `conditional` action. Raise it, and add an upgrade step to [`Profile::into_current`], whenever a change
    /// would make an older build misread a profile.
    pub const CURRENT_SCHEMA: u32 = 4;

    /// Parses bounded JSON and checks its complete semantic constraints.
    pub fn from_json(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > Self::MAX_JSON_BYTES {
            return Err(format!("profile exceeds {} bytes", Self::MAX_JSON_BYTES));
        }
        // A profile from a newer version may carry fields this one does not know, so its schema is read first.
        #[derive(Deserialize)]
        struct Peek {
            schema_version: u32,
        }
        if let Ok(Peek { schema_version }) = serde_json::from_slice(bytes)
            && schema_version > Self::CURRENT_SCHEMA
        {
            return Err(format!(
                "this profile is schema {schema_version}, made by a newer version of Open Stream Hub; this version reads schemas {} through {}. Update the app to open it",
                Self::OLDEST_SCHEMA,
                Self::CURRENT_SCHEMA
            ));
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
            (2..=4, Some(keypad)) => keypad.validate()?,
            (1, Some(_)) => return Err("schema 1 profiles must omit keypad pages".into()),
            (version @ 2..=4, None) => {
                return Err(format!("schema {version} profiles require keypad pages"));
            }
            (version, _) => {
                return Err(format!(
                    "unsupported profile schema {version} (expected {} through {})",
                    Self::OLDEST_SCHEMA,
                    Self::CURRENT_SCHEMA
                ));
            }
        }
        // Key artwork and a dial animation are the bulky parts; the encoded
        // size is checked as a whole whenever either is present.
        let has_media = self
            .keypad
            .as_ref()
            .is_some_and(|layout| layout.pages.iter().any(|page| !page.artwork.is_empty()))
            || self
                .dial_display
                .as_ref()
                .is_some_and(|display| display.animation.is_some());
        if has_media
            && serde_json::to_vec(self)
                .map_err(|error| error.to_string())?
                .len()
                > Self::MAX_JSON_BYTES
        {
            return Err(format!("profile exceeds {} bytes", Self::MAX_JSON_BYTES));
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
        if let Some(display) = &self.dial_display {
            crate::dial::validate_display(display)?;
        }
        if let Some(brightness) = &self.screen_brightness {
            brightness.validate()?;
        }
        if self.binding_count() > Self::MAX_BINDINGS {
            return Err(format!("profile exceeds {} bindings", Self::MAX_BINDINGS));
        }
        let mut selectors = HashSet::new();
        for (index, binding) in self.bindings.iter().enumerate() {
            if self.uses_keypad_pages() && binding.module == ServiceModule::M4315 {
                return Err(format!(
                    "binding {index}: keypad bindings belong in keypad action tiles (schema {})",
                    self.schema_version
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
            if (binding.module == ServiceModule::M005S) != binding.action.is_fader_template() {
                return Err(format!(
                    "binding {index}: fader bindings require a fader action (volume_from_position, mic_input_volume_from_position or obs_input_volume_from_position), which are supported only for faders"
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

    /// Whether the keypad is a page tree (schema 2 and later) rather than flat bindings (schema 1).
    pub fn uses_keypad_pages(&self) -> bool {
        self.schema_version >= 2
    }

    /// Upgrades a valid profile to the current schema without writing to storage.
    ///
    /// A flat profile gets its keypad tree first: every M4315 selector becomes a binding on its original root key,
    /// retaining gesture, serial selector, and action, and other module bindings keep their original order. A schema-2
    /// profile only has its version raised. A profile already at the current schema is returned unchanged.
    pub fn into_current(mut self) -> Result<Self, String> {
        self.validate()?;
        if self.schema_version == Self::CURRENT_SCHEMA {
            return Ok(self);
        }
        if self.uses_keypad_pages() {
            self.schema_version = Self::CURRENT_SCHEMA;
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
        self.schema_version = Self::CURRENT_SCHEMA;
        self.keypad = Some(KeypadLayout {
            pages: vec![KeypadPage {
                id: KeypadLayout::ROOT_PAGE_ID.into(),
                name: "Root".into(),
                slots,
                artwork: Default::default(),
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
            dial_display: None,
            screen_brightness: None,
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

    /// A valid 240×240 baseline JFIF JPEG, as the dial display accepts.
    fn dial_jpeg() -> String {
        use base64::{Engine, engine::general_purpose::STANDARD};
        let image = image::RgbImage::from_pixel(240, 240, image::Rgb([10, 20, 30]));
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut jpeg)
            .encode_image(&image)
            .unwrap();
        STANDARD.encode(jpeg)
    }

    #[test]
    fn screen_brightness_is_optional_bounded_and_strict() {
        let plain =
            Profile::from_json(br#"{"schema_version":1,"name":"Desk","bindings":[]}"#).unwrap();
        assert_eq!(plain.screen_brightness, None);
        assert!(
            !serde_json::to_string(&plain)
                .unwrap()
                .contains("screen_brightness")
        );
        let set = Profile::from_json(
            br#"{"schema_version":1,"name":"Desk","bindings":[],"screen_brightness":{"keys":60,"dial":10}}"#,
        )
        .unwrap();
        assert_eq!(
            set.screen_brightness,
            Some(ScreenBrightness {
                keys: Some(60),
                dial: Some(10)
            })
        );
        let half = Profile::from_json(
            br#"{"schema_version":1,"name":"Desk","bindings":[],"screen_brightness":{"keys":100}}"#,
        )
        .unwrap();
        assert_eq!(half.screen_brightness.unwrap().dial, None);
        for bad in [
            r#"{"keys":9}"#,
            r#"{"dial":101}"#,
            r#"{"dial":0}"#,
            r#"{"keys":50,"other":1}"#,
        ] {
            let json = format!(
                r#"{{"schema_version":1,"name":"Desk","bindings":[],"screen_brightness":{bad}}}"#
            );
            assert!(Profile::from_json(json.as_bytes()).is_err(), "{bad}");
        }
    }

    #[test]
    fn dial_display_is_optional_and_parses_strictly() {
        // Absent: parses, and serializes without the field.
        let plain =
            Profile::from_json(br#"{"schema_version":1,"name":"Desk","bindings":[]}"#).unwrap();
        assert_eq!(plain.dial_display, None);
        assert!(
            !serde_json::to_vec(&plain)
                .unwrap()
                .windows(12)
                .any(|window| window == b"dial_display")
        );
        // Every content value is spelled exactly once, and only image may
        // carry the picture.
        let image = dial_jpeg();
        for (content, with_image) in [
            ("off", false),
            ("function", false),
            ("clock", false),
            ("analog_clock", false),
            ("cpu", false),
            ("image", true),
        ] {
            let image_field = if with_image {
                format!(r#","image":"{image}""#)
            } else {
                String::new()
            };
            let display = format!(r#"{{"content":"{content}"{image_field}}}"#);
            let json = format!(
                r#"{{"schema_version":1,"name":"Desk","bindings":[],"dial_display":{display}}}"#
            );
            let profile = Profile::from_json(json.as_bytes()).unwrap();
            let parsed = profile.dial_display.clone().unwrap();
            assert_eq!(parsed.image.is_some(), with_image, "content {content}");
            // The exact bytes round-trip.
            assert_eq!(
                serde_json::to_value(&profile).unwrap()["dial_display"],
                serde_json::from_str::<serde_json::Value>(&display).unwrap()
            );
        }
        for bad in [
            // Unknown content and wrong spelling.
            r#"{"schema_version":1,"name":"Desk","bindings":[],"dial_display":{"content":"gpu"}}"#,
            r#"{"schema_version":1,"name":"Desk","bindings":[],"dial_display":{"content":"analog-clock"}}"#,
            r#"{"schema_version":1,"name":"Desk","bindings":[],"dial_display":{"content":"Image"}}"#,
            // Image content without the picture, and the picture elsewhere.
            r#"{"schema_version":1,"name":"Desk","bindings":[],"dial_display":{"content":"image"}}"#,
            r#"{"schema_version":1,"name":"Desk","bindings":[],"dial_display":{"content":"clock","image":"AAAA"}}"#,
            // Unknown fields, on the setting and beside the image.
            r#"{"schema_version":1,"name":"Desk","bindings":[],"dial_display":{"content":"clock","style":"bold"}}"#,
            r#"{"schema_version":1,"name":"Desk","bindings":[],"dial_display":{"content":"image","image":"AAAA","fit":"cover"}}"#,
            r#"{"schema_version":1,"name":"Desk","bindings":[],"dial_display":"clock"}"#,
        ] {
            assert!(
                Profile::from_json(bad.as_bytes()).is_err(),
                "accepted {bad}"
            );
        }
    }

    #[test]
    fn a_dial_display_image_must_be_a_bounded_valid_dial_jpeg() {
        use base64::Engine as _;
        let image = dial_jpeg();
        let with_image = |image: &str| {
            format!(
                r#"{{"schema_version":1,"name":"Desk","bindings":[],"dial_display":{{"content":"image","image":"{image}"}}}}"#
            )
        };
        Profile::from_json(with_image(&image).as_bytes()).unwrap();
        for (label, image) in [
            ("not base64", "not base64!"),
            ("empty", ""),
            (
                "wrong device bytes",
                &base64::engine::general_purpose::STANDARD.encode(b"at least it decodes"),
            ),
        ] {
            assert!(
                Profile::from_json(with_image(image).as_bytes()).is_err(),
                "accepted {label}"
            );
        }
        // Over the decoded cap, by exactly two bytes: the text passes the
        // pre-check, so the refusal names the decoded size.
        let oversized = "A".repeat(DialDisplay::MAX_IMAGE_BYTES.div_ceil(3) * 4);
        let error = Profile::from_json(with_image(&oversized).as_bytes()).unwrap_err();
        assert!(error.contains("64 KiB"), "{error}");
        // The real cap accepts a full encoder; one byte more of decoded JPEG
        // is refused. Build a JPEG just over the cap by appending noise is not
        // possible (it must stay valid), so a padded base64 of a valid JPEG
        // over the text cap is the same refusal path.
        let padded = format!("{image}{}", "A".repeat(4));
        assert!(
            Profile::from_json(with_image(&padded).as_bytes())
                .unwrap_err()
                .contains("not valid base64")
        );
    }

    #[test]
    fn a_dial_animation_is_bounded_and_starts_with_the_still_image() {
        use base64::{Engine, engine::general_purpose::STANDARD};
        let first = dial_jpeg();
        let other = {
            let image = image::RgbImage::from_pixel(240, 240, image::Rgb([200, 20, 30]));
            let mut jpeg = Vec::new();
            image::codecs::jpeg::JpegEncoder::new(&mut jpeg)
                .encode_image(&image)
                .unwrap();
            STANDARD.encode(jpeg)
        };
        let frame =
            |jpeg: &str, delay: u32| format!(r#"{{"jpeg_base64":"{jpeg}","delay_ms":{delay}}}"#);
        let profile = |content: &str, image: &str, frames: &[String], extra: &str| {
            let image = if image.is_empty() {
                String::new()
            } else {
                format!(r#","image":"{image}""#)
            };
            Profile::from_json(
                format!(
                    r#"{{"schema_version":1,"name":"Desk","bindings":[],"dial_display":{{"content":"{content}"{image}{extra},"animation":{{"frames":[{}],"loop_count":0}}}}}}"#,
                    frames.join(",")
                )
                .as_bytes(),
            )
        };
        let two = [frame(&first, 200), frame(&other, 60_000)];
        let ok = profile("image", &first, &two, "").unwrap();
        // Serializing and reading it back changes nothing.
        let again = Profile::from_json(&serde_json::to_vec(&ok).unwrap()).unwrap();
        assert_eq!(again, ok);
        let many: Vec<String> = (0..31).map(|_| frame(&first, 200)).collect();
        let source = format!(
            r#","source":{{"image":"{}","fit":"contain","quarter_turns":0,"background":[0,0,0]}}"#,
            STANDARD.encode(b"\x89PNG\r\n\x1a\n")
        );
        for (label, result) in [
            ("one frame", profile("image", &first, &two[..1], "")),
            ("31 frames", profile("image", &first, &many, "")),
            (
                "too short",
                profile(
                    "image",
                    &first,
                    &[frame(&first, 65), frame(&other, 150)],
                    "",
                ),
            ),
            (
                "too long",
                profile(
                    "image",
                    &first,
                    &[frame(&first, 200), frame(&other, 60_001)],
                    "",
                ),
            ),
            ("still differs", profile("image", &other, &two, "")),
            ("no still", profile("image", "", &two, "")),
            ("wrong content", profile("clock", &first, &two, "")),
            ("with a source", profile("image", &first, &two, &source)),
            (
                "not a jpeg",
                profile(
                    "image",
                    &first,
                    &[frame(&first, 200), frame("AAAA", 200)],
                    "",
                ),
            ),
            (
                "bad base64",
                profile(
                    "image",
                    &first,
                    &[frame(&first, 200), frame("!!!!", 200)],
                    "",
                ),
            ),
        ] {
            assert!(result.is_err(), "{label} should be refused");
        }
        // The source rule itself is what refuses a source beside an animation.
        let error = profile("image", &first, &two, &source).unwrap_err();
        assert!(error.to_string().contains("no source picture"), "{error}");
    }

    #[test]
    fn a_dial_frame_may_not_last_less_than_its_size_allows() {
        use base64::{Engine, engine::general_purpose::STANDARD};
        // Pseudo-random pixels do not compress, so the JPEG size follows the quality.
        let noise = |quality: u8| {
            let mut seed = 0x2545_f491_u32;
            let image = image::RgbImage::from_fn(240, 240, |_, _| {
                let mut next = || {
                    seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    (seed >> 24) as u8
                };
                image::Rgb([next(), next(), next()])
            });
            let mut jpeg = Vec::new();
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, quality)
                .encode_image(&image)
                .unwrap();
            jpeg
        };
        let profile = |jpeg: &[u8], delay: u32| {
            let encoded = STANDARD.encode(jpeg);
            Profile::from_json(
                format!(
                    r#"{{"schema_version":1,"name":"Desk","bindings":[],"dial_display":{{"content":"image","image":"{encoded}","animation":{{"frames":[{{"jpeg_base64":"{encoded}","delay_ms":{delay}}},{{"jpeg_base64":"{encoded}","delay_ms":60000}}],"loop_count":0}}}}}}"#
                )
                .as_bytes(),
            )
        };
        let mut floors = std::collections::BTreeSet::new();
        for quality in [4, 12, 25, 40] {
            let jpeg = noise(quality);
            let floor = crate::dial::DialAnimation::min_delay_ms_for(jpeg.len());
            floors.insert(floor);
            let at = profile(&jpeg, floor);
            assert!(at.is_ok(), "{} bytes at {floor} ms: {at:?}", jpeg.len());
            let below = profile(&jpeg, floor - 1).unwrap_err();
            assert!(
                below.contains(&format!("lasts {floor} through")),
                "{} bytes: {below}",
                jpeg.len()
            );
        }
        // The four qualities reach all three size classes.
        assert_eq!(
            floors.into_iter().collect::<Vec<_>>(),
            [
                crate::dial::DialAnimation::FASTEST_DELAY_MS,
                crate::dial::DialAnimation::MEDIUM_DELAY_MS,
                crate::dial::DialAnimation::MIN_DELAY_MS
            ]
        );
    }

    #[test]
    fn the_dial_floor_has_three_steps_at_the_size_boundaries() {
        let small = crate::dial::DialAnimation::SMALL_FRAME_BYTES;
        let medium = crate::dial::DialAnimation::MEDIUM_FRAME_BYTES;
        for (bytes, floor) in [
            (0, 66),
            (small, 66),
            (small + 1, 100),
            (medium, 100),
            (medium + 1, 150),
            (crate::dial::DialAnimation::MAX_FRAME_BYTES, 150),
        ] {
            assert_eq!(
                crate::dial::DialAnimation::min_delay_ms_for(bytes),
                floor,
                "{bytes} bytes"
            );
        }
    }

    #[test]
    fn a_dial_source_picture_is_kept_only_with_the_image_it_made() {
        use base64::Engine as _;
        let encode = |bytes: &[u8]| base64::engine::general_purpose::STANDARD.encode(bytes);
        let png = encode(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 1, 2, 3]);
        let image = dial_jpeg();
        let profile = |content: &str, image: Option<&str>, source: &str| {
            let image = image.map_or(String::new(), |image| format!(r#","image":"{image}""#));
            format!(
                r#"{{"schema_version":1,"name":"Desk","bindings":[],"dial_display":{{"content":"{content}"{image},"source":{source}}}}}"#
            )
        };
        let source = |image: &str, turns: u8| {
            format!(
                r#"{{"image":"{image}","fit":"contain","quarter_turns":{turns},"background":[1,2,3]}}"#
            )
        };
        let parsed =
            Profile::from_json(profile("image", Some(&image), &source(&png, 3)).as_bytes())
                .unwrap();
        let kept = parsed.dial_display.clone().unwrap().source.unwrap();
        assert_eq!(kept.fit, crate::DialFit::Contain);
        assert_eq!(kept.background, [1, 2, 3]);
        // It survives a round trip.
        let again = Profile::from_json(&serde_json::to_vec(&parsed).unwrap()).unwrap();
        assert_eq!(again.dial_display, parsed.dial_display);
        for (label, json) in [
            (
                "turns above 3",
                profile("image", Some(&image), &source(&png, 4)),
            ),
            (
                "not a picture",
                profile("image", Some(&image), &source(&encode(b"plain text"), 0)),
            ),
            (
                "not base64",
                profile("image", Some(&image), &source("not base64!", 0)),
            ),
            ("other content", profile("clock", None, &source(&png, 0))),
            (
                "without the image",
                profile("image", None, &source(&png, 0)),
            ),
            (
                "unknown fit",
                profile(
                    "image",
                    Some(&image),
                    &format!(
                        r#"{{"image":"{png}","fit":"zoom","quarter_turns":0,"background":[0,0,0]}}"#
                    ),
                ),
            ),
        ] {
            assert!(
                Profile::from_json(json.as_bytes()).is_err(),
                "accepted {label}"
            );
        }
        let oversized = encode(&vec![0xff; crate::DialSource::MAX_IMAGE_BYTES + 3]);
        let error =
            Profile::from_json(profile("image", Some(&image), &source(&oversized, 0)).as_bytes())
                .unwrap_err();
        assert!(error.contains("256 KiB"), "{error}");
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
    fn a_profile_from_a_newer_version_is_named_as_such_even_with_fields_this_version_lacks() {
        let next = Profile::CURRENT_SCHEMA + 1;
        let json = format!(
            r#"{{"schema_version":{next},"name":"Desk","bindings":[],"a_future_field":{{"x":1}}}}"#
        );
        let error = Profile::from_json(json.as_bytes()).unwrap_err();
        assert!(error.contains(&format!("schema {next}")), "{error}");
        assert!(error.contains("newer version"), "{error}");
        assert!(error.contains("Update the app"), "{error}");
        // Schema 0 was never written, so that is malformed rather than newer.
        let zero = Profile::from_json(br#"{"schema_version":0,"name":"Desk","bindings":[]}"#);
        assert!(zero.unwrap_err().contains("unsupported profile schema 0"));
        // A file claiming a schema this version reads still gets the unknown-field refusal.
        let current = format!(
            r#"{{"schema_version":{},"name":"Desk","bindings":[],"a_future_field":1}}"#,
            Profile::CURRENT_SCHEMA
        );
        assert!(Profile::from_json(current.as_bytes()).is_err());
    }

    #[test]
    fn every_older_schema_upgrades_to_the_current_one_and_stays_valid() {
        let flat = sample();
        assert_eq!(flat.schema_version, 1);
        let from_flat = flat.clone().into_current().unwrap();
        assert_eq!(from_flat.schema_version, Profile::CURRENT_SCHEMA);
        assert!(from_flat.keypad.is_some());
        let mut pages = from_flat.clone();
        pages.schema_version = 2;
        assert!(pages.validate().is_ok());
        let from_pages = pages.clone().into_current().unwrap();
        assert_eq!(from_pages.schema_version, Profile::CURRENT_SCHEMA);
        assert_eq!(from_pages.keypad, pages.keypad);
        assert_eq!(from_pages, from_flat);
        // Already current: returned as it is, and upgrading again changes nothing.
        assert_eq!(from_flat.clone().into_current().unwrap(), from_flat);
        // Round trip through JSON keeps the schema.
        let json = serde_json::to_vec(&from_flat).unwrap();
        assert_eq!(Profile::from_json(&json).unwrap(), from_flat);
    }

    #[test]
    fn schemas_two_and_three_need_pages_and_one_must_not_have_them() {
        let mut profile = sample().into_current().unwrap();
        profile.keypad = None;
        for version in [2, 3] {
            profile.schema_version = version;
            assert!(
                profile
                    .validate()
                    .unwrap_err()
                    .contains("require keypad pages")
            );
        }
        let mut with_pages = sample().into_current().unwrap();
        with_pages.schema_version = 1;
        assert!(
            with_pages
                .validate()
                .unwrap_err()
                .contains("must omit keypad pages")
        );
        with_pages.schema_version = Profile::CURRENT_SCHEMA + 1;
        assert!(
            with_pages
                .validate()
                .unwrap_err()
                .contains("unsupported profile schema")
        );
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
    fn microphone_volume_actions_parse_validate_and_stay_out_of_multi_actions() {
        let template: Action =
            serde_json::from_str(r#"{"type":"mic_input_volume_from_position"}"#).unwrap();
        assert_eq!(template, Action::MicInputVolumeFromPosition);
        assert_eq!(template.type_name(), "mic_input_volume_from_position");
        assert!(template.is_fader_template());
        assert!(template.validate().is_ok());
        assert!(template.validate_executable().is_err());
        assert!(
            serde_json::from_str::<Action>(
                r#"{"type":"mic_input_volume_from_position","percent":1}"#
            )
            .is_err()
        );
        let fixed: Action =
            serde_json::from_str(r#"{"type":"set_mic_input_volume","percent":40}"#).unwrap();
        assert_eq!(fixed, Action::SetMicInputVolume { percent: 40 });
        assert!(fixed.validate_executable().is_ok());
        assert!(
            Action::SetMicInputVolume { percent: 101 }
                .validate()
                .is_err()
        );
        let nested = Action::MultiAction {
            actions: vec![Action::MicInputVolumeFromPosition],
        };
        assert!(nested.validate().is_err());
    }

    #[test]
    fn obs_input_volume_actions_validate_names_levels_and_fader_scope() {
        let template = Action::ObsInputVolumeFromPosition {
            input: "Mic/Aux".into(),
        };
        let fixed = Action::ObsSetInputVolume {
            input: "Mic/Aux".into(),
            percent: 100,
        };
        assert!(template.validate().is_ok() && fixed.validate().is_ok());
        assert!(template.is_fader_template() && !fixed.is_fader_template());
        assert!(template.uses_obs() && fixed.uses_obs());
        // The template is not executable until a fader event resolves it.
        assert!(
            template
                .validate_executable()
                .unwrap_err()
                .contains("obs_input_volume_from_position")
        );
        assert!(fixed.validate_executable().is_ok());
        for bad in ["", "  ", "a\nb", "a\u{200b}b", &"x".repeat(129)] {
            assert!(
                Action::ObsSetInputVolume {
                    input: bad.into(),
                    percent: 5
                }
                .validate()
                .is_err()
            );
            assert!(
                Action::ObsInputVolumeFromPosition { input: bad.into() }
                    .validate()
                    .is_err()
            );
        }
        assert!(
            Action::ObsSetInputVolume {
                input: "Mic".into(),
                percent: 101
            }
            .validate()
            .is_err()
        );

        // Fader bindings take either template and nothing else; no other control takes one.
        let mut profile = sample();
        profile.bindings[0].module = ServiceModule::M005S;
        profile.bindings[0].gesture = Gesture::PositionChanged;
        profile.bindings[0].control = 1;
        profile.bindings[0].action = template.clone();
        assert!(profile.validate().is_ok());
        profile.bindings[0].action = fixed.clone();
        assert!(profile.validate().is_err());
        profile.bindings[0].module = ServiceModule::M003K;
        profile.bindings[0].control = 0;
        profile.bindings[0].gesture = Gesture::Clockwise;
        profile.bindings[0].action = template.clone();
        assert!(profile.validate().is_err());

        // Never a multi_action step.
        assert!(
            Action::MultiAction {
                actions: vec![Action::ToggleMute, template]
            }
            .validate()
            .is_err()
        );
        assert!(
            Action::MultiAction {
                actions: vec![Action::ToggleMute, fixed]
            }
            .validate()
            .is_ok()
        );
    }

    #[test]
    fn obs_input_volume_json_is_strict() {
        let parsed: Action =
            serde_json::from_str(r#"{"type":"obs_set_input_volume","input":"Mic","percent":40}"#)
                .unwrap();
        assert_eq!(
            parsed,
            Action::ObsSetInputVolume {
                input: "Mic".into(),
                percent: 40
            }
        );
        let parsed: Action =
            serde_json::from_str(r#"{"type":"obs_input_volume_from_position","input":"Mic"}"#)
                .unwrap();
        assert_eq!(parsed.type_name(), "obs_input_volume_from_position");
        for bad in [
            r#"{"type":"obs_set_input_volume","input":"Mic"}"#,
            r#"{"type":"obs_set_input_volume","input":"Mic","percent":-1}"#,
            r#"{"type":"obs_input_volume_from_position","input":"Mic","percent":1}"#,
            r#"{"type":"obs_input_volume_from_position"}"#,
        ] {
            assert!(serde_json::from_str::<Action>(bad).is_err(), "{bad}");
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
            (
                r#"{"type":"macro","steps":[{"key":"c","modifiers":["command"],"delay_before_ms":0,"hold_ms":80},{"key":"arrow_left","modifiers":[],"delay_before_ms":120,"hold_ms":40}]}"#,
                macro_action(vec![
                    macro_step("c", vec![Modifier::Command], 0, 80),
                    macro_step("arrow_left", vec![], 120, 40),
                ]),
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
            r#"{"type":"macro","steps":[{"key":"c","modifiers":["command"],"delay_before_ms":0,"hold_ms":80}],"timeout_ms":500}"#,
            r#"{"type":"macro","steps":[{"key":"c","modifiers":["command"],"delay_before_ms":0}]}"#,
            r#"{"type":"macro","steps":[{"key":"c","modifiers":["command"],"delay_before_ms":0,"hold_ms":80,"extra":1}]}"#,
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

    fn timer(seconds: u32, label: Option<&str>, sound: Option<TimerSound>) -> Action {
        Action::Timer {
            seconds,
            label: label.map(str::to_owned),
            sound,
        }
    }

    fn obs(scene: &str) -> Action {
        Action::ObsScene {
            scene: scene.into(),
        }
    }

    #[test]
    fn timer_actions_have_strict_bounded_round_trips() {
        for (json, action) in [
            (r#"{"type":"timer","seconds":1}"#, timer(1, None, None)),
            (
                r#"{"type":"timer","seconds":86400,"label":"Tea","sound":"submarine"}"#,
                timer(86_400, Some("Tea"), Some(TimerSound::Submarine)),
            ),
            (
                r#"{"type":"timer","seconds":300,"sound":"none"}"#,
                timer(300, None, Some(TimerSound::None)),
            ),
            (
                r#"{"type":"timer","seconds":5,"label":"\" & do shell script \"id\" `x` $(id)"}"#,
                timer(5, Some("\" & do shell script \"id\" `x` $(id)"), None),
            ),
        ] {
            let decoded: Action = serde_json::from_str(json).unwrap();
            assert_eq!(decoded, action);
            assert_eq!(serde_json::to_string(&decoded).unwrap(), json);
            assert_eq!(decoded.type_name(), "timer");
            decoded.validate_executable().unwrap();
        }
        for json in [
            r#"{"type":"timer"}"#,
            r#"{"type":"timer","seconds":0}"#,
            r#"{"type":"timer","seconds":86401}"#,
            r#"{"type":"timer","seconds":-1}"#,
            r#"{"type":"timer","seconds":1.5}"#,
            r#"{"type":"timer","seconds":"60"}"#,
            r#"{"type":"timer","seconds":4294967296}"#,
            r#"{"type":"timer","seconds":60,"minutes":1}"#,
            r#"{"type":"timer","seconds":60,"label":null}"#,
            r#"{"type":"timer","seconds":60,"sound":null}"#,
            r#"{"type":"timer","seconds":60,"label":7}"#,
            r#"{"type":"timer","seconds":60,"sound":"Glass"}"#,
            r#"{"type":"timer","seconds":60,"sound":"Glass.aiff"}"#,
            r#"{"type":"timer","seconds":60,"sound":"/System/Library/Sounds/Glass.aiff"}"#,
            r#"{"type":"timer","seconds":60,"sound":"../../tmp/x"}"#,
            r#"{"type":"timer","seconds":60,"sound":"beep"}"#,
            r#"{"type":"timer","seconds":60,"label":""}"#,
            r#"{"type":"timer","seconds":60,"label":"a\nb"}"#,
        ] {
            let rejected = serde_json::from_str::<Action>(json)
                .map_err(|error| error.to_string())
                .and_then(|action| action.validate_executable());
            assert!(rejected.is_err(), "{json}");
        }
    }

    #[test]
    fn timer_labels_follow_the_name_and_invisible_character_rules() {
        let longest = "é".repeat(Action::MAX_TIMER_LABEL_BYTES / 2);
        for valid in [
            "Tea",
            "Stream break — 5 min",
            "zażółć 🎉",
            "'quotes' \"and\" `ticks` $(sub)",
            longest.as_str(),
        ] {
            assert!(timer(60, Some(valid), None).validate().is_ok(), "{valid:?}");
        }
        for invalid in [
            String::new(),
            " ".into(),
            " lead".into(),
            "trail ".into(),
            "line\nbreak".into(),
            "cr\rx".into(),
            "tab\tx".into(),
            "nul\0".into(),
            "esc\u{1b}[31m".into(),
            "rtl\u{202E}txt".into(),
            "zw\u{200B}sp".into(),
            "sep\u{2028}x".into(),
            "bom\u{FEFF}".into(),
            "x".repeat(Action::MAX_TIMER_LABEL_BYTES + 1),
        ] {
            assert!(
                timer(60, Some(&invalid), None).validate().is_err(),
                "{invalid:?}"
            );
        }
    }

    #[test]
    fn obs_scene_has_a_strict_additive_json_shape_and_label_rules() {
        let json = r#"{"type":"obs_scene","scene":"Starting soon"}"#;
        let action: Action = serde_json::from_str(json).unwrap();
        assert_eq!(action, obs("Starting soon"));
        assert_eq!(serde_json::to_string(&action).unwrap(), json);
        assert_eq!(action.type_name(), "obs_scene");
        action.validate_executable().unwrap();
        assert!(action.uses_obs());
        assert!(!Action::ToggleMute.uses_obs());
        for valid in [
            "Live",
            "  padded  ",
            "Zażółć 🎥 \"quoted\" {json} ' & do shell script",
            &"x".repeat(Action::MAX_OBS_SCENE_BYTES),
        ] {
            assert!(obs(valid).validate().is_ok(), "{valid:?}");
        }
        for invalid in [
            String::new(),
            "   ".into(),
            "x".repeat(Action::MAX_OBS_SCENE_BYTES + 1),
            "é".repeat(Action::MAX_OBS_SCENE_BYTES / 2 + 1),
            "line\nbreak".into(),
            "tab\t".into(),
            "nul\0".into(),
            "\u{1b}[31m".into(),
            "zero\u{200B}width".into(),
            "bidi\u{202E}".into(),
            "sep\u{2028}".into(),
            "bom\u{FEFF}".into(),
            "tag\u{E0041}".into(),
        ] {
            assert!(obs(&invalid).validate().is_err(), "{invalid:?}");
        }
        for json in [
            r#"{"type":"obs_scene"}"#,
            r#"{"type":"obs_scene","scene":1}"#,
            r#"{"type":"obs_scene","scene":null}"#,
            r#"{"type":"obs_scene","scene":"Live","endpoint":"127.0.0.1:4455"}"#,
            r#"{"type":"obs_scene","scene":"Live","password":"x"}"#,
            r#"{"type":"obs_scene","name":"Live"}"#,
            r#"{"type":"OBS_scene","scene":"Live"}"#,
        ] {
            assert!(serde_json::from_str::<Action>(json).is_err(), "{json}");
        }
    }

    #[test]
    fn timer_sounds_are_a_closed_set_of_fixed_system_files() {
        let names: Vec<_> = TimerSound::ALL
            .iter()
            .map(|sound| serde_json::to_value(sound).unwrap())
            .collect();
        assert_eq!(names.len(), 15);
        assert_eq!(names.last().unwrap(), "none");
        for sound in TimerSound::ALL {
            match sound.path() {
                None => assert_eq!(sound, TimerSound::None),
                Some(path) => {
                    let name = serde_json::to_value(sound).unwrap();
                    let name = name.as_str().unwrap();
                    let expected = format!(
                        "/System/Library/Sounds/{}{}.aiff",
                        name[..1].to_uppercase(),
                        &name[1..]
                    );
                    assert_eq!(path, expected);
                }
            }
        }
    }

    #[test]
    fn conditional_round_trips_and_is_strict() {
        let json = r#"{"type":"conditional","when":{"obs_recording":true},"then":[{"type":"obs_record","op":"stop"}],"else":[{"type":"obs_record","op":"start"}]}"#;
        let action: Action = serde_json::from_str(json).unwrap();
        action.validate().unwrap();
        assert_eq!(
            action,
            Action::Conditional {
                when: Condition::Recording(true),
                then: vec![Action::ObsRecord {
                    op: ObsRecordOp::Stop
                }],
                otherwise: vec![Action::ObsRecord {
                    op: ObsRecordOp::Start
                }],
            }
        );
        assert_eq!(serde_json::to_string(&action).unwrap(), json);

        // `else` is optional and is left out again when empty.
        let short = r#"{"type":"conditional","when":{"obs_streaming":false},"then":[{"type":"toggle_mute"}]}"#;
        let action: Action = serde_json::from_str(short).unwrap();
        action.validate().unwrap();
        assert_eq!(serde_json::to_string(&action).unwrap(), short);

        for bad in [
            // two keys in `when`, an unknown condition, a non-boolean value
            r#"{"type":"conditional","when":{"obs_recording":true,"obs_streaming":true},"then":[{"type":"toggle_mute"}]}"#,
            r#"{"type":"conditional","when":{"obs_cloud":true},"then":[{"type":"toggle_mute"}]}"#,
            r#"{"type":"conditional","when":{"obs_recording":"yes"},"then":[{"type":"toggle_mute"}]}"#,
            // a missing condition or branch, and an unknown field
            r#"{"type":"conditional","then":[{"type":"toggle_mute"}]}"#,
            r#"{"type":"conditional","when":{"obs_recording":true}}"#,
            r#"{"type":"conditional","when":{"obs_recording":true},"then":[{"type":"toggle_mute"}],"extra":1}"#,
        ] {
            assert!(serde_json::from_str::<Action>(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn conditional_branches_follow_the_multi_action_step_rules() {
        let mute = || Action::ToggleMute;
        let conditional = |then: Vec<Action>, otherwise: Vec<Action>| Action::Conditional {
            when: Condition::Streaming(true),
            then,
            otherwise,
        };
        conditional(vec![mute()], vec![]).validate().unwrap();
        conditional(vec![mute(); 8], vec![mute(); 8])
            .validate()
            .unwrap();

        assert!(
            conditional(vec![], vec![mute()])
                .validate()
                .unwrap_err()
                .contains("1 to 8 steps")
        );
        assert!(
            conditional(vec![mute(); 9], vec![])
                .validate()
                .unwrap_err()
                .contains("1 to 8 steps")
        );
        assert!(
            conditional(vec![mute()], vec![mute(); 9])
                .validate()
                .unwrap_err()
                .contains("at most 8 steps")
        );

        // No nesting, no loops, and nothing a multi_action step cannot be.
        let nested = conditional(vec![mute()], vec![]);
        for step in [
            nested.clone(),
            Action::MultiAction {
                actions: vec![mute(), mute()],
            },
            Action::Macro {
                steps: vec![MacroStep {
                    key: "a".into(),
                    modifiers: vec![],
                    delay_before_ms: 0,
                    hold_ms: 50,
                }],
            },
            Action::Profile {
                direction: ProfileDirection::Next,
            },
            Action::Timer {
                seconds: 5,
                label: None,
                sound: None,
            },
            Action::VolumeFromPosition,
        ] {
            let name = step.type_name();
            let error = conditional(vec![mute()], vec![step.clone()])
                .validate()
                .unwrap_err();
            assert!(
                error.contains(&format!("conditional else step 1: {name} cannot be")),
                "{error}"
            );
            let error = conditional(vec![step], vec![]).validate().unwrap_err();
            assert!(
                error.contains(&format!("conditional then step 1: {name} cannot be")),
                "{error}"
            );
        }

        // An invalid step is named with its branch.
        let error = conditional(vec![mute(), Action::SetVolume { percent: 101 }], vec![])
            .validate()
            .unwrap_err();
        assert!(error.contains("conditional then step 2"), "{error}");
        // A conditional is never a multi_action step either.
        let error = Action::MultiAction {
            actions: vec![mute(), nested],
        }
        .validate()
        .unwrap_err();
        assert!(
            error.contains("conditional cannot be a multi_action step"),
            "{error}"
        );
    }

    #[test]
    fn system_conditions_parse_validate_and_need_obs_only_for_obs_steps() {
        let json = r#"{"type":"conditional","when":{"output_volume_at_least":50},"then":[{"type":"toggle_mute"}],"else":[{"type":"set_volume","percent":50}]}"#;
        let action: Action = serde_json::from_str(json).unwrap();
        action.validate().unwrap();
        assert_eq!(serde_json::to_string(&action).unwrap(), json);
        assert!(!action.uses_obs());
        let muted: Action = serde_json::from_str(
            r#"{"type":"conditional","when":{"output_muted":false},"then":[{"type":"obs_scene","scene":"Live"}]}"#,
        )
        .unwrap();
        muted.validate().unwrap();
        assert!(muted.uses_obs(), "an OBS step still needs the endpoint");
        let high: Action = serde_json::from_str(
            r#"{"type":"conditional","when":{"output_volume_at_least":101},"then":[{"type":"toggle_mute"}]}"#,
        )
        .unwrap();
        assert!(high.validate().unwrap_err().contains("between 0 and 100"));
        assert!(
            serde_json::from_str::<Action>(
                r#"{"type":"conditional","when":{"output_volume_at_least":-1},"then":[{"type":"toggle_mute"}]}"#
            )
            .is_err()
        );
    }

    #[test]
    fn a_conditional_needs_obs_even_when_its_steps_do_not() {
        let action = Action::Conditional {
            when: Condition::Recording(true),
            then: vec![Action::ToggleMute],
            otherwise: vec![],
        };
        assert!(action.uses_obs());
        assert_eq!(action.type_name(), "conditional");
    }

    #[test]
    fn timers_bind_to_discrete_controls_only_and_never_as_multi_action_steps() {
        let error = Action::MultiAction {
            actions: vec![Action::ToggleMute, timer(5, None, None)],
        }
        .validate()
        .unwrap_err();
        assert!(
            error.contains("step 2: timer cannot be a multi_action step"),
            "{error}"
        );
        let mut profile = sample();
        profile.bindings[0].action = timer(90, Some("Tea"), Some(TimerSound::Glass));
        assert!(profile.validate().is_ok());
        let encoded = serde_json::to_vec(&profile).unwrap();
        assert_eq!(Profile::from_json(&encoded).unwrap(), profile);
        profile.bindings[0].module = ServiceModule::M4315;
        profile.bindings[0].gesture = Gesture::Pressed;
        assert!(profile.validate().is_ok());
        profile.bindings[0].module = ServiceModule::M005S;
        profile.bindings[0].gesture = Gesture::PositionChanged;
        assert!(
            profile.validate().is_err(),
            "faders accept only volume_from_position"
        );
        let mut profile = sample();
        profile.bindings[0].action = timer(0, None, None);
        assert!(
            profile
                .validate()
                .unwrap_err()
                .starts_with("binding 0: timer seconds")
        );
    }

    #[test]
    fn dry_run_plans_timers_without_side_effects() {
        use crate::{ActionDispatcher, DryRunDispatcher, TimerAlert};
        let mut dispatcher = DryRunDispatcher::default();
        let action = timer(5, Some("Tea"), None);
        dispatcher.dispatch(&action).unwrap();
        assert!(dispatcher.dispatch(&timer(0, None, None)).is_err());
        let alert = TimerAlert::from_action(&action).unwrap();
        dispatcher.timer_alert(&alert).unwrap();
        assert_eq!(dispatcher.actions, [action]);
        assert_eq!(dispatcher.timer_alerts, [alert]);
    }

    #[test]
    fn obs_scene_binds_to_discrete_controls_and_multi_action_steps_only() {
        let mut profile = sample();
        profile.bindings[0].action = obs("Live");
        assert!(profile.validate().is_ok());
        let encoded = serde_json::to_vec(&profile).unwrap();
        assert_eq!(Profile::from_json(&encoded).unwrap(), profile);
        profile.bindings[0].module = ServiceModule::M005S;
        profile.bindings[0].gesture = Gesture::PositionChanged;
        assert!(
            profile.validate().is_err(),
            "faders accept only volume_from_position"
        );
        let multi = Action::MultiAction {
            actions: vec![obs("Live"), Action::ToggleMute],
        };
        multi.validate_executable().unwrap();
        assert!(multi.uses_obs());
        let error = Action::MultiAction {
            actions: vec![Action::ToggleMute, obs("")],
        }
        .validate()
        .unwrap_err();
        assert!(
            error.starts_with("multi_action step 2: obs_scene"),
            "{error}"
        );
        let mut profile = sample();
        profile.bindings[0].action = obs("bad\u{200D}");
        assert!(
            profile
                .validate()
                .unwrap_err()
                .starts_with("binding 0: obs_scene")
        );
    }

    #[test]
    fn obs_audio_and_media_actions_have_a_closed_json_shape() {
        for json in [
            r#"{"type":"obs_input_mute","input":"Mic","op":"toggle"}"#,
            r#"{"type":"obs_input_mute","input":"Mic","op":"mute"}"#,
            r#"{"type":"obs_input_mute","input":"Mic","op":"unmute"}"#,
            r#"{"type":"obs_adjust_input_volume","input":"Mic","step_db":-3}"#,
            r#"{"type":"obs_adjust_input_volume","input":"Mic","step_db":20}"#,
            r#"{"type":"obs_media_source","input":"Intro","op":"play_pause"}"#,
            r#"{"type":"obs_media_source","input":"Intro","op":"play_stop"}"#,
            r#"{"type":"obs_media_source","input":"Intro","op":"play_restart"}"#,
            r#"{"type":"obs_media_source","input":"Intro","op":"stop"}"#,
        ] {
            let action: Action = serde_json::from_str(json).unwrap();
            assert!(action.uses_obs());
            action.validate_executable().unwrap();
            assert_eq!(serde_json::to_string(&action).unwrap(), json);
        }
        for json in [
            r#"{"type":"obs_input_mute","input":"Mic"}"#,
            r#"{"type":"obs_input_mute","input":"Mic","op":"on"}"#,
            r#"{"type":"obs_media_source","input":"Intro","op":"play"}"#,
            r#"{"type":"obs_media_source","op":"stop"}"#,
            r#"{"type":"obs_adjust_input_volume","input":"Mic"}"#,
            r#"{"type":"obs_adjust_input_volume","input":"Mic","step_db":1.5}"#,
            r#"{"type":"obs_adjust_input_volume","input":"Mic","step_db":300}"#,
        ] {
            assert!(serde_json::from_str::<Action>(json).is_err(), "{json}");
        }
        for step_db in [0, 21, -21, i8::MIN] {
            let action = Action::ObsAdjustInputVolume {
                input: "Mic".into(),
                step_db,
            };
            assert!(action.validate().is_err(), "{step_db}");
        }
        let empty = Action::ObsMediaSource {
            input: " ".into(),
            op: ObsMediaOp::Stop,
        };
        assert!(empty.validate().is_err());
    }

    #[test]
    fn a_stream_confirm_is_closed_and_only_for_a_keys_own_action() {
        let json = r#"{"type":"obs_stream","op":"toggle","confirm":"press_twice"}"#;
        let action: Action = serde_json::from_str(json).unwrap();
        assert!(action.needs_confirmation());
        action.validate_executable().unwrap();
        assert_eq!(serde_json::to_string(&action).unwrap(), json);
        let plain: Action = serde_json::from_str(r#"{"type":"obs_stream","op":"stop"}"#).unwrap();
        assert!(!plain.needs_confirmation());
        for json in [
            r#"{"type":"obs_stream","op":"toggle","confirm":null}"#,
            r#"{"type":"obs_stream","op":"toggle","confirm":"hold"}"#,
            r#"{"type":"obs_stream","op":"toggle","confirm":true}"#,
            r#"{"type":"obs_record","op":"toggle","confirm":"press_twice"}"#,
        ] {
            assert!(serde_json::from_str::<Action>(json).is_err(), "{json}");
        }
        // A multi_action step cannot carry it; the unguarded stream is fine.
        let step = |confirm| Action::MultiAction {
            actions: vec![
                Action::ToggleMute,
                Action::ObsStream {
                    op: ObsOutputOp::Start,
                    confirm,
                },
            ],
        };
        step(None).validate().unwrap();
        assert!(step(Some(ObsConfirm::PressTwice)).validate().is_err());
    }

    #[test]
    fn obs_stream_and_record_actions_have_a_closed_json_shape() {
        for json in [
            r#"{"type":"obs_stream","op":"toggle"}"#,
            r#"{"type":"obs_stream","op":"start"}"#,
            r#"{"type":"obs_stream","op":"stop"}"#,
            r#"{"type":"obs_record","op":"toggle"}"#,
            r#"{"type":"obs_record","op":"start"}"#,
            r#"{"type":"obs_record","op":"stop"}"#,
            r#"{"type":"obs_record","op":"pause"}"#,
            r#"{"type":"obs_record","op":"resume"}"#,
            r#"{"type":"obs_record","op":"toggle_pause"}"#,
        ] {
            let action: Action = serde_json::from_str(json).unwrap();
            assert!(action.uses_obs());
            action.validate_executable().unwrap();
            assert_eq!(serde_json::to_string(&action).unwrap(), json);
        }
        for json in [
            r#"{"type":"obs_stream"}"#,
            r#"{"type":"obs_stream","op":"pause"}"#,
            r#"{"type":"obs_stream","op":"toggle","endpoint":"127.0.0.1:4455"}"#,
            r#"{"type":"obs_record"}"#,
            r#"{"type":"obs_record","op":"save"}"#,
            r#"{"type":"obs_record","op":"Toggle"}"#,
        ] {
            assert!(serde_json::from_str::<Action>(json).is_err(), "{json}");
        }
        let nested = Action::MultiAction {
            actions: vec![
                Action::ObsRecord {
                    op: ObsRecordOp::Start,
                },
                Action::ObsStream {
                    op: ObsOutputOp::Start,
                    confirm: None,
                },
            ],
        };
        assert!(nested.uses_obs());
        nested.validate_executable().unwrap();
    }

    #[test]
    fn obs_output_actions_have_a_strict_closed_json_shape() {
        for (json, type_name) in [
            (
                r#"{"type":"obs_virtual_camera","op":"toggle"}"#,
                "obs_virtual_camera",
            ),
            (
                r#"{"type":"obs_virtual_camera","op":"start"}"#,
                "obs_virtual_camera",
            ),
            (
                r#"{"type":"obs_virtual_camera","op":"stop"}"#,
                "obs_virtual_camera",
            ),
            (
                r#"{"type":"obs_replay_buffer","op":"toggle"}"#,
                "obs_replay_buffer",
            ),
            (
                r#"{"type":"obs_replay_buffer","op":"start"}"#,
                "obs_replay_buffer",
            ),
            (
                r#"{"type":"obs_replay_buffer","op":"stop"}"#,
                "obs_replay_buffer",
            ),
            (
                r#"{"type":"obs_replay_buffer","op":"save"}"#,
                "obs_replay_buffer",
            ),
        ] {
            let action: Action = serde_json::from_str(json).unwrap();
            assert_eq!(action.type_name(), type_name);
            assert!(action.uses_obs());
            action.validate_executable().unwrap();
            assert_eq!(serde_json::to_string(&action).unwrap(), json);
        }
        for json in [
            r#"{"type":"obs_virtual_camera"}"#,
            r#"{"type":"obs_virtual_camera","op":null}"#,
            r#"{"type":"obs_virtual_camera","op":"Toggle"}"#,
            r#"{"type":"obs_virtual_camera","op":"save"}"#,
            r#"{"type":"obs_virtual_camera","op":"toggle","endpoint":"127.0.0.1:4455"}"#,
            r#"{"type":"obs_replay_buffer","op":"pause"}"#,
            r#"{"type":"obs_replay_buffer","op":1}"#,
            r#"{"type":"obs_replay_buffer","operation":"save"}"#,
            r#"{"type":"obs_replay_buffer","op":"save","password":"x"}"#,
        ] {
            assert!(serde_json::from_str::<Action>(json).is_err(), "{json}");
        }
    }

    #[test]
    fn obs_output_actions_bind_to_discrete_controls_and_multi_action_steps_only() {
        let action = Action::ObsReplayBuffer {
            op: ObsReplayBufferOp::Save,
        };
        let mut profile = sample();
        profile.bindings[0].action = action.clone();
        assert!(profile.validate().is_ok());
        profile.bindings[0].module = ServiceModule::M005S;
        profile.bindings[0].gesture = Gesture::PositionChanged;
        assert!(profile.validate().is_err());
        let multi = Action::MultiAction {
            actions: vec![
                Action::ToggleMute,
                action,
                Action::ObsVirtualCamera {
                    op: ObsOutputOp::Stop,
                },
            ],
        };
        multi.validate_executable().unwrap();
        assert!(multi.uses_obs());
    }

    #[test]
    fn obs_source_actions_have_a_strict_json_shape_and_name_rules() {
        for json in [
            r#"{"type":"obs_studio_mode","op":"toggle"}"#,
            r#"{"type":"obs_studio_mode","op":"on"}"#,
            r#"{"type":"obs_studio_mode","op":"off"}"#,
            r#"{"type":"obs_transition"}"#,
            r#"{"type":"obs_filter","source":"Webcam","filter":"Color Correction","op":"toggle"}"#,
            r#"{"type":"obs_source_visibility","scene":"Live","source":"Camera","op":"off"}"#,
        ] {
            let action: Action = serde_json::from_str(json).unwrap();
            assert!(action.uses_obs(), "{json}");
            action.validate_executable().unwrap();
            assert_eq!(serde_json::to_string(&action).unwrap(), json);
        }
        assert_eq!(Action::ObsTransition.type_name(), "obs_transition");
        for json in [
            r#"{"type":"obs_studio_mode"}"#,
            r#"{"type":"obs_studio_mode","op":"start"}"#,
            r#"{"type":"obs_studio_mode","op":"Toggle"}"#,
            r#"{"type":"obs_transition","op":"toggle"}"#,
            r#"{"type":"obs_filter","source":"Webcam","op":"on"}"#,
            r#"{"type":"obs_filter","filter":"Color","op":"on"}"#,
            r#"{"type":"obs_filter","source":"Webcam","filter":"Color","op":"on","endpoint":"127.0.0.1:4455"}"#,
            r#"{"type":"obs_source_visibility","scene":"Live","op":"on"}"#,
            r#"{"type":"obs_source_visibility","scene":"Live","source":null,"op":"on"}"#,
            r#"{"type":"obs_source_visibility","scene":"Live","source":"Camera","op":"save"}"#,
        ] {
            assert!(serde_json::from_str::<Action>(json).is_err(), "{json}");
        }
        // Every name follows the scene-name rules, and the error says which field.
        let long = "x".repeat(Action::MAX_OBS_NAME_BYTES + 1);
        for bad in ["", "   ", "line\nbreak", "zero\u{200B}width", long.as_str()] {
            for action in [
                Action::ObsFilter {
                    source: bad.into(),
                    filter: "Color".into(),
                    op: ObsSwitchOp::On,
                },
                Action::ObsFilter {
                    source: "Webcam".into(),
                    filter: bad.into(),
                    op: ObsSwitchOp::On,
                },
                Action::ObsSourceVisibility {
                    scene: bad.into(),
                    source: "Camera".into(),
                    op: ObsSwitchOp::On,
                },
                Action::ObsSourceVisibility {
                    scene: "Live".into(),
                    source: bad.into(),
                    op: ObsSwitchOp::On,
                },
            ] {
                assert!(action.validate().is_err(), "{action:?}");
            }
        }
        let error = Action::ObsFilter {
            source: "Webcam".into(),
            filter: String::new(),
            op: ObsSwitchOp::On,
        }
        .validate()
        .unwrap_err();
        assert!(
            error.starts_with("obs_filter filter must be nonempty"),
            "{error}"
        );
        // They bind to discrete controls and as multi_action steps, like obs_scene.
        let multi = Action::MultiAction {
            actions: vec![
                Action::ObsStudioMode {
                    op: ObsSwitchOp::On,
                },
                Action::ObsTransition,
            ],
        };
        multi.validate_executable().unwrap();
        assert!(multi.uses_obs());
        let mut profile = sample();
        profile.bindings[0].action = Action::ObsTransition;
        profile.validate().unwrap();
        profile.bindings[0].module = ServiceModule::M005S;
        profile.bindings[0].gesture = Gesture::PositionChanged;
        assert!(profile.validate().is_err());
    }

    #[test]
    fn obs_collection_and_profile_actions_have_a_strict_json_shape_and_name_rules() {
        for json in [
            r#"{"type":"obs_scene_collection","name":"Streaming"}"#,
            r#"{"type":"obs_profile","name":"Zażółć 🎥"}"#,
        ] {
            let action: Action = serde_json::from_str(json).unwrap();
            assert!(action.uses_obs(), "{json}");
            action.validate_executable().unwrap();
            assert_eq!(serde_json::to_string(&action).unwrap(), json);
        }
        for json in [
            r#"{"type":"obs_scene_collection"}"#,
            r#"{"type":"obs_profile","name":null}"#,
            r#"{"type":"obs_profile","name":"Work","op":"on"}"#,
            r#"{"type":"obs_scene_collection","scene":"Work"}"#,
        ] {
            assert!(serde_json::from_str::<Action>(json).is_err(), "{json}");
        }
        let long = "x".repeat(Action::MAX_OBS_NAME_BYTES + 1);
        for bad in ["", "   ", "line\nbreak", "zero\u{200B}width", long.as_str()] {
            for action in [
                Action::ObsSceneCollection { name: bad.into() },
                Action::ObsProfile { name: bad.into() },
            ] {
                assert!(action.validate().is_err(), "{action:?}");
            }
        }
        let error = Action::ObsProfile {
            name: String::new(),
        }
        .validate()
        .unwrap_err();
        assert!(
            error.starts_with("obs_profile name must be nonempty"),
            "{error}"
        );
        assert_eq!(
            Action::ObsSceneCollection { name: "a".into() }.type_name(),
            "obs_scene_collection"
        );
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
        assert_eq!(NAMED_KEYS.len(), 52);
        let mut codes: Vec<_> = NAMED_KEYS.iter().map(|(_, code)| *code).collect();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), NAMED_KEYS.len(), "key codes must be distinct");
        // Media keys are shortcut keys without codes, checked separately.
        assert_eq!(MEDIA_KEYS.len(), 3);
        for name in ["play_pause", "next_track", "previous_track"] {
            assert!(key(name, vec![]).validate().is_ok(), "{name}");
            assert!(named_key_code(name).is_none(), "{name} has no key code");
            assert!(key(name, vec![Modifier::Command]).validate().is_err());
        }
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

    fn macro_step(
        key: &str,
        modifiers: Vec<Modifier>,
        delay_before_ms: u32,
        hold_ms: u32,
    ) -> MacroStep {
        MacroStep {
            key: key.into(),
            modifiers,
            delay_before_ms,
            hold_ms,
        }
    }

    fn macro_action(steps: Vec<MacroStep>) -> Action {
        Action::Macro { steps }
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
        for (name, _) in NAMED_KEYS {
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

    #[test]
    fn macros_are_bounded_sequences_of_known_keys() {
        assert!(
            macro_action(vec![macro_step("c", vec![Modifier::Command], 0, 80)])
                .validate()
                .is_ok()
        );
        // Every named key and every literal is a macro key; media keys are not.
        for (name, _) in NAMED_KEYS {
            assert!(
                macro_action(vec![macro_step(name, vec![], 0, 0)])
                    .validate()
                    .is_ok(),
                "{name}"
            );
        }
        let media = macro_action(vec![macro_step("play_pause", vec![], 0, 0)])
            .validate()
            .unwrap_err();
        assert!(
            media.contains("step 1: media keys cannot be macro keys"),
            "{media}"
        );
        // Bounds: counts, per-step waits and holds, duplicates, and the total.
        for (steps, fragment) in [
            (vec![], "must contain 1 to"),
            (
                vec![macro_step("a", vec![], 0, 0); Action::MAX_MACRO_STEPS + 1],
                "must contain 1 to",
            ),
            (
                vec![macro_step("a", vec![Modifier::Command; 5], 0, 0)],
                "at most four modifiers",
            ),
            (
                vec![macro_step(
                    "a",
                    vec![Modifier::Shift, Modifier::Shift],
                    0,
                    0,
                )],
                "must not contain duplicates",
            ),
            (
                vec![macro_step("a", vec![], Action::MAX_MACRO_DELAY_MS + 1, 0)],
                "the wait must be at most",
            ),
            (
                vec![macro_step("a", vec![], 0, Action::MAX_MACRO_HOLD_MS + 1)],
                "the hold must be at most",
            ),
            (
                vec![";", "-", "f21", "Return", "key code 36"]
                    .into_iter()
                    .map(|key| macro_step(key, vec![], 0, 0))
                    .collect(),
                "key must be one ASCII alphanumeric key",
            ),
        ] {
            let error = macro_action(steps).validate().unwrap_err();
            assert!(error.contains(fragment), "{error}");
        }
        // Exactly at the limits passes: 10000 + 5000 + 10000 + 5000 = 30000 ms.
        assert!(
            macro_action(vec![
                macro_step(
                    "a",
                    vec![],
                    Action::MAX_MACRO_DELAY_MS,
                    Action::MAX_MACRO_HOLD_MS
                ),
                macro_step(
                    "b",
                    vec![],
                    Action::MAX_MACRO_DELAY_MS,
                    Action::MAX_MACRO_HOLD_MS
                ),
            ])
            .validate()
            .is_ok()
        );
        let over = macro_action(vec![
            macro_step(
                "a",
                vec![],
                Action::MAX_MACRO_DELAY_MS,
                Action::MAX_MACRO_HOLD_MS,
            ),
            macro_step(
                "b",
                vec![],
                Action::MAX_MACRO_DELAY_MS,
                Action::MAX_MACRO_HOLD_MS,
            ),
            macro_step("c", vec![], 0, 1),
        ])
        .validate()
        .unwrap_err();
        assert!(
            over.contains("waits and holds must total at most"),
            "{over}"
        );
        // The upper-case letter is the same physical key as the lower-case one.
        assert!(
            macro_action(vec![macro_step("C", vec![Modifier::Shift], 0, 10)])
                .validate()
                .is_ok()
        );
        // A macro never nests inside a multi_action.
        let nested = Action::MultiAction {
            actions: vec![
                Action::ToggleMute,
                macro_action(vec![macro_step("a", vec![], 0, 0)]),
            ],
        }
        .validate()
        .unwrap_err();
        assert!(
            nested.contains("step 2: macro cannot be a multi_action step"),
            "{nested}"
        );
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
        let media = Action::Media {
            command: MediaCommand::NextTrack,
        };
        let multi = Action::MultiAction {
            actions: vec![url("https://example.com"), media.clone(), text("x")],
        };
        dispatcher.dispatch(&media).unwrap();
        dispatcher.dispatch(&multi).unwrap();
        // Dry runs plan obs_scene without any network access.
        dispatcher.dispatch(&obs("Live")).unwrap();
        assert!(dispatcher.unavailable(&obs("Live")).is_none());
        for invalid in [
            url("file:///etc/passwd"),
            text(""),
            Action::MultiAction {
                actions: vec![Action::ToggleMute, multi.clone()],
            },
        ] {
            assert!(dispatcher.dispatch(&invalid).is_err());
        }
        assert_eq!(dispatcher.actions, [media, multi, obs("Live")]);
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
        assert_eq!(
            fixture["timer_sounds"],
            serde_json::to_value(TimerSound::ALL).unwrap()
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
