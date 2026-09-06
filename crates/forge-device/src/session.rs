//! A connected device: apps, input, the screen, and files.

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use serde::Serialize;

use crate::adb::Adb;
use crate::ui::{Hierarchy, Selector, UiNode};

/// How long `wait_for` polls before giving up, and how long it sleeps between dumps.
const WAIT_POLL: Duration = Duration::from_millis(500);

/// Static facts about a device, read once per call rather than cached: an emulator can be
/// rotated or resized between turns and a stale screen size produces taps in the wrong place.
#[derive(Debug, Clone, Serialize)]
pub struct DeviceProfile {
    pub serial: String,
    pub model: String,
    pub manufacturer: String,
    pub android_version: String,
    pub sdk: String,
    pub screen: Option<(i32, i32)>,
    pub density: Option<i32>,
    pub rooted: bool,
}

/// A device bound to one serial. Cheap to clone; holds no connection of its own.
#[derive(Debug, Clone)]
pub struct Device {
    adb: Adb,
}

impl Device {
    pub fn new(adb: Adb) -> Self {
        Self { adb }
    }

    pub fn adb(&self) -> &Adb {
        &self.adb
    }

    pub fn serial(&self) -> &str {
        self.adb.serial().unwrap_or("<unbound>")
    }

    pub async fn profile(&self) -> Result<DeviceProfile> {
        let prop = |name: &'static str| async move {
            self.adb
                .shell(&format!("getprop {name}"))
                .await
                .unwrap_or_default()
        };
        let (model, manufacturer, release, sdk) = tokio::join!(
            prop("ro.product.model"),
            prop("ro.product.manufacturer"),
            prop("ro.build.version.release"),
            prop("ro.build.version.sdk"),
        );
        let (screen, density) = tokio::join!(self.screen_size(), self.density());
        // `id` returning uid 0 is the honest test: `adb root` succeeding says nothing about the
        // shell we actually get, and `su` may exist without working.
        let rooted = self
            .adb
            .shell("id -u")
            .await
            .map(|out| out.trim() == "0")
            .unwrap_or(false);
        Ok(DeviceProfile {
            serial: self.serial().to_string(),
            model,
            manufacturer,
            android_version: release,
            sdk,
            screen: screen.ok(),
            density: density.ok(),
            rooted,
        })
    }

    /// Physical screen size in pixels, honouring an override set by `wm size`.
    pub async fn screen_size(&self) -> Result<(i32, i32)> {
        let text = self.adb.shell("wm size").await?;
        // "Physical size: 1080x2400" and optionally "Override size: 720x1600" — the override wins.
        let line = text
            .lines()
            .find(|line| line.contains("Override size"))
            .or_else(|| text.lines().find(|line| line.contains("Physical size")))
            .unwrap_or_default();
        let raw = line.split(':').nth(1).unwrap_or_default().trim();
        let (width, height) = raw.split_once('x').context("could not read screen size")?;
        Ok((width.trim().parse()?, height.trim().parse()?))
    }

    pub async fn density(&self) -> Result<i32> {
        let text = self.adb.shell("wm density").await?;
        let line = text
            .lines()
            .find(|line| line.contains("Override density"))
            .or_else(|| text.lines().find(|line| line.contains("Physical density")))
            .unwrap_or_default();
        Ok(line.split(':').nth(1).unwrap_or_default().trim().parse()?)
    }

    // ---- apps -------------------------------------------------------------------------------

    /// Install an APK. `grant` pre-grants runtime permissions, which is almost always what a test
    /// wants — otherwise the first launch stalls on a permission dialog.
    pub async fn install(&self, apk: &Path, reinstall: bool, grant: bool) -> Result<String> {
        if !apk.is_file() {
            bail!("no APK at {}", apk.display());
        }
        let path = apk.to_string_lossy().to_string();
        let mut args = vec!["install"];
        if reinstall {
            args.push("-r");
        }
        if grant {
            args.push("-g");
        }
        args.push(&path);
        let out = self
            .adb
            .output(&args, Duration::from_secs(600))
            .await
            .map(|bytes| String::from_utf8_lossy(&bytes).trim().to_string())?;
        // `adb install` exits 0 while printing Failure for some errors, so check the text too.
        if out.contains("Failure") || out.contains("Error:") {
            bail!("install failed: {out}");
        }
        Ok(out)
    }

    pub async fn uninstall(&self, package: &str) -> Result<String> {
        self.adb
            .run(&["uninstall", package])
            .await
            .map(|out| out.trim().to_string())
    }

    /// Launch an app. With no activity, uses the launcher intent, which is what a user tapping the
    /// icon would get.
    pub async fn launch(&self, package: &str, activity: Option<&str>) -> Result<String> {
        let command = match activity {
            Some(activity) if activity.starts_with('.') || activity.contains('/') => {
                let component = if activity.contains('/') {
                    activity.to_string()
                } else {
                    format!("{package}/{activity}")
                };
                format!("am start -n {component}")
            }
            Some(activity) => format!("am start -n {package}/{activity}"),
            None => format!("monkey -p {package} -c android.intent.category.LAUNCHER 1"),
        };
        let out = self.adb.shell(&command).await?;
        if out.contains("Error:") || out.contains("No activities found") {
            bail!("launch failed: {out}");
        }
        Ok(out)
    }

    pub async fn force_stop(&self, package: &str) -> Result<()> {
        self.adb
            .shell(&format!("am force-stop {package}"))
            .await
            .map(|_| ())
    }

    /// Wipe an app's data — the reliable way to get back to a first-run state between tests.
    pub async fn clear_data(&self, package: &str) -> Result<String> {
        let out = self.adb.shell(&format!("pm clear {package}")).await?;
        if !out.contains("Success") {
            bail!("pm clear failed: {out}");
        }
        Ok(out)
    }

    /// Installed packages, optionally filtered by substring. Third-party only by default: the
    /// full list is ~600 system packages and useless in a transcript.
    pub async fn packages(
        &self,
        filter: Option<&str>,
        include_system: bool,
    ) -> Result<Vec<String>> {
        let command = if include_system {
            "pm list packages"
        } else {
            "pm list packages -3"
        };
        let text = self.adb.shell(command).await?;
        let mut names: Vec<String> = text
            .lines()
            .filter_map(|line| line.trim().strip_prefix("package:"))
            .filter(|name| filter.is_none_or(|want| name.contains(want)))
            .map(str::to_string)
            .collect();
        names.sort();
        Ok(names)
    }

    /// The package and activity currently in the foreground.
    pub async fn current_activity(&self) -> Result<String> {
        let text = self
            .adb
            .shell(
                "dumpsys activity activities | grep -m1 -E 'mResumedActivity|topResumedActivity'",
            )
            .await
            .unwrap_or_default();
        if text.trim().is_empty() {
            // Older/other builds: fall back to the window manager's focused window.
            let focus = self
                .adb
                .shell("dumpsys window | grep -m1 mCurrentFocus")
                .await
                .unwrap_or_default();
            return Ok(focus.trim().to_string());
        }
        Ok(text.trim().to_string())
    }

    // ---- input ------------------------------------------------------------------------------

    pub async fn tap(&self, x: i32, y: i32) -> Result<()> {
        self.adb
            .shell(&format!("input tap {x} {y}"))
            .await
            .map(|_| ())
    }

    pub async fn swipe(&self, from: (i32, i32), to: (i32, i32), ms: u32) -> Result<()> {
        let (x1, y1) = from;
        let (x2, y2) = to;
        self.adb
            .shell(&format!("input swipe {x1} {y1} {x2} {y2} {ms}"))
            .await
            .map(|_| ())
    }

    /// A long press is a swipe that does not move.
    pub async fn long_press(&self, x: i32, y: i32, ms: u32) -> Result<()> {
        self.swipe((x, y), (x, y), ms).await
    }

    /// Type text into the focused field.
    ///
    /// `input text` is ASCII-only and treats spaces specially; both limits are the platform's, so
    /// non-ASCII is rejected up front rather than silently typing the wrong thing.
    pub async fn type_text(&self, text: &str) -> Result<()> {
        if !text.is_ascii() {
            bail!(
                "`input text` only handles ASCII; {text:?} contains other characters. Set the \
                 value directly (for a test fixture) or paste it via the clipboard instead."
            );
        }
        self.adb
            .shell(&format!("input text {}", escape_input_text(text)))
            .await
            .map(|_| ())
    }

    pub async fn key(&self, key: &str) -> Result<()> {
        self.adb
            .shell(&format!("input keyevent {}", keycode(key)))
            .await
            .map(|_| ())
    }

    // ---- screen -----------------------------------------------------------------------------

    /// A PNG of the current screen. `exec-out` keeps the bytes binary-clean.
    pub async fn screenshot(&self) -> Result<Vec<u8>> {
        let bytes = self
            .adb
            .output(&["exec-out", "screencap", "-p"], Duration::from_secs(60))
            .await?;
        if bytes.len() < 8 || &bytes[1..4] != b"PNG" {
            bail!("screencap did not return a PNG ({} bytes)", bytes.len());
        }
        Ok(bytes)
    }

    /// Dump the view hierarchy.
    ///
    /// Written to a file and read back rather than using `dump /dev/tty`: the tty form interleaves
    /// a status line with the XML and mangles output on several vendor builds.
    ///
    /// The path carries this process's pid because two Forge sessions may drive one device, and a
    /// shared filename means one reads the other's half-written dump. The retry is for the same
    /// reason plus animations: `uiautomator` refuses while another dump holds it or while the
    /// window is still settling, and it reports that by failing with an empty stderr — which,
    /// surfaced as-is, tells a caller nothing and invites a blind retry loop.
    pub async fn dump_ui(&self) -> Result<Hierarchy> {
        const ATTEMPTS: usize = 3;
        let remote = format!("/sdcard/forge-ui-dump-{}.xml", std::process::id());
        let mut last = String::new();
        for attempt in 0..ATTEMPTS {
            match self.try_dump(&remote).await {
                Ok(tree) => return Ok(tree),
                Err(error) => {
                    last = error.to_string();
                    if attempt + 1 < ATTEMPTS {
                        tokio::time::sleep(Duration::from_millis(600)).await;
                    }
                }
            }
        }
        bail!(
            "could not read the screen after {ATTEMPTS} attempts: {last}\n\
             uiautomator refuses to dump while another client holds it (a second Forge session, \
             or an Appium/uiautomator2 server), while the screen is off, and on a window marked \
             FLAG_SECURE such as a password or payment sheet. A screenshot still works on a \
             secure window; the hierarchy does not."
        )
    }

    async fn try_dump(&self, remote: &str) -> Result<Hierarchy> {
        let status = self
            .adb
            .shell_timeout(
                &format!("uiautomator dump --compressed {remote}"),
                Duration::from_secs(60),
            )
            .await?;
        if status.contains("ERROR") || status.contains("could not get idle state") {
            bail!("uiautomator said: {status}");
        }
        let xml = self
            .adb
            .shell_timeout(&format!("cat {remote}"), Duration::from_secs(60))
            .await?;
        let tree = Hierarchy::parse(&xml);
        if tree.nodes.is_empty() {
            bail!("the dump contained no nodes");
        }
        Ok(tree)
    }

    /// Find one element, erroring when the selector matches nothing.
    ///
    /// A miss reports what IS on screen. "no element matched" on its own tells a caller nothing
    /// about whether the screen moved on, the label differs, or the element has not appeared yet,
    /// and the only move left is to guess again — which is how a tap/dump/tap retry loop starts.
    pub async fn find_one(&self, selector: &Selector) -> Result<UiNode> {
        let tree = self.dump_ui().await?;
        let matches = tree.find(selector);
        match matches.as_slice() {
            [] => {
                let (visible, _) = tree.interesting();
                let listing =
                    crate::ui::outline(&visible.iter().take(20).copied().collect::<Vec<_>>());
                let more = visible.len().saturating_sub(20);
                bail!(
                    "no element matched. {} element(s) are on screen{}:\n{listing}",
                    visible.len(),
                    if more > 0 {
                        format!(", first 20 of them shown, {more} not listed")
                    } else {
                        String::new()
                    }
                )
            }
            [only] => Ok((*only).clone()),
            many => Ok((*many[0]).clone()),
        }
    }

    /// Tap the first element matching a selector.
    pub async fn tap_selector(&self, selector: &Selector) -> Result<UiNode> {
        let node = self.find_one(selector).await?;
        if node.bounds.is_empty() {
            bail!("{} has zero size and cannot be tapped", node.describe());
        }
        let (x, y) = node.bounds.center();
        self.tap(x, y).await?;
        Ok(node)
    }

    /// Poll the screen until an element appears. Returns how long it took.
    pub async fn wait_for(
        &self,
        selector: &Selector,
        timeout: Duration,
    ) -> Result<(UiNode, Duration)> {
        let started = Instant::now();
        let mut last_error = None;
        while started.elapsed() < timeout {
            match self.dump_ui().await {
                Ok(tree) => {
                    if let Some(node) = tree.find(selector).first() {
                        return Ok(((*node).clone(), started.elapsed()));
                    }
                }
                // A dump can fail transiently mid-animation; keep polling and report the last
                // failure only if we run out of time.
                Err(error) => last_error = Some(error),
            }
            tokio::time::sleep(WAIT_POLL).await;
        }
        match last_error {
            Some(error) => bail!("timed out after {:?}; last dump failed: {error}", timeout),
            None => bail!("timed out after {:?} waiting for the element", timeout),
        }
    }

    // ---- files ------------------------------------------------------------------------------

    pub async fn push(&self, local: &Path, remote: &str) -> Result<String> {
        if !local.exists() {
            bail!("nothing to push at {}", local.display());
        }
        let local = local.to_string_lossy().to_string();
        self.adb
            .output(&["push", &local, remote], Duration::from_secs(600))
            .await
            .map(|bytes| String::from_utf8_lossy(&bytes).trim().to_string())
    }

    pub async fn pull(&self, remote: &str, local: &Path) -> Result<String> {
        let local = local.to_string_lossy().to_string();
        self.adb
            .output(&["pull", remote, &local], Duration::from_secs(600))
            .await
            .map(|bytes| String::from_utf8_lossy(&bytes).trim().to_string())
    }
}

/// Escape text for `input text`, whose argument reaches a shell before `input` sees it.
///
/// Spaces must become `%s` — `input` splits on whitespace regardless of quoting — and the shell
/// metacharacters have to survive the trip.
pub fn escape_input_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    for character in text.chars() {
        match character {
            ' ' => out.push_str("%s"),
            '%' => out.push_str("\\%"),
            '(' | ')' | '<' | '>' | '|' | ';' | '&' | '*' | '\\' | '~' | '"' | '\'' | '`' | '$'
            | '#' | '!' => {
                out.push('\\');
                out.push(character);
            }
            other => out.push(other),
        }
    }
    out
}

/// Map a friendly key name to an Android keycode. Unknown names pass through, so any
/// `KEYCODE_*` or numeric code still works.
pub fn keycode(key: &str) -> String {
    let named = match key.to_ascii_lowercase().as_str() {
        "back" => "KEYCODE_BACK",
        "home" => "KEYCODE_HOME",
        "enter" | "return" => "KEYCODE_ENTER",
        "tab" => "KEYCODE_TAB",
        "delete" | "backspace" => "KEYCODE_DEL",
        "escape" | "esc" => "KEYCODE_ESCAPE",
        "space" => "KEYCODE_SPACE",
        "menu" => "KEYCODE_MENU",
        "search" => "KEYCODE_SEARCH",
        "power" => "KEYCODE_POWER",
        "wake" => "KEYCODE_WAKEUP",
        "sleep" => "KEYCODE_SLEEP",
        "app_switch" | "recents" => "KEYCODE_APP_SWITCH",
        "volume_up" => "KEYCODE_VOLUME_UP",
        "volume_down" => "KEYCODE_VOLUME_DOWN",
        "up" => "KEYCODE_DPAD_UP",
        "down" => "KEYCODE_DPAD_DOWN",
        "left" => "KEYCODE_DPAD_LEFT",
        "right" => "KEYCODE_DPAD_RIGHT",
        "center" | "ok" => "KEYCODE_DPAD_CENTER",
        _ => return key.to_string(),
    };
    named.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spaces_become_percent_s_and_metacharacters_are_escaped() {
        assert_eq!(escape_input_text("hello world"), "hello%sworld");
        assert_eq!(escape_input_text("a&b"), "a\\&b");
        assert_eq!(escape_input_text("100%"), "100\\%");
        assert_eq!(escape_input_text("p@ssw0rd!"), "p@ssw0rd\\!");
    }

    #[test]
    fn plain_text_is_left_alone() {
        assert_eq!(
            escape_input_text("user.name+tag@example.com"),
            "user.name+tag@example.com"
        );
    }

    #[test]
    fn friendly_key_names_map_and_raw_codes_pass_through() {
        assert_eq!(keycode("back"), "KEYCODE_BACK");
        assert_eq!(keycode("ENTER"), "KEYCODE_ENTER");
        assert_eq!(keycode("KEYCODE_CAMERA"), "KEYCODE_CAMERA");
        assert_eq!(keycode("66"), "66");
    }
}
