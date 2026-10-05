//! MasterHUB input bindings and action dispatch independent of device access.
//!
//! Profiles and input sessions are pure and can be exercised with recorded or
//! synthetic reports. Device selection, transport lifecycle, and deciding when
//! to execute actions belong to the calling service.

#![deny(missing_docs)]

mod binding;
mod input;
mod keypad;
mod profile;

pub mod macos;

pub use binding::{ActionDispatcher, BindingEngine, DeviceContext, DryRunDispatcher};
pub use input::{InputEvent, InputSession, InputUpdate, InputValue};
pub use keypad::{
    KeypadBinding, KeypadLayout, KeypadNavigation, KeypadPage, KeypadResolution, KeypadTile,
    ProfileSession,
};
pub use macos::MacOsDispatcher;
pub use profile::{Action, Binding, Gesture, Modifier, Profile, ServiceModule};
