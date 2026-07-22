//! This module contains common data structures and value objects used in the crate.

mod product_id;
mod vendor_id;

// -------------------------------
//  Exports
// -------------------------------

pub use product_id::{ProductId, ProductIdError};
pub use vendor_id::{VendorId, VendorIdError};
