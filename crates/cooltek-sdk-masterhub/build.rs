//! Compiles the macOS-only IOUSBHost shim used to send the roller's HID SET_IDLE request.

fn main() {
    println!("cargo:rerun-if-changed=src/set_idle_macos.m");
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    cc::Build::new()
        .file("src/set_idle_macos.m")
        .flag("-fobjc-arc")
        .compile("cooltek_set_idle");
    for framework in ["Foundation", "IOKit", "IOUSBHost"] {
        println!("cargo:rustc-link-lib=framework={framework}");
    }
}
