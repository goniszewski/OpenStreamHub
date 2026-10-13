//! MasterHUB input bindings and action dispatch independent of device access.
//!
//! Profiles and input sessions are pure and can be exercised with recorded or
//! synthetic reports. Device selection, transport lifecycle, and deciding when
//! to execute actions belong to the calling service.

#![deny(missing_docs)]

mod binding;
mod dial;
mod input;
mod key_artwork;
mod keypad;
mod profile;
mod timer;

pub mod display_sync;
pub mod macos;
pub mod obs;

pub use binding::{ActionDispatcher, BindingEngine, DeviceContext, DryRunDispatcher, TimerAlerter};
pub use dial::{
    DialBinding, DialDisplay, DialDisplayContent, DialFit, DialFunction, DialFunctionNavigation,
    DialSource,
};
pub use input::{InputEvent, InputSession, InputUpdate, InputValue};
pub use key_artwork::{KeyArtwork, KeyArtworkDesign, KeyArtworkFrame, KeyArtworkTextAlign};
pub use keypad::{
    KeypadBinding, KeypadLayout, KeypadNavigation, KeypadPage, KeypadResolution, KeypadTile,
    ProfileResolution, ProfileSession,
};
pub use macos::MacOsDispatcher;
pub use profile::{
    Action, Binding, Gesture, MacroStep, MediaCommand, Modifier, ObsOutputOp, ObsReplayBufferOp,
    ObsSwitchOp, Profile, ProfileDirection, ScreenBrightness, ServiceModule, SystemCommand,
    TimerSound, is_invisible_text_character, key_for_virtual_code, virtual_key_code,
};
pub use timer::{RunningTimer, TimerAlert, TimerKey, TimerTable, TimerToggle, format_duration};
