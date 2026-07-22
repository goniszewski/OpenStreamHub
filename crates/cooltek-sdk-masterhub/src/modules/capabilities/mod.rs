//! Standardized operations for modules.
//!
//! Capabilities allow modules to easily expose operations and behaviors. Each capability consists
//! of a function (the actual operation) and members (accommodating the different circuit boards
//! exposing the same functionality under different report IDs).
//!
//! For example: you can read the serial number by using:
//! * `Get_Report 0x30` on an M0 (M003K, M005S, M0064, or M032LE) module
//! * `Get_Report 0x4A` on an M131E module
//! * `Get_Report 0x3A` on an M4315 module
//!
//! These modules all do the same thing (reading the serial number) but use different report IDs.
//! We can define a trait the implements the function that builds the report and contains a
//! `REPORT_ID` member that is different for each module type.
//!
//! The functions themselves accept an instance-specific connection to operate on. This provides an
//! easy way to provide an oracle for unit testing purposes.
//!
//! The capability names are derived from symbols in the official CooTEK SDK binary shipped with
//! the MasterHUB software. The symbols have been "cleaned up" to be more consistent, informative,
//! and grammatically correct.

mod can_get_serial_number;
mod can_set_software_mode;

// -------------------------------
//  Exports
// -------------------------------

pub use can_get_serial_number::CanGetSerialNumber;
pub use can_set_software_mode::CanSetSoftwareMode;
