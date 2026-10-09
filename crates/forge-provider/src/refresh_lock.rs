//! Cross-process advisory lock around an OAuth token refresh.
//!
//! The in-process single-flight latch cannot stop the daemon, a CLI run and `mcp-serve` children
//! from each spending the same rotating refresh token: the loser gets `invalid_grant` although the
//! keyring already holds the winner's fresh tokens. An exclusive `flock` on a small file serializes
//! them; whoever gets the lock second re-reads the keyring and finds the work already done.

use std::path::Path;
use std::time::Duration;

/// Held for the duration of a refresh; the lock is released when the file descriptor closes.
#[derive(Debug)]
pub(crate) struct RefreshFileLock {
    _file: std::fs::File,
}

const POLL: Duration = Duration::from_millis(50);

/// Take the exclusive lock at `path`, waiting up to `timeout`. `None` means the lock could not be
/// taken (unsupported platform, unwritable directory, or a peer holding it past `timeout`): the
/// caller proceeds unlocked and relies on reloading the stored tokens after a rejected refresh.
#[cfg(unix)]
pub(crate) async fn acquire(path: &Path, timeout: Duration) -> Option<RefreshFileLock> {
    use std::os::fd::AsRawFd;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok()?;
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)
        .ok()?;
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        // SAFETY: `fd` is a valid descriptor owned by `file` for the duration of the call.
        let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if rc == 0 {
            return Some(RefreshFileLock { _file: file });
        }
        let err = std::io::Error::last_os_error();
        if err.kind() != std::io::ErrorKind::WouldBlock || tokio::time::Instant::now() >= deadline {
            return None;
        }
        tokio::time::sleep(POLL).await;
    }
}

#[cfg(not(unix))]
pub(crate) async fn acquire(_path: &Path, _timeout: Duration) -> Option<RefreshFileLock> {
    None
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn second_holder_waits_until_the_first_releases() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("refresh.lock");
        let first = acquire(&path, Duration::from_secs(1)).await.expect("first");
        assert!(
            acquire(&path, Duration::from_millis(120)).await.is_none(),
            "a held lock must not be granted twice"
        );
        let waiter = tokio::spawn({
            let path = path.clone();
            async move { acquire(&path, Duration::from_secs(5)).await.is_some() }
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        drop(first);
        assert!(waiter.await.unwrap(), "released lock must be granted");
    }
}
