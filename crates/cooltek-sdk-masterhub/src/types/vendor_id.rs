/// A strongly-typed vendor ID value object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct VendorId(u16);

impl VendorId {
    /// Attempts to construct a new [`VendorId`] value object from a primitive value.
    ///
    /// # Parameters
    ///
    /// * `value` - The primitive value to use.
    ///
    /// # Returns
    ///
    /// A [`Result`] containing either a new [`VendorId`] or a [`VendorIdError`].
    ///
    /// # Errors
    ///
    /// * [`VendorIdError::ReservedId`] when the value is either `0x0000` or `0xFFFF`.
    pub const fn new(value: u16) -> Result<Self, VendorIdError> {
        match value {
            0x0000 | 0xFFFF => {
                // Both of these values are typically used as uninitialized (0x0000) or some
                // sentinel value (0xFFFF). They are not really meant to be "valid" vendor IDs.

                Err(VendorIdError::ReservedId(value))
            }
            _ => Ok(Self(value)),
        }
    }
}

impl From<VendorId> for u16 {
    fn from(value: VendorId) -> Self {
        value.0
    }
}

impl TryFrom<u16> for VendorId {
    type Error = VendorIdError;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl std::fmt::Display for VendorId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:#06x}", self.0)
    }
}

// -------------------------------
//  Associated error type
// -------------------------------

/// Enumerates all the possible errors that can arise when working with a [`VendorId`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum VendorIdError {
    /// The value is reserved and cannot be used normally.
    ReservedId(u16),
}

impl std::error::Error for VendorIdError {}

impl std::fmt::Display for VendorIdError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ReservedId(value) => write!(f, "Vendor ID {} is reserved.", value),
        }
    }
}

// -------------------------------
//  Unit tests
// -------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn displays_as_zero_padded_hex() {
        assert_eq!(VendorId::new(0x01f6).unwrap().to_string(), "0x01f6");
        assert_eq!(VendorId::new(0x2516).unwrap().to_string(), "0x2516");
        assert_eq!(VendorId::new(0x0001).unwrap().to_string(), "0x0001");
    }

    mod new {
        use super::*;

        #[test]
        fn returns_reserved_id_error_on_default_value() {
            let value = 0x0000;
            let result = VendorId::new(value);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err(), VendorIdError::ReservedId(value));
        }

        #[test]
        fn returns_reserved_id_error_on_sentinel_value() {
            let value = 0xFFFF;
            let result = VendorId::new(value);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err(), VendorIdError::ReservedId(value));
        }

        #[test]
        fn returns_self_on_valid_value() {
            let value = 0x2516; // Cooler Master
            let result = VendorId::new(value);

            assert!(result.is_ok());
            assert_eq!(u16::from(result.unwrap()), value);
        }
    }
}
