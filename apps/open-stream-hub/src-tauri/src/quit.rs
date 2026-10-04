use tauri::{
    AppHandle, Manager,
    menu::{Menu, MenuEvent, MenuItem, MenuItemKind},
};

const QUIT_ID: &str = "open-stream-hub-quit";

pub fn menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let menu = Menu::default(app)?;
    let items = menu.items()?;
    let app_menu = items
        .first()
        .and_then(MenuItemKind::as_submenu)
        .ok_or_else(|| invalid_menu("the native app menu is missing"))?;
    let app_items = app_menu.items()?;
    // Tauri's macOS default menu ends its app submenu with predefined Quit.
    // Verify that structure before replacing it, keeping all the standard
    // editing, window, services, and app menu items intact.
    let Some(MenuItemKind::Predefined(original_quit)) = app_items.last() else {
        return Err(invalid_menu("the native Quit item is missing"));
    };
    let title = original_quit.text()?;
    if title != "Quit" && !title.starts_with("Quit ") {
        return Err(invalid_menu(
            "the native app menu has an unexpected final item",
        ));
    }
    app_menu.remove(original_quit)?;
    app_menu.append(&MenuItem::with_id(
        app,
        QUIT_ID,
        "Quit OpenStreamHub",
        true,
        Some("CmdOrCtrl+Q"),
    )?)?;
    Ok(menu)
}

pub fn on_menu_event(app: &AppHandle, event: MenuEvent) {
    if event.id().as_ref() != QUIT_ID {
        return;
    }
    if let Some(window) = app.get_webview_window("main") {
        // A predefined macOS Quit item calls Cocoa terminate: directly and
        // bypasses Tauri's ExitRequested event. This custom item instead
        // follows the same CloseRequested path as the window close button.
        if let Err(error) = window.close() {
            eprintln!("could not request closing the OpenStreamHub window: {error}");
        }
    } else {
        app.exit(0);
    }
}

fn invalid_menu(message: &str) -> tauri::Error {
    std::io::Error::other(message).into()
}
