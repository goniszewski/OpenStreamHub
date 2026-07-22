/// 3xKnob (2x2) module
pub struct M003K;

impl crate::Module for M003K {
    const PRODUCT_ID: u16 = 0x0202;
    const PRODUCT_NAME: Option<&'static str> = Some("3xKnob (2x2)");
}

impl crate::private::Sealed for M003K {}

impl super::capabilities::CanGetSerialNumber for M003K {}
