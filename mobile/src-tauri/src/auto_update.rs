//! Installs a newer signed release at launch without waiting for a click.
//!
//! The Install button lives in the web UI. When a release ships a UI that cannot be clicked
//! (v2.23.0 did), the button can never be pressed and the app cannot repair itself. Running the
//! update from the native side means the next fixed release always reaches the user.

use tauri::AppHandle;
use tauri_plugin_updater::UpdaterExt;

/// Debug builds and `FORGE_DESKTOP_AUTO_UPDATE=0` opt out. On Linux only an AppImage can replace
/// itself; a .deb install would need a root password prompt, so those keep the manual button.
fn enabled() -> bool {
    if cfg!(debug_assertions) {
        return false;
    }
    if std::env::var("FORGE_DESKTOP_AUTO_UPDATE").ok().as_deref() == Some("0") {
        return false;
    }
    if cfg!(target_os = "linux") {
        return std::env::var_os("APPIMAGE").is_some();
    }
    true
}

pub fn spawn(app: AppHandle) {
    if !enabled() {
        return;
    }
    tauri::async_runtime::spawn(async move {
        if let Err(error) = run(&app).await {
            eprintln!("forge: automatic update failed: {error}");
        }
    });
}

async fn run(app: &AppHandle) -> tauri_plugin_updater::Result<()> {
    let Some(update) = app.updater()?.check().await? else {
        return Ok(());
    };
    eprintln!("forge: installing update {}", update.version);
    // Windows exits inside this call and the installer relaunches Forge.
    update.download_and_install(|_, _| {}, || {}).await?;
    app.restart();
}
