//! `forge update`: self-update a standalone binary install to the latest GitHub release, or print
//! the right upgrade command for package-manager installs. Also powers the optional startup
//! "update now?" prompt (see [`crate::update_check`]).

use anyhow::{Context, Result};

const REPO_OWNER: &str = "Adulari";
const REPO_NAME: &str = "forge";
const CURRENT: &str = env!("CARGO_PKG_VERSION");

/// How this `forge` was installed — decides whether we can safely swap the binary in place. We never
/// clobber a package-manager-owned file, which would desync its bookkeeping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallMethod {
    /// Standalone binary (curl `install.sh` / manual download): safe to self-replace.
    Binary,
    /// Homebrew-managed: defer to `brew upgrade`.
    Homebrew,
    /// `cargo install`-managed: defer to `cargo install … --force`.
    Cargo,
    /// Arch `forge-bin` package: defer to the AUR helper.
    Aur,
    /// Scoop-managed (Windows): defer to `scoop update`.
    Scoop,
    /// Built from source (no `FORGE_DISTRIBUTION` baked in at build time): never replaced by a
    /// release, which would overwrite a developer's own build with an older or different one.
    Source,
}

impl InstallMethod {
    /// Short name used on the daemon's `/api/update` wire.
    pub fn wire_name(self) -> &'static str {
        match self {
            Self::Binary => "binary",
            Self::Homebrew => "homebrew",
            Self::Cargo => "cargo",
            Self::Aur => "aur",
            Self::Scoop => "scoop",
            Self::Source => "source",
        }
    }

    /// The exact command that upgrades a package-manager install; `None` for a standalone
    /// binary, which replaces itself.
    pub fn upgrade_command(self) -> Option<String> {
        match self {
            Self::Binary => None,
            Self::Homebrew => Some("brew upgrade forge".to_string()),
            Self::Cargo => Some(format!(
                "cargo install --git https://github.com/{REPO_OWNER}/{REPO_NAME} forge-cli --force"
            )),
            Self::Aur => Some("yay -Syu forge-bin".to_string()),
            Self::Scoop => Some("scoop update forge".to_string()),
            Self::Source => Some("git pull && cargo build --release -p forge-agent".to_string()),
        }
    }
}

/// Best-effort classification of the running binary's install method from its path.
pub fn detect_install_method() -> InstallMethod {
    let exe = std::env::current_exe().unwrap_or_default();
    let in_cargo_home = std::env::var_os("CARGO_HOME")
        .map(|c| exe.starts_with(std::path::Path::new(&c).join("bin")))
        .unwrap_or(false);
    let raw = exe.to_string_lossy();
    if raw == "/usr/bin/forge" && std::path::Path::new("/etc/arch-release").exists() {
        return InstallMethod::Aur;
    }
    with_build_origin(
        classify_path(&raw, in_cargo_home),
        option_env!("FORGE_DISTRIBUTION").is_some(),
        swap_target_override().is_some(),
    )
}

/// A path that looks like a standalone install is only one when the binary is an official release
/// build; a source build in its place (a symlink into `target/`, a copied dev build) is `Source`.
/// `FORGE_UPDATE_TARGET` (tests) swaps a separate file, so it keeps the standalone path.
fn with_build_origin(
    method: InstallMethod,
    release_build: bool,
    test_target: bool,
) -> InstallMethod {
    match method {
        InstallMethod::Binary if !release_build && !test_target => InstallMethod::Source,
        other => other,
    }
}

/// Pure path classifier (separated for testing). `in_cargo_home` is a caller-resolved hint that the
/// path sits under `$CARGO_HOME/bin` even when the literal `.cargo/bin` substring isn't present.
fn classify_path(raw_path: &str, in_cargo_home: bool) -> InstallMethod {
    let path = raw_path.replace('\\', "/").to_lowercase();
    if path.contains("/cellar/") || path.contains("/homebrew/") {
        return InstallMethod::Homebrew;
    }
    if path.contains("/scoop/apps/") || path.contains("/scoop/shims/") {
        return InstallMethod::Scoop;
    }
    if path.contains("/.cargo/bin/") || in_cargo_home {
        return InstallMethod::Cargo;
    }
    InstallMethod::Binary
}

/// Run `forge update`. For a standalone binary install, download the latest release and swap it in
/// place; for brew/cargo, print the correct upgrade command. With `check_only`, just report whether a
/// newer release exists without changing anything.
pub fn run(check_only: bool) -> Result<()> {
    if check_only {
        return match latest_release_tag()? {
            Some(tag) if crate::update_check::is_newer(&tag, CURRENT) => {
                println!(
                    "a newer Forge is available: {tag} (you have {CURRENT}). Run `forge update`."
                );
                Ok(())
            }
            _ => {
                println!("forge is up to date ({CURRENT}).");
                Ok(())
            }
        };
    }
    let method = detect_install_method();
    match method.upgrade_command() {
        Some(cmd) => {
            println!(
                "forge was installed via {} — update with:\n  {cmd}",
                method.wire_name()
            );
            Ok(())
        }
        None => self_replace(),
    }
}

/// The latest release tag (e.g. `v0.3.0`), or `None` if the lookup fails (offline, rate-limited).
fn latest_release_tag() -> Result<Option<String>> {
    let releases = release_list_builder()
        .build()
        .context("configuring release lookup")?
        .fetch()
        .context("fetching releases")?;
    Ok(releases.latest().map(|r| r.version().to_string()))
}

/// The GitHub API base: `FORGE_UPDATE_FEED_URL` when set (tests, mirrors), else github.com.
fn release_list_builder() -> self_update::backends::github::ReleaseListBuilder {
    let mut builder = self_update::backends::github::ReleaseList::configure();
    builder.repo_owner(REPO_OWNER).repo_name(REPO_NAME);
    if let Some(base) = crate::update_feed::feed_override() {
        builder.api_base_url(base);
    }
    builder
}

/// Download the latest GitHub release asset for this target and replace the running binary.
/// Returns the new version, or `None` when already current — including a source/dev build whose
/// version is ahead of the latest release (self_update only applies a strictly-greater version,
/// never a downgrade). `FORGE_UPDATE_TARGET` points the swap at another file (tests); progress
/// is only drawn on request, since a bar would corrupt a full-screen TUI.
pub fn apply_in_place(show_progress: bool) -> Result<Option<String>> {
    apply_to(swap_target_override(), show_progress)
}

/// `FORGE_UPDATE_TARGET`: replace this file instead of the running binary (tests).
pub(crate) fn swap_target_override() -> Option<std::path::PathBuf> {
    std::env::var_os("FORGE_UPDATE_TARGET").map(Into::into)
}

/// [`apply_in_place`] aimed at an explicit file: the newest release's binary is written over
/// `target` (the running binary when `None`). Staging uses this to fetch a build without
/// touching the one that is running.
pub fn apply_to(target: Option<std::path::PathBuf>, show_progress: bool) -> Result<Option<String>> {
    // Our release archives lay the binary out as `forge-<target>/forge[.exe]`.
    let bin_in_archive = format!("forge-{{{{target}}}}/forge{}", std::env::consts::EXE_SUFFIX);
    let mut builder = self_update::backends::github::Update::configure();
    builder
        .repo_owner(REPO_OWNER)
        .repo_name(REPO_NAME)
        .bin_name("forge")
        .bin_path_in_archive(&bin_in_archive)
        .current_version(CURRENT)
        .show_download_progress(show_progress)
        .show_output(show_progress)
        .no_confirm(true);
    if let Some(base) = crate::update_feed::feed_override() {
        builder.api_base_url(base);
    }
    if let Some(target) = target {
        builder.bin_install_path(target);
    }
    let status = builder
        .build()
        .context("configuring the updater")?
        .update()
        .context("downloading/applying the update")?;
    Ok(status.is_updated().then(|| status.version().to_string()))
}

fn self_replace() -> Result<()> {
    match apply_in_place(true)? {
        Some(version) => {
            println!("✔ updated forge {CURRENT} → {version}");
            println!("  restart forge to use the new version.");
        }
        None => println!("forge is already up to date ({CURRENT})."),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_source_build_is_never_a_standalone_install() {
        assert_eq!(
            with_build_origin(InstallMethod::Binary, false, false),
            InstallMethod::Source
        );
        assert_eq!(
            with_build_origin(InstallMethod::Binary, true, false),
            InstallMethod::Binary
        );
        assert_eq!(
            with_build_origin(InstallMethod::Binary, false, true),
            InstallMethod::Binary,
            "tests aiming the swap at a temp file keep the standalone path"
        );
        assert_eq!(
            with_build_origin(InstallMethod::Homebrew, false, false),
            InstallMethod::Homebrew
        );
    }

    #[test]
    fn classify_path_distinguishes_install_methods() {
        // Homebrew (Apple Silicon + Intel Cellar layouts).
        assert_eq!(
            classify_path("/opt/homebrew/bin/forge", false),
            InstallMethod::Homebrew
        );
        assert_eq!(
            classify_path("/usr/local/Cellar/forge/0.2.0/bin/forge", false),
            InstallMethod::Homebrew
        );
        // Cargo, by path substring and by the resolved-CARGO_HOME hint.
        assert_eq!(
            classify_path("/home/me/.cargo/bin/forge", false),
            InstallMethod::Cargo
        );
        assert_eq!(
            classify_path("/custom/cargohome/bin/forge", true),
            InstallMethod::Cargo
        );
        assert_eq!(
            classify_path(
                "C:\\Users\\me\\scoop\\apps\\forge\\current\\forge.exe",
                false
            ),
            InstallMethod::Scoop
        );
        assert_eq!(
            InstallMethod::Aur.upgrade_command().as_deref(),
            Some("yay -Syu forge-bin")
        );
        assert_eq!(InstallMethod::Binary.upgrade_command(), None);
        assert!(InstallMethod::Source.upgrade_command().is_some());
        // Standalone binary installs (curl install.sh / manual).
        assert_eq!(
            classify_path("/home/me/.local/bin/forge", false),
            InstallMethod::Binary
        );
        assert_eq!(
            classify_path("/usr/local/bin/forge", false),
            InstallMethod::Binary
        );
        // Windows path normalization.
        assert_eq!(
            classify_path("C:\\Users\\me\\.cargo\\bin\\forge.exe", false),
            InstallMethod::Cargo
        );
    }
}
