//! Installs a newer signed release without waiting for a click.
//!
//! The Install button lives in the web UI. When a release ships a UI that cannot be clicked
//! (v2.23.0 did), the button can never be pressed and the app cannot repair itself. Running the
//! update from the native side means the next fixed release always reaches the user.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_updater::UpdaterExt;

/// Event the web UI listens for once an update is on disk and only a restart is left.
pub const INSTALLED_EVENT: &str = "forge://desktop-update-installed";

/// The version installed in the background, kept so a UI that mounts after the event still sees it.
static INSTALLED_VERSION: Mutex<Option<String>> = Mutex::new(None);

#[tauri::command]
pub fn desktop_update_installed_version() -> Option<String> {
    INSTALLED_VERSION.lock().ok().and_then(|v| v.clone())
}

/// Whether this install can replace itself with a downloaded update.
///
/// On Linux the updater plugin writes the new AppImage over `$APPIMAGE`, or over the running
/// executable when that is unset, because the binary itself was built for the AppImage bundle.
/// An AppImage extracted to a directory and started through its `AppRun` has `APPDIR` but no
/// `APPIMAGE`, so v2.23.3 landed on top of its `usr/bin/forge-desktop`. Only a process running
/// from a mounted AppImage file may install; .deb, rpm, AUR and unpacked installs get a link to
/// the release page instead.
pub fn self_install_supported() -> bool {
    #[cfg(feature = "test-self-install")]
    if std::env::var("FORGE_TEST_SELF_INSTALL").ok().as_deref() == Some("1") {
        return true;
    }
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

/// How long the launch check may take before Forge stops waiting and opens the app.
const LAUNCH_CHECK_TIMEOUT: Duration = Duration::from_secs(4);
/// The whole launch update (check + download + install); past it the app opens as it is.
const LAUNCH_BUDGET: Duration = Duration::from_secs(20);
/// How often a running app looks for a newer release.
const BACKGROUND_INTERVAL: Duration = Duration::from_secs(6 * 3600);
/// Set across the relaunch after an update so a swap that did not take can never loop.
const JUST_UPDATED_ENV: &str = "FORGE_DESKTOP_JUST_UPDATED";

/// What the environment and the install allow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct Policy {
    /// Any lookup at all. `FORGE_NO_UPDATE_CHECK` turns everything off.
    pub check: bool,
    /// Installing without asking: only a self-installable build, not opted out.
    pub auto: bool,
}

/// The raw environment switches, so the decision is testable without touching the process env.
#[derive(Debug, Default, Clone)]
pub struct EnvSwitches {
    pub no_update_check: bool,
    pub no_auto_update: Option<String>,
    pub desktop_auto_update: Option<String>,
}

impl EnvSwitches {
    fn read() -> Self {
        Self {
            no_update_check: std::env::var_os("FORGE_NO_UPDATE_CHECK").is_some(),
            no_auto_update: std::env::var("FORGE_NO_AUTO_UPDATE").ok(),
            desktop_auto_update: std::env::var("FORGE_DESKTOP_AUTO_UPDATE").ok(),
        }
    }
}

/// `debug_build` is true for development builds, which never install on their own.
pub fn policy(env: &EnvSwitches, self_install: bool, debug_build: bool) -> Policy {
    if env.no_update_check {
        return Policy {
            check: false,
            auto: false,
        };
    }
    let opted_out = env.no_auto_update.as_deref() == Some("1")
        || env.desktop_auto_update.as_deref() == Some("0");
    Policy {
        check: true,
        auto: self_install && !debug_build && !opted_out,
    }
}

fn current_policy() -> Policy {
    let debug_build = cfg!(debug_assertions) && !cfg!(feature = "test-self-install");
    policy(&EnvSwitches::read(), self_install_supported(), debug_build)
}

#[tauri::command]
pub fn desktop_update_policy() -> Policy {
    current_policy()
}

/// Whether launch should hold the main window for an update check: auto-install is on and this
/// process is not the relaunch that follows an update.
pub fn launch_gate(policy: Policy, just_updated: bool) -> bool {
    policy.check && policy.auto && !just_updated
}

/// Time left of `budget` after `elapsed`; zero once spent.
pub fn budget_left(budget: Duration, elapsed: Duration) -> Duration {
    budget.saturating_sub(elapsed)
}

/// Whether a background find should be installed: it is not the version already on disk.
pub fn should_install(found: &str, installed: Option<&str>) -> bool {
    installed != Some(found)
}

fn updater(app: &AppHandle) -> tauri_plugin_updater::Result<tauri_plugin_updater::Updater> {
    #[allow(unused_mut)]
    let mut builder = app.updater_builder();
    #[cfg(feature = "test-self-install")]
    {
        if let Ok(endpoint) = std::env::var("FORGE_TEST_UPDATE_ENDPOINT") {
            if let Ok(url) = endpoint.parse() {
                builder = builder.endpoints(vec![url])?;
            }
        }
        if let Ok(key) = std::env::var("FORGE_TEST_UPDATE_PUBKEY") {
            builder = builder.pubkey(key);
        }
    }
    builder.build()
}

/// Called once from setup. Holds the main window for the launch update when that applies, then
/// keeps the background loop going.
pub fn start(app: AppHandle) {
    let policy = current_policy();
    let just_updated = std::env::var_os(JUST_UPDATED_ENV).is_some();
    if launch_gate(policy, just_updated) {
        if let Some(main) = app.get_webview_window("main") {
            let _ = main.hide();
        }
        tauri::async_runtime::spawn(async move {
            if let Err(error) = launch_update(&app).await {
                eprintln!("forge: launch update skipped: {error}");
            }
            // Every path ends here: the app always opens.
            if let Some(main) = app.get_webview_window("main") {
                let _ = main.show();
            }
            background_loop(app, policy).await;
        });
    } else if policy.check && policy.auto {
        tauri::async_runtime::spawn(background_loop(app, policy));
    }
}

async fn launch_update(app: &AppHandle) -> Result<(), String> {
    let started = Instant::now();
    let updater = updater(app).map_err(|e| e.to_string())?;
    let found = tokio::time::timeout(LAUNCH_CHECK_TIMEOUT, updater.check())
        .await
        .map_err(|_| "update check timed out".to_string())?
        .map_err(|e| e.to_string())?;
    let Some(update) = found else {
        return Ok(());
    };
    eprintln!("forge: updating to {} before launch", update.version);
    let splash = open_splash(app, &update.version);
    let left = budget_left(LAUNCH_BUDGET, started.elapsed());
    let result = tokio::time::timeout(left, update.download_and_install(|_, _| {}, || {})).await;
    if let Some(splash) = splash {
        let _ = splash.close();
    }
    match result {
        Ok(Ok(())) => {
            // Windows exits inside the install and its installer relaunches Forge.
            std::env::set_var(JUST_UPDATED_ENV, "1");
            app.restart()
        }
        Ok(Err(error)) => Err(error.to_string()),
        Err(_) => Err("update timed out".to_string()),
    }
}

/// The splash page shipped with the web bundle (`public/update-splash.html`), with the version
/// reduced to characters that cannot break out of the query string.
fn splash_path(version: &str) -> String {
    let version: String = version
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'))
        .collect();
    format!("update-splash.html?v={version}")
}

fn open_splash(app: &AppHandle, version: &str) -> Option<tauri::WebviewWindow> {
    WebviewWindowBuilder::new(
        app,
        "update-splash",
        WebviewUrl::App(splash_path(version).into()),
    )
    .title("Updating Forge")
    .inner_size(380.0, 150.0)
    .resizable(false)
    .decorations(false)
    .center()
    .build()
    .map_err(|error| eprintln!("forge: update splash unavailable: {error}"))
    .ok()
}

/// Every six hours: look for a release and, when there is one that is not already on disk,
/// install it for the next launch. The restart banner is a courtesy; nothing waits on it.
async fn background_loop(app: AppHandle, policy: Policy) {
    let mut installed: Option<String> = None;
    loop {
        tokio::time::sleep(BACKGROUND_INTERVAL).await;
        if !policy.auto {
            continue;
        }
        let Ok(updater) = updater(&app) else { continue };
        let Ok(Some(update)) = updater.check().await else {
            continue;
        };
        if !should_install(&update.version, installed.as_deref()) {
            continue;
        }
        eprintln!(
            "forge: installing update {} in the background",
            update.version
        );
        if update.download_and_install(|_, _| {}, || {}).await.is_ok() {
            installed = Some(update.version.clone());
            if let Ok(mut v) = INSTALLED_VERSION.lock() {
                *v = Some(update.version.clone());
            }
            let _ = app.emit(INSTALLED_EVENT, update.version.clone());
        }
    }
}

#[cfg(test)]
mod policy_tests {
    use super::*;

    fn env() -> EnvSwitches {
        EnvSwitches::default()
    }

    #[test]
    fn a_self_installable_release_build_updates_on_its_own() {
        let p = policy(&env(), true, false);
        assert_eq!(
            p,
            Policy {
                check: true,
                auto: true
            }
        );
        assert!(launch_gate(p, false));
    }

    #[test]
    fn installs_that_cannot_replace_themselves_and_dev_builds_never_auto_install() {
        assert!(!policy(&env(), false, false).auto, "deb/rpm/AUR");
        assert!(!policy(&env(), true, true).auto, "debug build");
        assert!(
            policy(&env(), false, false).check,
            "the banner can still look"
        );
    }

    #[test]
    fn both_opt_outs_turn_auto_install_off_but_keep_the_banner() {
        let a = EnvSwitches {
            no_auto_update: Some("1".into()),
            ..env()
        };
        let b = EnvSwitches {
            desktop_auto_update: Some("0".into()),
            ..env()
        };
        for e in [a, b] {
            let p = policy(&e, true, false);
            assert_eq!(
                p,
                Policy {
                    check: true,
                    auto: false
                }
            );
            assert!(!launch_gate(p, false));
        }
        let unrelated = EnvSwitches {
            desktop_auto_update: Some("1".into()),
            ..env()
        };
        assert!(policy(&unrelated, true, false).auto);
    }

    #[test]
    fn no_update_check_disables_everything() {
        let e = EnvSwitches {
            no_update_check: true,
            ..env()
        };
        assert_eq!(
            policy(&e, true, false),
            Policy {
                check: false,
                auto: false
            }
        );
    }

    #[test]
    fn the_relaunch_after_an_update_never_updates_again() {
        let p = policy(&env(), true, false);
        assert!(!launch_gate(p, true));
    }

    #[test]
    fn the_launch_budget_only_shrinks() {
        let b = Duration::from_secs(20);
        assert_eq!(
            budget_left(b, Duration::from_secs(5)),
            Duration::from_secs(15)
        );
        assert_eq!(budget_left(b, Duration::from_secs(25)), Duration::ZERO);
    }

    #[test]
    fn a_version_already_on_disk_is_not_installed_twice() {
        assert!(should_install("2.0.0", None));
        assert!(should_install("2.1.0", Some("2.0.0")));
        assert!(!should_install("2.0.0", Some("2.0.0")));
    }

    #[test]
    fn the_splash_path_carries_only_a_sanitised_version() {
        assert_eq!(splash_path("2.0.0"), "update-splash.html?v=2.0.0");
        assert_eq!(
            splash_path("2.0.0&x=<script>"),
            "update-splash.html?v=2.0.0xscript"
        );
    }
}
