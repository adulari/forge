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

#[tauri::command]
pub fn system_host_name() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .ok()
        .filter(|name| !name.trim().is_empty())
        .or_else(|| std::fs::read_to_string("/etc/hostname").ok())
        .map(|name| {
            name.trim()
                .chars()
                .filter(|character| !character.is_control())
                .take(80)
                .collect()
        })
        .filter(|name: &String| !name.is_empty())
        .unwrap_or_else(|| "localhost".to_string())
}

#[derive(serde::Deserialize)]
pub struct AnywhereHostState {
    version: u8,
    account_id: String,
    github_login: Option<String>,
    device_id: String,
    signing_private_key: String,
    exchange_private_key: String,
    account_data_key: String,
    key_epoch: u32,
    data_key_epochs: std::collections::BTreeMap<u32, String>,
    access_token: String,
    refresh_token: String,
    access_expires_at_ms: u64,
    next_sequence: u64,
}

impl AnywhereHostState {
    fn validate(&self) -> Result<(), String> {
        if self.version != 1
            || self.account_id.len() != 32
            || self.device_id.len() != 32
            || !self.account_id.bytes().all(|byte| byte.is_ascii_hexdigit())
            || !self.device_id.bytes().all(|byte| byte.is_ascii_hexdigit())
            || self.signing_private_key.is_empty()
            || self.exchange_private_key.is_empty()
            || self.account_data_key.is_empty()
            || self.access_token.is_empty()
            || self.refresh_token.is_empty()
            || self.key_epoch == 0
            || !self.data_key_epochs.contains_key(&self.key_epoch)
        {
            return Err("the local host enrollment state is invalid".to_string());
        }
        Ok(())
    }

    fn into_json(self) -> serde_json::Value {
        serde_json::json!({
            "version": self.version,
            "account_id": self.account_id,
            "github_login": self.github_login,
            "device_id": self.device_id,
            "signing_private_key": self.signing_private_key,
            "exchange_private_key": self.exchange_private_key,
            "account_data_key": self.account_data_key,
            "key_epoch": self.key_epoch,
            "data_key_epochs": self.data_key_epochs,
            "access_token": self.access_token,
            "refresh_token": self.refresh_token,
            "access_expires_at_ms": self.access_expires_at_ms,
            "host_id": null,
            "next_sequence": self.next_sequence,
            "accepted_sequences": {},
            "command_journal": {},
            "capsule_journal": {},
            "capsule_replay": {},
            "outgoing_handoffs": {},
            "preparing_handoffs": {},
            "refresh_lease_id": null,
            "refresh_lease_until_ms": 0
        })
    }
}

fn anywhere_state_path() -> Option<std::path::PathBuf> {
    directories::ProjectDirs::from("dev", "forge", "forge")
        .map(|directories| directories.data_dir().join("anywhere").join("state.json"))
}

#[tauri::command]
pub fn forge_anywhere_host_enrolled() -> bool {
    let Some(path) = anywhere_state_path() else {
        return false;
    };
    let Ok(contents) = std::fs::read_to_string(path) else {
        return false;
    };
    serde_json::from_str::<serde_json::Value>(&contents)
        .ok()
        .and_then(|state| {
            state
                .get("refresh_token")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
        .is_some_and(|token| !token.is_empty())
}

/// Install a distinct, encrypted-pairing-derived CLI identity and activate it as a host.
/// Existing CLI enrollment is never overwritten.
#[tauri::command]
pub fn install_forge_anywhere_host(state: AnywhereHostState, name: String) -> Result<(), String> {
    state.validate()?;
    let host_name = name.trim();
    if host_name.is_empty()
        || host_name.chars().count() > 80
        || host_name.chars().any(char::is_control)
    {
        return Err("enter a host name with at most 80 visible characters".to_string());
    }
    if forge_anywhere_host_enrolled() {
        return Err("Forge CLI is already enrolled on this computer".to_string());
    }
    let path =
        anywhere_state_path().ok_or_else(|| "Forge data directory is unavailable".to_string())?;
    let parent = path
        .parent()
        .ok_or_else(|| "Forge Anywhere state path is invalid".to_string())?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("create Forge Anywhere state directory: {error}"))?;
    set_owner_directory(parent)?;
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| format!("create protected host state timestamp: {error}"))?
        .as_nanos();
    let temporary = parent.join(format!(".desktop-state-{}-{nonce}.tmp", std::process::id()));
    let bytes = serde_json::to_vec_pretty(&state.into_json())
        .map_err(|error| format!("serialize Forge Anywhere host state: {error}"))?;
    write_owner_file(&temporary, &bytes)?;
    if let Err(error) = std::fs::rename(&temporary, &path) {
        let _ = std::fs::remove_file(&temporary);
        return Err(format!("install Forge Anywhere host state: {error}"));
    }
    set_owner_file(&path)?;
    activate_forge_anywhere_host(host_name.to_string())
}

#[tauri::command]
pub fn activate_forge_anywhere_host(name: String) -> Result<(), String> {
    let host_name = name.trim();
    if host_name.is_empty()
        || host_name.chars().count() > 80
        || host_name.chars().any(char::is_control)
    {
        return Err("enter a host name with at most 80 visible characters".to_string());
    }
    let forge_binary =
        find_forge_binary().ok_or_else(|| "Forge is not installed on this computer".to_string())?;
    let output = std::process::Command::new(&forge_binary)
        .args(["anywhere", "enable", "--name", host_name])
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|error| format!("start Forge Anywhere host activation: {error}"))?;
    if output.status.success() {
        return Ok(());
    }
    Err(
        "Forge Anywhere could not activate this host. Check the Forge Anywhere screen and retry."
            .to_string(),
    )
}

#[cfg(unix)]
fn set_owner_directory(path: &std::path::Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
        .map_err(|error| format!("protect Forge Anywhere directory: {error}"))
}

#[cfg(not(unix))]
fn set_owner_directory(_path: &std::path::Path) -> Result<(), String> {
    Ok(())
}

#[cfg(unix)]
fn set_owner_file(path: &std::path::Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .map_err(|error| format!("protect Forge Anywhere state: {error}"))
}

#[cfg(not(unix))]
fn set_owner_file(_path: &std::path::Path) -> Result<(), String> {
    Ok(())
}

fn write_owner_file(path: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|error| format!("create protected host state: {error}"))?;
    use std::io::Write as _;
    file.write_all(bytes)
        .map_err(|error| format!("write protected host state: {error}"))?;
    file.sync_all()
        .map_err(|error| format!("sync protected host state: {error}"))
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
