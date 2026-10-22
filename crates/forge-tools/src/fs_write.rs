//! Writing to a file the user owns but has marked read-only (`chmod 444`).
//!
//! Marking a file read-only guards it against accidents, not against its owner: `chmod u+w` is
//! the owner's to run, and so is an editor's save, which replaces the file and keeps its mode.
//! Claude Code's `Edit` behaves that way and so do the edit tools here. A plain `write(2)` to such
//! a file fails with `EACCES`, which sent models on a detour of `ls`, `stat`, `chmod` and a retry,
//! and sometimes to `sudo`. Lifting the bit for the duration of one write and putting the mode
//! back leaves the file exactly as it was apart from its content.
//!
//! This only helps for a file the process owns. Anything else (someone else's file, a read-only
//! mount, an immutable flag) stays an error, and [`explain_denied`] says why in terms a model can
//! act on: report it, do not `chmod` or `sudo` around it.

use std::io;
use std::path::Path;

use crate::ToolError;

/// Write `bytes` to `path`, lifting an owner-read-only bit for the duration of the write when that
/// is what stands in the way. The mode is restored afterwards.
pub fn write_through(path: &Path, bytes: &[u8]) -> io::Result<()> {
    match std::fs::write(path, bytes) {
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
            write_owner_readonly(path, bytes).map_err(|_| error)
        }
        other => other,
    }
}

/// The tool-facing write: the plain write, then the owner-read-only retry, and an error that names
/// the path and what blocks it instead of a bare `Permission denied (os error 13)`.
pub(crate) async fn write(
    path: impl AsRef<Path>,
    bytes: impl AsRef<[u8]>,
) -> Result<(), ToolError> {
    let (path, bytes) = (path.as_ref(), bytes.as_ref());
    let error = match tokio::fs::write(path, bytes).await {
        Ok(()) => return Ok(()),
        Err(error) => error,
    };
    if !is_blocked(&error) {
        return Err(error.into());
    }
    let (owned_path, owned_bytes) = (path.to_path_buf(), bytes.to_vec());
    let retried =
        tokio::task::spawn_blocking(move || write_owner_readonly(&owned_path, &owned_bytes)).await;
    match retried {
        Ok(Ok(())) => Ok(()),
        _ => Err(ToolError::Failed(explain_denied(path, &error))),
    }
}

fn is_blocked(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::PermissionDenied | io::ErrorKind::ReadOnlyFilesystem
    )
}

#[cfg(unix)]
fn write_owner_readonly(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    let meta = std::fs::metadata(path)?;
    // SAFETY: `geteuid` has no preconditions and cannot fail.
    let me = unsafe { libc::geteuid() };
    let mode = meta.permissions().mode();
    if !meta.is_file() || meta.uid() != me || mode & 0o200 != 0 {
        // Not ours to unlock, or already owner-writable, so the denial has another cause.
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode | 0o200))?;
    let written = std::fs::write(path, bytes);
    if let Err(error) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)) {
        tracing::warn!(path = %path.display(), %error, "could not restore the read-only mode after writing");
    }
    written
}

#[cfg(not(unix))]
fn write_owner_readonly(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let original = std::fs::metadata(path)?.permissions();
    if !original.readonly() {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    let mut writable = original.clone();
    writable.set_readonly(false);
    std::fs::set_permissions(path, writable)?;
    let written = std::fs::write(path, bytes);
    if let Err(error) = std::fs::set_permissions(path, original) {
        tracing::warn!(path = %path.display(), %error, "could not restore the read-only attribute after writing");
    }
    written
}

/// Why a write to `path` failed, for the model: the path, the mode and owner that matter, and the
/// instruction to report rather than escalate.
pub fn explain_denied(target: &Path, error: &io::Error) -> String {
    let path = target.display();
    let blocker = if error.kind() == io::ErrorKind::ReadOnlyFilesystem {
        "The file system holding it is mounted read-only.".to_string()
    } else {
        describe_blocker(target)
    };
    format!(
        "cannot write {path}: {error}. {blocker} Do not try to get around this with `chmod`, \
         `chown` or `sudo`; tell the user what blocks the edit and let them decide."
    )
}

#[cfg(unix)]
fn describe_blocker(target: &Path) -> String {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    // SAFETY: `geteuid` has no preconditions and cannot fail.
    let me = unsafe { libc::geteuid() };
    let creating = !target.exists();
    let probe = if creating {
        target
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."))
    } else {
        target
    };
    let Ok(meta) = std::fs::metadata(probe) else {
        return "It could not be inspected.".to_string();
    };
    let mode = meta.permissions().mode() & 0o7777;
    let mine = meta.uid() == me;
    let owner = if mine {
        "owned by you".to_string()
    } else {
        format!("owned by uid {} (you are uid {me})", meta.uid())
    };
    if creating {
        return format!(
            "The directory is mode {mode:04o} and {owner}, so a file cannot be created in it."
        );
    }
    if !mine {
        return format!(
            "The file is mode {mode:04o} and {owner}, so it cannot be changed from here."
        );
    }
    let lifted = if mode & 0o200 == 0 {
        ", but it could not be written even with its read-only bit lifted"
    } else {
        " and is writable by you"
    };
    format!(
        "The file is mode {mode:04o} and {owner}{lifted}; an ACL, an immutable flag or a mount \
         option is in the way."
    )
}

#[cfg(not(unix))]
fn describe_blocker(target: &Path) -> String {
    match std::fs::metadata(target) {
        Ok(meta) if meta.permissions().readonly() => {
            "The file is marked read-only and could not be unlocked.".to_string()
        }
        _ => "The file or its folder is not writable by this user.".to_string(),
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn mode(path: &Path) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o7777
    }

    fn read_only_file(dir: &Path, content: &str) -> std::path::PathBuf {
        let path = dir.join("limits.conf");
        std::fs::write(&path, content).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o444)).unwrap();
        path
    }

    /// Root ignores mode bits, so a "denied" write succeeds and the test proves nothing.
    fn running_as_root() -> bool {
        unsafe { libc::geteuid() == 0 }
    }

    #[test]
    fn writing_through_an_owner_read_only_file_keeps_its_mode() {
        if running_as_root() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let path = read_only_file(dir.path(), "max_connections = 100\n");
        write_through(&path, b"max_connections = 250\n").unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "max_connections = 250\n"
        );
        assert_eq!(mode(&path), 0o444, "the read-only mode is put back");
    }

    #[test]
    fn writing_through_keeps_the_inode_and_follows_a_symlink() {
        if running_as_root() {
            return;
        }
        use std::os::unix::fs::MetadataExt;
        let dir = tempfile::tempdir().unwrap();
        let target = read_only_file(dir.path(), "old");
        let link = dir.path().join("link.conf");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let inode = std::fs::metadata(&target).unwrap().ino();
        write_through(&link, b"new").unwrap();
        assert!(std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "new");
        assert_eq!(std::fs::metadata(&target).unwrap().ino(), inode);
        assert_eq!(mode(&target), 0o444);
    }

    #[test]
    fn a_writable_file_is_written_the_ordinary_way() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        std::fs::write(&path, "a").unwrap();
        let before = mode(&path);
        write_through(&path, b"b").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "b");
        assert_eq!(mode(&path), before);
    }

    #[tokio::test]
    async fn the_tool_write_goes_through_and_a_failure_names_the_blocker() {
        if running_as_root() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let path = read_only_file(dir.path(), "old");
        write(&path, b"new").await.unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");

        let locked = dir.path().join("locked");
        std::fs::create_dir(&locked).unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();
        let blocked = locked.join("new.txt");
        let error = write(&blocked, b"x").await.unwrap_err().to_string();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(error.contains(blocked.to_str().unwrap()), "{error}");
        assert!(error.contains("The directory is mode 0555"), "{error}");
        assert!(error.contains("owned by you"), "{error}");
        assert!(error.contains("Do not try to get around this"), "{error}");
        assert!(!error.contains("chmod +x"), "{error}");
    }

    /// d06: two `chmod 444` config files, edited by the model. Claude Code's `Edit` writes straight
    /// through; every Forge edit tool must too, and leave the files as read-only as it found them.
    #[tokio::test]
    async fn every_edit_tool_writes_through_owner_read_only_files_and_keeps_the_mode() {
        use crate::{AppendFileTool, EditFileTool, MultiEditTool, Tool as _, WriteFileTool};
        use serde_json::json;
        if running_as_root() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let limits = read_only_file(&root, "max_connections = 100\nkeepalive = on\n");
        let pool = root.join("pool.conf");
        std::fs::write(&pool, "pool_size = 8\n").unwrap();
        std::fs::set_permissions(&pool, std::fs::Permissions::from_mode(0o444)).unwrap();
        let notes = root.join("notes.txt");
        std::fs::write(&notes, "a\n").unwrap();
        std::fs::set_permissions(&notes, std::fs::Permissions::from_mode(0o444)).unwrap();
        let (limits_s, pool_s, notes_s) = (
            limits.display().to_string(),
            pool.display().to_string(),
            notes.display().to_string(),
        );

        crate::SESSION_WORKSPACE
            .scope(root.clone(), async {
                EditFileTool
                    .run(&json!({"path": limits_s, "old": "= 100", "new": "= 250"}))
                    .await
                    .unwrap();
                MultiEditTool
                    .run(&json!({"path": pool_s, "edits": [{"old": "= 8", "new": "= 25"}]}))
                    .await
                    .unwrap();
                AppendFileTool
                    .run(&json!({"path": notes_s, "content": "b\n"}))
                    .await
                    .unwrap();
                WriteFileTool
                    .run(&json!({"path": notes_s, "content": "c\n"}))
                    .await
                    .unwrap();
            })
            .await;

        assert_eq!(
            std::fs::read_to_string(&limits).unwrap(),
            "max_connections = 250\nkeepalive = on\n"
        );
        assert_eq!(std::fs::read_to_string(&pool).unwrap(), "pool_size = 25\n");
        assert_eq!(std::fs::read_to_string(&notes).unwrap(), "c\n");
        for file in [&limits, &pool, &notes] {
            assert_eq!(mode(file), 0o444, "{}", file.display());
        }
    }

    #[tokio::test]
    async fn append_to_an_owner_read_only_file_keeps_what_was_there() {
        use crate::{AppendFileTool, Tool as _};
        if running_as_root() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let path = read_only_file(&root, "first\n");
        let path_s = path.display().to_string();
        crate::SESSION_WORKSPACE
            .scope(root, async {
                AppendFileTool
                    .run(&serde_json::json!({"path": path_s, "content": "second\n"}))
                    .await
                    .unwrap();
            })
            .await;
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "first\nsecond\n");
        assert_eq!(mode(&path), 0o444);
    }

    #[test]
    fn the_explanation_never_offers_privilege_escalation() {
        let dir = tempfile::tempdir().unwrap();
        let path = read_only_file(dir.path(), "x");
        let text = explain_denied(&path, &io::Error::from(io::ErrorKind::PermissionDenied));
        for forbidden in ["try `chmod", "prefix with", "try sudo", "run with sudo"] {
            assert!(!text.contains(forbidden), "{text}");
        }
        assert!(text.contains("tell the user"), "{text}");
    }
}
