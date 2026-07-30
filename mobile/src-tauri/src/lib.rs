// Forge desktop shell (ARCHITECTURE.md §6.1). The webview owns the integrated title bar:
// macOS keeps its native menu for standard editing bindings, while Windows/Linux hide native
// decorations and use the React Native Web chrome.
//
// Native surfaces (docs/design/machined INVENTORY.md § 08): `menu.rs` (application menu bar,
// macOS only), `tray.rs` (menu-bar extra, all desktops), `about.rs` (About panel). All three
// are thin — they hold no daemon state and defer every real action to the webview.
//
// `Manager::get_webview_window` is used in the non-macOS setup hook; `menu.rs` and `about.rs`
// bring their own imports.
#[cfg(not(target_os = "macos"))]
use tauri::Manager;
use tauri::{RunEvent, WindowEvent};

mod about;
mod menu;
mod preview;
mod serve_discovery;
mod tray;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_http::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_websocket::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .manage(about::AboutState::default())
        .register_uri_scheme_protocol(about::URI_SCHEME, about::serve)
        .invoke_handler(tauri::generate_handler![
            serve_discovery::detect_forge_serve,
            serve_discovery::forge_binary_available,
            serve_discovery::start_forge_serve,
            serve_discovery::system_host_name,
            serve_discovery::forge_anywhere_host_enrolled,
            serve_discovery::install_forge_anywhere_host,
            serve_discovery::activate_forge_anywhere_host,
            preview::preview_open,
            preview::preview_set_bounds,
            preview::preview_navigate,
            preview::preview_history,
            preview::preview_reload,
            preview::preview_set_zoom,
            preview::preview_set_picker,
            preview::preview_hide,
            preview::preview_close,
            tray::set_tray_summary,
            about::set_about_info,
            about::open_about_window,
        ])
        .setup(|app| {
            #[cfg(not(target_os = "macos"))]
            {
                let main_window = app
                    .get_webview_window("main")
                    .ok_or_else(|| std::io::Error::other("main window is missing"))?;
                main_window.set_decorations(false)?;

                #[cfg(target_os = "linux")]
                install_linux_microphone_permission(&main_window)?;
            }

            // macOS-only: Windows/Linux run with `set_decorations(false)` and draw the Machined
            // chrome in the webview, so attaching a native menu bar there would stack a second,
            // un-themed strip above it. Those platforms reach the same actions through the
            // in-webview hotkey registry (src/lib/shortcuts/) and the tray.
            #[cfg(target_os = "macos")]
            menu::install(app.handle())?;

            // A tray failure (no StatusNotifier host on a bare Linux session, say) must not take
            // the whole app down with it — the window is the primary surface.
            if let Err(error) = tray::install(app.handle()) {
                eprintln!("forge: tray unavailable: {error}");
            }

            Ok(())
        })
        .on_menu_event(|app, event| menu::handle_event(app, event.id().as_ref()))
        // Closing the window parks Forge in the menu bar instead of ending the process. The
        // tray's whole reason to exist (glance at a waiting decision while the app is in the
        // background, per the D Tray design) needs the app to outlive its window, and ⌘W —
        // bound twice in `menu.rs`, on File and on Window — means "close this window" to every
        // macOS user, never "quit". Only "main" is intercepted: the About panel is disposable
        // and `about::open` rebuilds it on demand.
        //
        // Gated on the tray having actually come up. Without it (a bare Linux session with no
        // StatusNotifier host) hiding the sole window would leave Forge running with nothing
        // left to click, which is strictly worse than the close-quits behaviour it replaces.
        .on_window_event(|window, event| {
            if window.label() != "main" || !tray::is_installed() {
                return;
            }
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app, event| match event {
            // Tauri ends the event loop the moment the last window is destroyed, tray icon or
            // not, so hiding the window is only half the job. `code: None` is precisely that
            // path — tauri-runtime-wry only sets `Some` for `AppHandle::exit`, which is how
            // both Quit items (tray and the macOS Forge menu) leave, so this never makes the
            // app unquittable.
            RunEvent::ExitRequested {
                code: None, api, ..
            } if tray::is_installed() => {
                api.prevent_exit();
            }
            // With no visible window macOS has no Dock preview to click through; the Dock icon
            // reopen request is the OS's only "give me the window back" gesture.
            #[cfg(target_os = "macos")]
            RunEvent::Reopen {
                has_visible_windows: false,
                ..
            } => menu::raise_main_window(_app),
            _ => {}
        });
}

/// WebKitGTK denies `getUserMedia()` unless the embedder handles its permission signal. Forge's
/// only user-media request is the composer's microphone recorder, so grant audio-only requests
/// from the bundled application webview and leave camera or unrelated permissions to WebKit's
/// default denial path.
#[cfg(target_os = "linux")]
fn install_linux_microphone_permission(window: &tauri::WebviewWindow) -> tauri::Result<()> {
    window.with_webview(|webview| {
        use webkit2gtk::{
            glib::prelude::Cast, PermissionRequestExt, UserMediaPermissionRequest,
            UserMediaPermissionRequestExt, WebViewExt,
        };

        webview.inner().connect_permission_request(|_, request| {
            let Some(user_media) = request.downcast_ref::<UserMediaPermissionRequest>() else {
                return false;
            };
            if !user_media.is_for_audio_device() || user_media.is_for_video_device() {
                return false;
            }
            request.allow();
            true
        });
    })?;
    Ok(())
}
