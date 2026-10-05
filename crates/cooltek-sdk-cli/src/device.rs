//! Exact device-path parsing and selection shared by both binaries.

use cooltek_sdk_masterhub::{ModuleIdentity, ModuleKind};

use crate::capture::decode_hex;

/// Maximum accepted HID device path length, excluding the trailing NUL.
pub const MAX_PATH_BYTES: usize = 4096;

/// Read timeout bounds shared by `record` and `run`, in milliseconds.
pub const TIMEOUT_MS_RANGE: std::ops::RangeInclusive<i64> = 1..=1000;

/// A lossless, nonempty device path without interior NUL bytes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DevicePath(pub Vec<u8>);

impl std::ops::Deref for DevicePath {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        &self.0
    }
}

/// Parses a hexadecimal path copied from `masterhub-diag list`.
pub fn parse_path(raw: &str) -> Result<DevicePath, String> {
    let bytes = decode_hex(raw, MAX_PATH_BYTES)?;
    if bytes.is_empty() || bytes.contains(&0) {
        return Err("device path must be nonempty and contain no NUL bytes".into());
    }
    Ok(DevicePath(bytes))
}

/// Selects exactly one inventory entry by path; software mode requires an IPS module.
pub fn select_device<'a>(
    devices: &'a [ModuleIdentity],
    path: &[u8],
    software_mode: bool,
) -> Result<&'a ModuleIdentity, String> {
    let mut matches = devices
        .iter()
        .filter(|device| device.path.as_bytes() == path);
    let selected = matches
        .next()
        .ok_or("device path is not in the current inventory; run `masterhub-diag list` again")?;
    if matches.next().is_some() {
        return Err(
            "device path identifies multiple interfaces; ambiguous selection is unsupported".into(),
        );
    }
    if software_mode
        && !crate::mode::supported(ModuleKind::from_ids(
            selected.vendor_id,
            selected.product_id,
        ))
    {
        return Err("software mode is supported only for M131E and M4315".into());
    }
    Ok(selected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_limit_is_independent_and_rejects_nul_and_empty() {
        assert_eq!(parse_path("61ff").unwrap().0, vec![0x61, 0xff]);
        assert!(parse_path("").is_err());
        assert!(parse_path("6100").is_err());
        assert!(parse_path(&"61".repeat(MAX_PATH_BYTES)).is_ok());
        assert!(parse_path(&"61".repeat(MAX_PATH_BYTES + 1)).is_err());
    }
}
