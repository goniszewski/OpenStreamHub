//! This module contains the definitions for official modules in the MasterHUB ecosystem and their
//! capabilities.
//!
//! A module contains static metadata (such as vendor and product identifiers) as well as
//! _capabilities_: functionality that requires a connection to the physical HID and performs
//! instance-specific operations or returns instance-specific data.
//!
//! Each capability is typically implemented the same across supported modules with the only
//! difference typically being which HID endpoint is used. Thus capabilities are encoded into
//! traits, exposing a function (the actual operation) and members (allowing modules to use
//! different report IDs).

pub mod capabilities;
mod m003k;
mod m005s;
mod m0064;
mod m032le;
mod m131e;
mod m4315;

// -------------------------------
//  Exports
// -------------------------------

pub use m003k::M003K;
pub use m005s::M005S;
pub use m032le::M032LE;
pub use m0064::M0064;
pub use m131e::M131E;
pub use m4315::M4315;
