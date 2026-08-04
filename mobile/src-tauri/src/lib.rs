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
use serde::Serialize;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;
use tauri::{RunEvent, WindowEvent};
use tauri::webview::{PageLoadEvent, PageLoadPayload};

mod about;
mod menu;
mod preview;
mod serve_discovery;
mod tray;

#[derive(Clone, Copy, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct PerfNativeTimeline {
    process_start_ms: Option<f64>,
    builder_start_ms: Option<f64>,
    window_created_ms: Option<f64>,
    navigation_start_ms: Option<f64>,
    dom_content_loaded_ms: Option<f64>,
    plugins_registered_ms: Option<f64>,
    plugin_notification_ms: Option<f64>,
    plugin_opener_ms: Option<f64>,
    plugin_http_ms: Option<f64>,
    plugin_dialog_ms: Option<f64>,
    plugin_websocket_ms: Option<f64>,
    plugin_updater_ms: Option<f64>,
    plugin_process_ms: Option<f64>,
    build_finished_ms: Option<f64>,
}

static PERF_START: OnceLock<Instant> = OnceLock::new();
static PERF_TIMELINE: OnceLock<Mutex<PerfNativeTimeline>> = OnceLock::new();

fn perf_timeline() -> &'static Mutex<PerfNativeTimeline> {
    PERF_TIMELINE.get_or_init(|| Mutex::new(PerfNativeTimeline::default()))
}

fn perf_mark<F>(update: F)
where
    F: FnOnce(&mut PerfNativeTimeline),
{
    if std::env::var_os("FORGE_PERF_OUT").is_none() {
        return;
    }
    if let Ok(mut timeline) = perf_timeline().lock() {
        update(&mut timeline);
    }
}

fn perf_elapsed_ms() -> f64 {
    PERF_START.get().map_or(0.0, |start| start.elapsed().as_secs_f64() * 1000.0)
}


#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PerfSeedServer {
    id: String,
    name: String,
    base_url: String,
    token: String,
    host: String,
    added_at: u64,
}

#[tauri::command]
fn perf_seed_server() -> Result<Option<PerfSeedServer>, String> {
    if std::env::var("FORGE_PERF_SEED_SERVER").ok().as_deref() != Some("1") {
        return Ok(None);
    }
    let path = directories::ProjectDirs::from("dev", "forge", "forge")
        .ok_or_else(|| "config directory unavailable".to_string())?
        .config_dir()
        .join("serve-state.json");
    let raw = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
    let state: serde_json::Value = serde_json::from_str(&raw).map_err(|error| error.to_string())?;
    let base_url = state.get("base_url").and_then(serde_json::Value::as_str).ok_or_else(|| "serve state missing base_url".to_string())?;
    let token = state.get("token").and_then(serde_json::Value::as_str).ok_or_else(|| "serve state missing token".to_string())?;
    let port = state.get("port").and_then(serde_json::Value::as_u64).unwrap_or(7420);
    Ok(Some(PerfSeedServer {
        id: format!("perf-daemon-{port}"),
        name: "Local daemon".to_string(),
        base_url: base_url.to_string(),
        token: token.to_string(),
        host: format!("127.0.0.1:{port}"),
        added_at: 0,
    }))
}
#[tauri::command]
fn perf_native_now() -> f64 { perf_elapsed_ms() }

#[tauri::command]
fn perf_native_timeline() -> PerfNativeTimeline {
    perf_timeline().lock().map(|timeline| *timeline).unwrap_or_default()
}

#[tauri::command]
fn perf_dump(snapshot: String) -> Result<(), String> {
    let Some(path) = std::env::var_os("FORGE_PERF_OUT") else {
        return Ok(());
    };
    std::fs::write(path, snapshot).map_err(|error| error.to_string())
}
#[tauri::command]
fn perf_phase() -> String {
    std::env::var("FORGE_PERF_PHASE").unwrap_or_else(|_| "idle".to_string())
}

#[tauri::command]
fn perf_seed_update_seen() -> bool {
    std::env::var("FORGE_PERF_SEED_UPDATE_SEEN").ok().as_deref() == Some("1")
}

#[tauri::command]
fn perf_enabled() -> bool {
    std::env::var_os("FORGE_PERF_OUT").is_some()
}

#[tauri::command]
fn perf_fixture_enabled() -> bool {
    std::env::var("FORGE_PERF_FIXTURE").ok().as_deref() == Some("1")
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run(process_start: Instant) {
    let _ = PERF_START.set(process_start);
    perf_mark(|timeline| timeline.process_start_ms = Some(0.0));
    perf_mark(|timeline| timeline.builder_start_ms = Some(perf_elapsed_ms()));
    let mut builder = tauri::Builder::default();
    builder = builder.plugin(tauri_plugin_notification::init());
    perf_mark(|timeline| timeline.plugin_notification_ms = Some(perf_elapsed_ms()));
    builder = builder.plugin(tauri_plugin_opener::init());
    perf_mark(|timeline| timeline.plugin_opener_ms = Some(perf_elapsed_ms()));
    builder = builder.plugin(tauri_plugin_http::init());
    perf_mark(|timeline| timeline.plugin_http_ms = Some(perf_elapsed_ms()));
    builder = builder.plugin(tauri_plugin_dialog::init());
    perf_mark(|timeline| timeline.plugin_dialog_ms = Some(perf_elapsed_ms()));
    builder = builder.plugin(tauri_plugin_websocket::init());
    perf_mark(|timeline| timeline.plugin_websocket_ms = Some(perf_elapsed_ms()));
    builder = builder.plugin(tauri_plugin_updater::Builder::new().build());
    perf_mark(|timeline| timeline.plugin_updater_ms = Some(perf_elapsed_ms()));
    builder = builder.plugin(tauri_plugin_process::init());
    perf_mark(|timeline| timeline.plugin_process_ms = Some(perf_elapsed_ms()));
    builder = builder.manage(about::AboutState::default());
    perf_mark(|timeline| timeline.plugins_registered_ms = Some(perf_elapsed_ms()));
    builder
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
            menu::set_menu_accelerators,
            about::set_about_info,
            about::open_about_window,
            perf_native_now,
            perf_native_timeline,
            perf_seed_server,
            perf_dump,
            perf_phase,
            perf_enabled,
            perf_fixture_enabled,
            perf_seed_update_seen,
        ])
        .on_page_load(|_webview, payload: &PageLoadPayload<'_>| {
            perf_mark(|timeline| {
                match payload.event() {
                    PageLoadEvent::Started => timeline.navigation_start_ms = Some(perf_elapsed_ms()),
                    PageLoadEvent::Finished => timeline.dom_content_loaded_ms = Some(perf_elapsed_ms()),
                }
            });
        })
        .setup(|app| {
            #[cfg(not(target_os = "macos"))]
            {
                let main_window = app
                    .get_webview_window("main")
                    .ok_or_else(|| std::io::Error::other("main window is missing"))?;
                perf_mark(|timeline| timeline.window_created_ms = Some(perf_elapsed_ms()));
                main_window.set_decorations(false)?;

                #[cfg(target_os = "linux")]
                {
                    if let Some(size) = std::env::var("FORGE_CAPTURE_SIZE").ok().and_then(|value| {
                        let (width, height) = value.split_once('x')?;
                        Some((width.parse::<f64>().ok()?, height.parse::<f64>().ok()?))
                    }) {
                        main_window.set_size(tauri::Size::Logical(tauri::LogicalSize::new(
                            size.0, size.1,
                        )))?;
                    }
                }

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
        .map(|app| {
            perf_mark(|timeline| timeline.build_finished_ms = Some(perf_elapsed_ms()));
            app
        })
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
