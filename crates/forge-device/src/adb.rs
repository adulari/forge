//! The adb transport: find the binary, pick a device, run commands against it.
//!
//! Everything else in this crate is expressed in terms of [`Adb`], so the rest of the crate never
//! shells out directly and there is exactly one place that knows how adb is invoked.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;
use tokio::process::Command;

/// Default per-command timeout. Generous because installs and `wait-for-device` are legitimately
/// slow, but bounded because a wedged adb server otherwise hangs the whole turn.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);

/// One entry from `adb devices -l`.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DeviceInfo {
    pub serial: String,
    /// `device`, `offline`, `unauthorized`, `bootloader`…
    pub state: String,
    pub model: Option<String>,
    /// True for `emulator-NNNN` serials.
    pub is_emulator: bool,
}

impl DeviceInfo {
    pub fn usable(&self) -> bool {
        self.state == "device"
    }
}

/// A located adb binary, optionally bound to one device serial.
#[derive(Debug, Clone)]
pub struct Adb {
    binary: PathBuf,
    serial: Option<String>,
}

impl Adb {
    /// Locate adb. Explicit env override first, then `PATH`, then the standard SDK layouts, so
    /// this works on a machine where the SDK was installed but never added to `PATH`.
    pub fn locate() -> Result<Self> {
        if let Some(path) = std::env::var_os("FORGE_ADB").or_else(|| std::env::var_os("ADB")) {
            let path = PathBuf::from(path);
            if path.is_file() {
                return Ok(Self {
                    binary: path,
                    serial: None,
                });
            }
            bail!(
                "FORGE_ADB/ADB points at {}, which is not a file",
                path.display()
            );
        }
        if let Some(path) = which("adb") {
            return Ok(Self {
                binary: path,
                serial: None,
            });
        }
        for root in sdk_roots() {
            let candidate = root.join("platform-tools").join("adb");
            if candidate.is_file() {
                return Ok(Self {
                    binary: candidate,
                    serial: None,
                });
            }
        }
        bail!(
            "adb was not found. Install Android platform-tools and put adb on PATH, set \
             ANDROID_SDK_ROOT, or set FORGE_ADB to the binary."
        )
    }

    pub fn binary(&self) -> &std::path::Path {
        &self.binary
    }

    pub fn serial(&self) -> Option<&str> {
        self.serial.as_deref()
    }

    pub fn with_serial(&self, serial: impl Into<String>) -> Self {
        Self {
            binary: self.binary.clone(),
            serial: Some(serial.into()),
        }
    }

    /// Run adb and return stdout, failing on a non-zero exit with stderr attached.
    pub async fn run(&self, args: &[&str]) -> Result<String> {
        let out = self.output(args, DEFAULT_TIMEOUT).await?;
        Ok(String::from_utf8_lossy(&out).into_owned())
    }

    /// Run adb and return raw stdout bytes — for `exec-out`, whose payload is binary.
    pub async fn output(&self, args: &[&str], timeout: Duration) -> Result<Vec<u8>> {
        let mut command = Command::new(&self.binary);
        if let Some(serial) = &self.serial {
            command.arg("-s").arg(serial);
        }
        command.args(args);
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let rendered = || {
            let mut parts = vec![self.binary.display().to_string()];
            if let Some(serial) = &self.serial {
                parts.push(format!("-s {serial}"));
            }
            parts.extend(args.iter().map(|a| (*a).to_string()));
            parts.join(" ")
        };

        let child = command
            .spawn()
            .with_context(|| format!("spawn {}", rendered()))?;
        let done = tokio::time::timeout(timeout, child.wait_with_output())
            .await
            .map_err(|_| anyhow!("`{}` timed out after {}s", rendered(), timeout.as_secs()))?
            .with_context(|| format!("run {}", rendered()))?;

        if !done.status.success() {
            let stderr = String::from_utf8_lossy(&done.stderr).trim().to_string();
            let stdout = String::from_utf8_lossy(&done.stdout).trim().to_string();
            // adb writes some genuine errors to stdout, so surface whichever is non-empty.
            let detail = if stderr.is_empty() { stdout } else { stderr };
            bail!("`{}` failed: {detail}", rendered());
        }
        Ok(done.stdout)
    }

    /// `adb shell <command>`, returning trimmed stdout.
    pub async fn shell(&self, command: &str) -> Result<String> {
        let out = self.output(&["shell", command], DEFAULT_TIMEOUT).await?;
        Ok(String::from_utf8_lossy(&out).trim_end().to_string())
    }

    /// `adb shell` with an explicit timeout, for commands that can legitimately run long.
    pub async fn shell_timeout(&self, command: &str, timeout: Duration) -> Result<String> {
        let out = self.output(&["shell", command], timeout).await?;
        Ok(String::from_utf8_lossy(&out).trim_end().to_string())
    }

    /// Connected devices. An unreachable adb server is an error; zero devices is not.
    pub async fn devices(&self) -> Result<Vec<DeviceInfo>> {
        let text = self.run(&["devices", "-l"]).await?;
        Ok(parse_devices(&text))
    }

    /// Resolve which device to act on: an explicit serial if given, else the only usable one.
    ///
    /// Refusing to guess between two devices is deliberate — silently driving the wrong phone is
    /// far worse than an error that names both.
    pub async fn resolve(&self, requested: Option<&str>) -> Result<Self> {
        let devices = self.devices().await?;
        if let Some(serial) = requested {
            return match devices.iter().find(|d| d.serial == serial) {
                Some(found) if found.usable() => Ok(self.with_serial(serial)),
                Some(found) => bail!("device {serial} is in state '{}', not ready", found.state),
                None => bail!("no device with serial {serial} is connected"),
            };
        }
        let usable: Vec<_> = devices.iter().filter(|d| d.usable()).collect();
        match usable.as_slice() {
            [] if devices.is_empty() => bail!(
                "no devices are connected. Start an emulator (device action \"emulator_start\") \
                 or plug in a phone with USB debugging enabled."
            ),
            [] => bail!(
                "no device is ready: {}",
                devices
                    .iter()
                    .map(|d| format!("{} ({})", d.serial, d.state))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            [only] => Ok(self.with_serial(&only.serial)),
            many => bail!(
                "{} devices are connected — pass `serial` to choose: {}",
                many.len(),
                many.iter()
                    .map(|d| d.serial.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }
}

/// Parse `adb devices -l` output.
fn parse_devices(text: &str) -> Vec<DeviceInfo> {
    text.lines()
        .skip_while(|line| line.starts_with("List of devices"))
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('*') || line.starts_with("List of devices") {
                return None;
            }
            let mut parts = line.split_whitespace();
            let serial = parts.next()?.to_string();
            let state = parts.next()?.to_string();
            let model = parts
                .find_map(|field| field.strip_prefix("model:"))
                .map(|value| value.replace('_', " "));
            let is_emulator = serial.starts_with("emulator-");
            Some(DeviceInfo {
                serial,
                state,
                model,
                is_emulator,
            })
        })
        .collect()
}

/// Candidate Android SDK roots, most specific first.
pub(crate) fn sdk_roots() -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = ["ANDROID_SDK_ROOT", "ANDROID_HOME"]
        .iter()
        .filter_map(std::env::var_os)
        .map(PathBuf::from)
        .collect();
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        roots.push(home.join("Android/Sdk"));
        roots.push(home.join("Library/Android/sdk"));
    }
    roots.push(PathBuf::from("/opt/android-sdk"));
    roots.push(PathBuf::from("/usr/lib/android-sdk"));
    roots
}

/// Minimal `which`: scan `PATH` for an executable file.
pub(crate) fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|c| c.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_mixed_device_list() {
        let text = "List of devices attached\n\
                    emulator-5554          device product:sdk_gphone64 model:Medium_Phone\n\
                    R5CT10ABCDE            unauthorized\n";
        let devices = parse_devices(text);
        assert_eq!(devices.len(), 2);
        assert!(devices[0].is_emulator);
        assert!(devices[0].usable());
        assert_eq!(devices[0].model.as_deref(), Some("Medium Phone"));
        assert!(!devices[1].usable());
        assert!(!devices[1].is_emulator);
    }

    #[test]
    fn an_empty_list_is_not_an_error() {
        assert!(parse_devices("List of devices attached\n\n").is_empty());
    }

    #[test]
    fn daemon_startup_noise_is_ignored() {
        let text = "* daemon not running; starting now at tcp:5037\n\
                    * daemon started successfully\n\
                    List of devices attached\n\
                    emulator-5554\tdevice\n";
        let devices = parse_devices(text);
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].serial, "emulator-5554");
    }
}
