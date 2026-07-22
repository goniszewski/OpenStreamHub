/// Represents a discrete module in the MasterHUB ecosystem.
///
/// Modules are Human Interface Devices (HIDs) that expose input controls (knobs, buttons, dials,
/// sliders, etc.) as physical features and capabilities as software functionality.
///
/// # Implementation
///
/// A module contains associated constants (e.g. vendor ID and product ID pair) that are the same
/// for every instance of a module. It also contains functions that can be passed an instance-
/// specific connection to operate on.
///
/// New modules will implement this trait (allowing them to be used throughout this crate) and
/// typically one or more capabilities.
///
/// ## Examples
///
/// ```
/// struct MyModule;
/// impl cooltek_sdk_masterhub::Module for MyModule {
///     const PRODUCT_ID: u16 = 0xBAD;
///     const PRODUCT_NAME: Option<&'static str> = Some("My Module");
///     const VENDOR_ID: u16 = 0xBAD;
/// }
/// ```
pub trait Module: Sized {
    /// The ID of the product or product family of the device.
    ///
    /// This is specified by the vendor or manufacturer.
    const PRODUCT_ID: u16;

    /// The name of the product or product family of the device.
    ///
    /// This is specified by the vendor or manufacturer. It does not have to be present.
    const PRODUCT_NAME: Option<&'static str> = None;

    /// The ID of the vendor or manufacturer of the device.
    ///
    /// This is assigned to a particular vendor or manufacturer by the
    /// [USB Implementers Forum (USB-IF)](https://www.usb.org/about).
    const VENDOR_ID: u16 = 0x2516;
}
