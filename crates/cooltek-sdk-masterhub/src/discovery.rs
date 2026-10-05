use std::ffi::CString;

use hidapi::{DeviceInfo, HidApi, HidDevice, HidError, HidResult};

/// Recognized MasterHUB model, independent of implemented protocol capabilities.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleKind {
    /// MasterHUB base.
    M0064,
    /// Three-knob module.
    M003K,
    /// Five-fader module.
    M005S,
    /// Two-roller module.
    M032LE,
    /// IPS dial module.
    M131E,
    /// Fifteen-key IPS module.
    M4315,
}

impl ModuleKind {
    /// Recognizes an official VID/PID without opening the device.
    ///
    /// Recognition is delegated to each module's [`crate::Module`] implementation
    /// (`VENDOR_ID` and `accepts_product_id`), so IDs are defined in one place.
    pub fn from_ids(vendor_id: u16, product_id: u16) -> Option<Self> {
        use crate::{
            module::matches_module,
            modules::{M003K, M005S, M032LE, M0064, M131E, M4315},
        };

        type Matcher = fn(u16, u16) -> bool;
        let candidates: [(Self, Matcher); 6] = [
            (Self::M0064, matches_module::<M0064>),
            (Self::M003K, matches_module::<M003K>),
            (Self::M005S, matches_module::<M005S>),
            (Self::M032LE, matches_module::<M032LE>),
            (Self::M131E, matches_module::<M131E>),
            (Self::M4315, matches_module::<M4315>),
        ];
        candidates
            .into_iter()
            .find(|(_, matches)| matches(vendor_id, product_id))
            .map(|(kind, _)| kind)
    }

    /// Returns the model identifier used by SDK metadata and capture files.
    pub const fn name(self) -> &'static str {
        match self {
            Self::M0064 => "M0064",
            Self::M003K => "M003K",
            Self::M005S => "M005S",
            Self::M032LE => "M032LE",
            Self::M131E => "M131E",
            Self::M4315 => "M4315",
        }
    }
}

/// An enumeration snapshot of one HID interface, not a persistent device ID.
///
/// Paths are retained losslessly. A device may expose multiple interfaces;
/// VID/PID or a serial alone must not silently select the first interface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleIdentity {
    /// Platform-specific HID path; use its bytes for lossless serialization.
    pub path: CString,
    /// USB vendor identifier.
    pub vendor_id: u16,
    /// USB product identifier.
    pub product_id: u16,
    /// USB enumeration serial, which may differ from vendor feature-report serials.
    ///
    /// Empty or whitespace-only enumeration strings are normalized to `None`.
    pub serial_number: Option<String>,
    /// Manufacturer string returned by enumeration.
    pub manufacturer_string: Option<String>,
    /// Product string returned by enumeration.
    pub product_string: Option<String>,
    /// Device release number reported during enumeration.
    pub release_number: u16,
    /// Interface number, or HIDAPI's unavailable sentinel.
    pub interface_number: i32,
    /// HID usage page reported by the backend.
    pub usage_page: u16,
    /// HID usage reported by the backend.
    pub usage: u16,
    /// Recognized model, if the VID/PID is known.
    pub module: Option<ModuleKind>,
}

impl From<&DeviceInfo> for ModuleIdentity {
    fn from(device: &DeviceInfo) -> Self {
        Self {
            path: device.path().to_owned(),
            vendor_id: device.vendor_id(),
            product_id: device.product_id(),
            serial_number: normalize_serial(device.serial_number()),
            manufacturer_string: device.manufacturer_string().map(str::to_owned),
            product_string: device.product_string().map(str::to_owned),
            release_number: device.release_number(),
            interface_number: device.interface_number(),
            usage_page: device.usage_page(),
            usage: device.usage(),
            module: ModuleKind::from_ids(device.vendor_id(), device.product_id()),
        }
    }
}

/// Treats a missing, empty, or whitespace-only serial as unavailable.
///
/// Some HIDAPI backends report `Some("")` for devices without a serial, which
/// would otherwise never match a serial-bound profile or a wildcard-as-`None`.
fn normalize_serial(serial: Option<&str>) -> Option<String> {
    serial
        .filter(|serial| !serial.trim().is_empty())
        .map(str::to_owned)
}

impl ModuleIdentity {
    /// Compares only fields that identify the physical interface.
    ///
    /// Path, VID/PID, serial, and interface number must match. Usage page,
    /// usage, strings, and release number are excluded: backends may report a
    /// different top-level collection or string set for an opened device than
    /// for its enumeration entry.
    fn same_physical_interface(&self, other: &Self) -> bool {
        self.path == other.path
            && self.vendor_id == other.vendor_id
            && self.product_id == other.product_id
            && self.serial_number == other.serial_number
            && self.interface_number == other.interface_number
    }
}

/// Owns a shared HIDAPI discovery context and opens explicitly selected interfaces.
///
/// Enumeration and opening do not send software-mode commands. Call a module's
/// initialization separately only when intended. Refresh before selecting after
/// reattachment; this registry does not automatically reconnect.
pub struct DeviceRegistry {
    api: HidApi,
}

impl DeviceRegistry {
    /// Enumerates HID interfaces without opening or initializing any device.
    pub fn new() -> HidResult<Self> {
        Ok(Self {
            api: HidApi::new()?,
        })
    }

    /// Refreshes enumeration after attachment changes without initializing devices.
    pub fn refresh(&mut self) -> HidResult<()> {
        self.api.refresh_devices()
    }

    /// Returns all enumerated Cooler Master interfaces, including unknown products.
    pub fn devices(&self) -> Vec<ModuleIdentity> {
        self.api
            .device_list()
            .filter(|device| device.vendor_id() == 0x2516)
            .map(ModuleIdentity::from)
            .collect()
    }

    /// Opens exactly the selected enumeration snapshot without initializing it.
    ///
    /// Rejects identities absent from this context rather than falling back to a
    /// different VID/PID match. Physical removal may still cause an HID open error.
    pub fn open(&self, identity: &ModuleIdentity) -> HidResult<HidDevice> {
        open_verified(
            identity,
            &self.devices(),
            |path| self.api.open_path(path),
            |device| Ok(ModuleIdentity::from(&device.get_device_info()?)),
        )
    }
}

/// Opens `identity` and verifies the opened device's physical identity.
///
/// Separated from HIDAPI so the verification can be tested with fake devices.
fn open_verified<D>(
    identity: &ModuleIdentity,
    current: &[ModuleIdentity],
    open: impl FnOnce(&std::ffi::CStr) -> HidResult<D>,
    device_info: impl FnOnce(&D) -> HidResult<ModuleIdentity>,
) -> HidResult<D> {
    validate_identity(identity, current)?;
    let device = open(&identity.path)?;
    // A path can be reused after reattachment. Verify the opened interface
    // before a caller can send initialization to the wrong physical device.
    validate_identity(identity, &[device_info(&device)?])?;
    Ok(device)
}

fn validate_identity(identity: &ModuleIdentity, current: &[ModuleIdentity]) -> HidResult<()> {
    if current
        .iter()
        .any(|candidate| candidate.same_physical_interface(identity))
    {
        Ok(())
    } else {
        Err(HidError::HidApiError {
            message: "Selected interface is absent or changed; refresh discovery and select again"
                .into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(path: &[u8], interface: i32) -> ModuleIdentity {
        ModuleIdentity {
            path: CString::new(path).unwrap(),
            vendor_id: 0x2516,
            product_id: 0x0202,
            serial_number: Some("same-serial".into()),
            manufacturer_string: None,
            product_string: None,
            release_number: 1,
            interface_number: interface,
            usage_page: 0xff00,
            usage: 1,
            module: Some(ModuleKind::M003K),
        }
    }

    #[test]
    fn rejects_stale_selection_instead_of_opening_another_matching_module() {
        let selected = identity(b"first", 0);
        let current = [identity(b"second", 0)];
        assert!(validate_identity(&selected, &current).is_err());
    }

    #[test]
    fn distinguishes_interfaces_and_preserves_non_utf8_paths() {
        let selected = identity(b"path-\xff", 1);
        assert_eq!(selected.path.as_bytes(), b"path-\xff");
        assert!(validate_identity(&selected, std::slice::from_ref(&selected)).is_ok());
        assert!(validate_identity(&identity(b"path-\xff", 0), &[selected]).is_err());
    }

    #[test]
    fn open_verification_ignores_usage_and_strings_but_not_physical_identity() {
        let selected = identity(b"dev", 1);
        let current = [selected.clone()];
        let open = |info: ModuleIdentity| {
            open_verified(
                &selected,
                &current,
                |path| {
                    assert_eq!(path.to_bytes(), b"dev");
                    Ok(())
                },
                move |_| Ok(info),
            )
        };

        // get_device_info() may report another top-level collection or strings.
        let mut cosmetic = selected.clone();
        cosmetic.usage_page = 0x0001;
        cosmetic.usage = 6;
        cosmetic.manufacturer_string = Some("Cooler Master".into());
        cosmetic.product_string = Some("MasterHUB".into());
        cosmetic.release_number = 2;
        assert!(open(cosmetic).is_ok());

        let mut other_serial = selected.clone();
        other_serial.serial_number = Some("other".into());
        let mut other_interface = selected.clone();
        other_interface.interface_number = 0;
        let mut other_product = selected.clone();
        other_product.product_id = 0x0206;
        let mut other_path = selected.clone();
        other_path.path = CString::new("other").unwrap();
        for changed in [other_serial, other_interface, other_product, other_path] {
            assert!(open(changed).is_err());
        }

        // An absent selection never reaches the open callback.
        let result = open_verified(
            &identity(b"gone", 1),
            &current,
            |_| -> HidResult<()> { panic!("must not open an absent identity") },
            |_| Ok(selected.clone()),
        );
        assert!(result.is_err());
    }

    #[test]
    fn blank_enumeration_serials_are_unavailable() {
        assert_eq!(normalize_serial(None), None);
        assert_eq!(normalize_serial(Some("")), None);
        assert_eq!(normalize_serial(Some("  \t")), None);
        assert_eq!(normalize_serial(Some("A1")), Some("A1".into()));
        assert_eq!(normalize_serial(Some(" A1 ")), Some(" A1 ".into()));
    }

    #[test]
    fn requires_the_cooler_master_vendor_for_model_recognition() {
        assert_eq!(
            ModuleKind::from_ids(0x2516, 0x0206),
            Some(ModuleKind::M131E)
        );
        assert_eq!(ModuleKind::from_ids(0x1234, 0x0202), None);
        assert_eq!(ModuleKind::from_ids(0x2516, 0xabcd), None);
    }

    #[test]
    fn recognizes_legacy_and_software_catalog_ids_for_all_control_modules() {
        use crate::{
            Module,
            modules::{M003K, M005S, M032LE, M0064, M131E, M4315},
        };

        let models = [
            (ModuleKind::M0064, 0x01f6, 0x9999),
            (ModuleKind::M003K, 0x0202, 0x9996),
            (ModuleKind::M131E, 0x0206, 0x9994),
            (ModuleKind::M005S, 0x0200, 0x9991),
            (ModuleKind::M032LE, 0x0204, 0x9995),
            (ModuleKind::M4315, 0x01fe, 0x9997),
        ];
        for (model, legacy, current) in models {
            for product_id in [legacy, current] {
                assert_eq!(ModuleKind::from_ids(0x2516, product_id), Some(model));
                assert_eq!(ModuleKind::from_ids(0x1234, product_id), None);
                let product_id = crate::ProductId::new(product_id).unwrap();
                assert_eq!(
                    M0064::accepts_product_id(product_id),
                    model == ModuleKind::M0064
                );
                assert_eq!(
                    M003K::accepts_product_id(product_id),
                    model == ModuleKind::M003K
                );
                assert_eq!(
                    M131E::accepts_product_id(product_id),
                    model == ModuleKind::M131E
                );
                assert_eq!(
                    M005S::accepts_product_id(product_id),
                    model == ModuleKind::M005S
                );
                assert_eq!(
                    M032LE::accepts_product_id(product_id),
                    model == ModuleKind::M032LE
                );
                assert_eq!(
                    M4315::accepts_product_id(product_id),
                    model == ModuleKind::M4315
                );
            }
        }
        assert_eq!(u16::from(M0064::PRODUCT_ID), 0x01f6);
        assert_eq!(u16::from(M003K::PRODUCT_ID), 0x0202);
        assert_eq!(u16::from(M131E::PRODUCT_ID), 0x0206);
        assert_eq!(u16::from(M005S::PRODUCT_ID), 0x0200);
        assert_eq!(u16::from(M032LE::PRODUCT_ID), 0x0204);
        assert_eq!(u16::from(M4315::PRODUCT_ID), 0x01fe);
    }

    #[test]
    fn does_not_infer_unidentified_products_from_similar_catalog_ids() {
        use crate::{
            Module,
            modules::{M003K, M0064, M131E},
        };

        for product_id in [0x9993, 0x9998, 0xabcd] {
            assert_eq!(ModuleKind::from_ids(0x2516, product_id), None);
            let product_id = crate::ProductId::new(product_id).unwrap();
            assert!(!M0064::accepts_product_id(product_id));
            assert!(!M003K::accepts_product_id(product_id));
            assert!(!M131E::accepts_product_id(product_id));
        }
    }
}
