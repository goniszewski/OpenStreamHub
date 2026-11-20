//! Compiles the macOS-only IOUSBHost shims: the roller's HID SET_IDLE request, and re-enumerating a module's USB
//! device.

fn main() {
    println!("cargo:rerun-if-changed=src/set_idle_macos.m");
    println!("cargo:rerun-if-changed=src/reenumerate_macos.c");
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    cc::Build::new()
        .file("src/set_idle_macos.m")
        .flag("-fobjc-arc")
        .compile("cooltek_set_idle");
    cc::Build::new()
        .file("src/reenumerate_macos.c")
        .compile("cooltek_reenumerate");
    for framework in ["CoreFoundation", "Foundation", "IOKit", "IOUSBHost"] {
        println!("cargo:rustc-link-lib=framework={framework}");
    }
}
