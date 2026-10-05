use hidapi::{HidDevice, HidError};

use crate::{
    types::{ProductId, VendorId},
    vendor_id,
};

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
/// use cooltek_sdk_masterhub::{ProductId, VendorId, product_id, vendor_id};
///
/// struct MyModule;
/// impl cooltek_sdk_masterhub::Module for MyModule {
///     const PRODUCT_ID: ProductId = product_id!(0xBAD);
///     const PRODUCT_NAME: Option<&'static str> = Some("My Module");
///     const VENDOR_ID: VendorId = vendor_id!(0xBAD);
/// }
/// ```
pub trait Module: Sized {
    /// The ID of the product or product family of the device.
    ///
    /// This is specified by the vendor or manufacturer.
    const PRODUCT_ID: ProductId;

    /// The name of the product or product family of the device.
    ///
    /// This is specified by the vendor or manufacturer. It does not have to be present.
    const PRODUCT_NAME: Option<&'static str> = None;

    /// The ID of the vendor or manufacturer of the device.
    ///
    /// This is assigned to a particular vendor or manufacturer by the
    /// [USB Implementers Forum (USB-IF)](https://www.usb.org/about).
    const VENDOR_ID: VendorId = vendor_id!(0x2516);

    /// Returns whether a product identifier belongs to this module.
    ///
    /// The default recognizes only [`Self::PRODUCT_ID`]. Modules with multiple
    /// hardware revisions can override this without changing their original ID.
    /// The vendor identifier must be checked separately against [`Self::VENDOR_ID`].
    fn accepts_product_id(product_id: ProductId) -> bool {
        product_id == Self::PRODUCT_ID
    }

    /// Attempts to perform the initialization procedure for the device.
    ///
    /// This allows implementations to define some operations that should be attempted before the
    /// [`Module`] is used.
    ///
    /// # Parameters
    ///
    /// * `device` - The [`HidDevice`] handle.
    ///
    /// # Returns
    ///
    /// A [`Result`] containing either the unit type or a [`HidError`].
    ///
    /// # Errors
    ///
    /// * [`HidError`] when the underlying [`HidDevice`] returns an error.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use cooltek_sdk_masterhub::Module;
    /// # use cooltek_sdk_masterhub::{ProductId, product_id};
    /// use hidapi::{HidApi, HidDevice};
    ///
    /// struct MyModule;
    /// impl Module for MyModule {
    ///     // ...
    /// #   const PRODUCT_ID: ProductId = product_id!(0xBAD);
    /// }
    ///
    /// let context = HidApi::new().expect("Failed to initialize HidApi");
    /// let device = context.open(MyModule::VENDOR_ID.into(), MyModule::PRODUCT_ID.into()).expect("Failed to connect to device");
    /// MyModule::initialize(&device).expect("Failed to initialize the device");
    /// ```
    fn initialize(_device: &HidDevice) -> Result<(), HidError> {
        Ok(())
    }
}

/// Returns whether a raw VID/PID pair identifies module `M`.
///
/// This is the single source of truth for VID/PID recognition, shared by
/// typed handles and [`crate::ModuleKind::from_ids`].
pub(crate) fn matches_module<M: Module>(vendor_id: u16, product_id: u16) -> bool {
    vendor_id == u16::from(M::VENDOR_ID)
        && ProductId::new(product_id).is_ok_and(M::accepts_product_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::product_id;

    struct OriginalModule;

    impl Module for OriginalModule {
        const PRODUCT_ID: ProductId = product_id!(0x1234);
    }

    struct RevisedModule;

    impl Module for RevisedModule {
        const PRODUCT_ID: ProductId = product_id!(0x1234);

        fn accepts_product_id(product_id: ProductId) -> bool {
            product_id == Self::PRODUCT_ID || product_id == product_id!(0x5678)
        }
    }

    #[test]
    fn accepts_only_the_original_product_id_by_default() {
        assert!(OriginalModule::accepts_product_id(product_id!(0x1234)));
        assert!(!OriginalModule::accepts_product_id(product_id!(0x5678)));
    }

    #[test]
    fn permits_custom_modules_to_recognize_multiple_revisions() {
        assert!(RevisedModule::accepts_product_id(product_id!(0x1234)));
        assert!(RevisedModule::accepts_product_id(product_id!(0x5678)));
        assert!(!RevisedModule::accepts_product_id(product_id!(0x9999)));
        assert_eq!(RevisedModule::PRODUCT_ID, product_id!(0x1234));
    }
}
