//! Keeps an installed AppImage launchable on hosts where AppImageLauncher's binfmt handler
//! intercepts it.
//!
//! AppImageLauncher (2.2.x) registers `appimage-type2` in binfmt_misc, so the kernel hands every
//! file carrying the type-2 magic (`AI\x02` at byte 8) to its own, old runtime. That runtime cannot
//! read Forge's zstd squashfs ("This doesn't look like a squashfs image"), and
//! `APPIMAGELAUNCHER_DISABLE` does not bypass it. The AppImage itself is fine: with the three magic
//! bytes zeroed the kernel runs it as an ordinary ELF through its own runtime.
//!
//! Nothing can happen before the first launch (the intercept is in the kernel), so the one-off
//! install needs the manual step in docs/desktop-linux.md. What this module keeps from breaking is
//! every launch after that: an update rewrites the file with the magic back in place, and the
//! updater runs in the old, working process, so it clears the magic on the file it just wrote.
//! The signature was already verified before install; the file is only touched afterwards, and the
//! next update replaces it whole.

use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

pub const KEEP_MAGIC: &str = "FORGE_DESKTOP_KEEP_APPIMAGE_MAGIC";
const BINFMT_ENTRY: &str = "/proc/sys/fs/binfmt_misc/appimage-type2";
const MAGIC_OFFSET: u64 = 8;
const TYPE2_MAGIC: [u8; 3] = [0x41, 0x49, 0x02];

/// True when the binfmt entry is enabled and its interpreter is AppImageLauncher's.
pub fn binfmt_is_appimagelauncher(entry: &str) -> bool {
    let mut lines = entry.lines();
    lines.next().is_some_and(|first| first.trim() == "enabled")
        && lines.any(|line| {
            line.strip_prefix("interpreter")
                .is_some_and(|rest| rest.to_ascii_lowercase().contains("appimagelauncher"))
        })
}

/// Zeroes the type-2 magic of the file at `path`. Returns whether it changed anything.
pub fn clear_type2_magic(path: &Path) -> std::io::Result<bool> {
    let mut file = OpenOptions::new().read(true).write(true).open(path)?;
    file.seek(SeekFrom::Start(MAGIC_OFFSET))?;
    let mut found = [0u8; 3];
    if file.read_exact(&mut found).is_err() || found != TYPE2_MAGIC {
        return Ok(false);
    }
    file.seek(SeekFrom::Start(MAGIC_OFFSET))?;
    file.write_all(&[0; 3])?;
    Ok(true)
}

/// Clears the magic on `$APPIMAGE` when AppImageLauncher's handler would otherwise swallow it.
pub fn neutralize_current() {
    if std::env::var(KEEP_MAGIC).is_ok_and(|v| crate::linux_render::truthy(Some(&v)))
        || std::env::var(crate::linux_render::OPT_OUT)
            .is_ok_and(|v| crate::linux_render::truthy(Some(&v)))
    {
        return;
    }
    let Some(appimage) = std::env::var_os("APPIMAGE") else {
        return;
    };
    let Ok(entry) = std::fs::read_to_string(BINFMT_ENTRY) else {
        return;
    };
    if !binfmt_is_appimagelauncher(&entry) {
        return;
    }
    match clear_type2_magic(Path::new(&appimage)) {
        Ok(true) => eprintln!("forge: cleared the AppImage type-2 magic so AppImageLauncher's binfmt handler does not intercept launches"),
        Ok(false) => {}
        Err(error) => eprintln!("forge: could not clear the AppImage type-2 magic: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "forge-appimage-magic-{name}-{}",
            std::process::id()
        ));
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn only_an_enabled_appimagelauncher_entry_counts() {
        let on = "enabled\ninterpreter /usr/lib/appimagelauncher/binfmt-interpreter\nflags: F\n";
        assert!(binfmt_is_appimagelauncher(on));
        assert!(!binfmt_is_appimagelauncher(
            &on.replace("enabled", "disabled")
        ));
        assert!(!binfmt_is_appimagelauncher(
            "enabled\ninterpreter /usr/bin/appimaged\n"
        ));
        assert!(!binfmt_is_appimagelauncher(""));
    }

    #[test]
    fn clears_exactly_the_three_magic_bytes() {
        let mut image = b"\x7fELF\x02\x01\x01\x00AI\x02rest-of-the-runtime".to_vec();
        let path = scratch("clear", &image);
        assert!(clear_type2_magic(&path).unwrap());
        image[8..11].copy_from_slice(&[0, 0, 0]);
        assert_eq!(std::fs::read(&path).unwrap(), image);
        assert!(!clear_type2_magic(&path).unwrap(), "idempotent");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn leaves_other_files_alone() {
        let plain = b"\x7fELF\x02\x01\x01\x00\x00\x00\x00 payload".to_vec();
        let path = scratch("plain", &plain);
        assert!(!clear_type2_magic(&path).unwrap());
        assert_eq!(std::fs::read(&path).unwrap(), plain);
        let tiny = scratch("tiny", b"ELF");
        assert!(!clear_type2_magic(&tiny).unwrap());
        std::fs::remove_file(path).unwrap();
        std::fs::remove_file(tiny).unwrap();
    }
}
