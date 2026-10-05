mod artwork;
#[cfg(test)]
mod contract_fixtures;
mod diagnostics;
mod export;
mod feed;
mod migration;
#[cfg(target_os = "macos")]
mod quit;
mod replay;
mod service;
mod store;
#[cfg(test)]
mod test_profiles;

use cooltek_sdk_service::{Gesture, InputValue, Profile, ServiceModule};
use serde::Serialize;
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tauri::{Emitter, Manager, State};

struct AppState {
    store: Mutex<Result<store::ProfileStore, String>>,
    app_data: Result<PathBuf, String>,
    feeds: feed::Feeds,
    inventory: feed::InventoryWatcher,
    services: service::Services,
}

/// Bootstrap failures carry the expected storage folder so the frontend can
/// tell the user where to look before retrying.
#[derive(Debug, Serialize)]
struct BootstrapError {
    message: String,
    storage_path: Option<String>,
}

impl AppState {
    fn open(app_data: Result<PathBuf, String>) -> Self {
        let store = app_data
            .clone()
            .and_then(store::ProfileStore::open_for_app_data);
        Self {
            store: Mutex::new(store),
            app_data,
            feeds: feed::Feeds::default(),
            inventory: feed::InventoryWatcher::default(),
            services: service::Services::default(),
        }
    }

    fn with_store<T>(
        &self,
        operation: impl FnOnce(&store::ProfileStore) -> Result<T, String>,
    ) -> Result<T, String> {
        let store = self
            .store
            .lock()
            .map_err(|_| "profile storage lock is unavailable".to_string())?;
        operation(store.as_ref().map_err(Clone::clone)?)
    }

    /// Opens storage again if the previous attempt failed, so a retry after
    /// the user fixes the folder does not require restarting the app.
    fn bootstrap(&self) -> Result<store::AppBootstrap, BootstrapError> {
        if let (Ok(mut store), Ok(app_data)) = (self.store.lock(), &self.app_data)
            && store.is_err()
        {
            *store = store::ProfileStore::open_for_app_data(app_data.clone());
        }
        self.with_store(store::ProfileStore::bootstrap)
            .map_err(|message| BootstrapError {
                message,
                storage_path: self
                    .app_data
                    .as_ref()
                    .ok()
                    .map(|path| path.join("profiles").to_string_lossy().into_owned()),
            })
    }
}

#[derive(Serialize)]
struct ProfileValidity {
    valid: bool,
}

#[tauri::command]
fn bootstrap(state: State<'_, AppState>) -> Result<store::AppBootstrap, BootstrapError> {
    state.bootstrap()
}

#[tauri::command]
fn list_profiles(state: State<'_, AppState>) -> Result<store::ProfileListing, String> {
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
    expected_modified_at_ms: Option<u64>,
) -> Result<store::StoredProfile, String> {
    replay::validate_profile(&profile)?;
    state.with_store(|store| store.save(id.as_deref(), profile, expected_modified_at_ms))
}

#[tauri::command]
fn delete_profile(
    state: State<'_, AppState>,
    id: String,
    expected_modified_at_ms: Option<u64>,
) -> Result<(), String> {
    state.with_store(|store| store.delete(&id, expected_modified_at_ms))
}

#[tauri::command]
fn validate_profile(profile: Profile) -> Result<ProfileValidity, String> {
    replay::validate_profile(&profile)?;
    Ok(ProfileValidity { valid: true })
}

// Writes one profile as a local JSON file under `<app data>/exports/`.
#[tauri::command]
fn export_profile(
    state: State<'_, AppState>,
    profile: Profile,
) -> Result<export::ExportResult, String> {
    let app_data = state.app_data.as_deref().map_err(Clone::clone)?;
    export::export(&profile, app_data, export::now_seconds()?)
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

// Device feeds are read-only: enumeration and input reads only, never software
// mode, profiles, or actions. Enumeration and joining run off the main thread.
//
// `list_live_devices` enumerates on demand with its own registry rather than
// returning the watcher's latest snapshot, so it is never up to one poll
// interval stale and reports enumeration errors. Both share `live_devices`, so
// the list matches `device://inventory` for the same enumeration.
#[tauri::command(async)]
fn list_live_devices() -> Result<Vec<feed::LiveDevice>, String> {
    let registry = cooltek_sdk_masterhub::DeviceRegistry::new()
        .map_err(|error| format!("could not enumerate HID devices: {error}"))?;
    Ok(feed::live_devices(&registry.devices()))
}

#[tauri::command(async)]
fn start_feed(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    source: feed::FeedSource,
) -> Result<String, String> {
    let feed_id = feed::feed_id(&source)?;
    let thread_id = feed_id.clone();
    state.feeds.start(feed_id, move |stop| {
        feed::run_feed(&thread_id, &source, stop, |message| {
            let emitted = match message {
                feed::FeedMessage::Input(payload) => app.emit(feed::INPUT_EVENT, payload),
                feed::FeedMessage::Status(payload) => app.emit(feed::STATUS_EVENT, payload),
            };
            if let Err(error) = emitted {
                eprintln!("could not emit a device feed event: {error}");
            }
        })
    })
}

#[tauri::command(async)]
fn stop_feed(state: State<'_, AppState>, feed_id: String) -> Result<(), String> {
    state.feeds.stop(&feed_id)
}

// Profile actions run only in a supervised `masterhub-service` child process,
// never in the app. A read-only feed on the same interface is stopped first
// so the service owns the device.
#[tauri::command(async)]
fn start_service(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    request: service::StartRequest,
) -> Result<String, String> {
    let emit: service::Emit = Arc::new(move |message| {
        let emitted = match message {
            service::ServiceMessage::Record(payload) => app.emit(service::RECORD_EVENT, payload),
            service::ServiceMessage::Status(payload) => app.emit(service::STATUS_EVENT, payload),
        };
        if let Err(error) = emitted {
            eprintln!("could not emit a service event: {error}");
        }
    });
    service::start_service(
        &state.services,
        request,
        state.app_data.as_deref().map_err(Clone::clone),
        service_program(),
        |feed_id| state.feeds.stop(feed_id),
        emit,
    )
}

fn service_program() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().ok();
    service::resolve_service_binary(
        std::env::var_os(service::BINARY_ENV).as_deref(),
        exe.as_deref().and_then(Path::parent),
        cfg!(debug_assertions),
    )
}

// Writes a local diagnostic bundle; nothing is uploaded. Probing is read-only
// and runs on a blocking worker; interfaces held by a running feed or service
// are checked right before each open and skipped.
#[tauri::command]
async fn create_diagnostics(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    request: diagnostics::DiagnosticsRequest,
) -> Result<diagnostics::DiagnosticsResult, String> {
    let app_data = state.app_data.clone()?;
    let running = diagnostics::Running {
        feeds: state.feeds.running_ids(),
        services: state.services.running_ids(),
    };
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let in_use = |path: &[u8]| {
            let hex = cooltek_sdk_cli::capture::encode_hex(path);
            state.feeds.is_running(&format!("live:{hex}"))
                || state.services.is_running(&format!("service:{hex}"))
        };
        diagnostics::create(
            request,
            &app_data,
            &running,
            &in_use,
            service_program(),
            &mut diagnostics::HidProbe::default(),
        )
    })
    .await
    .map_err(|error| format!("the diagnostics worker failed: {error}"))?
}

// Offline artwork preparation: decode, fit and encode on a blocking worker.
// Nothing is sent to a device or over the network.
#[tauri::command]
async fn prepare_artwork(
    request: artwork::PrepareRequest,
) -> Result<artwork::PrepareResult, String> {
    tauri::async_runtime::spawn_blocking(move || artwork::prepare_artwork(&request))
        .await
        .map_err(|error| format!("the artwork worker failed: {error}"))?
}

// Writes the prepared JPEG under `<app data>/artwork/`; nothing is uploaded.
#[tauri::command]
async fn save_artwork(
    state: State<'_, AppState>,
    request: artwork::SaveRequest,
) -> Result<artwork::SaveResult, String> {
    let app_data = state.app_data.clone()?;
    let generated_at = export::now_seconds()?;
    tauri::async_runtime::spawn_blocking(move || {
        artwork::save_artwork(&request, &app_data, generated_at)
    })
    .await
    .map_err(|error| format!("the artwork worker failed: {error}"))?
}

#[tauri::command(async)]
fn stop_service(state: State<'_, AppState>, service_id: String) -> Result<(), String> {
    state.services.stop(&service_id, service::STOP_GRACE)
}

pub fn run() {
    // A second launch focuses the existing window instead of opening another
    // editor over the same profile files.
    let builder = tauri::Builder::default().plugin(tauri_plugin_single_instance::init(
        |app, _arguments, _working_directory| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        },
    ));
    #[cfg(target_os = "macos")]
    let builder = builder.menu(quit::menu).on_menu_event(quit::on_menu_event);

    let app = builder
        .setup(|app| {
            let app_data = app
                .path()
                .app_data_dir()
                .map_err(|error| format!("could not resolve profile storage: {error}"));
            if let Ok(app_data) = &app_data {
                service::clean_stale_profiles(&service::profile_dir(app_data));
            }
            app.manage(AppState::open(app_data));
            // Hot-plug detection: the watcher emits `device://inventory` once
            // at start and whenever the input-capable device list changes.
            // Without it the app still works; devices can be listed on demand.
            let handle = app.handle().clone();
            let started = app.state::<AppState>().inventory.start(move |stop| {
                feed::watch_inventory(stop, feed::hid_inventory(), feed::wait, |payload| {
                    if let Err(error) = handle.emit(feed::INVENTORY_EVENT, payload) {
                        eprintln!("could not emit the device inventory: {error}");
                    }
                })
            });
            if let Err(error) = started {
                eprintln!("{error}");
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            bootstrap,
            list_profiles,
            load_profile,
            save_profile,
            delete_profile,
            validate_profile,
            export_profile,
            replay_demo,
            preview_binding,
            preview_keypad,
            list_live_devices,
            start_feed,
            stop_feed,
            start_service,
            stop_service,
            create_diagnostics,
            prepare_artwork,
            save_artwork
        ])
        .build(tauri::generate_context!())
        .expect("could not start the OpenStreamHub desktop app");

    app.run(|app, event| {
        if let tauri::RunEvent::Exit = event {
            let state = app.state::<AppState>();
            // Bounded: SIGINT, up to STOP_GRACE, then SIGKILL; never joins.
            state.services.stop_all(service::STOP_GRACE);
            state.feeds.stop_all();
            state.inventory.stop();
            return;
        }
        // Only user/system exit requests (code None) are routed through the
        // draft guard. An explicit app.exit(code) proceeds as requested.
        if let tauri::RunEvent::ExitRequested {
            api, code: None, ..
        } = event
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

#[cfg(test)]
mod tests {
    use super::*;

    fn failed(app_data: Result<PathBuf, String>) -> AppState {
        AppState {
            store: Mutex::new(Err("could not open storage".into())),
            app_data,
            feeds: feed::Feeds::default(),
            inventory: feed::InventoryWatcher::default(),
            services: service::Services::default(),
        }
    }

    #[test]
    fn app_state_reports_the_store_open_error_for_every_storage_operation() {
        let state = failed(Err("no app data".into()));
        let mut called = false;
        let result = state.with_store(|_| {
            called = true;
            Ok(())
        });
        assert_eq!(result.unwrap_err(), "could not open storage");
        assert!(!called);
        assert_eq!(
            state.with_store(|store| store.list()).unwrap_err(),
            "could not open storage"
        );
        let error = state.bootstrap().unwrap_err();
        assert_eq!(error.message, "could not open storage");
        assert_eq!(error.storage_path, None);
    }

    #[test]
    fn bootstrap_retry_reopens_storage_after_the_folder_is_fixed() {
        let directory = tempfile::tempdir().unwrap();
        let app_data = directory.path().join("dev.cooltek.openstreamhub");
        // A file where the app data folder belongs blocks opening storage.
        std::fs::write(&app_data, b"not a folder").unwrap();
        let state = AppState::open(Ok(app_data.clone()));
        let error = state.bootstrap().unwrap_err();
        assert!(error.message.contains("must be a directory"));
        assert_eq!(
            error.storage_path.as_deref(),
            Some(app_data.join("profiles").to_str().unwrap())
        );
        assert!(state.with_store(|store| store.list()).is_err());

        std::fs::remove_file(&app_data).unwrap();
        std::fs::create_dir(&app_data).unwrap();
        let bootstrap = state.bootstrap().unwrap();
        assert_eq!(bootstrap.profiles.len(), 1);
        assert!(state.with_store(|store| store.list()).is_ok());
    }

    #[test]
    fn app_state_runs_operations_against_an_open_store() {
        let directory = tempfile::tempdir().unwrap();
        let state = AppState::open(Ok(directory.path().join("app")));
        let bootstrap = state.bootstrap().unwrap();
        assert_eq!(bootstrap.profiles.len(), 1);
    }
}
