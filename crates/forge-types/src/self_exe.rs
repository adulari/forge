//! Where to find THIS binary when Forge launches itself.
//!
//! `forge chat` and `forge serve` run for days; deploying a new build replaces the file they were
//! started from (`cargo build`, `install`, `mv`) while they keep running the old inode. On Linux
//! `std::env::current_exe()` then reads `/proc/self/exe`, whose target the kernel renders as
//! `"<path> (deleted)"` — a string that names no file. Every later self-launch built from it
//! (the `forge mcp-serve` a bridged `claude`/`codex` is told to start, the self-MCP server, a
//! scheduled run) failed with ENOENT, and the session stayed up but could no longer use its tools.

use std::path::{Path, PathBuf};

const DELETED_SUFFIX: &str = " (deleted)";

/// Whether the kernel reported `link` (a `current_exe()` result) as an unlinked file.
pub fn is_deleted(link: &Path) -> bool {
    link.to_str().is_some_and(|s| s.ends_with(DELETED_SUFFIX))
}

/// The on-disk path an unlinked `link` used to have. Equal to `link` when it was not unlinked.
pub fn without_deleted_suffix(link: &Path) -> PathBuf {
    match link.to_str().and_then(|s| s.strip_suffix(DELETED_SUFFIX)) {
        Some(path) => PathBuf::from(path),
        None => link.to_path_buf(),
    }
}

/// What to exec to run the build that process `pid` is running, given its `current_exe()` result.
///
/// An unlinked binary is reached through `/proc/<pid>/exe`, which the kernel keeps executable for
/// as long as the process lives. That path also keeps a child on the SAME build as its parent: the
/// file now at the old path is a different build, and a bridge child speaking a newer
/// sink/schema to an older parent is a worse failure than the one this exists to prevent.
pub fn launch_path(link: PathBuf, pid: u32) -> PathBuf {
    if is_deleted(&link) {
        PathBuf::from(format!("/proc/{pid}/exe"))
    } else {
        link
    }
}

/// The path to hand to a child process this process is about to spawn. Valid only while this
/// process is alive: do not write it into a file that outlives it (use [`installed_path`]).
pub fn forge_executable() -> PathBuf {
    match std::env::current_exe() {
        Ok(link) => launch_path(link, std::process::id()),
        Err(_) => PathBuf::from("forge"),
    }
}

/// The path this binary was installed at, for something that must outlive this process (a unit
/// file, a schedule). After an upgrade it names the replacement build, which is what a later
/// start should run.
pub fn installed_path() -> PathBuf {
    match std::env::current_exe() {
        Ok(link) => without_deleted_suffix(&link),
        Err(_) => PathBuf::from("forge"),
    }
}

/// Whether the file this process is running has been replaced or removed since it started, so
/// the running build is older than what is on disk. Only Linux reports it (the `(deleted)`
/// marker); elsewhere it is `false`.
pub fn running_binary_replaced() -> bool {
    std::env::current_exe().is_ok_and(|link| is_deleted(&link))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unlinked_binary_is_launched_through_its_live_proc_entry() {
        let link = PathBuf::from("/home/u/.local/bin/forge (deleted)");
        assert!(is_deleted(&link));
        assert_eq!(
            launch_path(link.clone(), 4242),
            PathBuf::from("/proc/4242/exe")
        );
        assert_eq!(
            without_deleted_suffix(&link),
            PathBuf::from("/home/u/.local/bin/forge")
        );
    }

    #[test]
    fn a_binary_still_on_disk_keeps_its_path() {
        let link = PathBuf::from("/home/u/.local/bin/forge");
        assert!(!is_deleted(&link));
        assert_eq!(launch_path(link.clone(), 7), link);
        assert_eq!(without_deleted_suffix(&link), link);
    }

    /// A directory that really is named like the marker is not an unlinked file.
    #[test]
    fn only_a_trailing_marker_counts() {
        let link = PathBuf::from("/opt/forge (deleted)/bin/forge");
        assert!(!is_deleted(&link));
        assert_eq!(without_deleted_suffix(&link), link);
    }

    /// The real kernel behaviour, end to end: a copy of this test binary deletes itself while
    /// running, and the path [`forge_executable`] then gives must still start a process. Skipped
    /// off Linux, where `current_exe` never carries the marker.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_process_whose_binary_was_replaced_can_still_launch_itself() {
        const PROBE_DIR: &str = "FORGE_SELF_EXE_PROBE_DIR";
        if let Ok(dir) = std::env::var(PROBE_DIR) {
            let me = std::env::current_exe().unwrap();
            std::fs::remove_file(&me).unwrap();
            assert!(running_binary_replaced());
            let launch = forge_executable();
            assert_eq!(
                launch,
                PathBuf::from(format!("/proc/{}/exe", std::process::id()))
            );
            let out = std::process::Command::new(&launch)
                .args(["--list", "--format", "terse"])
                .output()
                .expect("an unlinked binary must still be launchable by the path we hand out");
            assert!(out.status.success(), "{out:?}");
            std::fs::write(
                Path::new(&dir).join("ok"),
                installed_path().to_string_lossy().as_bytes(),
            )
            .unwrap();
            return;
        }
        let dir = std::env::temp_dir().join(format!("forge-self-exe-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let copy = dir.join("probe-bin");
        std::fs::copy(std::env::current_exe().unwrap(), &copy).unwrap();
        let status = std::process::Command::new(&copy)
            .args([
                "--exact",
                "self_exe::tests::a_process_whose_binary_was_replaced_can_still_launch_itself",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(PROBE_DIR, &dir)
            .status()
            .unwrap();
        assert!(status.success(), "probe child failed: {status:?}");
        let installed = std::fs::read_to_string(dir.join("ok")).unwrap();
        assert_eq!(
            installed,
            copy.to_string_lossy(),
            "the installed path is the on-disk path the binary used to have"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
