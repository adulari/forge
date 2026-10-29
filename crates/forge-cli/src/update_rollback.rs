//! Keeping the build an update replaced, so `forge update --rollback` can put it back.
//!
//! Every update that swaps the installed binary first leaves the old one beside it as
//! `<name>.prev` (a hard link where the filesystem allows, else a copy). Rolling back swaps the
//! two names, so a second rollback undoes the first.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

pub(crate) fn previous_path(exe: &Path) -> PathBuf {
    let mut name = exe.file_name().unwrap_or_default().to_os_string();
    name.push(".prev");
    exe.with_file_name(name)
}

fn pending_path(exe: &Path) -> PathBuf {
    let mut name = previous_path(exe)
        .file_name()
        .unwrap_or_default()
        .to_os_string();
    name.push(".pending");
    exe.with_file_name(name)
}

fn link_or_copy(from: &Path, to: &Path) -> std::io::Result<()> {
    let _ = std::fs::remove_file(to);
    std::fs::hard_link(from, to).or_else(|_| std::fs::copy(from, to).map(|_| ()))
}

/// Set the current binary aside before an update that may replace it. Pair with [`settle`].
pub(crate) fn stash(exe: &Path) -> Option<PathBuf> {
    let pending = pending_path(exe);
    link_or_copy(exe, &pending).ok().map(|()| pending)
}

/// After the update ran: keep the stashed binary as the previous build if the update replaced
/// the binary, drop it if nothing changed.
pub(crate) fn settle(exe: &Path, pending: Option<PathBuf>, replaced: bool) {
    let Some(pending) = pending else { return };
    if replaced {
        let _ = std::fs::rename(&pending, previous_path(exe));
    } else {
        let _ = std::fs::remove_file(&pending);
    }
}

/// Keep the current binary as the previous build, for a swap that is certain to follow.
pub(crate) fn keep_previous(exe: &Path) {
    let _ = link_or_copy(exe, &previous_path(exe));
}

/// Swap the installed binary with the kept previous one. Returns the previous build's
/// `--version` line.
pub(crate) fn rollback(exe: &Path) -> Result<String> {
    let previous = previous_path(exe);
    if !previous.exists() {
        bail!(
            "no previous Forge is kept next to {} (an update leaves one behind)",
            exe.display()
        );
    }
    let version = version_line(&previous)?;
    let aside = exe.with_file_name(format!(
        ".forge-rollback-{}{}",
        std::process::id(),
        std::env::consts::EXE_SUFFIX
    ));
    std::fs::rename(exe, &aside).with_context(|| format!("moving {} aside", exe.display()))?;
    if let Err(error) = std::fs::rename(&previous, exe) {
        let _ = std::fs::rename(&aside, exe);
        return Err(error).context("putting the previous build in place");
    }
    std::fs::rename(&aside, &previous).context("keeping the replaced build as the new previous")?;
    Ok(version)
}

fn version_line(binary: &Path) -> Result<String> {
    let output = std::process::Command::new(binary)
        .arg("--version")
        .output()
        .with_context(|| format!("running {}", binary.display()))?;
    if !output.status.success() {
        bail!(
            "the previous build does not run (--version exited {})",
            output.status
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn fake_forge(path: &Path, version: &str) {
        std::fs::write(path, format!("#!/bin/sh\necho \"forge {version}\"\n")).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn version_of(path: &Path) -> String {
        version_line(path).unwrap()
    }

    #[test]
    fn an_update_that_replaced_the_binary_leaves_the_old_one_as_previous() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("forge");
        fake_forge(&exe, "1.0.0");
        let pending = stash(&exe);
        assert!(pending.is_some());
        let incoming = dir.path().join("incoming");
        fake_forge(&incoming, "2.0.0");
        std::fs::rename(&incoming, &exe).unwrap();
        settle(&exe, pending, true);
        assert_eq!(version_of(&exe), "forge 2.0.0");
        assert_eq!(version_of(&previous_path(&exe)), "forge 1.0.0");
        assert!(!pending_path(&exe).exists());
    }

    #[test]
    fn an_update_that_changed_nothing_leaves_no_previous() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("forge");
        fake_forge(&exe, "1.0.0");
        settle(&exe, stash(&exe), false);
        assert!(!previous_path(&exe).exists());
        assert!(!pending_path(&exe).exists());
    }

    #[test]
    fn rolling_back_swaps_the_builds_and_a_second_rollback_undoes_it() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("forge");
        fake_forge(&exe, "2.0.0");
        fake_forge(&previous_path(&exe), "1.0.0");
        assert_eq!(rollback(&exe).unwrap(), "forge 1.0.0");
        assert_eq!(version_of(&exe), "forge 1.0.0");
        assert_eq!(version_of(&previous_path(&exe)), "forge 2.0.0");
        assert_eq!(rollback(&exe).unwrap(), "forge 2.0.0");
        assert_eq!(version_of(&exe), "forge 2.0.0");
    }

    #[test]
    fn rolling_back_with_nothing_kept_says_so_and_a_broken_previous_is_not_installed() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("forge");
        fake_forge(&exe, "2.0.0");
        let error = rollback(&exe).unwrap_err().to_string();
        assert!(error.contains("no previous Forge"), "{error}");
        std::fs::write(previous_path(&exe), "not a program").unwrap();
        std::fs::set_permissions(previous_path(&exe), std::fs::Permissions::from_mode(0o755))
            .unwrap();
        assert!(rollback(&exe).is_err());
        assert_eq!(
            version_of(&exe),
            "forge 2.0.0",
            "the working build is untouched"
        );
    }
}
