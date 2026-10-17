//! Reap the descendants of a bridge CLI that a process-group kill cannot reach.
//!
//! The bridge CLI (`claude`/`codex`) runs in its own process group, and so does the `forge
//! mcp-serve` it launches. But the shell tool deliberately puts EVERY command it runs in a group of
//! its own (so a timeout can kill that command's whole tree). When the bridge turn is aborted the
//! bridge group is SIGKILLed, `mcp-serve` dies without running any cleanup, and each in-flight
//! shell command (a different group) is orphaned and keeps running.
//!
//! The fix walks the parent/child tree under the bridge root BEFORE the group kill (afterwards the
//! orphans are reparented to init and the link is gone) and kills every descendant that is still in
//! the bridge's session. `shell{background:true}` jobs are spawned with `setsid`, so they sit in a
//! session of their own and are left alone, exactly as intended.

#[cfg(target_os = "linux")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ProcInfo {
    pub pid: i32,
    pub ppid: i32,
    pub pgid: i32,
    pub sid: i32,
}

/// Parse one `/proc/<pid>/stat` line. The command name is parenthesised and may itself contain
/// spaces and parentheses, so the fields are located after the LAST `)`.
#[cfg(target_os = "linux")]
pub(super) fn parse_stat(stat: &str) -> Option<ProcInfo> {
    let (head, rest) = stat.split_once(' ')?;
    let pid: i32 = head.parse().ok()?;
    let after = &rest[rest.rfind(')')? + 1..];
    let mut fields = after.split_whitespace();
    let _state = fields.next()?;
    let ppid: i32 = fields.next()?.parse().ok()?;
    let pgid: i32 = fields.next()?.parse().ok()?;
    let sid: i32 = fields.next()?.parse().ok()?;
    Some(ProcInfo {
        pid,
        ppid,
        pgid,
        sid,
    })
}

/// Pure tree walk: every transitive descendant of `root` whose session equals `root`'s own.
#[cfg(target_os = "linux")]
pub(super) fn select_descendants(procs: &[ProcInfo], root: i32) -> Vec<ProcInfo> {
    let Some(root_info) = procs.iter().find(|p| p.pid == root) else {
        return Vec::new();
    };
    let session = root_info.sid;
    let mut found: Vec<ProcInfo> = Vec::new();
    let mut frontier = vec![root];
    while let Some(parent) = frontier.pop() {
        for p in procs.iter().filter(|p| p.ppid == parent) {
            if p.sid != session || found.iter().any(|f| f.pid == p.pid) {
                continue;
            }
            found.push(*p);
            frontier.push(p.pid);
        }
    }
    found
}

/// A snapshot of the bridge root's same-session descendants, to be killed after (or alongside) the
/// root's own group. Empty off Linux, where there is no cheap process table to walk.
#[derive(Debug, Default)]
pub(super) struct Descendants {
    #[cfg(target_os = "linux")]
    procs: Vec<ProcInfo>,
}

impl Descendants {
    pub(super) fn snapshot(root: Option<i32>) -> Self {
        #[cfg(target_os = "linux")]
        {
            let Some(root) = root else {
                return Self::default();
            };
            let mut all = Vec::new();
            if let Ok(dir) = std::fs::read_dir("/proc") {
                for entry in dir.flatten() {
                    let name = entry.file_name();
                    if !name.to_string_lossy().bytes().all(|b| b.is_ascii_digit()) {
                        continue;
                    }
                    if let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) {
                        if let Some(info) = parse_stat(&stat) {
                            all.push(info);
                        }
                    }
                }
            }
            Self {
                procs: select_descendants(&all, root),
            }
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = root;
            Self::default()
        }
    }

    /// SIGKILL every snapshotted process, and its group when it leads one (the shell tool's
    /// per-command groups). ESRCH for already-dead processes is harmless.
    pub(super) fn kill(&self) {
        #[cfg(target_os = "linux")]
        for p in &self.procs {
            unsafe {
                if p.pgid == p.pid {
                    libc::kill(-p.pgid, libc::SIGKILL);
                }
                libc::kill(p.pid, libc::SIGKILL);
            }
        }
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        #[cfg(target_os = "linux")]
        {
            self.procs.len()
        }
        #[cfg(not(target_os = "linux"))]
        {
            0
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    fn p(pid: i32, ppid: i32, pgid: i32, sid: i32) -> ProcInfo {
        ProcInfo {
            pid,
            ppid,
            pgid,
            sid,
        }
    }

    #[test]
    fn parse_stat_handles_parens_and_spaces_in_comm() {
        let line = "4242 (weird) name (x)) S 100 4242 77 0 -1 4194560";
        assert_eq!(parse_stat(line), Some(p(4242, 100, 4242, 77)));
        assert_eq!(parse_stat("garbage"), None);
    }

    #[test]
    fn walk_follows_shell_groups_but_skips_other_sessions() {
        let procs = [
            p(10, 1, 10, 5),   // bridge root
            p(11, 10, 10, 5),  // mcp-serve (same group)
            p(12, 11, 12, 5),  // shell command: own group, same session
            p(13, 12, 12, 5),  // its grandchild
            p(14, 11, 14, 14), // shell{background:true}: setsid, own session
            p(15, 14, 14, 14), // the background job's child
            p(20, 1, 20, 5),   // unrelated, not a descendant
        ];
        let got: Vec<i32> = select_descendants(&procs, 10)
            .iter()
            .map(|p| p.pid)
            .collect();
        let mut sorted = got.clone();
        sorted.sort();
        assert_eq!(sorted, vec![11, 12, 13]);
    }

    fn alive(pid: i32) -> bool {
        unsafe { libc::kill(pid, 0) == 0 }
    }

    /// A bridge root whose child lives in a DIFFERENT process group (the shell tool's per-command
    /// group) must not leave that child behind when the bridge is torn down.
    #[tokio::test]
    async fn snapshot_kill_reaps_a_child_in_its_own_group() {
        use std::process::Stdio;
        use std::time::Duration;
        let dir = tempfile::tempdir().unwrap();
        let pidfile = dir.path().join("pid");
        // `set -m` gives the backgrounded job its own process group, same session.
        let script = format!("set -m; sleep 30 & echo $! > {}; wait", pidfile.display());
        let mut cmd = tokio::process::Command::new("sh");
        cmd.arg("-c")
            .arg(script)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .process_group(0);
        let mut root = cmd.spawn().unwrap();
        let root_pid = root.id().unwrap() as i32;
        let mut child_pid = None;
        for _ in 0..200 {
            if let Ok(s) = std::fs::read_to_string(&pidfile) {
                if let Ok(p) = s.trim().parse::<i32>() {
                    child_pid = Some(p);
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let child_pid = child_pid.expect("child pid");
        let snap = Descendants::snapshot(Some(root_pid));
        assert!(snap.len() >= 1, "the sleep must be found under the root");
        // What the group kill alone does: leaves the other-group child running.
        unsafe { libc::kill(-root_pid, libc::SIGKILL) };
        let _ = root.wait().await;
        assert!(alive(child_pid), "premise: a group kill misses the child");
        snap.kill();
        let mut gone = false;
        for _ in 0..100 {
            if !alive(child_pid) {
                gone = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        if !gone {
            unsafe { libc::kill(child_pid, libc::SIGKILL) };
        }
        assert!(gone, "descendant sweep did not reap the other-group child");
    }
}
