use hidapi::{HidDevice, HidError};

use crate::{ProductId, product_id};

/// 1xDial w/ IPS (2x2) module
pub struct M131E;

impl crate::Module for M131E {
    const PRODUCT_ID: ProductId = product_id!(0x0206);
    const PRODUCT_NAME: Option<&'static str> = Some("1xDial w/ IPS (2x2)");

    fn accepts_product_id(product_id: ProductId) -> bool {
        product_id == Self::PRODUCT_ID || product_id == product_id!(0x9994)
    }

    fn initialize(device: &HidDevice) -> Result<(), HidError> {
        use super::capabilities::CanSetSoftwareMode;

        // No interactions appear to have any effect until we enter software mode, so treat it as
        // an initialization requirement.
        Self::set_software_mode(device)
    }
}

impl crate::private::Sealed for M131E {}

impl super::capabilities::CanGetSerialNumber for M131E {
    const REPORT_ID: u8 = 0x4A;
}

impl super::capabilities::CanSetSoftwareMode for M131E {
    const COMMAND: u8 = 0x49;
}
