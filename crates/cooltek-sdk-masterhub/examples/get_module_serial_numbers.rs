use cooltek_sdk_masterhub::{
    Module,
    modules::{
        M003K, M005S, M032LE, M0064, M131E, M4315,
        capabilities::{CanGetSerialNumber, CanSetSoftwareMode},
    },
};

fn print_serial_number_for<M: Module + CanGetSerialNumber>() {
    let context = hidapi::HidApi::new().expect("Failed to initialize HidApi");
    let device = match context.open(M::VENDOR_ID, M::PRODUCT_ID) {
        Ok(device) => device,
        Err(_) => return, // silently swallow errors; module probably not connected
    };
    let serial_number = match M::get_serial_number(&device) {
        Ok(serial_number) => serial_number,
        Err(_) => return, // silently swallow errors; probably transient error
    };

    println!(
        "{}: {}",
        M::PRODUCT_NAME.unwrap_or("Unknown"),
        serial_number.unwrap_or("".to_string())
    );
}

fn print_serial_number_for_ips<M: Module + CanGetSerialNumber + CanSetSoftwareMode>() {
    let context = hidapi::HidApi::new().expect("Failed to initialize HidApi");
    let device = match context.open(M::VENDOR_ID, M::PRODUCT_ID) {
        Ok(device) => device,
        Err(_) => return, // silently swallow errors; module probably not connected
    };

    // Modules with an IPS display need to be in software mode before they can be used.
    match M::set_software_mode(&device) {
        Ok(_) => (),
        Err(_) => return, // silently swallow errors; probably transient error
    };

    let serial_number = match M::get_serial_number(&device) {
        Ok(serial_number) => serial_number,
        Err(_) => return, // silently swallow errors; probably transient error
    };

    println!(
        "{}: {}",
        M::PRODUCT_NAME.unwrap_or("Unknown"),
        serial_number.unwrap_or("".to_string())
    );
}

fn main() {
    print_serial_number_for::<M0064>();
    print_serial_number_for::<M003K>();
    print_serial_number_for::<M005S>();
    print_serial_number_for::<M032LE>();
    print_serial_number_for_ips::<M131E>();
    print_serial_number_for_ips::<M4315>();
}
