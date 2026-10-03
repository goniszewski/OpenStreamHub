use crate::{ProductId, product_id};

/// Base (4x6) module
pub struct M0064;

impl crate::Module for M0064 {
    const PRODUCT_ID: ProductId = product_id!(0x01F6);
    const PRODUCT_NAME: Option<&'static str> = Some("Base (4x6)");

    fn accepts_product_id(product_id: ProductId) -> bool {
        product_id == Self::PRODUCT_ID || product_id == product_id!(0x9999)
    }
}

impl crate::private::Sealed for M0064 {}

impl super::capabilities::CanGetSerialNumber for M0064 {
    const REPORT_ID: u8 = 0x30;
}
