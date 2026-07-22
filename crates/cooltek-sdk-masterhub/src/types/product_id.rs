/// A strongly-typed product ID value object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ProductId(u16);

impl ProductId {
    /// Attempts to construct a new [`ProductId`] value object from a primitive value.
    ///
    /// # Parameters
    ///
    /// * `value` - The primitive value to use.
    ///
    /// # Returns
    ///
    /// A [`Result`] containing either a new [`ProductId`] or a [`ProductIdError`].
    ///
    /// # Errors
    ///
    /// * [`ProductIdError::ReservedId`] when the value is either `0x0000` or `0xFFFF`.
    pub const fn new(value: u16) -> Result<Self, ProductIdError> {
        match value {
            0x0000 | 0xFFFF => {
                // Both of these values are typically used as uninitialized (0x0000) or some
                // sentinel value (0xFFFF). They are not really meant to be "valid" product IDs.

                Err(ProductIdError::ReservedId(value))
            }
            _ => Ok(Self(value)),
        }
    }
}

impl From<ProductId> for u16 {
    fn from(value: ProductId) -> Self {
        value.0
    }
}

impl TryFrom<u16> for ProductId {
    type Error = ProductIdError;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl std::fmt::Display for ProductId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

// -------------------------------
//  Associated error type
// -------------------------------

/// Enumerates all the possible errors that can arise when working with a [`ProductId`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProductIdError {
    /// The value is reserved and cannot be used normally.
    ReservedId(u16),
}

impl std::error::Error for ProductIdError {}

impl std::fmt::Display for ProductIdError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ReservedId(value) => write!(f, "Product ID {} is reserved.", value),
        }
    }
}

// -------------------------------
//  Unit tests
// -------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    mod new {
        use super::*;

        #[test]
        fn returns_reserved_id_error_on_default_value() {
            let value = 0x0000;
            let result = ProductId::new(value);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err(), ProductIdError::ReservedId(value));
        }

        #[test]
        fn returns_reserved_id_error_on_sentinel_value() {
            let value = 0xFFFF;
            let result = ProductId::new(value);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err(), ProductIdError::ReservedId(value));
        }

        #[test]
        fn returns_self_on_valid_value() {
            let value = 0x01F6; // M0064
            let result = ProductId::new(value);

            assert!(result.is_ok());
            assert_eq!(u16::from(result.unwrap()), value);
        }
    }
}
