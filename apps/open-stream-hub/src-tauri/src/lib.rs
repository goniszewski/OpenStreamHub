mod migration;
#[cfg(target_os = "macos")]
mod quit;
mod replay;
mod store;
#[cfg(test)]
mod test_profiles;

use cooltek_sdk_service::{Gesture, InputValue, Profile, ServiceModule};
use serde::Serialize;
use std::sync::Mutex;
use tauri::{Manager, State};

struct AppState(Mutex<Result<store::ProfileStore, String>>);

impl AppState {
    fn with_store<T>(
        &self,
        operation: impl FnOnce(&store::ProfileStore) -> Result<T, String>,
    ) -> Result<T, String> {
        let store = self
            .0
            .lock()
            .map_err(|_| "profile storage lock is unavailable".to_string())?;
        operation(store.as_ref().map_err(Clone::clone)?)
    }
}

#[derive(Serialize)]
struct ProfileValidity {
    valid: bool,
}

#[tauri::command]
fn bootstrap(state: State<'_, AppState>) -> Result<store::AppBootstrap, String> {
    state.with_store(store::ProfileStore::bootstrap)
}

#[tauri::command]
fn list_profiles(state: State<'_, AppState>) -> Result<Vec<store::ProfileSummary>, String> {
    state.with_store(store::ProfileStore::list)
}

#[tauri::command]
fn load_profile(state: State<'_, AppState>, id: String) -> Result<store::StoredProfile, String> {
    state.with_store(|store| store.load(&id))
}

#[tauri::command]
fn save_profile(
    state: State<'_, AppState>,
    id: Option<String>,
    profile: Profile,
) -> Result<store::StoredProfile, String> {
    replay::validate_profile(&profile)?;
    state.with_store(|store| store.save(id.as_deref(), profile))
}

#[tauri::command]
fn validate_profile(profile: Profile) -> Result<ProfileValidity, String> {
    replay::validate_profile(&profile)?;
    Ok(ProfileValidity { valid: true })
}

#[tauri::command]
fn replay_demo(profile: Profile, kind: replay::DemoKind) -> Result<replay::ReplayResult, String> {
    replay::replay_demo(profile, kind)
}

#[tauri::command]
fn preview_binding(
    profile: Profile,
    module: ServiceModule,
    control: u8,
    gesture: Gesture,
    serial_number: Option<String>,
    value: Option<InputValue>,
) -> Result<replay::ReplayEvent, String> {
    replay::preview_binding(profile, module, control, gesture, serial_number, value)
}

#[tauri::command]
fn preview_keypad(
    profile: Profile,
    path: Vec<String>,
    control: u8,
    serial_number: Option<String>,
) -> Result<replay::KeypadPreview, String> {
    replay::preview_keypad(profile, path, control, serial_number)
}

pub fn run() {
    let builder = tauri::Builder::default();
    #[cfg(target_os = "macos")]
    let builder = builder.menu(quit::menu).on_menu_event(quit::on_menu_event);

    let app = builder
        .setup(|app| {
            let store = app
                .path()
                .app_data_dir()
                .map_err(|error| format!("could not resolve profile storage: {error}"))
                .and_then(store::ProfileStore::open_for_app_data);
            app.manage(AppState(Mutex::new(store)));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            bootstrap,
            list_profiles,
            load_profile,
            save_profile,
            validate_profile,
            replay_demo,
            preview_binding,
            preview_keypad
        ])
        .build(tauri::generate_context!())
        .expect("could not start the OpenStreamHub desktop app");

    app.run(|app, event| {
        if let tauri::RunEvent::ExitRequested { api, .. } = event
            && let Some(window) = app.get_webview_window("main")
        {
            // Tauri exit requests pass through the same draft guard as the
            // close button. Closing queues CloseRequested; the frontend can cancel
            // it or destroy the window after the user's decision. Once the
            // last window is destroyed, Tauri removes it before requesting
            // exit again, so that final request proceeds without a loop.
            api.prevent_exit();
            if let Err(error) = window.close() {
                eprintln!("could not request closing the OpenStreamHub window: {error}");
            }
        }
    });
}
