//! Test-only helper for materializing fake executables (fake `gh`, hooks, CLIs) race-free.

use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

/// Write `contents` to `path` as an executable file that is safe to `exec` immediately.
///
/// The test binary is multi-threaded: a thread that forks while another thread still holds a
/// write fd on the script gives its child a copy of that fd, and `execve` of a file with an open
/// writer fails with `ETXTBSY`. So the bytes go to a sibling `.src` data file, an external `cp`
/// copies them to `path` (its write fd lives only in `cp`'s descriptor table), and the mode is set
/// with `chmod`, which never opens the file. `path` itself has never had a writer in this process.
/// Scripts only ever *read* by `sh <path>` don't need this.
pub fn install_executable(path: &Path, contents: &str) {
    let mut source = OsString::from(path.as_os_str());
    source.push(".src");
    let source = std::path::PathBuf::from(source);
    std::fs::write(&source, contents).unwrap();
    // A fresh inode: `cp` onto an existing file would reopen it for writing.
    let _ = std::fs::remove_file(path);
    let copied = std::process::Command::new("cp")
        .arg(&source)
        .arg(path)
        .status()
        .expect("run `cp` to install the fake executable");
    assert!(copied.success(), "cp {source:?} -> {path:?} failed");
    let _ = std::fs::remove_file(&source);
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}
