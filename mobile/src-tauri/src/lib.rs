// Forge desktop shell (ARCHITECTURE.md §6.1). The webview owns the integrated title bar:
// macOS keeps its native menu for standard editing bindings, while Windows/Linux hide native
// decorations and use the React Native Web chrome.
#[cfg(all(debug_assertions, target_os = "macos"))]
use tauri::menu::MenuItemBuilder;
#[cfg(target_os = "macos")]
use tauri::menu::{Menu, PredefinedMenuItem, SubmenuBuilder};
#[cfg(debug_assertions)]
use tauri::Manager;

mod serve_discovery;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_http::init())
        .plugin(tauri_plugin_websocket::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .invoke_handler(tauri::generate_handler![
            serve_discovery::detect_forge_serve,
            serve_discovery::forge_binary_available,
            serve_discovery::start_forge_serve,
        ])
        .setup(|app| {
            #[cfg(not(target_os = "macos"))]
            app.get_webview_window("main")
                .ok_or_else(|| std::io::Error::other("main window is missing"))?
                .set_decorations(false)?;

            #[cfg(target_os = "macos")]
            install_macos_menu(app)?;

            Ok(())
        })
        .on_menu_event(|app, event| {
            #[cfg(debug_assertions)]
            if event.id() == "reload" {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.eval("window.location.reload()");
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(target_os = "macos")]
fn install_macos_menu(app: &mut tauri::App) -> tauri::Result<()> {
    let handle = app.handle();
    let about = PredefinedMenuItem::about(handle, Some("About Forge"), None)?;
    let quit = PredefinedMenuItem::quit(handle, Some("Quit Forge"))?;
    let app_menu_builder = SubmenuBuilder::new(handle, "Forge")
        .item(&about)
        .separator();

    #[cfg(debug_assertions)]
    let app_menu_builder = {
        let reload = MenuItemBuilder::with_id("reload", "Reload")
            .accelerator("CmdOrCtrl+R")
            .build(handle)?;
        app_menu_builder.item(&reload).separator()
    };

    let app_menu = app_menu_builder.item(&quit).build()?;
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
    app.set_menu(menu)
}
