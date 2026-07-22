/// 1xDial w/ IPS (2x2) module
pub struct M131E;

impl crate::Module for M131E {
    const PRODUCT_ID: u16 = 0x0206;
    const PRODUCT_NAME: Option<&'static str> = Some("1xDial w/ IPS (2x2)");
}

impl crate::private::Sealed for M131E {}

impl super::capabilities::CanGetSerialNumber for M131E {
    const REPORT_ID: u8 = 0x4A;
}

impl super::capabilities::CanSetSoftwareMode for M131E {
    const COMMAND: u8 = 0x49;
}
