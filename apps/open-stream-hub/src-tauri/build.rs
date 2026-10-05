fn main() {
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "bootstrap",
            "list_profiles",
            "load_profile",
            "save_profile",
            "delete_profile",
            "validate_profile",
            "export_profile",
            "replay_demo",
            "preview_binding",
            "preview_keypad",
            "list_live_devices",
            "start_feed",
            "stop_feed",
            "start_service",
            "stop_service",
            "resume_session",
            "stop_session",
            "create_diagnostics",
            "prepare_artwork",
            "save_artwork",
        ]),
    ))
    .expect("could not generate the OpenStreamHub app permissions");
}
