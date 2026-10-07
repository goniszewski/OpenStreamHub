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

pub mod display_sync;
pub mod macos;
pub mod obs;

pub use binding::{ActionDispatcher, BindingEngine, DeviceContext, DryRunDispatcher};
pub use dial::{DialBinding, DialFunction, DialFunctionNavigation};
pub use input::{InputEvent, InputSession, InputUpdate, InputValue};
pub use keypad::{
    KeypadBinding, KeypadLayout, KeypadNavigation, KeypadPage, KeypadResolution, KeypadTile,
    ProfileResolution, ProfileSession,
};
pub use macos::MacOsDispatcher;
pub use profile::{
    Action, Binding, Gesture, MediaCommand, Modifier, Profile, ProfileDirection, ServiceModule,
    SystemCommand, is_invisible_text_character,
};
