//! Keep running in the macOS menu bar when the window is closed.
//!
//! With the setting on (the default), the window's close button hides the
//! window instead of ending the app: sessions, feeds and the focused-app rules
//! keep running, the Dock icon goes away, and a menu bar icon offers Show and
//! Quit. Quit (the menu bar item, the app menu, or a system quit request) shows
//! the window and asks the frontend to run its usual unsaved-draft guard before
//! the app exits. With the setting off, nothing here acts and a close ends the
//! app as before.
//!
//! The choice lives in `<app data>/menubar.json` (`{"enabled":true}`); a
//! missing or unreadable file means on, and an unreadable file is never
//! overwritten except by an explicit change of the setting.

use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};
use tauri::{
    AppHandle, Emitter, Manager,
    image::Image,
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
};

const FILE_NAME: &str = "menubar.json";
const MAX_FILE_BYTES: u64 = 1024;
const TRAY_ID: &str = "open-stream-hub-tray";
const SHOW_ID: &str = "open-stream-hub-tray-show";
const UPDATES_ID: &str = "open-stream-hub-tray-updates";
const QUIT_ID: &str = "open-stream-hub-tray-quit";
/// Menu items that open a page: (menu id, label, frontend page).
const PAGES: [(&str, &str, &str); 4] = [
    ("open-stream-hub-tray-workspace", "Workspace", "workspace"),
    ("open-stream-hub-tray-lighting", "Lighting", "lighting"),
    ("open-stream-hub-tray-demos", "Demos", "demos"),
    ("open-stream-hub-tray-settings", "Settings", "settings"),
];
/// Sent to the frontend with the page to open (a string).
pub const NAVIGATE_EVENT: &str = "app://navigate";
/// Sent to the frontend to run an update check and show the result.
pub const CHECK_UPDATES_EVENT: &str = "app://check-updates";
/// Sent to the frontend to run its unsaved-draft guard and close the window.
pub const QUIT_EVENT: &str = "app://quit-requested";

pub struct MenubarState {
    path: Option<PathBuf>,
    enabled: AtomicBool,
}

impl MenubarState {
    pub fn open(app_data: Option<&Path>) -> Self {
        let path = app_data.map(|dir| dir.join(FILE_NAME));
        let enabled = path.as_deref().map(read).unwrap_or(true);
        Self {
            path,
            enabled: AtomicBool::new(enabled),
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }

    fn save(&self, enabled: bool) -> Result<(), String> {
        let path = self.path.as_ref().ok_or("the app data folder is unknown")?;
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, format!("{{\"enabled\":{enabled}}}\n"))
            .and_then(|()| std::fs::rename(&temporary, path))
            .map_err(|error| format!("could not write {}: {error}", path.display()))?;
        self.enabled.store(enabled, Ordering::SeqCst);
        Ok(())
    }
}

fn read(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return true;
    };
    if metadata.len() > MAX_FILE_BYTES {
        return true;
    }
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|value| value.get("enabled")?.as_bool())
        .unwrap_or(true)
}

/// The tray icon: the logo's square eye (a rounded square outline around a
/// rounded square pupil with a small highlight), drawn as a macOS template
/// image (black with alpha; the system tints it for the menu bar).
fn icon() -> Image<'static> {
    const SIZE: u32 = 44;
    const CENTRE: f32 = SIZE as f32 / 2.0;
    // Antialiased coverage of a rounded square of half-width `half` and corner
    // radius `radius`, centred at (`cx`, `cy`).
    let square = |x: f32, y: f32, cx: f32, cy: f32, half: f32, radius: f32| {
        let qx = (x - cx).abs() - (half - radius);
        let qy = (y - cy).abs() - (half - radius);
        let distance = qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - radius;
        (0.5 - distance).clamp(0.0, 1.0)
    };
    let mut rgba = vec![0u8; (SIZE * SIZE * 4) as usize];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let ring = square(px, py, CENTRE, CENTRE, 20.0, 9.0)
                * (1.0 - square(px, py, CENTRE, CENTRE, 16.5, 6.5));
            let pupil = square(px, py, CENTRE, CENTRE, 6.5, 4.0)
                * (1.0 - square(px, py, CENTRE - 0.3, CENTRE - 0.3, 2.8, 1.8));
            rgba[((y * SIZE + x) * 4 + 3) as usize] = (ring.max(pupil) * 255.0) as u8;
        }
    }
    Image::new_owned(rgba, SIZE, SIZE)
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    if app.tray_by_id(TRAY_ID).is_some() {
        return Ok(());
    }
    let show = MenuItem::with_id(app, SHOW_ID, "Show OpenStreamHub", true, None::<&str>)?;
    let pages = PAGES
        .iter()
        .map(|(id, label, _)| MenuItem::with_id(app, *id, *label, true, None::<&str>))
        .collect::<tauri::Result<Vec<_>>>()?;
    let updates = MenuItem::with_id(app, UPDATES_ID, "Check for new version", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, QUIT_ID, "Quit", true, None::<&str>)?;
    let separators = [
        PredefinedMenuItem::separator(app)?,
        PredefinedMenuItem::separator(app)?,
        PredefinedMenuItem::separator(app)?,
    ];
    let mut items: Vec<&dyn tauri::menu::IsMenuItem<tauri::Wry>> = vec![&show, &separators[0]];
    items.extend(
        pages
            .iter()
            .map(|item| item as &dyn tauri::menu::IsMenuItem<tauri::Wry>),
    );
    items.extend([
        &separators[1] as &dyn tauri::menu::IsMenuItem<tauri::Wry>,
        &updates,
        &separators[2],
        &quit,
    ]);
    let menu = Menu::with_items(app, &items)?;
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon())
        .icon_as_template(true)
        .tooltip("OpenStreamHub")
        .menu(&menu)
        .on_menu_event(|app, event: MenuEvent| match event.id().as_ref() {
            SHOW_ID => show_window(app),
            UPDATES_ID => {
                show_window(app);
                if let Err(error) = app.emit(CHECK_UPDATES_EVENT, ()) {
                    eprintln!("could not ask OpenStreamHub to check for updates: {error}");
                }
            }
            QUIT_ID => request_quit(app),
            id => {
                if let Some((_, _, page)) = PAGES.iter().find(|(page_id, ..)| *page_id == id) {
                    show_window(app);
                    if let Err(error) = app.emit(NAVIGATE_EVENT, *page) {
                        eprintln!("could not open the {page} page: {error}");
                    }
                }
            }
        })
        .build(app)?;
    Ok(())
}

fn remove_tray(app: &AppHandle) {
    drop(app.remove_tray_by_id(TRAY_ID));
}

/// Creates the menu bar icon at launch when the setting is on.
pub fn init(app: &AppHandle) {
    if app.state::<MenubarState>().enabled()
        && let Err(error) = build_tray(app)
    {
        eprintln!("could not create the menu bar icon: {error}");
    }
}

/// Applies a changed setting: the icon appears or goes away, and a hidden
/// window comes back when the setting is turned off (nothing else could
/// reach it).
pub fn set_enabled(app: &AppHandle, enabled: bool) -> Result<bool, String> {
    app.state::<MenubarState>().save(enabled)?;
    if enabled {
        build_tray(app).map_err(|error| format!("could not create the menu bar icon: {error}"))?;
    } else {
        remove_tray(app);
        show_window(app);
    }
    Ok(enabled)
}

/// The window's close button with the setting on: hide, keep running.
pub fn hide_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        if let Err(error) = window.hide() {
            eprintln!("could not hide the OpenStreamHub window: {error}");
            return;
        }
        #[cfg(target_os = "macos")]
        drop(app.set_activation_policy(tauri::ActivationPolicy::Accessory));
    }
}

/// Brings the window back (and the Dock icon with it).
pub fn show_window(app: &AppHandle) {
    #[cfg(target_os = "macos")]
    drop(app.set_activation_policy(tauri::ActivationPolicy::Regular));
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// Quits through the frontend's unsaved-draft guard. With the setting on the
/// window may be hidden, so it is shown first; the frontend closes it when
/// the guard allows, which ends the app. Without the setting (or without a
/// window) a plain window close does the same.
pub fn request_quit(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        app.exit(0);
        return;
    };
    if app.state::<MenubarState>().enabled() {
        show_window(app);
        if let Err(error) = app.emit(QUIT_EVENT, ()) {
            eprintln!("could not ask OpenStreamHub to quit: {error}");
        }
    } else if let Err(error) = window.close() {
        eprintln!("could not request closing the OpenStreamHub window: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_or_damaged_file_means_on_and_a_stored_choice_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        assert!(MenubarState::open(Some(dir.path())).enabled());
        std::fs::write(dir.path().join(FILE_NAME), "not json").unwrap();
        assert!(MenubarState::open(Some(dir.path())).enabled());
        let state = MenubarState::open(Some(dir.path()));
        state.save(false).unwrap();
        assert!(!state.enabled());
        assert!(!MenubarState::open(Some(dir.path())).enabled());
        state.save(true).unwrap();
        assert!(MenubarState::open(Some(dir.path())).enabled());
    }

    #[test]
    fn the_tray_icon_is_a_ring_around_a_pupil_with_a_highlight() {
        let image = icon();
        let alpha = |x: u32, y: u32| image.rgba()[((y * 44 + x) * 4 + 3) as usize];
        assert_eq!(alpha(3, 22), 255, "ring");
        assert_eq!(alpha(11, 22), 0, "gap between ring and pupil");
        assert_eq!(alpha(17, 26), 255, "pupil");
        assert_eq!(alpha(21, 21), 0, "highlight");
        assert_eq!(alpha(0, 0), 0, "corner");
    }
}
