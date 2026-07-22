//! # CoolTEK SDK - MasterHUB
//!
//! An unofficial SDK for the MasterHUB by Cooler Master.

#![deny(missing_docs)]

mod hid;
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
mod types;

// -------------------------------
//  Exports
// -------------------------------

pub use hid::ModuleHandle;
pub use module::Module;
pub use types::{ProductId, ProductIdError, VendorId, VendorIdError};

// -------------------------------
//  Re-exports
// -------------------------------

pub use hidapi;
