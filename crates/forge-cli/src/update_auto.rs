//! Automatic updates: install a newer release without being asked, where that is safe.
//!
//! * A standalone install swaps itself on an interactive launch, before the TUI opens: a build
//!   staged by an earlier session is swapped in at once; otherwise a cached newer release is
//!   downloaded within a hard time budget. Any failure or timeout just launches the current
//!   version and tries again next time. With nothing known yet, the session itself stages the
//!   download in the background, so launch stays instant and the swap happens next launch.
//! * Homebrew and Scoop (no root needed) upgrade in the background during the session.
//! * AUR, cargo and other installs keep the update card: they need root or a long build.
//! * The `forge serve` daemon updates itself when no session is mid-turn (`serve_update`).
//!
//! Nothing here runs unless the launch is interactive (a terminal on stdin and stdout, not CI).

use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::update::InstallMethod;
use crate::update_check::is_newer;
use crate::update_feed::notes_preview;
use crate::update_state::{disabled, UpdateState};

const CURRENT: &str = env!("CARGO_PKG_VERSION");
const DEFAULT_BUDGET_SECS: u64 = 20;
const STAGE_DIR: &str = "staged-update";
const STAGED_FILE: &str = "staged.json";
const MARKER_FILE: &str = "just-updated.json";

/// Serialises staging inside one process: a launch download that outran its budget keeps running
/// while the session's own background staging starts, and the two must not share a directory.
static STAGING: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Automatic installs are on: `[update] auto` (default true), no `FORGE_NO_AUTO_UPDATE`, and
/// update checks are not disabled altogether.
pub(crate) fn auto_enabled(config: &forge_config::Config) -> bool {
    config.update.auto && std::env::var_os("FORGE_NO_AUTO_UPDATE").is_none() && !disabled(config)
}

/// A person is at a terminal: stdin and stdout are ttys, this is not CI, and this process was
/// not itself started by an automatic update (which would risk a restart loop).
pub(crate) fn interactive_launch() -> bool {
    at_a_terminal() && std::env::var_os("FORGE_AUTO_UPDATED").is_none()
}

/// A person is at a terminal: stdin and stdout are ttys and this is not CI.
pub(crate) fn at_a_terminal() -> bool {
    std::io::stdin().is_terminal()
        && std::io::stdout().is_terminal()
        && std::env::var_os("CI").is_none()
}

/// The hard limit for the download made before the TUI opens.
pub(crate) fn launch_budget() -> Duration {
    Duration::from_secs(
        std::env::var("FORGE_AUTO_UPDATE_BUDGET_SECS")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(DEFAULT_BUDGET_SECS),
    )
}

pub(crate) struct LaunchFacts {
    pub interactive: bool,
    pub auto: bool,
    pub method: InstallMethod,
    /// The cached release info names a version newer than this build.
    pub cached_newer: bool,
    /// A verified build newer than this one is already staged.
    pub staged_newer: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LaunchAction {
    /// Nothing automatic: any offer goes to the card.
    Nothing,
    /// Swap the staged build in now, then restart into it.
    SwapStaged,
    /// Download within the budget, swap, restart.
    UpdateNow,
    /// Stage in the background during the session; the swap is next launch.
    StageInBackground,
    /// Run the package manager's upgrade in the background during the session.
    UpgradeInBackground(String),
}

/// What an interactive launch does about updates. Pure.
pub(crate) fn plan_launch(f: &LaunchFacts) -> LaunchAction {
    if !f.interactive || !f.auto {
        return LaunchAction::Nothing;
    }
    match f.method {
        InstallMethod::Binary if f.staged_newer => LaunchAction::SwapStaged,
        InstallMethod::Binary if f.cached_newer => LaunchAction::UpdateNow,
        InstallMethod::Binary => LaunchAction::StageInBackground,
        InstallMethod::Homebrew | InstallMethod::Scoop => f
            .method
            .upgrade_command()
            .map_or(LaunchAction::Nothing, LaunchAction::UpgradeInBackground),
        InstallMethod::Aur | InstallMethod::Cargo => LaunchAction::Nothing,
    }
}

/// Run blocking work with a hard time limit. `None` means it did not finish in time; the work is
/// left running on its thread (it only stages, so finishing late is harmless).
pub(crate) async fn run_with_budget<T: Send + 'static>(
    budget: Duration,
    work: impl FnOnce() -> T + Send + 'static,
) -> Option<T> {
    tokio::time::timeout(budget, tokio::task::spawn_blocking(work))
        .await
        .ok()?
        .ok()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Staged {
    version: String,
    file: String,
}

fn stage_dir() -> Option<PathBuf> {
    forge_config::data_dir().map(|d| d.join(STAGE_DIR))
}

fn exe_suffix() -> &'static str {
    std::env::consts::EXE_SUFFIX
}

/// A verified staged build newer than `current`, with its path.
fn staged_newer(current: &str) -> Option<(String, PathBuf)> {
    let dir = stage_dir()?;
    let staged: Staged =
        serde_json::from_str(&std::fs::read_to_string(dir.join(STAGED_FILE)).ok()?).ok()?;
    let file = dir.join(&staged.file);
    (is_newer(&staged.version, current) && file.is_file()).then_some((staged.version, file))
}

/// Whether the build in `file` runs and reports `version`.
fn verify_binary(file: &Path, version: &str) -> bool {
    let Ok(mut child) = std::process::Command::new(file)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
    else {
        return false;
    };
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            _ => {
                let _ = child.kill();
                return false;
            }
        }
    }
    let mut out = String::new();
    if let Some(mut stdout) = child.stdout.take() {
        let _ = std::io::Read::read_to_string(&mut stdout, &mut out);
    }
    out.contains(version)
}

/// Download the newest release next to (not over) the running binary, verify that it runs, and
/// leave it staged for the next launch's swap. `Ok(None)`: already the latest.
pub(crate) fn stage_latest() -> Result<Option<String>> {
    let _one_at_a_time = STAGING.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((version, _)) = staged_newer(CURRENT) {
        return Ok(Some(version));
    }
    let dir = stage_dir().context("no data directory to stage into")?;
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    let partial = dir.join(format!("forge.partial{}", exe_suffix()));
    std::fs::write(&partial, b"")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&partial, std::fs::Permissions::from_mode(0o755))?;
    }
    let Some(version) = crate::update::apply_to(Some(partial.clone()), false)? else {
        let _ = std::fs::remove_dir_all(&dir);
        return Ok(None);
    };
    if !verify_binary(&partial, &version) {
        let _ = std::fs::remove_dir_all(&dir);
        bail!("the downloaded Forge {version} did not run");
    }
    let file = format!("forge-{version}{}", exe_suffix());
    std::fs::rename(&partial, dir.join(&file))?;
    std::fs::write(
        dir.join(STAGED_FILE),
        serde_json::to_string(&Staged {
            version: version.clone(),
            file,
        })?,
    )?;
    Ok(Some(version))
}

/// Where a swap lands: the running binary, or `FORGE_UPDATE_TARGET` (tests).
pub(crate) fn swap_target() -> PathBuf {
    crate::update::swap_target_override().unwrap_or_else(forge_types::self_exe::installed_path)
}

#[derive(Debug, Serialize, Deserialize)]
struct Marker {
    from: String,
    to: String,
    notes: Vec<String>,
    url: String,
}

/// Replace `dest` with a copy of `file`, atomically. The copy is made beside `dest`, so the
/// renames stay on one filesystem, and the old file is kept aside until the new one is in place.
fn replace_file(file: &Path, dest: &Path) -> Result<()> {
    let pid = std::process::id();
    let incoming = dest.with_file_name(format!(".forge-new-{pid}{}", exe_suffix()));
    let outgoing = dest.with_file_name(format!(".forge-old-{pid}{}", exe_suffix()));
    std::fs::copy(file, &incoming).context("copying the staged build next to the install")?;
    let moved = self_update::Move::from_source(&incoming)
        .replace_using_temp(&outgoing)
        .to_dest(dest);
    let _ = std::fs::remove_file(&incoming);
    moved.context("replacing the installed build")?;
    let _ = std::fs::remove_file(&outgoing);
    Ok(())
}

/// Put the staged build in place of `dest` and leave a note for the new build to announce.
/// Returns the new version.
pub(crate) fn swap_staged(dest: &Path) -> Result<String> {
    let (version, file) = staged_newer(CURRENT).context("no staged build to swap in")?;
    if !verify_binary(&file, &version) {
        bail!("the staged Forge {version} no longer runs");
    }
    replace_file(&file, dest)?;
    let release = UpdateState::load().latest.filter(|r| r.version == version);
    let marker = Marker {
        from: CURRENT.to_string(),
        to: version.clone(),
        notes: release
            .as_ref()
            .map(|r| notes_preview(&r.notes, 2, 80))
            .unwrap_or_default(),
        url: release.map(|r| r.url).unwrap_or_default(),
    };
    if let (Some(dir), Ok(json)) = (stage_dir(), serde_json::to_string(&marker)) {
        if let Some(parent) = dir.parent() {
            let _ = std::fs::write(parent.join(MARKER_FILE), json);
        }
        let _ = std::fs::remove_dir_all(dir);
    }
    Ok(version)
}

/// The one-line "Updated to X · what's new" for the first session after an automatic update.
/// Consumed once; a stale marker (the swap did not take) is dropped silently.
pub(crate) fn take_updated_notice() -> Option<String> {
    let path = forge_config::data_dir()?.join(MARKER_FILE);
    let marker: Marker = serde_json::from_str(&std::fs::read_to_string(&path).ok()?).ok()?;
    let _ = std::fs::remove_file(&path);
    (marker.to == CURRENT).then(|| notice_text(&marker))
}

fn notice_text(m: &Marker) -> String {
    let mut text = format!("⚒ Updated Forge {} → {}", m.from, m.to);
    if !m.notes.is_empty() {
        text.push_str(&format!(" · what's new: {}", m.notes.join("; ")));
    }
    if !m.url.is_empty() {
        text.push_str(&format!(" · {}", m.url));
    }
    text
}

/// Re-execute this program on `dest` with the same arguments (or `chat --resume <id>`). Only
/// returns on failure. The child is told it was started by an update so it cannot update again.
pub(crate) fn exec_new_build(dest: &Path, resume: Option<&str>) -> std::io::Result<()> {
    let mut cmd = std::process::Command::new(dest);
    match resume {
        Some(id) => {
            cmd.args(["chat", "--resume", id]);
        }
        None => {
            cmd.args(std::env::args_os().skip(1));
        }
    }
    cmd.env("FORGE_AUTO_UPDATED", "1");
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        Err(cmd.exec())
    }
    #[cfg(not(unix))]
    {
        let status = cmd.status()?;
        std::process::exit(status.code().unwrap_or(0));
    }
}

/// Before the TUI (or plain chat) opens: swap in a newer standalone build and restart into it.
/// Prints one line when it starts. Returns normally, having changed nothing, on any failure,
/// timeout or when nothing applies; only a successful swap re-executes.
pub(crate) async fn on_launch(config: &forge_config::Config) {
    let facts = LaunchFacts {
        interactive: interactive_launch(),
        auto: auto_enabled(config),
        method: crate::update::detect_install_method(),
        cached_newer: UpdateState::load().offer(CURRENT).is_some(),
        staged_newer: staged_newer(CURRENT).is_some(),
    };
    let action = plan_launch(&facts);
    let swapped = match action {
        LaunchAction::SwapStaged => {
            let Some((version, _)) = staged_newer(CURRENT) else {
                return;
            };
            announce(&version);
            tokio::task::spawn_blocking(|| swap_staged(&swap_target()))
                .await
                .is_ok_and(|r| r.is_ok())
        }
        LaunchAction::UpdateNow => {
            let Some(version) = UpdateState::load()
                .offer(CURRENT)
                .map(|r| r.version.clone())
            else {
                return;
            };
            announce(&version);
            run_with_budget(launch_budget(), || {
                stage_latest().and_then(|v| v.map(|_| swap_staged(&swap_target())).transpose())
            })
            .await
            .is_some_and(|r| matches!(r, Ok(Some(_))))
        }
        _ => false,
    };
    if swapped {
        let _ = exec_new_build(&swap_target(), None);
    }
}

fn announce(version: &str) {
    println!("Updating Forge {CURRENT} → {version}…");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(method: InstallMethod) -> LaunchFacts {
        LaunchFacts {
            interactive: true,
            auto: true,
            method,
            cached_newer: false,
            staged_newer: false,
        }
    }

    #[test]
    fn nothing_automatic_happens_unless_interactive_and_enabled() {
        let mut f = facts(InstallMethod::Binary);
        f.cached_newer = true;
        f.interactive = false;
        assert_eq!(
            plan_launch(&f),
            LaunchAction::Nothing,
            "pipes, headless, CI"
        );
        f.interactive = true;
        f.auto = false;
        assert_eq!(plan_launch(&f), LaunchAction::Nothing, "opted out");
    }

    #[test]
    fn a_standalone_install_swaps_staged_then_downloads_then_stages() {
        let mut f = facts(InstallMethod::Binary);
        assert_eq!(
            plan_launch(&f),
            LaunchAction::StageInBackground,
            "no info yet"
        );
        f.cached_newer = true;
        assert_eq!(plan_launch(&f), LaunchAction::UpdateNow);
        f.staged_newer = true;
        assert_eq!(
            plan_launch(&f),
            LaunchAction::SwapStaged,
            "a staged build is instant"
        );
    }

    #[test]
    fn package_managers_that_need_no_root_upgrade_in_the_background() {
        assert_eq!(
            plan_launch(&facts(InstallMethod::Homebrew)),
            LaunchAction::UpgradeInBackground("brew upgrade forge".into())
        );
        assert_eq!(
            plan_launch(&facts(InstallMethod::Scoop)),
            LaunchAction::UpgradeInBackground("scoop update forge".into())
        );
    }

    #[test]
    fn root_or_build_installs_keep_the_card() {
        for m in [InstallMethod::Aur, InstallMethod::Cargo] {
            let mut f = facts(m);
            f.cached_newer = true;
            assert_eq!(plan_launch(&f), LaunchAction::Nothing);
        }
    }

    #[tokio::test]
    async fn work_past_the_budget_is_abandoned_not_waited_for() {
        let slow = run_with_budget(Duration::from_millis(40), || {
            std::thread::sleep(Duration::from_millis(400));
            7
        })
        .await;
        assert_eq!(slow, None);
        let fast = run_with_budget(Duration::from_secs(5), || 7).await;
        assert_eq!(fast, Some(7));
    }

    #[test]
    fn the_swap_replaces_the_install_and_leaves_nothing_behind() {
        let dir = std::env::temp_dir().join(format!("forge-swap-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (installed, staged) = (dir.join("forge"), dir.join("staged"));
        std::fs::write(&installed, b"old").unwrap();
        std::fs::write(&staged, b"new").unwrap();
        replace_file(&staged, &installed).unwrap();
        assert_eq!(std::fs::read(&installed).unwrap(), b"new");
        assert!(
            staged.exists(),
            "the staged copy is left for the caller to clear"
        );
        let mut names: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, ["forge", "staged"], "no temp files: {names:?}");
        assert!(replace_file(&dir.join("missing"), &installed).is_err());
        assert_eq!(
            std::fs::read(&installed).unwrap(),
            b"new",
            "a failed swap keeps the install"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_budget_defaults_to_twenty_seconds() {
        if std::env::var_os("FORGE_AUTO_UPDATE_BUDGET_SECS").is_none() {
            assert_eq!(launch_budget(), Duration::from_secs(20));
        }
    }

    #[test]
    fn the_after_update_notice_names_versions_notes_and_link() {
        let m = Marker {
            from: "2.23.7".into(),
            to: "2.23.8".into(),
            notes: vec!["Faster".into(), "Smaller".into()],
            url: "https://example.test/r".into(),
        };
        assert_eq!(
            notice_text(&m),
            "⚒ Updated Forge 2.23.7 → 2.23.8 · what's new: Faster; Smaller · https://example.test/r"
        );
        let bare = Marker {
            notes: vec![],
            url: String::new(),
            ..m
        };
        assert_eq!(notice_text(&bare), "⚒ Updated Forge 2.23.7 → 2.23.8");
    }
}
