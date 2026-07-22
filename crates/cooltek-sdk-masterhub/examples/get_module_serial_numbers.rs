use cooltek_sdk_masterhub::{
    Module,
    modules::{M0064, capabilities::CanGetSerialNumber},
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

    println!("{}", serial_number.unwrap_or("".to_string()));
}

fn main() {
    print_serial_number_for::<M0064>();
}
