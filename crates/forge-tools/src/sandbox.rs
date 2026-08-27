//! OS-level filesystem sandbox for the `shell` tool.
//!
//! On Linux (5.13+) the sandbox is implemented with the Landlock LSM: write access is confined
//! to a set of writable paths (the workspace cwd + the system temp directory + any user-supplied
//! extras) while read and execute access stays broad (all of `/`) so installed binaries, shared
//! libraries and config files remain reachable. On any other platform — and on Linux kernels
//! that predate Landlock — the implementation is a transparent no-op.
//!
//! The result of attempting to apply the sandbox is [`ApplyResult`]:
//! - `Applied` — the ruleset is active in the calling process/thread.
//! - `Unsupported` — Landlock is not available on this kernel; caller should warn once and
//!   proceed unconfined.
//!
//! This module is intentionally free of async code: it is called from `pre_exec` (post-fork,
//! pre-exec) where only async-signal-safe operations are permitted.

use std::path::{Path, PathBuf};

/// Policy supplied by the caller.
#[derive(Default)]
pub struct SandboxPolicy {
    /// Whether the sandbox is enabled at all. When `false`, the shell tool installs no
    /// `pre_exec` sandbox hook on any platform.
    pub enabled: bool,
    /// Extra writable paths beyond the cwd + temp dir pair that [`effective_writable`] always adds.
    pub writable: Vec<PathBuf>,
    /// Base directory for a scoped, writable `CARGO_TARGET_DIR` injected into cargo/rust build
    /// commands. `None` disables the behaviour (the default). When `Some`, the shell tool points
    /// `CARGO_TARGET_DIR` at a per-project subdir of this base so `cargo check`/`build` write
    /// their target tree here instead of `<workspace>/target` — letting an autonomous/bypass agent
    /// compile-check its own edits even when the workspace tree is read-only under confinement.
    /// The scoped dir is also added to the writable set so it works under the Landlock sandbox.
    /// Independent of `enabled`: an outer container can confine writes without Forge's Landlock.
    pub cargo_target_base: Option<PathBuf>,
}

/// Outcome of applying the Landlock ruleset (see [`linux::apply_landlock`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyResult {
    /// Landlock ruleset is active; the process is confined.
    Applied,
    /// Sandbox was disabled by policy or is not supported on this kernel/platform.
    Unsupported,
}

/// Build the effective set of writable paths: `cwd`, `std::env::temp_dir()`, and any extras
/// from the policy. Absolute paths are used as-is; relative ones are resolved against `cwd`.
pub fn effective_writable(cwd: &Path, extra: &[PathBuf]) -> Vec<PathBuf> {
    let mut paths = vec![cwd.to_path_buf(), std::env::temp_dir()];
    for p in extra {
        if p.is_absolute() {
            paths.push(p.clone());
        } else {
            paths.push(cwd.join(p));
        }
    }
    paths
}

// ── Linux implementation ──────────────────────────────────────────────────────

#[cfg(target_os = "linux")]
pub(crate) mod linux {
    use std::path::Path;

    use landlock::{
        Access, AccessFs, CompatLevel, Compatible, PathBeneath, PathFd, Ruleset, RulesetAttr,
        RulesetCreated, RulesetCreatedAttr, ABI,
    };

    /// Probe whether this kernel supports Landlock without installing any ruleset.
    /// Returns `true` if at least Landlock ABI v1 is available.
    pub fn is_supported() -> bool {
        Ruleset::default()
            .set_compatibility(CompatLevel::HardRequirement)
            .handle_access(AccessFs::from_all(ABI::V1))
            .is_ok()
    }

    /// Build the Landlock ruleset in the **parent**, before fork.
    ///
    /// Everything that allocates lives here: creating the ruleset, opening each path, and
    /// formatting errors. The child then only has to call
    /// [`RulesetCreated::restrict_self`], which is a `prctl` plus one `landlock_restrict_self`
    /// syscall over an already-open fd.
    ///
    /// This split matters because only async-signal-safe calls are legal between `fork` and
    /// `exec`, and Forge spawns shells from a tokio runtime — a multithreaded parent. Allocating
    /// in the child risks deadlocking in `malloc` if another thread held its lock at fork, which
    /// would hang the shell command with no output and no error.
    ///
    /// The returned ruleset owns a CLOEXEC fd: it survives fork and is closed by `exec`, so the
    /// child can restrict itself with it and nothing leaks into the executed program.
    pub fn prepare_landlock(writable: &[impl AsRef<Path>]) -> Result<RulesetCreated, String> {
        // Read-only access we grant on the whole filesystem so binaries/libs/configs resolve.
        let read_exec: landlock::BitFlags<AccessFs> =
            AccessFs::Execute | AccessFs::ReadFile | AccessFs::ReadDir;

        // Full write access granted only on explicitly listed paths.
        let write_flags: landlock::BitFlags<AccessFs> = AccessFs::from_all(ABI::V5);

        let ruleset = Ruleset::default()
            .set_compatibility(CompatLevel::BestEffort)
            .handle_access(AccessFs::from_all(ABI::V5))
            .map_err(|e| e.to_string())?
            .create()
            .map_err(|e| e.to_string())?;

        // Broad read+execute on `/`.
        let root_fd = PathFd::new("/").map_err(|e| e.to_string())?;
        let ruleset = ruleset
            .add_rule(PathBeneath::new(root_fd, read_exec))
            .map_err(|e| e.to_string())?;

        // Full read+write on each explicitly writable path.
        let mut ruleset = ruleset;
        for path in writable {
            let p = path.as_ref();
            if !p.exists() {
                let _ = std::fs::create_dir_all(p);
            }
            if let Ok(fd) = PathFd::new(p) {
                ruleset = ruleset
                    .add_rule(PathBeneath::new(fd, write_flags))
                    .map_err(|e| e.to_string())?;
            }
        }

        Ok(ruleset)
    }

    /// Take the prepared ruleset and apply it, or fail closed if it has already been used.
    ///
    /// `pre_exec` takes an `FnMut`, but `restrict_self` consumes the ruleset, so the closure moves
    /// it out of an `Option`. If the same `Command` is spawned twice the second child finds `None`
    /// — and must refuse to start rather than run unconfined, which is the whole point of the
    /// sandbox. Kept here rather than inline in the closure so that branch is testable.
    pub fn restrict_once(prepared: &mut Option<RulesetCreated>) -> std::io::Result<()> {
        match prepared.take() {
            Some(ruleset) => restrict_current_thread(ruleset),
            None => Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied)),
        }
    }

    /// Apply a ruleset prepared by [`prepare_landlock`] to the calling thread.
    ///
    /// Called from `pre_exec`, so it must not allocate: the error carries only an
    /// [`std::io::ErrorKind`], which `io::Error` stores inline. The parent adds the human-readable
    /// context when the spawn fails, where allocating is safe.
    pub fn restrict_current_thread(ruleset: RulesetCreated) -> std::io::Result<()> {
        ruleset
            .restrict_self()
            .map(|_| ())
            .map_err(|_| std::io::Error::from(std::io::ErrorKind::PermissionDenied))
    }
}

// ── Public entry points ───────────────────────────────────────────────────────

/// Probe (in the **parent** process) whether Landlock is supported on this kernel.
/// Returns `false` on non-Linux platforms and on Linux kernels without Landlock.
pub fn is_supported() -> bool {
    #[cfg(target_os = "linux")]
    {
        linux::is_supported()
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    /// A ruleset is consumed by its first use, so a second spawn of the same `Command` finds
    /// `None`. That case must refuse rather than fall back to running unconfined — silently
    /// dropping the sandbox is exactly the failure the fail-closed policy exists to prevent.
    ///
    /// Only the exhausted branch is exercised: calling `restrict_once` with a live ruleset would
    /// apply Landlock to the test runner itself, confining every test that follows it in this
    /// process. The applied path is covered by `sandbox_blocks_write_outside_and_allows_inside`,
    /// which spawns a real child.
    #[cfg(target_os = "linux")]
    #[test]
    fn an_exhausted_ruleset_refuses_rather_than_running_unconfined() {
        let mut exhausted: Option<landlock::RulesetCreated> = None;
        let error = linux::restrict_once(&mut exhausted)
            .expect_err("an exhausted ruleset must fail closed");
        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn effective_writable_resolves_relative() {
        let cwd = PathBuf::from("/workspace/proj");
        let extras = vec![PathBuf::from("out"), PathBuf::from("/abs/path")];
        let paths = effective_writable(&cwd, &extras);
        assert_eq!(paths[0], PathBuf::from("/workspace/proj"));
        assert_eq!(paths[1], std::env::temp_dir());
        assert_eq!(paths[2], PathBuf::from("/workspace/proj/out"));
        assert_eq!(paths[3], PathBuf::from("/abs/path"));
    }

    #[test]
    fn effective_writable_no_extras() {
        let cwd = PathBuf::from("/some/dir");
        let paths = effective_writable(&cwd, &[]);
        assert_eq!(paths.len(), 2);
        assert_eq!(paths[0], PathBuf::from("/some/dir"));
        assert_eq!(paths[1], std::env::temp_dir());
    }

    /// Runs only on Linux; skips gracefully if Landlock is not supported on the test kernel.
    #[cfg(target_os = "linux")]
    #[test]
    fn sandbox_blocks_write_outside_and_allows_inside() {
        if !is_supported() {
            eprintln!("Landlock not supported on this kernel — skipping sandbox test");
            return;
        }

        let tmp = std::env::temp_dir();
        let workspace = tmp.join(format!("forge-sandbox-test-{}", std::process::id()));
        std::fs::create_dir_all(&workspace).unwrap();

        let probe_inside = workspace.join("write_ok.txt");
        let writable = effective_writable(&workspace, &[]);

        // Test 1: write OUTSIDE the writable set (/etc) must fail.
        {
            let w = writable.clone();
            let mut cmd = std::process::Command::new("sh");
            cmd.arg("-c").arg("echo x > /etc/forge-sandbox-probe-test");
            // Prepared in the parent, applied in the child — the same split the real spawn path
            // uses, so this test exercises it rather than an easier variant.
            let mut prepared = Some(linux::prepare_landlock(&w).expect("prepare ruleset"));
            unsafe {
                use std::os::unix::process::CommandExt;
                cmd.pre_exec(move || match prepared.take() {
                    Some(ruleset) => linux::restrict_current_thread(ruleset),
                    None => Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied)),
                });
            }
            let status = cmd.status().expect("spawn");
            assert!(
                !status.success(),
                "write to /etc must be blocked by sandbox"
            );
        }

        // Test 2: write INSIDE the workspace must succeed.
        {
            let w = writable.clone();
            let inside = probe_inside.to_string_lossy().into_owned();
            let mut cmd = std::process::Command::new("sh");
            cmd.arg("-c").arg(format!("echo hi > {inside}"));
            let mut prepared = Some(linux::prepare_landlock(&w).expect("prepare ruleset"));
            unsafe {
                use std::os::unix::process::CommandExt;
                cmd.pre_exec(move || match prepared.take() {
                    Some(ruleset) => linux::restrict_current_thread(ruleset),
                    None => Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied)),
                });
            }
            let status = cmd.status().expect("spawn");
            assert!(status.success(), "write inside workspace must succeed");
            assert!(probe_inside.exists(), "file must exist after write");
        }

        let _ = std::fs::remove_dir_all(&workspace);
    }
}
