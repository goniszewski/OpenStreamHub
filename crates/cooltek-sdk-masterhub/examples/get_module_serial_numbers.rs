use cooltek_sdk_masterhub::{
    DeviceRegistry, Module, ModuleHandle, ModuleIdentity, ModuleKind,
    modules::{M003K, M005S, M032LE, M0064, M131E, M4315, capabilities::CanGetSerialNumber},
};

// Opening by identity does not initialize a module or change the dial's mode.
// Reading a serial explicitly issues a vendor feature query, unlike `list`.
fn print_serial_number_for<M: Module + CanGetSerialNumber>(
    registry: &DeviceRegistry,
    identity: &ModuleIdentity,
) -> Result<(), cooltek_sdk_masterhub::hidapi::HidError> {
    let handle = ModuleHandle::<M>::new().open_by_identity(registry, identity)?;
    let serial_number = handle.get_serial_number()?;

    println!(
        "{}: {}",
        M::PRODUCT_NAME.unwrap_or("Unknown"),
        serial_number.unwrap_or_default()
    );
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let registry = DeviceRegistry::new()?;
    for identity in registry.devices() {
        let result = match identity.module {
            Some(ModuleKind::M0064) => print_serial_number_for::<M0064>(&registry, &identity),
            Some(ModuleKind::M003K) => print_serial_number_for::<M003K>(&registry, &identity),
            Some(ModuleKind::M005S) => print_serial_number_for::<M005S>(&registry, &identity),
            Some(ModuleKind::M032LE) => print_serial_number_for::<M032LE>(&registry, &identity),
            Some(ModuleKind::M131E) => print_serial_number_for::<M131E>(&registry, &identity),
            Some(ModuleKind::M4315) => print_serial_number_for::<M4315>(&registry, &identity),
            None => continue,
        };
        if let Err(error) = result {
            eprintln!("Serial query failed for {:?}: {error}", identity.path);
        }
    }
    Ok(())
}
