use tokio::process::Command;

/// Put the child in a new session, which is what actually survives a process-group kill.
#[cfg(unix)]
pub(super) fn detach_into_new_session(cmd: &mut Command) {
    // SAFETY: setsid is async-signal-safe and touches only the calling (freshly forked) process.
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

#[cfg(not(unix))]
pub(super) fn detach_into_new_session(_cmd: &mut Command) {}

#[cfg(unix)]
pub(super) fn pid_alive(pid: u32) -> bool {
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

#[cfg(not(unix))]
pub(super) fn pid_alive(_pid: u32) -> bool {
    false
}

#[cfg(unix)]
pub(super) fn signal_group(pid: u32, signal: i32) {
    unsafe { libc::kill(-(pid as i32), signal) };
}

#[cfg(not(unix))]
pub(super) fn signal_group(_pid: u32, _signal: i32) {}

#[cfg(unix)]
pub(super) const SIGTERM: i32 = libc::SIGTERM;
#[cfg(not(unix))]
pub(super) const SIGTERM: i32 = 0;
#[cfg(unix)]
pub(super) const SIGKILL: i32 = libc::SIGKILL;
#[cfg(not(unix))]
pub(super) const SIGKILL: i32 = 0;

/// Process start time in clock ticks since boot (`/proc/<pid>/stat` field 22). Two processes with
/// the same pid but different start times are different processes.
#[cfg(target_os = "linux")]
pub(super) fn proc_start_ticks(pid: u32) -> Option<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let (_, rest) = stat.rsplit_once(')')?;
    // Field 22 overall; after `comm` the first entry is field 3, so index 19.
    rest.split_whitespace().nth(19)?.parse().ok()
}

#[cfg(not(target_os = "linux"))]
pub(super) fn proc_start_ticks(_pid: u32) -> Option<u64> {
    None
}
