// Forge desktop shell (ARCHITECTURE.md §6.1). Deliberately thin: window + app icons +
// native notifications + external-link opening + a basic app menu. NO custom Rust
// commands in v1 — all daemon communication happens in the webview via the JS
// transport seam (`mobile/src/lib/transport/index.ts`).
use tauri::menu::{Menu, MenuItemBuilder, PredefinedMenuItem, SubmenuBuilder};
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_http::init())
        .plugin(tauri_plugin_websocket::init())
        .setup(|app| {
            let handle = app.handle();

            let about = PredefinedMenuItem::about(handle, Some("About Forge"), None)?;
            let quit = PredefinedMenuItem::quit(handle, Some("Quit Forge"))?;
            let reload = MenuItemBuilder::with_id("reload", "Reload")
                .accelerator("CmdOrCtrl+R")
                .build(handle)?;

            let app_menu = SubmenuBuilder::new(handle, "Forge")
                .item(&about)
                .separator()
                .item(&reload)
                .separator()
                .item(&quit)
                .build()?;

            // Standard Edit menu — required on macOS for Cmd+C/V/X/A to work in the
            // webview at all (there is no default Edit menu without one).
            let edit_menu = SubmenuBuilder::new(handle, "Edit")
                .undo()
                .redo()
                .separator()
                .cut()
                .copy()
                .paste()
                .select_all()
                .build()?;

            let menu = Menu::with_items(handle, &[&app_menu, &edit_menu])?;
            app.set_menu(menu)?;

            Ok(())
        })
        .on_menu_event(|app, event| {
            if event.id() == "reload" {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.eval("window.location.reload()");
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
