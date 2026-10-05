fn main() {
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "bootstrap",
            "list_profiles",
            "load_profile",
            "save_profile",
            "validate_profile",
            "replay_demo",
            "preview_binding",
            "preview_keypad",
        ]),
    ))
    .expect("could not generate the OpenStreamHub app permissions");
}
