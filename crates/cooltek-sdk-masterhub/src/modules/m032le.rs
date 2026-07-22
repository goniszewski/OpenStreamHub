use crate::{ProductId, product_id};

/// 2xRoller (4x1) module
pub struct M032LE;

impl crate::Module for M032LE {
    const PRODUCT_ID: ProductId = product_id!(0x0204);
    const PRODUCT_NAME: Option<&'static str> = Some("2xRoller (4x1)");
}

impl crate::private::Sealed for M032LE {}

impl super::capabilities::CanGetSerialNumber for M032LE {
    const REPORT_ID: u8 = 0x30;
}
