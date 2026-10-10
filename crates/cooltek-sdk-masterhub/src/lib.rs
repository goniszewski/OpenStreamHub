//! # CoolTEK SDK - MasterHUB
//!
//! An unofficial SDK for the MasterHUB by Cooler Master.

#![deny(missing_docs)]

pub mod descriptor;
pub mod dial;
mod dial_session;
mod discovery;
mod hid;
pub mod input;
mod jpeg;
pub mod keypad;
pub mod lighting;
mod macros;
mod module;
pub mod modules;
#[doc(hidden)]
mod private {
    /// A marker trait that can only be implemented in this crate.
    ///
    /// This prevents consumers from creating their own implementation of certain traits and
    /// potentially violating assumptions or invariants.
    pub trait Sealed {}
}
mod transport;
mod types;

// -------------------------------
//  Exports
// -------------------------------

pub use dial_session::{
    DialGesture, DialGestureTracker, DialReadError, DialSession, DialSessionState,
};
pub use discovery::{DeviceRegistry, ModuleIdentity, ModuleKind};
pub use hid::ModuleHandle;
pub use module::Module;
pub use transport::{InputReader, ReportTransport, ShortWrite, output_report_length};
pub use types::{ProductId, ProductIdError, VendorId, VendorIdError};

// -------------------------------
//  Re-exports
// -------------------------------

pub use hidapi;
