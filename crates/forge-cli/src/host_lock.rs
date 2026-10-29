//! One `forge serve` per config directory.
//!
//! Two daemons on one config dir fight over the token file, the discovery file and the session
//! store, and the phone ends up paired with whichever answered last. The second one now stops at
//! startup and names the first. The lock is an OS advisory lock on `serve.lock`, so a daemon that
//! dies, however it dies, releases it; there is no stale file to clean up.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, Write};
use std::path::Path;

use anyhow::{Context, Result};
use fs2::FileExt;

const LOCK_FILE: &str = "serve.lock";

/// Held for the life of the daemon; dropping it releases the lock.
pub(crate) struct HostLock {
    _file: File,
}

/// Take the lock for this machine's config directory; `None` when the platform has no such directory.
pub(crate) fn acquire_for_config() -> Result<Option<HostLock>> {
    forge_config::config_dir()
        .map(|dir| acquire(&dir))
        .transpose()
}

pub(crate) fn acquire(config_dir: &Path) -> Result<HostLock> {
    std::fs::create_dir_all(config_dir)
        .with_context(|| format!("creating {}", config_dir.display()))?;
    let path = config_dir.join(LOCK_FILE);
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .with_context(|| format!("opening {}", path.display()))?;
    if file.try_lock_exclusive().is_err() {
        let mut holder = String::new();
        let _ = file.read_to_string(&mut holder);
        anyhow::bail!("{}", refusal(holder.trim(), config_dir));
    }
    file.set_len(0)?;
    file.rewind()?;
    write!(file, "{}", std::process::id())?;
    file.flush()?;
    Ok(HostLock { _file: file })
}

fn refusal(holder: &str, config_dir: &Path) -> String {
    match holder.parse::<u32>() {
        Ok(pid) => format!(
            "another forge serve is already running for {} (pid {pid}); stop it first, or point \
             this one at a different config directory",
            config_dir.display()
        ),
        Err(_) => format!(
            "another forge serve is already running for {}; stop it first, or point this one at a \
             different config directory",
            config_dir.display()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_daemon_is_refused_and_told_who_holds_the_directory() {
        let dir = tempfile::tempdir().unwrap();
        let first = acquire(dir.path()).unwrap();
        let error = acquire(dir.path()).err().expect("second lock must fail");
        let text = format!("{error}");
        assert!(
            text.contains(&format!("pid {}", std::process::id())),
            "names the holder: {text}"
        );
        drop(first);
        // Another test thread may have forked a child while the lock was held; that child lets go
        // of its copy of the descriptor when it execs, a moment later.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let again = loop {
            match acquire(dir.path()) {
                Ok(lock) => break lock,
                Err(error) if std::time::Instant::now() >= deadline => {
                    panic!("released on drop, so a new daemon can start: {error}")
                }
                Err(_) => std::thread::sleep(std::time::Duration::from_millis(20)),
            }
        };
        drop(again);
    }

    #[test]
    fn separate_directories_do_not_contend() {
        let one = tempfile::tempdir().unwrap();
        let two = tempfile::tempdir().unwrap();
        let _a = acquire(one.path()).unwrap();
        let _b = acquire(two.path()).unwrap();
    }

    #[test]
    fn an_unreadable_holder_still_gets_a_refusal() {
        assert!(refusal("", Path::new("/c")).contains("already running"));
        assert!(refusal("41", Path::new("/c")).contains("pid 41"));
    }
}
