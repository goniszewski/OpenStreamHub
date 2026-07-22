//! # CoolTEK SDK - MasterHUB
//!
//! An unofficial SDK for the MasterHUB by Cooler Master.

#![deny(missing_docs)]

mod module;

// -------------------------------
//  Exports
// -------------------------------

pub use module::Module;

// -------------------------------
//  Re-exports
// -------------------------------

pub use hidapi;
