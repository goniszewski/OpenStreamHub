//! MasterHUB input bindings and action dispatch independent of device access.
//!
//! Profiles and input sessions are pure and can be exercised with recorded or
//! synthetic reports. Device selection, transport lifecycle, and deciding when
//! to execute actions belong to the calling service.

#![deny(missing_docs)]

mod binding;
mod dial;
mod input;
mod keypad;
mod profile;
mod timer;

pub mod display_sync;
pub mod macos;
pub mod obs;

pub use binding::{ActionDispatcher, BindingEngine, DeviceContext, DryRunDispatcher, TimerAlerter};
pub use dial::{DialBinding, DialFunction, DialFunctionNavigation};
pub use input::{InputEvent, InputSession, InputUpdate, InputValue};
pub use keypad::{
    KeypadBinding, KeypadLayout, KeypadNavigation, KeypadPage, KeypadResolution, KeypadTile,
    ProfileResolution, ProfileSession,
};
pub use macos::MacOsDispatcher;
pub use profile::{
    Action, Binding, Gesture, MediaCommand, Modifier, ObsOutputOp, ObsReplayBufferOp, Profile,
    ProfileDirection, ServiceModule, SystemCommand, TimerSound, is_invisible_text_character,
};
pub use timer::{RunningTimer, TimerAlert, TimerKey, TimerTable, TimerToggle, format_duration};
