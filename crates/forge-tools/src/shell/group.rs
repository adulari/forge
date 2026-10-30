//! Process-group handling for foreground shell commands: own group on spawn, group kill on drop,
//! and SIGTERM-then-SIGKILL on timeout.

use std::time::Duration;

use tokio::process::{Child, Command};

use super::KILL_GRACE;

/// Put the child in its own process group so a timeout kill takes down the whole tree.
pub(super) fn put_in_own_process_group(cmd: &mut Command) {
    #[cfg(unix)]
    {
        cmd.process_group(0);
    }
    #[cfg(not(unix))]
    {
        let _ = cmd;
    }
}

/// Best-effort SIGKILL of the child's process group, used on the *normal-exit* cleanup path to
/// reap any backgrounded grandchild that outlived the shell and is still holding the output pipes
/// open. Unlike [`terminate`] there is no SIGTERM/grace step (the main child has already exited and
/// been reaped) and no `child.wait()` — this just nukes leaked descendants. A no-such-group error
/// (the common case: nothing leaked) is harmless and swallowed. No-op on non-Unix.
/// Aborting a turn drops the run future; `kill_on_drop` then kills only the direct `sh`, orphaning
/// its children. This kills the whole group on that path; cleared once the wait completes.
pub(super) struct GroupKillOnDrop(pub(super) Option<i32>);

impl Drop for GroupKillOnDrop {
    fn drop(&mut self) {
        kill_process_group(self.0);
    }
}

pub(super) fn kill_process_group(pgid: Option<i32>) {
    #[cfg(unix)]
    if let Some(pg) = pgid {
        unsafe { libc::kill(-pg, libc::SIGKILL) };
    }
    #[cfg(not(unix))]
    {
        let _ = pgid;
    }
}

/// Kill the child's whole process tree on timeout: SIGTERM→grace→SIGKILL on the process group
/// (Unix); `taskkill /F /T` on Windows. The tree matters because `cmd /C`/`sh -c` spawn the real
/// command as a child — killing only the shell would leave it running and hold the output pipes
/// open, hanging the read until it exits on its own.
pub(super) async fn terminate(child: &mut Child, pgid: Option<i32>) -> Vec<String> {
    let mut errors = Vec::new();
    #[cfg(unix)]
    {
        if let Some(pg) = pgid {
            if let Err(error) = signal_process_group(pg, libc::SIGTERM) {
                errors.push(format!("SIGTERM: {error}"));
            }
            // Wait out the grace only while something in the group is still alive: a command
            // that dies on SIGTERM must not cost the full grace period.
            let deadline = tokio::time::Instant::now() + KILL_GRACE;
            loop {
                let _ = child.try_wait(); // reap, so a zombie leader doesn't count as alive
                let gone = unsafe { libc::kill(-pg, 0) } == -1
                    && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH);
                if gone || tokio::time::Instant::now() >= deadline {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            if let Err(error) = signal_process_group(pg, libc::SIGKILL) {
                errors.push(format!("SIGKILL: {error}"));
            }
        }
        if let Err(error) = child.wait().await {
            errors.push(format!("wait: {error}"));
        }
    }
    #[cfg(not(unix))]
    {
        let _ = pgid;
        if let Some(pid) = child.id() {
            // `/T` kills the tree (the command `cmd /C` spawned), `/F` forces it.
            match Command::new("taskkill")
                .args(["/F", "/T", "/PID", &pid.to_string()])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .await
            {
                Ok(status) if status.success() => {}
                Ok(status) => errors.push(format!("taskkill exited with {status}")),
                Err(error) => errors.push(format!("taskkill: {error}")),
            }
        }
        if let Err(error) = child.start_kill() {
            errors.push(format!("terminate: {error}"));
        }
        if let Err(error) = child.wait().await {
            errors.push(format!("wait: {error}"));
        }
    }
    errors
}

#[cfg(unix)]
pub(super) fn signal_process_group(pgid: i32, signal: libc::c_int) -> Result<(), std::io::Error> {
    // ESRCH means the group already exited, which is a successful cleanup outcome.
    if unsafe { libc::kill(-pgid, signal) } == -1 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::ESRCH) {
            return Err(error);
        }
    }
    Ok(())
}
