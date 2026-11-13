//! The private temporary copy `E` hands to the user's editor. Created with an unpredictable name
//! in a 0700 directory so another local user cannot pre-plant a symlink at it, and kept on disk
//! until the edit has been saved so a failed save never loses the user's work.

use std::path::{Path, PathBuf};

pub(super) struct EditCopy {
    dir: tempfile::TempDir,
    file: PathBuf,
}

impl EditCopy {
    /// `base` is the directory to create the private directory in (the runtime dir when there is
    /// one, otherwise the system temp dir).
    pub(super) fn create(
        base: &Path,
        name: &std::ffi::OsStr,
        content: &str,
    ) -> std::io::Result<Self> {
        use std::io::Write;
        let dir = tempfile::Builder::new()
            .prefix("forge-ide-")
            .tempdir_in(base)?;
        #[cfg(unix)]
        std::fs::set_permissions(
            dir.path(),
            std::os::unix::fs::PermissionsExt::from_mode(0o700),
        )?;
        let file = dir.path().join(name);
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        options.open(&file)?.write_all(content.as_bytes())?;
        Ok(Self { dir, file })
    }

    pub(super) fn path(&self) -> &Path {
        &self.file
    }

    /// Stop deleting the copy and return where it is.
    pub(super) fn keep(self) -> PathBuf {
        let _ = self.dir.keep();
        self.file
    }
}

pub(super) fn base_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
        .unwrap_or_else(std::env::temp_dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_copy_has_an_unpredictable_private_path() {
        let base = tempfile::tempdir().unwrap();
        let a = EditCopy::create(base.path(), "f.rs".as_ref(), "x").unwrap();
        let b = EditCopy::create(base.path(), "f.rs".as_ref(), "x").unwrap();
        assert_ne!(a.path().parent(), b.path().parent());
        assert!(!a
            .path()
            .to_string_lossy()
            .contains(&std::process::id().to_string()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(a.path()), 0o600);
            assert_eq!(mode(a.path().parent().unwrap()), 0o700);
        }
    }

    #[test]
    fn a_planted_symlink_is_not_followed() {
        let base = tempfile::tempdir().unwrap();
        let victim = base.path().join("victim");
        std::fs::write(&victim, "keep").unwrap();
        let copy = EditCopy::create(base.path(), "f".as_ref(), "new").unwrap();
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "keep");
        assert_eq!(std::fs::read_to_string(copy.path()).unwrap(), "new");
    }

    #[test]
    fn dropping_removes_the_copy_but_keep_preserves_it() {
        let base = tempfile::tempdir().unwrap();
        let gone = EditCopy::create(base.path(), "f".as_ref(), "a").unwrap();
        let gone_path = gone.path().to_path_buf();
        drop(gone);
        assert!(!gone_path.exists());

        let kept = EditCopy::create(base.path(), "f".as_ref(), "edit").unwrap();
        let path = kept.keep();
        assert_eq!(std::fs::read_to_string(path).unwrap(), "edit");
    }
}
