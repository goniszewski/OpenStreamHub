use crate::{ProductId, product_id};

/// 3xKnob (2x2) module
pub struct M003K;

impl crate::Module for M003K {
    const PRODUCT_ID: ProductId = product_id!(0x0202);
    const PRODUCT_NAME: Option<&'static str> = Some("3xKnob (2x2)");

    fn accepts_product_id(product_id: ProductId) -> bool {
        product_id == Self::PRODUCT_ID || product_id == product_id!(0x9996)
    }
}

impl crate::private::Sealed for M003K {}

impl super::capabilities::CanGetSerialNumber for M003K {
    const REPORT_ID: u8 = 0x30;
}
