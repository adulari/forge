// Desktop onboarding phase 0: the app has no `forge` binary or background service yet, so there
// is no daemon to talk to. These commands install the CLI from the latest GitHub release (checked
// against the release's checksum manifest, exactly like install.sh), install and start the
// service, and report what is on this machine. Everything after that goes through the daemon.
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use tauri::Emitter as _;
use tauri_plugin_http::reqwest;

use crate::serve_discovery::{config_dir, find_forge_binary, read_and_validate, ServeState};

pub const PROGRESS_EVENT: &str = "onboarding://install-progress";
const DEFAULT_BASE: &str = "https://github.com/Adulari/forge/releases/latest/download";
const MANIFESTS: [&str; 2] = ["checksums.txt", "SHA256SUMS"];
const MAX_BINARY_BYTES: u64 = 512 * 1024 * 1024;
const SERVICE_WAIT: Duration = Duration::from_secs(25);

#[derive(serde::Serialize, Clone, Debug)]
pub struct Progress {
    pub phase: &'static str,
    pub fraction: Option<f32>,
    pub message: String,
}

fn progress(
    app: &tauri::AppHandle,
    phase: &'static str,
    fraction: Option<f32>,
    message: impl Into<String>,
) {
    let _ = app.emit(
        PROGRESS_EVENT,
        Progress {
            phase,
            fraction,
            message: message.into(),
        },
    );
}

#[derive(serde::Serialize, Debug, Default, PartialEq)]
pub struct ServiceFacts {
    pub supported: bool,
    pub installed: bool,
    pub running: bool,
}

#[derive(serde::Serialize, Debug)]
pub struct PlatformFacts {
    pub os: &'static str,
    pub arch: &'static str,
}

#[derive(serde::Serialize, Debug)]
pub struct LocalStatus {
    pub forge_binary: Option<String>,
    pub forge_version: Option<String>,
    pub serve: Option<ServeState>,
    pub service: ServiceFacts,
    pub platform: PlatformFacts,
    pub install_dir: String,
}

#[derive(serde::Serialize, Debug)]
pub struct InstallResult {
    pub path: String,
    pub version: Option<String>,
}

#[derive(serde::Serialize, Debug)]
pub struct ServiceResult {
    pub running: bool,
    pub serve: Option<ServeState>,
}

/// Where the CLI is installed: `FORGE_INSTALL_DIR`, else `~/.local/bin` (`%LOCALAPPDATA%\Programs\forge` on Windows).
pub(crate) fn install_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("FORGE_INSTALL_DIR").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    if cfg!(windows) {
        let base = std::env::var_os("LOCALAPPDATA")?;
        return Some(PathBuf::from(base).join("Programs").join("forge"));
    }
    directories::UserDirs::new().map(|d| d.home_dir().join(".local").join("bin"))
}

/// The release target triple, as `install.sh` / `install.ps1` pick it.
pub(crate) fn target_triple(os: &str, arch: &str) -> Result<&'static str, String> {
    match (os, arch) {
        ("linux", "x86_64") => Ok("x86_64-unknown-linux-gnu"),
        ("linux", "aarch64") => Ok("aarch64-unknown-linux-gnu"),
        ("macos", "aarch64") => Ok("aarch64-apple-darwin"),
        ("macos", "x86_64") => Ok("x86_64-apple-darwin"),
        ("windows", "x86_64") => Ok("x86_64-pc-windows-msvc"),
        _ => Err(format!(
            "there is no prebuilt Forge for {os} {arch}; build it from source with cargo"
        )),
    }
}

fn asset_name(target: &str) -> String {
    if target.contains("windows") {
        format!("forge-{target}.zip")
    } else {
        format!("forge-{target}.tar.gz")
    }
}

/// The SHA-256 the manifest lists for `asset` (lines are `<hex>  <name>` or `<hex> *<name>`).
pub(crate) fn expected_hash(manifest: &str, asset: &str) -> Result<String, String> {
    for line in manifest.lines() {
        let mut parts = line.split_whitespace();
        let (Some(hash), Some(name)) = (parts.next(), parts.next()) else {
            continue;
        };
        if name.trim_start_matches('*') == asset {
            return if hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()) {
                Ok(hash.to_ascii_lowercase())
            } else {
                Err(format!(
                    "the checksum manifest has no valid hash for {asset}"
                ))
            };
        }
    }
    Err(format!("the checksum manifest does not list {asset}"))
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn exe_name_for(target: &str) -> &'static str {
    if target.contains("windows") {
        "forge.exe"
    } else {
        "forge"
    }
}

/// The `forge` executable inside the release archive (`forge-<target>/forge[.exe]`).
pub(crate) fn extract_binary(archive: &[u8], target: &str) -> Result<Vec<u8>, String> {
    let wanted = format!("forge-{target}/{}", exe_name_for(target));
    let mut out = Vec::new();
    if target.contains("windows") {
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(archive))
            .map_err(|_| "the downloaded archive is not a valid zip".to_string())?;
        let file = zip
            .by_name(&wanted)
            .map_err(|_| format!("the archive does not contain {wanted}"))?;
        file.take(MAX_BINARY_BYTES)
            .read_to_end(&mut out)
            .map_err(|_| "could not unpack the downloaded archive".to_string())?;
    } else {
        let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(archive));
        let entries = tar
            .entries()
            .map_err(|_| "the downloaded archive is not a valid tar.gz".to_string())?;
        let mut found = false;
        for entry in entries {
            let entry = entry.map_err(|_| "could not unpack the downloaded archive".to_string())?;
            let path = entry
                .path()
                .map_err(|_| "the archive has an unreadable entry".to_string())?;
            if path.to_string_lossy().trim_start_matches("./") == wanted {
                entry
                    .take(MAX_BINARY_BYTES)
                    .read_to_end(&mut out)
                    .map_err(|_| "could not unpack the downloaded archive".to_string())?;
                found = true;
                break;
            }
        }
        if !found {
            return Err(format!("the archive does not contain {wanted}"));
        }
    }
    if out.is_empty() {
        return Err("the forge binary in the archive is empty".to_string());
    }
    Ok(out)
}

/// Write `bytes` to `dir/name` through a temp file in the same directory and a rename, so a
/// running binary is replaced atomically and a failure leaves nothing half-written.
pub(crate) fn place_binary(dir: &Path, name: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    let temp = dir.join(format!(".{name}.tmp-{}", std::process::id()));
    let write = || -> std::io::Result<()> {
        std::fs::write(&temp, bytes)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(0o755))?;
        }
        std::fs::rename(&temp, dir.join(name))
    };
    if let Err(e) = write() {
        let _ = std::fs::remove_file(&temp);
        return Err(format!("could not install into {}: {e}", dir.display()));
    }
    Ok(dir.join(name))
}

fn short(text: &str) -> String {
    text.lines()
        .next()
        .unwrap_or("")
        .chars()
        .take(160)
        .collect()
}

async fn fetch_text(client: &reqwest::Client, url: &str) -> Result<Option<String>, String> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("could not reach {url}: {}", short(&e.to_string())))?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !response.status().is_success() {
        return Err(format!("{url} answered {}", response.status()));
    }
    response
        .text()
        .await
        .map(Some)
        .map_err(|_| "the checksum manifest could not be read".to_string())
}

type Emit<'a> = &'a (dyn Fn(&'static str, Option<f32>, String) + Sync);

/// Download, verify and install. `emit` receives each progress step. Nothing is written to
/// `dir` unless the archive's hash matches the manifest.
pub(crate) async fn install_from(
    base: &str,
    target: &str,
    dir: &Path,
    emit: Emit<'_>,
) -> Result<InstallResult, String> {
    let base = base.trim_end_matches('/');
    let asset = asset_name(target);
    let client = reqwest::Client::builder()
        .user_agent("forge-desktop-onboarding")
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(900))
        .build()
        .map_err(|_| "could not start the download".to_string())?;

    emit("resolve", None, "Finding the latest Forge release".into());
    let mut manifest = None;
    for name in MANIFESTS {
        if let Some(text) = fetch_text(&client, &format!("{base}/{name}")).await? {
            if !text.trim().is_empty() {
                manifest = Some(text);
                break;
            }
        }
    }
    let manifest =
        manifest.ok_or("the release has no checksum manifest, so it cannot be verified")?;
    let want = expected_hash(&manifest, &asset)?;

    emit("download", Some(0.0), format!("Downloading {asset}"));
    let mut response = client
        .get(format!("{base}/{asset}"))
        .send()
        .await
        .map_err(|e| format!("download failed: {}", short(&e.to_string())))?;
    if !response.status().is_success() {
        return Err(format!(
            "download failed: the server answered {}",
            response.status()
        ));
    }
    let total = response.content_length();
    let mut bytes: Vec<u8> = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| format!("download failed: {}", short(&e.to_string())))?
    {
        bytes.extend_from_slice(&chunk);
        if bytes.len() as u64 > MAX_BINARY_BYTES {
            return Err("the download is larger than any Forge release".to_string());
        }
        let fraction = total.map(|t| (bytes.len() as f32 / t.max(1) as f32).min(1.0));
        emit("download", fraction, format!("Downloading {asset}"));
    }

    emit("verify", None, "Checking the download".into());
    if sha256_hex(&bytes) != want {
        return Err(format!(
            "checksum mismatch for {asset}; nothing was installed"
        ));
    }

    emit(
        "install",
        None,
        format!("Installing into {}", dir.display()),
    );
    let binary = extract_binary(&bytes, target)?;
    let path = place_binary(dir, exe_name_for(target), &binary)?;
    let version = forge_version(&path);
    emit("done", Some(1.0), "Forge is installed".into());
    Ok(InstallResult {
        path: path.display().to_string(),
        version,
    })
}

/// `forge --version` → the semver token, within 3 seconds.
fn forge_version(binary: &Path) -> Option<String> {
    let mut child = Command::new(binary)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match child.try_wait().ok()? {
            Some(_) => break,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    }
    let mut text = String::new();
    child.stdout.take()?.read_to_string(&mut text).ok()?;
    parse_version(&text)
}

pub(crate) fn parse_version(text: &str) -> Option<String> {
    text.split_whitespace()
        .map(|t| t.trim_start_matches('v'))
        .find(|t| {
            t.split('.').count() >= 3
                && t.split('.')
                    .all(|p| p.chars().next().is_some_and(|c| c.is_ascii_digit()))
        })
        .map(str::to_string)
}

/// Parse `forge service status` output (`installed: yes`, `running:   no`, ...).
pub(crate) fn parse_service_status(text: &str) -> ServiceFacts {
    let flag = |key: &str| {
        text.lines()
            .any(|l| l.strip_prefix(key).is_some_and(|rest| rest.trim() == "yes"))
    };
    ServiceFacts {
        supported: true,
        installed: flag("installed:"),
        running: flag("running:"),
    }
}

const SERVICE_SUPPORTED: bool = cfg!(any(
    target_os = "linux",
    target_os = "macos",
    target_os = "windows"
));

fn service_facts(forge: &Path) -> ServiceFacts {
    if !SERVICE_SUPPORTED {
        return ServiceFacts::default();
    }
    // The command exits non-zero when the service is down, but still prints both flags.
    match Command::new(forge)
        .args(["service", "status"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
    {
        Ok(out) => parse_service_status(&String::from_utf8_lossy(&out.stdout)),
        Err(_) => ServiceFacts {
            supported: true,
            ..ServiceFacts::default()
        },
    }
}

#[tauri::command(async)]
pub fn onboarding_local_status() -> LocalStatus {
    let binary = find_forge_binary();
    let (version, service) = match &binary {
        Some(path) => (forge_version(path), service_facts(path)),
        None => (
            None,
            ServiceFacts {
                supported: SERVICE_SUPPORTED,
                ..ServiceFacts::default()
            },
        ),
    };
    LocalStatus {
        forge_binary: binary.map(|p| p.display().to_string()),
        forge_version: version,
        serve: config_dir().and_then(|d| read_and_validate(&d)),
        service,
        platform: PlatformFacts {
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
        },
        install_dir: install_dir()
            .map(|d| d.display().to_string())
            .unwrap_or_default(),
    }
}

#[tauri::command(async)]
pub async fn install_forge_cli(app: tauri::AppHandle) -> Result<InstallResult, String> {
    let result = async {
        let target = target_triple(std::env::consts::OS, std::env::consts::ARCH)?;
        let dir = install_dir().ok_or("cannot work out where to install Forge")?;
        let base = std::env::var("FORGE_RELEASE_BASE_URL")
            .ok()
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| DEFAULT_BASE.to_string());
        let emitter = app.clone();
        install_from(&base, target, &dir, &move |phase, fraction, message| {
            progress(&emitter, phase, fraction, message)
        })
        .await
    }
    .await;
    if let Err(message) = &result {
        progress(&app, "failed", None, message.clone());
    }
    result
}

fn last_line(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .map(|l| l.trim().chars().take(240).collect())
        .unwrap_or_default()
}

fn run_forge(forge: &Path, args: &[&str]) -> Result<(), String> {
    let mut cmd = Command::new(forge);
    cmd.args(args).stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd
        .output()
        .map_err(|e| format!("could not run forge: {e}"))?;
    if out.status.success() {
        return Ok(());
    }
    let detail = last_line(&out.stderr);
    Err(if detail.is_empty() {
        format!("`forge {}` failed", args.join(" "))
    } else {
        detail
    })
}

#[tauri::command(async)]
pub async fn install_forge_service(app: tauri::AppHandle) -> Result<ServiceResult, String> {
    let result = async {
        let forge = find_forge_binary().ok_or("Forge is not installed on this computer")?;
        progress(
            &app,
            "service-install",
            None,
            "Installing the background service",
        );
        let install_path = forge.clone();
        tauri::async_runtime::spawn_blocking(move || {
            run_forge(&install_path, &["service", "install", "--local"])
        })
        .await
        .map_err(|e| e.to_string())??;
        progress(
            &app,
            "service-start",
            None,
            "Starting the background service",
        );
        let start_path = forge.clone();
        // `service install` already starts it; a failed explicit start only matters if the
        // daemon never appears below.
        let start_error = tauri::async_runtime::spawn_blocking(move || {
            run_forge(&start_path, &["service", "start"]).err()
        })
        .await
        .map_err(|e| e.to_string())?;
        progress(&app, "wait", None, "Waiting for Forge to come up");
        let deadline = Instant::now() + SERVICE_WAIT;
        loop {
            if let Some(serve) = config_dir().and_then(|d| read_and_validate(&d)) {
                progress(&app, "done", Some(1.0), "The background service is running");
                return Ok(ServiceResult {
                    running: true,
                    serve: Some(serve),
                });
            }
            if Instant::now() >= deadline {
                return Err(start_error.unwrap_or_else(|| {
                    "the background service did not start in time".to_string()
                }));
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }
    .await;
    if let Err(message) = &result {
        progress(&app, "failed", None, message.clone());
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;
    use std::net::TcpListener;
    use std::sync::Arc;

    fn tar_gz(target: &str, body: &[u8]) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_size(body.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        builder
            .append_data(&mut header, format!("forge-{target}/forge"), body)
            .unwrap();
        let raw = builder.into_inner().unwrap();
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gz.write_all(&raw).unwrap();
        gz.finish().unwrap()
    }

    fn zip_of(target: &str, body: &[u8]) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        zip.start_file(
            format!("forge-{target}/forge.exe"),
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(body).unwrap();
        zip.finish().unwrap().into_inner()
    }

    fn tempdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("forge-onb-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn targets_match_the_installers() {
        assert_eq!(
            target_triple("linux", "x86_64").unwrap(),
            "x86_64-unknown-linux-gnu"
        );
        assert_eq!(
            target_triple("macos", "aarch64").unwrap(),
            "aarch64-apple-darwin"
        );
        assert_eq!(
            target_triple("windows", "x86_64").unwrap(),
            "x86_64-pc-windows-msvc"
        );
        assert!(target_triple("windows", "aarch64").is_err());
        assert_eq!(
            asset_name("x86_64-pc-windows-msvc"),
            "forge-x86_64-pc-windows-msvc.zip"
        );
        assert_eq!(
            asset_name("aarch64-apple-darwin"),
            "forge-aarch64-apple-darwin.tar.gz"
        );
    }

    #[test]
    fn manifest_lines_parse_in_both_styles_and_fail_closed() {
        let h = "a".repeat(64);
        let manifest = format!(
            "{h}  forge-x.tar.gz\n{} *forge-y.zip\nnoise\n",
            "B".repeat(64)
        );
        assert_eq!(expected_hash(&manifest, "forge-x.tar.gz").unwrap(), h);
        assert_eq!(
            expected_hash(&manifest, "forge-y.zip").unwrap(),
            "b".repeat(64)
        );
        assert!(expected_hash(&manifest, "forge-z.zip").is_err());
        assert!(expected_hash("abc  forge-x.tar.gz", "forge-x.tar.gz").is_err());
    }

    #[test]
    fn sha256_matches_a_known_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn archives_unpack_the_forge_entry_only() {
        let t = "x86_64-unknown-linux-gnu";
        assert_eq!(extract_binary(&tar_gz(t, b"bin"), t).unwrap(), b"bin");
        assert!(extract_binary(&tar_gz("other", b"bin"), t).is_err());
        let w = "x86_64-pc-windows-msvc";
        assert_eq!(extract_binary(&zip_of(w, b"exe"), w).unwrap(), b"exe");
        assert!(extract_binary(b"not an archive", t).is_err());
    }

    #[test]
    fn versions_and_service_status_parse() {
        assert_eq!(parse_version("forge 2.23.6\n").as_deref(), Some("2.23.6"));
        assert_eq!(
            parse_version("forge v1.0.0-rc1").as_deref(),
            Some("1.0.0-rc1")
        );
        assert_eq!(parse_version("nothing"), None);
        let s = parse_service_status("installed: yes\nrunning:   no\nport 7420:  not responding\n");
        assert!(s.installed && !s.running && s.supported);
    }

    #[cfg(unix)]
    #[test]
    fn placing_is_atomic_executable_and_leaves_no_temp_files() {
        let dir = tempdir("place").join("bin");
        let path = place_binary(&dir, "forge", b"one").unwrap();
        place_binary(&dir, "forge", b"two").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"two");
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o755
        );
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
    }

    /// Serves fixed bodies by path; unknown paths answer 404.
    fn serve(files: Vec<(String, Vec<u8>)>) -> String {
        use std::io::Read as _;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let files = Arc::new(files);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let files = files.clone();
                std::thread::spawn(move || {
                    let mut buf = [0u8; 4096];
                    let n = stream.read(&mut buf).unwrap_or(0);
                    let req = String::from_utf8_lossy(&buf[..n]).to_string();
                    let path = req.split_whitespace().nth(1).unwrap_or("/").to_string();
                    let body = files.iter().find(|(p, _)| *p == path).map(|(_, b)| b);
                    let head = match body {
                        Some(b) => format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            b.len()
                        ),
                        None => "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                            .to_string(),
                    };
                    let _ = stream.write_all(head.as_bytes());
                    if let Some(b) = body {
                        let _ = stream.write_all(b);
                    }
                });
            }
        });
        format!("http://127.0.0.1:{port}")
    }

    #[cfg(unix)]
    fn run_install(files: Vec<(String, Vec<u8>)>, dir: &Path) -> Result<InstallResult, String> {
        let base = serve(files);
        let events = std::sync::Mutex::new(Vec::new());
        let result = tauri::async_runtime::block_on(install_from(
            &base,
            "x86_64-unknown-linux-gnu",
            dir,
            &|phase, _, _| events.lock().unwrap().push(phase),
        ));
        let phases = events.into_inner().unwrap();
        if result.is_ok() {
            assert_eq!(phases.first(), Some(&"resolve"));
            assert_eq!(phases.last(), Some(&"done"));
            assert!(phases.contains(&"verify") && phases.contains(&"download"));
        }
        result
    }

    #[cfg(unix)]
    #[test]
    fn a_verified_release_installs_and_reports_its_version() {
        let t = "x86_64-unknown-linux-gnu";
        let archive = tar_gz(t, b"#!/bin/sh\necho forge 9.8.7\n");
        let manifest = format!("{}  forge-{t}.tar.gz\n", sha256_hex(&archive));
        let dir = tempdir("ok").join("bin");
        let result = run_install(
            vec![
                ("/checksums.txt".into(), manifest.into_bytes()),
                (format!("/forge-{t}.tar.gz"), archive),
            ],
            &dir,
        )
        .unwrap();
        assert_eq!(result.version.as_deref(), Some("9.8.7"));
        assert!(dir.join("forge").is_file());
    }

    #[cfg(unix)]
    #[test]
    fn a_hash_mismatch_installs_nothing() {
        let t = "x86_64-unknown-linux-gnu";
        let archive = tar_gz(t, b"bin");
        let manifest = format!("{}  forge-{t}.tar.gz\n", "0".repeat(64));
        let dir = tempdir("bad").join("bin");
        let err = run_install(
            vec![
                ("/checksums.txt".into(), manifest.into_bytes()),
                (format!("/forge-{t}.tar.gz"), archive),
            ],
            &dir,
        )
        .unwrap_err();
        assert!(err.contains("checksum mismatch"), "{err}");
        assert!(!dir.exists(), "nothing may be created on a failed verify");
    }

    #[cfg(unix)]
    #[test]
    fn a_missing_manifest_fails_closed() {
        let t = "x86_64-unknown-linux-gnu";
        let dir = tempdir("nomanifest").join("bin");
        let err = run_install(
            vec![(format!("/forge-{t}.tar.gz"), tar_gz(t, b"bin"))],
            &dir,
        )
        .unwrap_err();
        assert!(err.contains("no checksum manifest"), "{err}");
        assert!(!dir.exists());
    }

    #[cfg(unix)]
    #[test]
    fn the_legacy_manifest_name_is_accepted() {
        let t = "x86_64-unknown-linux-gnu";
        let archive = tar_gz(t, b"#!/bin/sh\necho forge 1.2.3\n");
        let manifest = format!("{}  forge-{t}.tar.gz\n", sha256_hex(&archive));
        let dir = tempdir("sums").join("bin");
        let result = run_install(
            vec![
                ("/SHA256SUMS".into(), manifest.into_bytes()),
                (format!("/forge-{t}.tar.gz"), archive),
            ],
            &dir,
        )
        .unwrap();
        assert_eq!(result.version.as_deref(), Some("1.2.3"));
    }
}
