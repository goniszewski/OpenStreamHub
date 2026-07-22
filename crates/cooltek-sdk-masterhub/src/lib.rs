//! # CoolTEK SDK - MasterHUB
//!
//! An unofficial SDK for the MasterHUB by Cooler Master.

#![deny(missing_docs)]

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

// -------------------------------
//  Exports
// -------------------------------

pub use module::Module;

// -------------------------------
//  Re-exports
// -------------------------------

pub use hidapi;
