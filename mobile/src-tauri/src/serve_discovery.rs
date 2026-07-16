// Desktop auto-detect + offer-to-start (ARCHITECTURE.md §6 Tauri desktop shell). The webview
// has no fs/shell plugin grant (capabilities/default.json) — these three narrow commands are
// the only way the desktop app can see or start a local `forge serve` daemon, instead of
// requiring the user to paste a connect URL every time.
//
// `forge serve` writes `<config_dir>/serve-state.json` right after a successful bind
// (crates/forge-cli/src/serve.rs) and removes it on a graceful Ctrl-C shutdown. This mirrors
// that struct's JSON shape exactly so `detect_forge_serve` can deserialize it directly.

/// Mirrors `forge_cli::serve::ServeState` field-for-field. Kept as a plain struct here (rather
/// than a shared crate) since src-tauri is deliberately its own standalone cargo workspace
/// (see Cargo.toml) and pulling in forge-cli would drag the whole CLI + its dependency tree
/// into the desktop shell.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
pub struct ServeState {
    pid: u32,
    port: u16,
    /// `"local"` | `"lan"` | `"anywhere"`. The desktop webview rejects the `--lan` self-signed
    /// certificate, so the frontend must not auto-connect to a `"lan"` state — it only surfaces
    /// a hint. That policy lives in TS (connect.tsx), not here: this command reports the raw
    /// state and lets the caller decide.
    exposure: String,
    base_url: String,
    token: String,
    started_at: u64,
}

/// Per-OS config dir, resolved the same way `forge_config::config_dir()` does.
fn config_dir() -> Option<std::path::PathBuf> {
    directories::ProjectDirs::from("dev", "forge", "forge").map(|d| d.config_dir().to_path_buf())
}

/// Whether `pid` names a live process. Unix: `kill(pid, 0)` sends no signal, it only asks the
/// kernel whether the pid exists and is signalable by us — the standard zero-cost liveness
/// probe. Windows has no equally cheap syscall available without another dependency, so it
/// always reports alive here and leans on the TCP port probe below as the real liveness signal
/// (a dead daemon has nothing listening).
#[cfg(unix)]
fn pid_alive(pid: u32) -> bool {
    // SAFETY: signal 0 is documented as a no-op existence check; `pid` is a plain integer, no
    // aliasing/lifetime concerns.
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

#[cfg(windows)]
fn pid_alive(_pid: u32) -> bool {
    true
}

/// ~300ms-bounded TCP connect — confirms something is actually listening, not just that the pid
/// happens to be alive (e.g. reused after a crash before the state file was cleaned up).
fn probe_port(port: u16) -> bool {
    use std::net::{SocketAddr, TcpStream};
    let addr: SocketAddr = ([127, 0, 0, 1], port).into();
    TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(300)).is_ok()
}

/// Reads and validates `serve-state.json`, returning `None` for anything short of "a live
/// daemon is definitely listening on this port": missing file, unparseable JSON, dead pid, or a
/// silent port. Advisory data only — never trust it without this validation.
fn read_and_validate(dir: &std::path::Path) -> Option<ServeState> {
    let raw = std::fs::read_to_string(dir.join("serve-state.json")).ok()?;
    let state: ServeState = serde_json::from_str(&raw).ok()?;
    if !pid_alive(state.pid) {
        return None;
    }
    if !probe_port(state.port) {
        return None;
    }
    Some(state)
}

#[tauri::command]
pub fn detect_forge_serve() -> Option<ServeState> {
    read_and_validate(&config_dir()?)
}

/// Bare executable name to look for on `PATH` — `forge.exe` on Windows, `forge` elsewhere.
fn forge_exe_name() -> &'static str {
    if cfg!(windows) {
        "forge.exe"
    } else {
        "forge"
    }
}

fn is_executable_file(path: &std::path::Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    true
}

/// [`find_forge_binary`] against an explicit `PATH` value (unit-testable without mutating the
/// process-wide environment).
fn find_forge_binary_in(path_var: &std::ffi::OsStr) -> Option<std::path::PathBuf> {
    let exe_name = forge_exe_name();
    std::env::split_paths(path_var).find_map(|dir| {
        let candidate = dir.join(exe_name);
        is_executable_file(&candidate).then_some(candidate)
    })
}

fn common_forge_binary_candidates(home: Option<&std::path::Path>) -> Vec<std::path::PathBuf> {
    let mut candidates = Vec::new();
    if let Some(home) = home {
        candidates.push(home.join(".cargo/bin").join(forge_exe_name()));
        candidates.push(home.join(".local/bin").join(forge_exe_name()));
    }
    #[cfg(unix)]
    {
        candidates.extend(
            [
                "/opt/homebrew/bin",
                "/usr/local/bin",
                "/home/linuxbrew/.linuxbrew/bin",
            ]
            .into_iter()
            .map(|dir| std::path::Path::new(dir).join(forge_exe_name())),
        );
    }
    candidates
}

/// GUI apps inherit a minimal `PATH` on macOS and Linux, so search both that path and the
/// standard Cargo, user-local, and Homebrew install locations.
fn find_forge_binary() -> Option<std::path::PathBuf> {
    if let Some(path_var) = std::env::var_os("PATH") {
        if let Some(binary) = find_forge_binary_in(&path_var) {
            return Some(binary);
        }
    }

    let user_dirs = directories::UserDirs::new();
    common_forge_binary_candidates(user_dirs.as_ref().map(directories::UserDirs::home_dir))
        .into_iter()
        .find(|candidate| is_executable_file(candidate))
}

#[tauri::command]
pub fn forge_binary_available() -> bool {
    find_forge_binary().is_some()
}

/// Spawns `forge serve --local` detached (null stdio, no window on Windows) and returns
/// immediately — it does NOT wait for the bind. The frontend polls `detect_forge_serve` until
/// `serve-state.json` appears (or a ~15s timeout), since that file is only written after a
/// successful bind. `--local` only: `--lan`'s self-signed cert is rejected by this webview, and
/// `--anywhere` needs a tunnel tool + is a much bigger commitment than a first-run "start it for
/// me" click.
#[tauri::command]
pub fn start_forge_serve() -> Result<(), String> {
    let forge_binary = find_forge_binary()
        .ok_or_else(|| "forge executable was not found in common install locations".to_string())?;
    let mut cmd = std::process::Command::new(&forge_binary);
    cmd.args(["serve", "--local"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.spawn().map_err(|e| {
        format!(
            "failed to start `{}` serve --local: {e}",
            forge_binary.display()
        )
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serve_state_parses_the_shape_forge_serve_writes() {
        let json = r#"{
            "pid": 12345,
            "port": 7452,
            "exposure": "local",
            "base_url": "http://127.0.0.1:7452/deadbeefdeadbeef",
            "token": "deadbeefdeadbeef",
            "started_at": 1700000000
        }"#;
        let state: ServeState = serde_json::from_str(json).unwrap();
        assert_eq!(state.pid, 12345);
        assert_eq!(state.port, 7452);
        assert_eq!(state.exposure, "local");
        assert_eq!(state.base_url, "http://127.0.0.1:7452/deadbeefdeadbeef");
    }

    #[test]
    fn read_and_validate_returns_none_when_the_file_is_absent() {
        let dir = std::env::temp_dir().join(format!("forge-desktop-detect-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(read_and_validate(&dir).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_and_validate_rejects_a_dead_pid() {
        let dir =
            std::env::temp_dir().join(format!("forge-desktop-detect2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // A pid vanishingly unlikely to be alive, paired with a port nothing listens on either
        // — both liveness checks must independently reject this, so pick a value that fails the
        // pid check even if the port check were skipped.
        std::fs::write(
            dir.join("serve-state.json"),
            r#"{"pid":2147483647,"port":1,"exposure":"local","base_url":"http://127.0.0.1:1/x","token":"x","started_at":0}"#,
        )
        .unwrap();
        assert!(read_and_validate(&dir).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn find_forge_binary_locates_an_executable_on_path() {
        let dir = std::env::temp_dir().join(format!("forge-desktop-which-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join(forge_exe_name());
        std::fs::write(&exe, b"").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let path_var = std::env::join_paths([dir.clone()]).unwrap();
        let found = find_forge_binary_in(&path_var);

        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(found, Some(exe));
    }

    #[test]
    fn find_forge_binary_returns_none_when_no_path_entry_has_it() {
        let dir =
            std::env::temp_dir().join(format!("forge-desktop-which-empty-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path_var = std::env::join_paths([dir.clone()]).unwrap();
        assert_eq!(find_forge_binary_in(&path_var), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn common_candidates_include_cargo_and_user_local_bins() {
        let home = std::path::Path::new("/tmp/forge-test-home");
        let candidates = common_forge_binary_candidates(Some(home));
        assert!(candidates.contains(&home.join(".cargo/bin").join(forge_exe_name())));
        assert!(candidates.contains(&home.join(".local/bin").join(forge_exe_name())));
    }

    #[cfg(unix)]
    #[test]
    fn find_forge_binary_ignores_non_executable_files() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!(
            "forge-desktop-which-not-executable-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join(forge_exe_name());
        std::fs::write(&exe, b"").unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o644)).unwrap();

        let path_var = std::env::join_paths([dir.clone()]).unwrap();
        assert_eq!(find_forge_binary_in(&path_var), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
