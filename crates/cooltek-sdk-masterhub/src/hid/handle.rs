use std::marker::PhantomData;

use hidapi::{HidApi, HidError};

use crate::{
    DeviceRegistry, Module, ModuleIdentity,
    hid::handle_state::{Connected, Disconnected, HandleState},
    module::matches_module,
};

/// Serves as a strongly-typed wrapper around [`Module`] information and functionality.
///
/// # Examples
///
/// ## Connect to an official module
///
/// ```no_run
/// use cooltek_sdk_masterhub::{ModuleHandle, modules::M0064};
///
/// let handle = ModuleHandle::<M0064>::new().connect().expect("Failed to connect to module");
///
/// let serial_number = handle.get_serial_number().expect("Failed to get serial number");
/// println!("{}", serial_number.unwrap_or("".to_string()));
/// ```
///
/// ## Connect to a custom module
///
/// ```no_run
/// use cooltek_sdk_masterhub::{Module, ModuleHandle};
/// # use cooltek_sdk_masterhub::{ProductId, product_id};
///
/// struct MyModule {}
/// impl Module for MyModule {
///     // ...
/// #   const PRODUCT_ID: ProductId = product_id!(0xBAD);
/// }
///
/// let handle = ModuleHandle::<MyModule>::new().connect().expect("Failed to connect to module");
/// ```
#[derive(Debug)]
pub struct ModuleHandle<M: Module, S: HandleState = Disconnected> {
    /// The type of the module to operate on.
    _module: PhantomData<M>,

    /// The instance-specific connection to the physical hardware.
    data: S::Data,
}

// Functionality available to any module in the Disconnected state.
impl<M: Module> ModuleHandle<M> {
    /// Constructs a new [`ModuleHandle`] by inferring the [`Module`].
    ///
    /// # Returns
    ///
    /// A new [`ModuleHandle`].
    ///
    /// # Remarks
    ///
    /// The [`ModuleHandle`] will not be connected to the physical module. You must call [`connect()`][ModuleHandle::connect] before use.
    ///
    /// # Examples
    ///
    /// You can specify the module by providing a type annotation for your variable:
    ///
    /// ```
    /// # use cooltek_sdk_masterhub::{ModuleHandle, modules::M0064};
    /// #
    /// let handle: ModuleHandle<M0064> = ModuleHandle::new();
    /// ```
    ///
    /// Alternatively, you can provide a type annotation on [`ModuleHandle`]:
    ///
    /// ```
    /// # use cooltek_sdk_masterhub::{ModuleHandle, modules::M0064};
    /// #
    /// let handle = ModuleHandle::<M0064>::new();
    /// ```
    pub fn new() -> Self {
        let _module = PhantomData;
        let data = ();

        Self { _module, data }
    }

    /// Attempts to connect to the physical hardware using [`hidapi`].
    ///
    /// # Returns
    ///
    /// A connected [`ModuleHandle`] or a [`HidError`].
    ///
    /// # Errors
    ///
    /// * [`HidError`] when the underlying [`HidDevice`](hidapi::HidDevice) returns an error.
    ///
    /// # Remarks
    ///
    /// In the event that multiple instances of a module are connected to the host, this function
    /// will connect to the first instance found.
    ///
    /// A module may enumerate several HID interfaces or top-level collections under one
    /// VID/PID. Among matches, the first entry on a vendor-defined usage page
    /// (`0xff00..=0xffff`), which carries the vendor feature/output reports, is preferred;
    /// otherwise the first VID/PID match is used. To select an exact interface, use
    /// [`Self::connect_by_identity`] with a [`DeviceRegistry`] snapshot.
    ///
    /// It also invokes module initialization, which may send software-mode commands.
    /// For explicit selection without initialization, use [`Self::open_by_identity`].
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use cooltek_sdk_masterhub::{ModuleHandle, modules::M0064};
    /// #
    /// let handle = ModuleHandle::<M0064>::new().connect().expect("Failed to connect to module");
    /// ```
    pub fn connect(self) -> Result<ModuleHandle<M, Connected>, HidError> {
        let context = HidApi::new()?;
        let selected = preferred_interface(
            context
                .device_list()
                .filter(|device| matches_module::<M>(device.vendor_id(), device.product_id())),
            |device| device.usage_page(),
        )
        .ok_or_else(|| HidError::HidApiError {
            message: "No matching module is present in the HID inventory".into(),
        })?;
        let device = selected.open_device(&context)?;
        M::initialize(&device)?;

        let _module = self._module;
        let data = device;

        Ok(ModuleHandle { _module, data })
    }

    /// Opens a selected interface without sending module initialization commands.
    ///
    /// Use this for neutral diagnostics. The identity must match this module's
    /// VID/PID and an interface currently enumerated by `registry`.
    pub fn open_by_identity(
        self,
        registry: &DeviceRegistry,
        identity: &ModuleIdentity,
    ) -> Result<ModuleHandle<M, Connected>, HidError> {
        if !matches_module::<M>(identity.vendor_id, identity.product_id) {
            return Err(HidError::HidApiError {
                message: "Selected interface does not match the requested module type".into(),
            });
        }
        Ok(ModuleHandle {
            _module: self._module,
            data: registry.open(identity)?,
        })
    }

    /// Opens and initializes an explicitly selected interface.
    ///
    /// Unlike [`Self::open_by_identity`], this may send configuration commands,
    /// such as entering software mode on an IPS dial.
    pub fn connect_by_identity(
        self,
        registry: &DeviceRegistry,
        identity: &ModuleIdentity,
    ) -> Result<ModuleHandle<M, Connected>, HidError> {
        let handle = self.open_by_identity(registry, identity)?;
        handle.initialize()?;
        Ok(handle)
    }
}

/// First HID usage page reserved for vendor-defined usages.
const VENDOR_USAGE_PAGE: u16 = 0xff00;

/// Picks the first vendor-usage-page candidate, else the first candidate.
fn preferred_interface<T>(
    candidates: impl Iterator<Item = T>,
    usage_page: impl Fn(&T) -> u16,
) -> Option<T> {
    let mut fallback = None;
    for candidate in candidates {
        if usage_page(&candidate) >= VENDOR_USAGE_PAGE {
            return Some(candidate);
        }
        fallback.get_or_insert(candidate);
    }
    fallback
}

// Functionality available to any module in the Connected state.
impl<M: Module> ModuleHandle<M, Connected> {
    /// Explicitly performs the module's initialization procedure.
    ///
    /// This may send configuration commands. Opening by identity does not invoke it.
    pub fn initialize(&self) -> Result<(), HidError> {
        M::initialize(&self.data)
    }

    /// Borrows the HID transport for descriptor queries or raw report diagnostics.
    ///
    /// The caller is responsible for coordinating input and feature operations.
    pub fn transport(&self) -> &hidapi::HidDevice {
        &self.data
    }

    /// Disconnects from the physical hardware.
    ///
    /// # Examples
    ///
    /// ```compile_fail
    /// # use cooltek_sdk_masterhub::{ModuleHandle, modules::M0064};
    /// #
    /// let handle = ModuleHandle::<M0064>::new().connect().expect("Failed to connect to module");
    /// let handle = handle.disconnect();
    ///
    /// handle.get_serial_number(); // fails to compile
    /// ```
    pub fn disconnect(self) -> ModuleHandle<M, Disconnected> {
        let _module = self._module;
        let data = ();

        ModuleHandle { _module, data }
    }
}

impl<M: Module> Default for ModuleHandle<M> {
    fn default() -> Self {
        Self::new()
    }
}

// -------------------------------
//  Capabilities
// -------------------------------

use crate::modules::capabilities::{CanGetSerialNumber, CanSetSoftwareMode};

impl<M: Module + CanGetSerialNumber> ModuleHandle<M, Connected> {
    /// Attempts to read the serial number of the module.
    ///
    /// # Returns
    ///
    /// A [`Result`] containing an [`Option<String>`] or a [`HidError`].
    ///
    /// # Errors
    ///
    /// * [`HidError`] when the underlying [`HidDevice`](hidapi::HidDevice) returns an error.
    ///
    /// # Remarks
    ///
    /// The USB HID specification does not require devices to actually implement a serial number.
    /// Therefore, we return an [`Option<String>`] (empty string is coalesced to `None`).
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use cooltek_sdk_masterhub::{ModuleHandle, modules::M0064};
    /// #
    /// let handle = ModuleHandle::<M0064>::new().connect().expect("Failed to connect to module");
    /// let serial_number = handle.get_serial_number().expect("Failed to get serial number");
    /// println!("{}", serial_number.unwrap_or("".to_string()));
    /// ```
    pub fn get_serial_number(&self) -> Result<Option<String>, HidError> {
        M::get_serial_number(&self.data)
    }
}

impl<M: Module + CanSetSoftwareMode> ModuleHandle<M, Connected> {
    /// Attempts to enter software mode on the device.
    ///
    /// This allows the program to drive device state.
    ///
    /// # Errors
    ///
    /// * [`HidError`] when the underlying [`HidDevice`](hidapi::HidDevice) returns an error.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use cooltek_sdk_masterhub::{ModuleHandle, modules::M4315};
    /// #
    /// let handle = ModuleHandle::<M4315>::new().connect().expect("Failed to connect to module");
    /// handle.set_software_mode().expect("Failed to set software mode");
    /// ```
    pub fn set_software_mode(&self) -> Result<(), HidError> {
        M::set_software_mode(&self.data)
    }
}

impl ModuleHandle<crate::modules::M131E, Connected> {
    /// Explicitly selects software (`true`) or hardware (`false`) mode.
    pub fn set_dial_mode(&self, software: bool) -> hidapi::HidResult<()> {
        crate::dial::set_software_mode(&self.data, software)
    }

    /// Sets dial backlight to an integer from 0 through 100.
    pub fn set_backlight(&self, value: u8) -> hidapi::HidResult<()> {
        crate::dial::set_backlight(&self.data, value)
    }

    /// Queries the dial backlight using feature report `4c`.
    pub fn get_backlight(&self) -> hidapi::HidResult<u8> {
        crate::dial::read_backlight(&self.data)
    }

    /// Sends the three color bytes in native argument order.
    ///
    /// RGB channel order has not been established by the analyzed application.
    pub fn set_color_channels(&self, channels: [u8; 3]) -> hidapi::HidResult<()> {
        crate::dial::set_color(&self.data, channels)
    }

    /// Queries a firmware-information slot without a Windows length adjustment.
    pub fn get_firmware_info(
        &self,
        kind: crate::dial::FirmwareInfoKind,
    ) -> hidapi::HidResult<crate::dial::FirmwareInfo> {
        crate::dial::read_firmware_info(&self.data, kind)
    }

    /// Uploads an encoded 240×240 baseline JFIF JPEG using descriptor-sized reports.
    ///
    /// Does not enable software mode, change backlight, or wait for an inferred
    /// device acknowledgement. All image/descriptor validation precedes writes.
    /// Report byte 6 and unused padding are zeroed deliberately; hardware
    /// acceptance and device pacing remain unverified.
    pub fn display_jpeg(&self, jpeg: &[u8]) -> hidapi::HidResult<usize> {
        let length = crate::transport::output_report_length(&self.data, 0x02)?;
        crate::dial::display_jpeg(&self.data, jpeg, length)
    }

    /// Transfers ownership into an explicit lifecycle/input session without I/O.
    ///
    /// The handle may already be in software mode (for example after
    /// [`ModuleHandle::connect`]), so the session's
    /// [`shutdown`](crate::DialSession::shutdown) restores hardware mode even
    /// if [`activate`](crate::DialSession::activate) was never called.
    pub fn into_dial_session(self) -> crate::DialSession<hidapi::HidDevice> {
        crate::DialSession::from_possibly_software_mode(self.data)
    }
}

#[cfg(test)]
mod identity_tests {
    use super::*;
    use crate::modules::{M003K, M0064, M131E};

    #[test]
    fn typed_matching_accepts_aliases_and_rejects_wrong_module_or_vendor() {
        for pid in [0x0206, 0x9994] {
            assert!(matches_module::<M131E>(0x2516, pid));
            assert!(!matches_module::<M131E>(0x1234, pid));
            assert!(!matches_module::<M003K>(0x2516, pid));
        }
        assert!(matches_module::<M003K>(0x2516, 0x9996));
        assert!(matches_module::<M0064>(0x2516, 0x9999));
        assert!(!matches_module::<M131E>(0x2516, 0));
        assert!(!matches_module::<M131E>(0x2516, 0xffff));
    }

    #[test]
    fn connect_prefers_the_vendor_usage_page_interface() {
        let pick = |pages: &[u16]| {
            preferred_interface(pages.iter().copied().enumerate(), |(_, page)| *page)
                .map(|(index, _)| index)
        };
        assert_eq!(pick(&[0x0001, 0x000c, 0xff00, 0xff01]), Some(2));
        assert_eq!(pick(&[0xffa0, 0xff00]), Some(0));
        assert_eq!(pick(&[0x0001, 0x000c]), Some(0));
        assert_eq!(pick(&[]), None);
    }
}
