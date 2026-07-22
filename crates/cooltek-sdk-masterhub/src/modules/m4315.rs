/// 15xKey w/ IPS (4x3) module
pub struct M4315;

impl crate::Module for M4315 {
    const PRODUCT_ID: u16 = 0x1FE;
    const PRODUCT_NAME: Option<&'static str> = Some("15xKey w/ IPS (4x3)");
}

impl crate::private::Sealed for M4315 {}

impl super::capabilities::CanGetSerialNumber for M4315 {
    const REPORT_ID: u8 = 0x3A;
}

impl super::capabilities::CanSetSoftwareMode for M4315 {
    const COMMAND: u8 = 0x3A;
}
