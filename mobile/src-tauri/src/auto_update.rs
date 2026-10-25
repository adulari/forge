//! Installs a newer signed release at launch without waiting for a click.
//!
//! The Install button lives in the web UI. When a release ships a UI that cannot be clicked
//! (v2.23.0 did), the button can never be pressed and the app cannot repair itself. Running the
//! update from the native side means the next fixed release always reaches the user.

use tauri::AppHandle;
use tauri_plugin_updater::UpdaterExt;

/// Whether this install can replace itself with a downloaded update.
///
/// On Linux the updater plugin writes the new AppImage over `$APPIMAGE`, or over the running
/// executable when that is unset, because the binary itself was built for the AppImage bundle.
/// An AppImage extracted to a directory and started through its `AppRun` has `APPDIR` but no
/// `APPIMAGE`, so v2.23.3 landed on top of its `usr/bin/forge-desktop`. Only a process running
/// from a mounted AppImage file may install; .deb, rpm, AUR and unpacked installs get a link to
/// the release page instead.
pub fn self_install_supported() -> bool {
    #[cfg(target_os = "linux")]
    {
        linux::runs_from_mounted_appimage(
            std::env::var_os("APPIMAGE"),
            std::env::var_os("APPDIR"),
            std::env::current_exe().ok(),
        )
    }
    #[cfg(not(target_os = "linux"))]
    {
        true
    }
}

#[tauri::command]
pub fn desktop_update_installable() -> bool {
    self_install_supported()
}

#[cfg(target_os = "linux")]
mod linux {
    use std::ffi::OsString;
    use std::os::unix::fs::MetadataExt;
    use std::path::{Path, PathBuf};

    pub fn runs_from_mounted_appimage(
        appimage: Option<OsString>,
        appdir: Option<OsString>,
        exe: Option<PathBuf>,
    ) -> bool {
        let (Some(appimage), Some(appdir), Some(exe)) = (appimage, appdir, exe) else {
            return false;
        };
        if !Path::new(&appimage).is_file() {
            return false;
        }
        let Ok(appdir) = Path::new(&appdir).canonicalize() else {
            return false;
        };
        exe.starts_with(&appdir) && is_mount_point(&appdir)
    }

    fn is_mount_point(dir: &Path) -> bool {
        let Some(parent) = dir.parent() else {
            return false;
        };
        match (dir.metadata(), parent.metadata()) {
            (Ok(dir), Ok(parent)) => dir.dev() != parent.dev(),
            _ => false,
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::fs;

        fn scratch(name: &str) -> PathBuf {
            let dir = std::env::temp_dir()
                .join(format!("forge-update-gate-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(dir.join("usr/bin")).unwrap();
            fs::write(dir.join("usr/bin/forge-desktop"), b"").unwrap();
            dir
        }

        fn appimage_file(dir: &Path) -> PathBuf {
            let file = dir.join("Forge.AppImage");
            fs::write(&file, b"").unwrap();
            file
        }

        #[test]
        fn unpacked_install_without_appimage_cannot_install() {
            let dir = scratch("unpacked");
            let exe = dir.join("usr/bin/forge-desktop");
            assert!(!runs_from_mounted_appimage(
                None,
                Some(dir.clone().into()),
                Some(exe)
            ));
            fs::remove_dir_all(dir).unwrap();
        }

        #[test]
        fn unpacked_install_with_inherited_appimage_cannot_install() {
            let dir = scratch("inherited");
            let file = appimage_file(&dir);
            let exe = dir.join("usr/bin/forge-desktop");
            assert!(!runs_from_mounted_appimage(
                Some(file.into()),
                Some(dir.clone().into()),
                Some(exe)
            ));
            fs::remove_dir_all(dir).unwrap();
        }

        #[test]
        fn system_package_cannot_install() {
            let exe = PathBuf::from("/usr/bin/forge-desktop");
            assert!(!runs_from_mounted_appimage(None, None, Some(exe)));
        }

        #[test]
        fn missing_appimage_file_cannot_install() {
            let exe = PathBuf::from("/proc/self/exe");
            assert!(!runs_from_mounted_appimage(
                Some("/nonexistent/Forge.AppImage".into()),
                Some("/proc".into()),
                Some(exe)
            ));
        }

        #[test]
        fn executable_outside_the_mount_cannot_install() {
            let dir = scratch("outside");
            let file = appimage_file(&dir);
            assert!(!runs_from_mounted_appimage(
                Some(file.into()),
                Some("/proc".into()),
                Some(PathBuf::from("/usr/bin/forge-desktop"))
            ));
            fs::remove_dir_all(dir).unwrap();
        }

        #[test]
        fn mounted_appimage_can_install() {
            // `/proc` stands in for the AppImage's FUSE mount: a real mount point holding the exe.
            let dir = scratch("mounted");
            let file = appimage_file(&dir);
            assert!(runs_from_mounted_appimage(
                Some(file.into()),
                Some("/proc".into()),
                Some(PathBuf::from("/proc/1/exe"))
            ));
            fs::remove_dir_all(dir).unwrap();
        }
    }
}

/// Debug builds and `FORGE_DESKTOP_AUTO_UPDATE=0` opt out, and so does every install that cannot
/// replace itself (see [`self_install_supported`]).
fn enabled() -> bool {
    if cfg!(debug_assertions) {
        return false;
    }
    if std::env::var("FORGE_DESKTOP_AUTO_UPDATE").ok().as_deref() == Some("0") {
        return false;
    }
    self_install_supported()
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
