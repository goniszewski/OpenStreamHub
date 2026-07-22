/// Attempts to construct a new [`ProductId`] value object from a primitive value at compile time.
///
/// # Usage:
///
/// This macro attempts to construct a [`ProductId`] from a `u16` literal, raising a compilation
/// error if it does not succeed. This enables [`ProductId`]s to be used in `const` scenarios.
///
/// # Examples:
///
/// ```rust
/// use cooltek_sdk_masterhub::{ProductId, product_id};
///
/// const PRODUCT_ID: ProductId = product_id!(0x01F6);
/// ```
#[macro_export]
macro_rules! product_id {
    ($product_id:expr) => {{
        const OUTPUT: $crate::ProductId = match $crate::ProductId::new($product_id) {
            std::result::Result::Ok(product_id) => product_id,
            std::result::Result::Err(_) => panic!("Invalid product ID"),
        };

        OUTPUT
    }};
}

/// Attempts to construct a new [`VendorId`] value object from a primitive value at compile time.
///
/// # Usage:
///
/// This macro attempts to construct a [`VendorId`] from a `u16` literal, raising a compilation
/// error if it does not succeed. This enables [`VendorId`]s to be used in `const` scenarios.
///
/// # Examples:
///
/// ```rust
/// use cooltek_sdk_masterhub::{VendorId, vendor_id};
///
/// const VENDOR_ID: VendorId = vendor_id!(0x2516);
/// ```
#[macro_export]
macro_rules! vendor_id {
    ($vendor_id:expr) => {{
        const OUTPUT: $crate::VendorId = match $crate::VendorId::new($vendor_id) {
            std::result::Result::Ok(vendor_id) => vendor_id,
            std::result::Result::Err(_) => panic!("Invalid vendor ID"),
        };

        OUTPUT
    }};
}
