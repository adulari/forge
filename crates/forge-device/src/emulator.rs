//! Android emulator lifecycle: list AVDs, boot one, wait for it, shut it down.
//!
//! Booting is deliberately fire-and-forget followed by an explicit wait. The emulator process
//! lives for minutes to hours and must not be tied to the tool call that started it.

use std::process::Stdio;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use tokio::process::Command;

use crate::adb::{sdk_roots, which, Adb};

/// How often to re-check a booting emulator.
const BOOT_POLL: Duration = Duration::from_secs(2);

/// Options for booting an AVD.
///
/// The defaults keep the device **persistent**: an app installed in one session is still there in
/// the next. That rests on two things, both defaults here — `wipe_data` is off, so the userdata
/// image survives, and the quick-boot snapshot is loaded and saved, so the disk state a session
/// ended with is the state the next one starts from. It also rests on shutting down through
/// [`stop`], which asks the emulator to save; a SIGKILL discards everything since the last save,
/// which for a freshly installed app means the app.
///
/// The defaults are otherwise deliberately frugal. An emulator left on its own defaults takes the host's
/// core count and the AVD's configured RAM, which on a developer laptop competes with the build
/// it is supposed to be testing. [`BootOptions::light`] caps both and turns off everything that
/// costs time or power without helping a test: audio, the boot animation, metrics upload, and —
/// when headless — hardware GL.
#[derive(Debug, Clone)]
pub struct BootOptions {
    /// Boot without a window. Useful on a headless box; the UI hierarchy still works.
    pub headless: bool,
    /// Reset to a clean image — the reliable way to start a test from a known state.
    pub wipe_data: bool,
    /// Mount /system writable, which is what installing a CA into the system store needs.
    pub writable_system: bool,
    /// `-http-proxy host:port`. Routes the emulator's traffic through a proxy at the emulator
    /// level, which catches apps that ignore the system proxy setting.
    pub http_proxy: Option<String>,
    /// Guest RAM in MB. `None` uses the AVD's own setting.
    pub memory_mb: Option<u32>,
    /// Guest CPU cores. `None` uses the AVD's own setting.
    pub cores: Option<u32>,
    /// `-gpu` mode. `None` picks `auto-no-window` headless, `host` with a window.
    pub gpu: Option<String>,
    /// Ignore the saved quick-boot snapshot and boot from scratch. Slower, but the only way to be
    /// sure of the starting state when a snapshot may be stale.
    pub cold_boot: bool,
    /// Extra raw emulator flags.
    pub extra_args: Vec<String>,
}

impl Default for BootOptions {
    fn default() -> Self {
        Self::light()
    }
}

impl BootOptions {
    /// A small, quick emulator: 2 cores and 2 GB, which is enough to drive an app under test and
    /// leaves the host free to keep compiling.
    pub fn light() -> Self {
        Self {
            headless: false,
            wipe_data: false,
            writable_system: false,
            http_proxy: None,
            memory_mb: Some(2048),
            cores: Some(2),
            gpu: None,
            cold_boot: false,
            extra_args: Vec::new(),
        }
    }

    /// The full argument list this configuration boots with, `-avd <name>` first.
    ///
    /// Split out from [`start`] so the flags are testable without booting anything.
    pub fn args(&self, avd: &str) -> Vec<String> {
        let mut args: Vec<String> = vec!["-avd".into(), avd.into()];
        if self.headless {
            args.push("-no-window".into());
        }
        // Headless still wants the host driver when there is one. `auto-no-window` is the
        // emulator's own headless renderer selection: it uses the host GPU where that works and
        // falls back to software itself, without the caller having to guess.
        //
        // Naming `swiftshader_indirect` here instead — the old default — is what made a headless
        // boot fragile. SwiftShader renders in JIT-compiled shader code, and a bad draw call from
        // the guest faults INSIDE that code (an out-of-bounds SIMD load), which is a SIGSEGV in
        // the emulator process, not a dropped frame: the whole device dies mid-test. It also
        // renders a phone-sized screen on the CPU, which on a laptop means thermal throttling for
        // whatever else is building. Software GL is still available by asking for it.
        let gpu = self.gpu.clone().unwrap_or_else(|| {
            if self.headless {
                "auto-no-window".into()
            } else {
                "host".into()
            }
        });
        args.push("-gpu".into());
        args.push(gpu);
        if let Some(memory) = self.memory_mb {
            args.push("-memory".into());
            args.push(memory.to_string());
        }
        if let Some(cores) = self.cores {
            args.push("-cores".into());
            args.push(cores.to_string());
        }
        if self.wipe_data {
            args.push("-wipe-data".into());
        }
        if self.cold_boot {
            args.push("-no-snapshot-load".into());
        }
        if self.writable_system {
            args.push("-writable-system".into());
        }
        if let Some(proxy) = &self.http_proxy {
            args.push("-http-proxy".into());
            args.push(proxy.clone());
        }
        // Nothing below helps a test, and all of it costs boot time, power, or network.
        args.push("-no-audio".into());
        args.push("-no-boot-anim".into());
        args.push("-no-metrics".into());
        args.push("-netfast".into());
        args.extend(self.extra_args.iter().cloned());
        args
    }
}

/// Locate the `emulator` binary the same way [`Adb::locate`] finds adb.
pub fn locate_emulator() -> Result<std::path::PathBuf> {
    if let Some(path) = std::env::var_os("FORGE_ANDROID_EMULATOR") {
        let path = std::path::PathBuf::from(path);
        if path.is_file() {
            return Ok(path);
        }
        bail!(
            "FORGE_ANDROID_EMULATOR points at {}, which is not a file",
            path.display()
        );
    }
    if let Some(path) = which("emulator") {
        return Ok(path);
    }
    for root in sdk_roots() {
        let candidate = root.join("emulator").join("emulator");
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    bail!(
        "the Android emulator was not found. Install it via the SDK manager, or set \
         FORGE_ANDROID_EMULATOR."
    )
}

/// Names of the configured AVDs.
pub async fn list_avds() -> Result<Vec<String>> {
    let binary = locate_emulator()?;
    let out = Command::new(&binary)
        .arg("-list-avds")
        .stdin(Stdio::null())
        .output()
        .await
        .context("run emulator -list-avds")?;
    Ok(parse_avds(&String::from_utf8_lossy(&out.stdout)))
}

/// Pull AVD names out of `emulator -list-avds`, which interleaves them with log banners.
///
/// The banners are `LEVEL | message`, so they are told apart by containing a pipe or whitespace —
/// an AVD name can contain neither. Filtering on capitalisation would eat `Medium_Phone`.
fn parse_avds(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| {
            !line.is_empty()
                && !line.contains('|')
                && !line.chars().any(char::is_whitespace)
                && line
                    .chars()
                    .all(|c| c.is_alphanumeric() || "._-".contains(c))
        })
        .map(str::to_string)
        .collect()
}

/// Boot an AVD and return the serial it came up as.
///
/// The new serial is identified by diffing the device list, so booting a second emulator
/// alongside an existing one still reports the right one.
pub async fn start(
    adb: &Adb,
    avd: &str,
    options: &BootOptions,
    timeout: Duration,
) -> Result<String> {
    let available = list_avds().await?;
    if !available.iter().any(|name| name == avd) {
        bail!(
            "no AVD named '{avd}'. Available: {}",
            if available.is_empty() {
                "none".to_string()
            } else {
                available.join(", ")
            }
        );
    }
    let before: Vec<String> = adb
        .devices()
        .await?
        .into_iter()
        .map(|device| device.serial)
        .collect();

    let binary = locate_emulator()?;
    let mut command = Command::new(&binary);
    command.args(options.args(avd));
    // Detach: the emulator must outlive this call, and its stdout would otherwise fill a pipe
    // nobody drains and wedge the process.
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // ...and outlive a process-group kill. Inheriting Forge's group means anything that signals
    // that group — a shell tool call cleaning up after itself, a supervisor stopping Forge — takes
    // the emulator down with it, minutes of boot thrown away for a reason nobody can see from the
    // device side.
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command.spawn().context("spawn the emulator")?;

    let started = Instant::now();
    while started.elapsed() < timeout {
        if let Ok(Some(status)) = child.try_wait() {
            bail!("the emulator exited immediately ({status}) — run it by hand to see why");
        }
        if let Some(serial) = new_serial(adb, &before).await? {
            let bound = adb.with_serial(&serial);
            if wait_for_boot(&bound, timeout.saturating_sub(started.elapsed()))
                .await
                .is_ok()
            {
                return Ok(serial);
            }
        }
        tokio::time::sleep(BOOT_POLL).await;
    }
    bail!(
        "the emulator did not finish booting within {}s",
        timeout.as_secs()
    )
}

async fn new_serial(adb: &Adb, before: &[String]) -> Result<Option<String>> {
    Ok(adb
        .devices()
        .await?
        .into_iter()
        .find(|device| device.is_emulator && !before.contains(&device.serial))
        .map(|device| device.serial))
}

/// Block until the device reports a completed boot.
///
/// `sys.boot_completed` alone is not enough: it flips before the launcher is up, and input sent in
/// that window is dropped. Waiting for the package manager too is what makes a boot usable.
pub async fn wait_for_boot(adb: &Adb, timeout: Duration) -> Result<Duration> {
    let started = Instant::now();
    while started.elapsed() < timeout {
        let booted = adb
            .shell("getprop sys.boot_completed")
            .await
            .unwrap_or_default();
        if booted.trim() == "1" {
            let packages = adb.shell("pm path android").await.unwrap_or_default();
            if packages.contains("package:") {
                return Ok(started.elapsed());
            }
        }
        tokio::time::sleep(BOOT_POLL).await;
    }
    Err(anyhow!(
        "device was not ready within {}s",
        timeout.as_secs()
    ))
}

/// Shut down a running emulator and wait for it to actually go away.
///
/// Returning as soon as `emu kill` is sent would be a lie: the process takes seconds to flush its
/// disk image, and a boot started in that window collides with the one still shutting down.
///
/// This is also the only shutdown that preserves state. `emu kill` is graceful, so the emulator
/// writes its quick-boot snapshot on the way out and an app installed this session is still
/// installed next session. Killing the process instead loses everything since the last save.
pub async fn stop(adb: &Adb, timeout: Duration) -> Result<Duration> {
    let serial = adb.serial().map(str::to_string);
    adb.run(&["emu", "kill"]).await?;
    let Some(serial) = serial else {
        return Ok(Duration::ZERO);
    };
    let started = Instant::now();
    while started.elapsed() < timeout {
        let gone = !adb
            .devices()
            .await?
            .iter()
            .any(|device| device.serial == serial);
        if gone {
            return Ok(started.elapsed());
        }
        tokio::time::sleep(BOOT_POLL).await;
    }
    bail!(
        "{serial} was asked to stop but was still listed after {}s",
        timeout.as_secs()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_profile_is_the_light_one() {
        let light = BootOptions::default();
        assert_eq!(light.memory_mb, Some(2048));
        assert_eq!(light.cores, Some(2));
        assert!(
            !light.cold_boot,
            "quick boot is what makes a repeat run fast"
        );
    }

    #[test]
    fn every_boot_drops_the_work_a_test_does_not_need() {
        let args = BootOptions::light().args("Pixel");
        for flag in ["-no-audio", "-no-boot-anim", "-no-metrics", "-netfast"] {
            assert!(
                args.iter().any(|arg| arg == flag),
                "{flag} missing from {args:?}"
            );
        }
        assert_eq!(&args[..2], &["-avd".to_string(), "Pixel".to_string()]);
    }

    #[test]
    fn neither_headless_nor_windowed_falls_back_to_software_gl_on_its_own() {
        let headless = BootOptions {
            headless: true,
            ..BootOptions::light()
        };
        let args = headless.args("Pixel");
        let gpu = args.iter().position(|arg| arg == "-gpu").expect("-gpu");
        assert_eq!(
            args[gpu + 1],
            "auto-no-window",
            "a headless boot must not silently pick swiftshader_indirect: its JIT shader code \
             segfaults the whole emulator on a bad guest draw call"
        );
        assert!(args.iter().any(|arg| arg == "-no-window"));

        let windowed = BootOptions::light().args("Pixel");
        let gpu = windowed.iter().position(|arg| arg == "-gpu").expect("-gpu");
        assert_eq!(windowed[gpu + 1], "host");
        assert!(!windowed.iter().any(|arg| arg == "-no-window"));
    }

    #[test]
    fn an_explicit_gpu_mode_wins_over_the_headless_default() {
        let options = BootOptions {
            headless: true,
            gpu: Some("off".into()),
            ..Default::default()
        };
        let args = options.args("Pixel");
        let gpu = args.iter().position(|arg| arg == "-gpu").expect("-gpu");
        assert_eq!(args[gpu + 1], "off");
    }

    #[test]
    fn resource_caps_can_be_lifted_by_asking_for_the_avds_own_settings() {
        let options = BootOptions {
            memory_mb: None,
            cores: None,
            ..Default::default()
        };
        let args = options.args("Pixel");
        assert!(!args.iter().any(|arg| arg == "-memory"));
        assert!(!args.iter().any(|arg| arg == "-cores"));
    }

    #[test]
    fn the_default_boot_never_discards_the_device_state() {
        // Installs have to survive a restart, so nothing that erases state may be a default.
        let args = BootOptions::light().args("Pixel");
        for destructive in ["-wipe-data", "-no-snapshot-load", "-no-snapshot-save"] {
            assert!(
                !args.iter().any(|arg| arg == destructive),
                "{destructive} would make the device forget installed apps"
            );
        }
    }

    #[test]
    fn cold_boot_and_wipe_are_opt_in() {
        let plain = BootOptions::light().args("Pixel");
        assert!(!plain.iter().any(|arg| arg == "-no-snapshot-load"));
        assert!(!plain.iter().any(|arg| arg == "-wipe-data"));

        let fresh = BootOptions {
            cold_boot: true,
            wipe_data: true,
            ..Default::default()
        };
        let args = fresh.args("Pixel");
        assert!(args.iter().any(|arg| arg == "-no-snapshot-load"));
        assert!(args.iter().any(|arg| arg == "-wipe-data"));
    }

    #[test]
    fn extra_args_come_last_so_they_can_override_the_defaults() {
        let options = BootOptions {
            extra_args: vec!["-timezone".into(), "UTC".into()],
            ..Default::default()
        };
        let args = options.args("Pixel");
        assert_eq!(
            &args[args.len() - 2..],
            &["-timezone".to_string(), "UTC".to_string()]
        );
    }

    #[test]
    fn avd_names_survive_the_banner_filter() {
        // Real `emulator -list-avds` output on a machine with one AVD, banners and all.
        let raw = "INFO    | Storing crashdata in: /tmp/avd, DataMessages disabled\n\
                   Medium_Phone\n\
                   Pixel_7_API_34\n\
                   my.avd-2\n";
        assert_eq!(
            parse_avds(raw),
            ["Medium_Phone", "Pixel_7_API_34", "my.avd-2"]
        );
    }

    #[test]
    fn an_absent_sdk_yields_no_names_rather_than_a_banner() {
        assert!(parse_avds("ERROR   | No AVDs found\n").is_empty());
        assert!(parse_avds("").is_empty());
    }
}
