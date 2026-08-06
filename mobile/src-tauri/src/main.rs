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
    #[cfg(target_os = "linux")]
    restore_wayland_inside_appimage();
    forge_desktop_lib::run(process_start);
}

/// Undo the AppImage's blanket `GDK_BACKEND=x11`, which silently costs every packaged user
/// native Wayland.
///
/// `linuxdeploy-plugin-gtk` writes `export GDK_BACKEND=x11` into the generated AppRun hook,
/// commented "Crash with Wayland backend on Wayland" and citing tauri-apps/tauri#8541. That crash
/// is the same WebKitGTK DMABUF failure the block above already prevents, so the packaged app was
/// falling back to XWayland for a reason that no longer applies to us: the locally built binary
/// reports `xwayland=false` while the AppImage built from it reports `xwayland=true` — a different
/// rendering and input stack in the artifact users actually install.
///
/// Deliberately narrow. It fires only when all three hold: we are running from an AppImage
/// (`APPDIR` is set by AppRun), the session really is Wayland, and the backend is exactly the
/// `x11` the hook forces. `FORGE_KEEP_APPIMAGE_X11=1` opts out for anyone who needs the old
/// behaviour on a compositor where Wayland still misbehaves.
#[cfg(target_os = "linux")]
fn restore_wayland_inside_appimage() {
    let inside_appimage = std::env::var_os("APPDIR").is_some();
    let wayland_session = std::env::var_os("WAYLAND_DISPLAY").is_some();
    let forced_x11 = std::env::var("GDK_BACKEND").is_ok_and(|backend| backend == "x11");
    let opted_out = std::env::var_os("FORGE_KEEP_APPIMAGE_X11").is_some();
    if inside_appimage && wayland_session && forced_x11 && !opted_out {
        std::env::set_var("GDK_BACKEND", "wayland");
    }
}
