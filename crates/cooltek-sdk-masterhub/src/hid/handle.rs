use std::marker::PhantomData;

use hidapi::{HidApi, HidDevice, HidError};

use crate::Module;

/// Serves as a strongly-typed wrapper around [`Module`] information and functionality.
///
/// # Examples
///
/// ## Connect to an official module
///
/// ```no_run
/// use cooltek_sdk_masterhub::{ModuleHandle, modules::M0064};
///
/// let mut handle = ModuleHandle::<M0064>::new();
/// handle.connect().expect("Failed to connect to module");
///
/// let serial_number = handle.get_serial_number().expect("Failed to get serial number");
/// println!("{}", serial_number.unwrap_or("".to_string()));
/// ```
///
/// ## Connect to a custom module
///
/// ```no_run
/// use cooltek_sdk_masterhub::{Module, ModuleHandle};
///
/// struct MyModule {}
/// impl Module for MyModule {
///     // ...
/// #   const PRODUCT_ID: u16 = 0xBAD;
/// }
///
/// let mut handle = ModuleHandle::<MyModule>::new();
/// handle.connect().expect("Failed to connect to module");
/// ```
#[derive(Debug)]
pub struct ModuleHandle<M: Module> {
    /// The type of the module to operate on.
    _module: PhantomData<M>,

    /// The instance-specific connection to the physical hardware.
    device: Option<HidDevice>,
}

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
        let device = None;

        Self { _module, device }
    }

    /// Attempts to connect to the physical hardware using [`hidapi`].
    ///
    /// # Returns
    ///
    /// A [`Result`] containing either the unit type or a [`HidError`].
    ///
    /// # Errors
    ///
    /// * [`HidError`] when the underlying [`HidDevice`] returns an error.
    ///
    /// # Remarks
    ///
    /// In the event that multiple instances of a module are connected to the host, this function
    /// will connect to the first instance found.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use cooltek_sdk_masterhub::{ModuleHandle, modules::M0064};
    /// #
    /// let mut handle = ModuleHandle::<M0064>::new();
    /// handle.connect().expect("Failed to connect to module");
    /// ```
    pub fn connect(&mut self) -> Result<(), HidError> {
        let context = HidApi::new()?;
        let device = context.open(M::VENDOR_ID, M::PRODUCT_ID)?;

        self.device = Some(device);

        Ok(())
    }

    /// Disconnects from the physical hardware.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use cooltek_sdk_masterhub::{ModuleHandle, modules::M0064};
    /// #
    /// let mut handle = ModuleHandle::<M0064>::new();
    /// handle.connect().expect("Failed to connect to module");
    /// handle.disconnect();
    ///
    /// handle.get_serial_number(); // panics
    /// ```
    pub fn disconnect(&mut self) {
        self.device = None;
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

impl<M: Module + CanGetSerialNumber> ModuleHandle<M> {
    /// Attempts to read the serial number of the module.
    ///
    /// # Returns
    ///
    /// A [`Result`] containing an [`Option<String>`] or a [`HidError`].
    ///
    /// # Errors
    ///
    /// * [`HidError`] when the underlying [`HidDevice`] returns an error.
    ///
    /// # Panics
    ///
    /// * When [`connect()`][ModuleHandle::connect] has not been used on the [`ModuleHandle`].
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
    /// let mut handle = ModuleHandle::<M0064>::new();
    /// handle.connect().expect("Failed to connect to module");
    ///
    /// let serial_number = handle.get_serial_number().expect("Failed to get serial number");
    /// println!("{}", serial_number.unwrap_or("".to_string()));
    /// ```
    pub fn get_serial_number(&self) -> Result<Option<String>, HidError> {
        M::get_serial_number(self.device.as_ref().expect("Handle not connected"))
    }
}

impl<M: Module + CanSetSoftwareMode> ModuleHandle<M> {
    /// Attempts to enter software mode on the device.
    ///
    /// This allows the program to drive device state.
    ///
    /// # Errors
    ///
    /// * [`HidError`] when the underlying [`HidDevice`] returns an error.
    ///
    /// # Panics
    ///
    /// * When [`connect()`][ModuleHandle::connect] has not been used on the [`ModuleHandle`].
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use cooltek_sdk_masterhub::{ModuleHandle, modules::M4315};
    /// #
    /// let mut handle = ModuleHandle::<M4315>::new();
    /// handle.connect().expect("Failed to connect to module");
    /// handle.set_software_mode().expect("Failed to set software mode");
    /// ```
    pub fn set_software_mode(&self) -> Result<(), HidError> {
        M::set_software_mode(self.device.as_ref().expect("Handle not connected"))
    }
}
