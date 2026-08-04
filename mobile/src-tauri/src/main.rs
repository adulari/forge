// Prevents an additional console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let process_start = std::time::Instant::now();
    #[cfg(target_os = "linux")]
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        // WebKitGTK can fail before Tauri/GTK initialization on Wayland wlroots sessions.
        // Set the compatibility renderer before the toolkit is initialized; callers can still
        // override it explicitly when testing a different renderer.
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }
    forge_desktop_lib::run(process_start);
}
