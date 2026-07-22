/// 2xRoller (4x1) module
pub struct M032LE;

impl crate::Module for M032LE {
    const PRODUCT_ID: u16 = 0x0204;
    const PRODUCT_NAME: Option<&'static str> = Some("2xRoller (4x1)");
}

impl crate::private::Sealed for M032LE {}

impl super::capabilities::CanGetSerialNumber for M032LE {
    const REPORT_ID: u8 = 0x30;
}
