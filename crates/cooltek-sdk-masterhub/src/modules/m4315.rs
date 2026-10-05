use hidapi::{HidDevice, HidError};

use crate::{ProductId, product_id};

/// 15xKey w/ IPS (4x3) module
pub struct M4315;

impl crate::Module for M4315 {
    const PRODUCT_ID: ProductId = product_id!(0x1FE);
    const PRODUCT_NAME: Option<&'static str> = Some("15xKey w/ IPS (4x3)");

    fn accepts_product_id(product_id: ProductId) -> bool {
        product_id == Self::PRODUCT_ID || product_id == product_id!(0x9997)
    }

    fn initialize(device: &HidDevice) -> Result<(), HidError> {
        use super::capabilities::CanSetSoftwareMode;

        // No interactions appear to have any effect until we enter software mode, so treat it as
        // an initialization requirement.
        Self::set_software_mode(device)
    }
}

impl crate::private::Sealed for M4315 {}

impl super::capabilities::CanGetSerialNumber for M4315 {
    // Source: inherited unchanged from the upstream commit that added M4315
    // support (e71d94f, "add initial support for M4315 modules"); no capture or
    // static-analysis evidence for it exists in docs/masterhub-protocol-evidence.md,
    // which records serial report `30` for M0 models and `4a` for M131E only.
    // It coincides with the software-mode command byte below (`03 3a <bool>`),
    // which is evidenced. Treat this report ID as unverified; do not change it
    // without a hardware capture or recovered `Get_Serial_Number_CM_M4315` code.
    const REPORT_ID: u8 = 0x3A;
}

impl super::capabilities::CanSetSoftwareMode for M4315 {
    const COMMAND: u8 = 0x3A;
}
