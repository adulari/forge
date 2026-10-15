//! Reading an image off the system clipboard by shelling out to whatever the platform offers.
//!
//! The ordered backends exist because the clipboard Forge needs is often not the local one: on a
//! remote box over `ssh -X` the only reachable clipboard is the forwarded X display, which
//! `arboard` cannot be trusted with (a dead forwarded `DISPLAY` hangs it). Every external call has
//! a short timeout, and every failure is remembered so the user is told why nothing was pasted.

use std::io::Read as _;
use std::time::Duration;

pub(crate) const READ_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_CLIPBOARD_BYTES: u64 = 40 * 1024 * 1024;

const IMAGE_TARGETS: [&str; 5] = [
    "image/png",
    "image/jpeg",
    "image/webp",
    "image/gif",
    "image/bmp",
];

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RunError {
    NotFound,
    Failed,
    TimedOut,
}

/// Process boundary, faked in tests.
pub(crate) trait Runner {
    fn run(&self, program: &str, args: &[&str], env: &[(&str, &str)]) -> Result<Vec<u8>, RunError>;
    /// Last-resort in-process read (arboard), already PNG-encoded.
    fn native_png(&self) -> Option<Vec<u8>>;
}

#[derive(Debug, Clone, Default)]
pub(crate) struct Env {
    pub os: &'static str,
    pub display: Option<String>,
    pub wayland_display: Option<String>,
    pub in_tmux: bool,
}

impl Env {
    pub fn current() -> Self {
        let nonempty = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        Self {
            os: std::env::consts::OS,
            display: nonempty("DISPLAY"),
            wayland_display: nonempty("WAYLAND_DISPLAY"),
            in_tmux: nonempty("TMUX").is_some(),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    Found {
        bytes: Vec<u8>,
        backend: &'static str,
    },
    /// `reached` means at least one backend talked to a clipboard and it held no image.
    NoImage { reached: bool, why: String },
}

impl Outcome {
    #[cfg(test)]
    pub fn explain(&self) -> Option<&str> {
        match self {
            Outcome::NoImage { why, .. } => Some(why),
            Outcome::Found { .. } => None,
        }
    }
}

pub(crate) fn read_image(env: &Env, runner: &dyn Runner) -> Outcome {
    let mut display = env.display.clone();
    if env.in_tmux {
        // After an ssh reconnect the process's DISPLAY is stale; tmux's session environment holds
        // the one the newest client forwarded. Read-only: we never write the tmux environment.
        if let Ok(out) = runner.run("tmux", &["show-environment", "DISPLAY"], &[]) {
            if let Some(d) = parse_tmux_display(&String::from_utf8_lossy(&out)) {
                display = Some(d);
            }
        }
    }

    let mut reached = false;
    let mut problems: Vec<String> = Vec::new();
    let linux_like = !matches!(env.os, "macos" | "windows");

    let mut attempt = |name: &'static str, result: Result<Option<Vec<u8>>, String>| match result {
        Ok(Some(bytes)) => Some(Outcome::Found {
            bytes,
            backend: name,
        }),
        Ok(None) => {
            reached = true;
            None
        }
        Err(problem) => {
            problems.push(problem);
            None
        }
    };

    if env.os == "macos" {
        if let Some(found) = attempt("pngpaste", run_stdout(runner, "pngpaste", &["-"], &[])) {
            return found;
        }
        if let Some(found) = attempt("osascript", macos_osascript(runner)) {
            return found;
        }
    }
    if env.os == "windows" {
        if let Some(found) = attempt("powershell", windows_powershell(runner)) {
            return found;
        }
    }
    if linux_like {
        if env.wayland_display.is_some() {
            if let Some(found) = attempt("wl-paste", wayland(runner)) {
                return found;
            }
        }
        if let Some(d) = display.as_deref() {
            if let Some(found) = attempt("xclip", x11(runner, d)) {
                return found;
            }
        }
        if env.wayland_display.is_none() && display.is_none() {
            return Outcome::NoImage {
                reached: false,
                why: "no clipboard image: no DISPLAY/WAYLAND_DISPLAY — over SSH use `ssh -X`, \
                      or drop a file path / /image <path>"
                    .to_string(),
            };
        }
    }
    if let Some(png) = runner.native_png() {
        return Outcome::Found {
            bytes: png,
            backend: "arboard",
        };
    }

    let why = if reached {
        "no image on the clipboard — copy or screenshot one first, or drop a file path / /image <path>"
            .to_string()
    } else if problems.is_empty() {
        "no clipboard image: no clipboard backend available — use /image <path>".to_string()
    } else {
        format!(
            "no clipboard image: {} — or drop a file path / /image <path>",
            problems.join("; ")
        )
    };
    Outcome::NoImage { reached, why }
}

fn parse_tmux_display(out: &str) -> Option<String> {
    let value = out.trim().strip_prefix("DISPLAY=")?;
    (!value.is_empty()).then(|| value.to_string())
}

/// `Err` = backend could not run (with a user-facing reason); `Ok(None)` = ran, no image.
fn run_stdout(
    runner: &dyn Runner,
    program: &str,
    args: &[&str],
    env: &[(&str, &str)],
) -> Result<Option<Vec<u8>>, String> {
    match runner.run(program, args, env) {
        Ok(out) if out.is_empty() => Ok(None),
        Ok(out) => Ok(Some(out)),
        Err(RunError::NotFound) => Err(format!("{program} is not installed")),
        Err(RunError::TimedOut) => Err(format!(
            "{program} timed out (unreachable display or clipboard owner?)"
        )),
        Err(RunError::Failed) => Ok(None),
    }
}

pub(crate) fn pick_image_target(listing: &str) -> Option<&'static str> {
    let offered: Vec<&str> = listing.split_whitespace().collect();
    IMAGE_TARGETS
        .into_iter()
        .find(|want| offered.iter().any(|o| o.eq_ignore_ascii_case(want)))
}

fn wayland(runner: &dyn Runner) -> Result<Option<Vec<u8>>, String> {
    let Some(types) = run_stdout(runner, "wl-paste", &["--list-types"], &[])? else {
        return Ok(None);
    };
    let Some(target) = pick_image_target(&String::from_utf8_lossy(&types)) else {
        return Ok(None);
    };
    run_stdout(runner, "wl-paste", &["--no-newline", "--type", target], &[])
}

fn x11(runner: &dyn Runner, display: &str) -> Result<Option<Vec<u8>>, String> {
    let env = [("DISPLAY", display)];
    let Some(targets) = run_stdout(
        runner,
        "xclip",
        &["-selection", "clipboard", "-t", "TARGETS", "-o"],
        &env,
    )?
    else {
        return Ok(None);
    };
    let Some(target) = pick_image_target(&String::from_utf8_lossy(&targets)) else {
        return Ok(None);
    };
    run_stdout(
        runner,
        "xclip",
        &["-selection", "clipboard", "-t", target, "-o"],
        &env,
    )
}

fn macos_osascript(runner: &dyn Runner) -> Result<Option<Vec<u8>>, String> {
    let Some(out) = run_stdout(
        runner,
        "osascript",
        &["-e", "the clipboard as «class PNGf»"],
        &[],
    )?
    else {
        return Ok(None);
    };
    Ok(decode_applescript_data(&String::from_utf8_lossy(&out)))
}

/// `«data PNGf89504E47…»` → raw bytes.
pub(crate) fn decode_applescript_data(text: &str) -> Option<Vec<u8>> {
    let hex = text.trim().strip_prefix("«data PNGf")?.strip_suffix('»')?;
    if hex.len() % 2 != 0 {
        return None;
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok())
        .collect()
}

const POWERSHELL_SCRIPT: &str = "Add-Type -AssemblyName System.Windows.Forms; \
    Add-Type -AssemblyName System.Drawing; \
    $i=[System.Windows.Forms.Clipboard]::GetImage(); \
    if($i){$m=New-Object System.IO.MemoryStream; \
    $i.Save($m,[System.Drawing.Imaging.ImageFormat]::Png); \
    [Convert]::ToBase64String($m.ToArray())}";

fn windows_powershell(runner: &dyn Runner) -> Result<Option<Vec<u8>>, String> {
    use base64::Engine as _;
    let Some(out) = run_stdout(
        runner,
        "powershell",
        &[
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            POWERSHELL_SCRIPT,
        ],
        &[],
    )?
    else {
        return Ok(None);
    };
    Ok(base64::engine::general_purpose::STANDARD
        .decode(String::from_utf8_lossy(&out).trim())
        .ok()
        .filter(|b| !b.is_empty()))
}

pub(crate) struct SystemRunner;

impl Runner for SystemRunner {
    fn run(&self, program: &str, args: &[&str], env: &[(&str, &str)]) -> Result<Vec<u8>, RunError> {
        let mut cmd = std::process::Command::new(program);
        cmd.args(args)
            .envs(env.iter().copied())
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null());
        let mut child = cmd.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                RunError::NotFound
            } else {
                RunError::Failed
            }
        })?;
        let mut stdout = child.stdout.take().ok_or(RunError::Failed)?;
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = (&mut stdout)
                .take(MAX_CLIPBOARD_BYTES)
                .read_to_end(&mut buf);
            let _ = tx.send(buf);
        });
        match rx.recv_timeout(READ_TIMEOUT) {
            Ok(buf) => {
                let ok = child.wait().map(|s| s.success()).unwrap_or(false);
                if ok {
                    Ok(buf)
                } else {
                    Err(RunError::Failed)
                }
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                Err(RunError::TimedOut)
            }
        }
    }

    fn native_png(&self) -> Option<Vec<u8>> {
        // arboard talks to the display server directly; give it the same deadline as the
        // subprocess backends so a dead forwarded display cannot freeze the prompt.
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(arboard_png());
        });
        rx.recv_timeout(READ_TIMEOUT).ok().flatten()
    }
}

fn arboard_png() -> Option<Vec<u8>> {
    let mut clipboard = arboard::Clipboard::new().ok()?;
    let img = clipboard.get_image().ok()?;
    let rgba =
        image::RgbaImage::from_raw(img.width as u32, img.height as u32, img.bytes.into_owned())?;
    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(rgba)
        .write_to(&mut png, image::ImageFormat::Png)
        .ok()?;
    Some(png.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    type Call = (String, Vec<(String, String)>);

    #[derive(Default)]
    struct Fake {
        // "prog arg arg" -> result
        replies: HashMap<String, Result<Vec<u8>, RunError>>,
        calls: RefCell<Vec<Call>>,
        native: Option<Vec<u8>>,
    }

    impl Fake {
        fn reply(mut self, key: &str, out: &[u8]) -> Self {
            self.replies.insert(key.into(), Ok(out.to_vec()));
            self
        }
        fn fail(mut self, key: &str, e: RunError) -> Self {
            self.replies.insert(key.into(), Err(e));
            self
        }
    }

    impl Runner for Fake {
        fn run(
            &self,
            program: &str,
            args: &[&str],
            env: &[(&str, &str)],
        ) -> Result<Vec<u8>, RunError> {
            let key = format!("{program} {}", args.join(" "));
            self.calls.borrow_mut().push((
                key.clone(),
                env.iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
            ));
            match self.replies.get(&key) {
                Some(Ok(v)) => Ok(v.clone()),
                Some(Err(RunError::NotFound)) => Err(RunError::NotFound),
                Some(Err(RunError::TimedOut)) => Err(RunError::TimedOut),
                Some(Err(RunError::Failed)) => Err(RunError::Failed),
                None => Err(RunError::NotFound),
            }
        }
        fn native_png(&self) -> Option<Vec<u8>> {
            self.native.clone()
        }
    }

    fn linux(display: Option<&str>, wayland: Option<&str>, tmux: bool) -> Env {
        Env {
            os: "linux",
            display: display.map(String::from),
            wayland_display: wayland.map(String::from),
            in_tmux: tmux,
        }
    }

    const XCLIP_TARGETS: &str = "xclip -selection clipboard -t TARGETS -o";
    const XCLIP_PNG: &str = "xclip -selection clipboard -t image/png -o";

    #[test]
    fn x11_picks_png_target_and_passes_display() {
        let fake = Fake::default()
            .reply(XCLIP_TARGETS, b"TARGETS\ntext/plain\nimage/png\n")
            .reply(XCLIP_PNG, b"\x89PNGdata");
        let out = read_image(&linux(Some("localhost:10.0"), None, false), &fake);
        assert_eq!(
            out,
            Outcome::Found {
                bytes: b"\x89PNGdata".to_vec(),
                backend: "xclip"
            }
        );
        let calls = fake.calls.borrow();
        assert!(calls
            .iter()
            .all(|(_, env)| env == &[("DISPLAY".to_string(), "localhost:10.0".to_string())]));
    }

    #[test]
    fn x11_falls_back_to_jpeg_when_no_png() {
        let fake = Fake::default()
            .reply(XCLIP_TARGETS, b"image/jpeg\ntext/plain")
            .reply(
                "xclip -selection clipboard -t image/jpeg -o",
                b"\xff\xd8jpeg",
            );
        let out = read_image(&linux(Some(":0"), None, false), &fake);
        assert!(matches!(
            out,
            Outcome::Found {
                backend: "xclip",
                ..
            }
        ));
    }

    #[test]
    fn wayland_goes_first_and_skips_xclip_when_it_works() {
        let fake = Fake::default()
            .reply("wl-paste --list-types", b"text/plain\nimage/png\n")
            .reply("wl-paste --no-newline --type image/png", b"PNGBYTES");
        let out = read_image(&linux(Some(":0"), Some("wayland-1"), false), &fake);
        assert_eq!(
            out,
            Outcome::Found {
                bytes: b"PNGBYTES".to_vec(),
                backend: "wl-paste"
            }
        );
        assert!(fake
            .calls
            .borrow()
            .iter()
            .all(|(k, _)| !k.starts_with("xclip")));
    }

    #[test]
    fn wayland_without_wl_paste_falls_through_to_xclip() {
        let fake = Fake::default()
            .reply(XCLIP_TARGETS, b"image/png")
            .reply(XCLIP_PNG, b"X");
        let out = read_image(&linux(Some(":0"), Some("wayland-1"), false), &fake);
        assert!(matches!(
            out,
            Outcome::Found {
                backend: "xclip",
                ..
            }
        ));
    }

    #[test]
    fn tmux_display_overrides_stale_process_display() {
        let fake = Fake::default()
            .reply("tmux show-environment DISPLAY", b"DISPLAY=localhost:11.0\n")
            .reply(XCLIP_TARGETS, b"image/png")
            .reply(XCLIP_PNG, b"X");
        read_image(&linux(Some("localhost:10.0"), None, true), &fake);
        let calls = fake.calls.borrow();
        let xclip_env = &calls.iter().find(|(k, _)| k == XCLIP_TARGETS).unwrap().1;
        assert_eq!(
            xclip_env,
            &[("DISPLAY".to_string(), "localhost:11.0".to_string())]
        );
    }

    #[test]
    fn tmux_supplies_display_when_process_has_none() {
        let fake = Fake::default()
            .reply("tmux show-environment DISPLAY", b"DISPLAY=localhost:10.0")
            .reply(XCLIP_TARGETS, b"image/png")
            .reply(XCLIP_PNG, b"X");
        let out = read_image(&linux(None, None, true), &fake);
        assert!(matches!(out, Outcome::Found { .. }));
    }

    #[test]
    fn tmux_unset_marker_keeps_process_display() {
        assert_eq!(parse_tmux_display("-DISPLAY\n"), None);
        assert_eq!(parse_tmux_display("DISPLAY=\n"), None);
        assert_eq!(parse_tmux_display("DISPLAY=:1\n").as_deref(), Some(":1"));
    }

    #[test]
    fn no_display_explains_ssh_x() {
        let out = read_image(&linux(None, None, false), &Fake::default());
        let why = out.explain().unwrap();
        assert!(
            why.contains("no DISPLAY/WAYLAND_DISPLAY") && why.contains("ssh -X"),
            "{why}"
        );
        assert!(matches!(out, Outcome::NoImage { reached: false, .. }));
    }

    #[test]
    fn missing_xclip_is_reported_not_silent() {
        let out = read_image(&linux(Some(":0"), None, false), &Fake::default());
        assert!(out.explain().unwrap().contains("xclip is not installed"));
    }

    #[test]
    fn dead_display_timeout_is_reported() {
        let fake = Fake::default().fail(XCLIP_TARGETS, RunError::TimedOut);
        let out = read_image(&linux(Some("localhost:10.0"), None, false), &fake);
        assert!(out.explain().unwrap().contains("timed out"));
    }

    #[test]
    fn text_only_clipboard_says_no_image() {
        let fake = Fake::default().reply(XCLIP_TARGETS, b"text/plain\nUTF8_STRING");
        let out = read_image(&linux(Some(":0"), None, false), &fake);
        assert!(matches!(out, Outcome::NoImage { reached: true, .. }));
        assert!(out.explain().unwrap().contains("no image on the clipboard"));
    }

    #[test]
    fn arboard_is_the_last_resort() {
        let fake = Fake {
            native: Some(b"PNG".to_vec()),
            ..Default::default()
        };
        let out = read_image(&linux(Some(":0"), None, false), &fake);
        assert_eq!(
            out,
            Outcome::Found {
                bytes: b"PNG".to_vec(),
                backend: "arboard"
            }
        );
    }

    #[test]
    fn macos_prefers_pngpaste_then_osascript() {
        let mac = Env {
            os: "macos",
            ..Default::default()
        };
        let fake = Fake::default().reply("pngpaste -", b"PNG");
        assert!(matches!(
            read_image(&mac, &fake),
            Outcome::Found {
                backend: "pngpaste",
                ..
            }
        ));

        let fake = Fake::default().reply(
            "osascript -e the clipboard as «class PNGf»",
            "«data PNGf89504E47»".as_bytes(),
        );
        assert_eq!(
            read_image(&mac, &fake),
            Outcome::Found {
                bytes: vec![0x89, 0x50, 0x4e, 0x47],
                backend: "osascript"
            }
        );
    }

    #[test]
    fn applescript_hex_rejects_garbage() {
        assert_eq!(decode_applescript_data("«data PNGf0A0»"), None);
        assert_eq!(decode_applescript_data("nope"), None);
    }

    #[test]
    fn windows_decodes_powershell_base64() {
        use base64::Engine as _;
        let win = Env {
            os: "windows",
            ..Default::default()
        };
        let b64 = base64::engine::general_purpose::STANDARD.encode(b"PNG!");
        let key = format!("powershell -NoProfile -NonInteractive -Command {POWERSHELL_SCRIPT}");
        let fake = Fake::default().reply(&key, format!("{b64}\r\n").as_bytes());
        assert_eq!(
            read_image(&win, &fake),
            Outcome::Found {
                bytes: b"PNG!".to_vec(),
                backend: "powershell"
            }
        );
    }

    #[cfg(unix)]
    fn script(dir: &std::path::Path, name: &str, body: &str) -> String {
        use std::os::unix::fs::PermissionsExt as _;
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path.display().to_string()
    }

    #[test]
    #[cfg(unix)]
    fn system_runner_captures_stdout_reports_failure_and_kills_hangs() {
        let dir = tempfile::tempdir().unwrap();
        let ok = script(dir.path(), "ok", "printf PNGDATA");
        let bad = script(dir.path(), "bad", "echo err >&2; exit 3");
        let hang = script(dir.path(), "hang", "sleep 30");
        let envp = script(dir.path(), "envp", "printf %s \"$DISPLAY\"");
        assert_eq!(SystemRunner.run(&ok, &[], &[]), Ok(b"PNGDATA".to_vec()));
        assert_eq!(SystemRunner.run(&bad, &[], &[]), Err(RunError::Failed));
        assert_eq!(
            SystemRunner.run("/nonexistent/forge-stub", &[], &[]),
            Err(RunError::NotFound)
        );
        assert_eq!(
            SystemRunner.run(&envp, &[], &[("DISPLAY", "localhost:10.0")]),
            Ok(b"localhost:10.0".to_vec())
        );
        let started = std::time::Instant::now();
        assert_eq!(SystemRunner.run(&hang, &[], &[]), Err(RunError::TimedOut));
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    /// Manual check: `PATH=<dir with stub xclip>:$PATH FORGE_STUB_XCLIP=1 cargo test ... --ignored`.
    #[test]
    #[ignore]
    fn real_path_lookup_finds_stub_xclip() {
        if std::env::var_os("FORGE_STUB_XCLIP").is_none() {
            return;
        }
        let env = Env {
            os: "linux",
            display: Some(":9".into()),
            ..Default::default()
        };
        match read_image(&env, &SystemRunner) {
            Outcome::Found { bytes, backend } => {
                assert_eq!(backend, "xclip");
                assert_eq!(bytes, b"\x89PNG-from-stub");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn target_choice_prefers_png() {
        assert_eq!(pick_image_target("image/jpeg image/png"), Some("image/png"));
        assert_eq!(pick_image_target("text/plain"), None);
    }
}
