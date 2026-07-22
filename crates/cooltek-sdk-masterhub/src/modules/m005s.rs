use crate::{ProductId, product_id};

/// 5xFader (4x2) module
pub struct M005S;

impl crate::Module for M005S {
    const PRODUCT_ID: ProductId = product_id!(0x0200);
    const PRODUCT_NAME: Option<&'static str> = Some("5xFader (4x2)");
}

impl crate::private::Sealed for M005S {}

impl super::capabilities::CanGetSerialNumber for M005S {
    const REPORT_ID: u8 = 0x30;
}
