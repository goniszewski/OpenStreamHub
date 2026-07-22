use crate::{ProductId, product_id};

/// Base (4x6) module
pub struct M0064;

impl crate::Module for M0064 {
    const PRODUCT_ID: ProductId = product_id!(0x01F6);
    const PRODUCT_NAME: Option<&'static str> = Some("Base (4x6)");
}

impl crate::private::Sealed for M0064 {}

impl super::capabilities::CanGetSerialNumber for M0064 {
    const REPORT_ID: u8 = 0x30;
}
