//! `forge host add`: install Forge on another machine over SSH, run it as a service and pair it
//! to the Forge Anywhere fleet.
//!
//! The engine is blocking and drives three seams so the terminal command, the daemon's
//! add-a-machine routes and the tests share it: [`Remote`] (how a script runs on the machine),
//! [`Approver`] (who approves the new device) and a progress callback.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

use anyhow::{anyhow, bail, Context, Result};

use crate::hosts::{sh_quote, validate_name, validate_target, HostRecord};

pub(crate) const DEFAULT_INSTALL_URL: &str =
    "https://raw.githubusercontent.com/Adulari/forge/main/install.sh";

#[derive(Debug, Clone)]
pub(crate) struct AddOptions {
    pub(crate) target: String,
    pub(crate) name: Option<String>,
    pub(crate) lan: bool,
    pub(crate) port: Option<u16>,
    pub(crate) pair: bool,
    pub(crate) installer: Option<String>,
    pub(crate) install_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Event {
    Step(String),
    Remote(String),
    AwaitingApproval { safety_code: Option<String> },
}

pub(crate) trait Remote {
    /// Run `script` under `sh` on the machine, feeding `stdin` when given, and call `on_line`
    /// for each output line as it arrives. Returns the exit code.
    fn run(&self, script: &str, stdin: Option<&[u8]>, on_line: &mut dyn FnMut(&str))
        -> Result<i32>;
}

pub(crate) trait Approver {
    /// Approve the waiting device. `Ok(false)` means the person declined.
    fn approve(&self, challenge: &str, safety_code: Option<&str>) -> Result<bool>;
}

pub(crate) struct SshRemote {
    pub(crate) ssh: String,
    pub(crate) target: String,
    pub(crate) extra: Vec<String>,
}

impl Remote for SshRemote {
    fn run(
        &self,
        script: &str,
        stdin: Option<&[u8]>,
        on_line: &mut dyn FnMut(&str),
    ) -> Result<i32> {
        let mut cmd = Command::new(&self.ssh);
        cmd.args([
            "-o",
            "BatchMode=yes",
            "-o",
            "ConnectTimeout=15",
            "-o",
            "ServerAliveInterval=15",
        ])
        .args(&self.extra)
        .arg("--")
        .arg(&self.target)
        .arg(format!("sh -c {}", sh_quote(script)))
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
        let mut child = cmd
            .spawn()
            .with_context(|| format!("starting {}", self.ssh))?;
        if let (Some(bytes), Some(mut pipe)) = (stdin, child.stdin.take()) {
            // A remote that exits early closes the pipe; its exit code says why.
            let _ = pipe.write_all(bytes);
        }
        let stderr = child.stderr.take().expect("piped");
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        let tx_err = tx.clone();
        let err_thread = std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(|l| l.ok()) {
                if tx_err.send(line).is_err() {
                    break;
                }
            }
        });
        let stdout = child.stdout.take().expect("piped");
        let out_thread = std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(|l| l.ok()) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        for line in rx {
            on_line(&line);
        }
        let _ = out_thread.join();
        let _ = err_thread.join();
        Ok(child.wait()?.code().unwrap_or(-1))
    }
}

/// Approve through this machine's own `forge anywhere approve`, feeding the confirmation word.
/// `FORGE_HOST_FORGE_BIN` names a different local binary (used by the end-to-end checks).
pub(crate) struct LocalApprover {
    /// Type APPROVE on the person's behalf (they already chose to, in the UI or with `--yes`).
    /// When false the child inherits the terminal and asks.
    pub(crate) confirm: bool,
}

impl Approver for LocalApprover {
    fn approve(&self, challenge: &str, _safety_code: Option<&str>) -> Result<bool> {
        let exe = match std::env::var_os("FORGE_HOST_FORGE_BIN") {
            Some(path) => path.into(),
            None => std::env::current_exe().context("locating the forge binary")?,
        };
        let mut cmd = Command::new(exe);
        cmd.args(["anywhere", "approve", challenge]);
        if self.confirm {
            cmd.stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
        }
        let mut child = cmd.spawn().context("running forge anywhere approve")?;
        if self.confirm {
            if let Some(mut stdin) = child.stdin.take() {
                stdin.write_all(b"APPROVE\n")?;
            }
        }
        let output = child.wait_with_output()?;
        if !output.status.success() {
            let text = String::from_utf8_lossy(&output.stderr);
            bail!(
                "approving the new device failed: {}",
                text.lines().last().unwrap_or("see `forge anywhere status`")
            );
        }
        Ok(true)
    }
}

pub(crate) fn probe_script() -> String {
    r#"bin=""
for c in "$HOME/.local/bin/forge" "$(command -v forge 2>/dev/null)"; do
  if [ -n "$c" ] && [ -x "$c" ]; then bin="$c"; break; fi
done
echo "FORGE_OS=$(uname -s)-$(uname -m)"
echo "FORGE_HOST=$(hostname 2>/dev/null)"
echo "FORGE_BIN=$bin"
if [ -n "$bin" ]; then echo "FORGE_VERSION=$("$bin" --version 2>/dev/null | head -n 1)"; fi
if command -v curl >/dev/null 2>&1; then echo FORGE_FETCH=curl
elif command -v wget >/dev/null 2>&1; then echo FORGE_FETCH=wget
else echo FORGE_FETCH=none; fi"#
        .to_string()
}

pub(crate) fn install_script(url: &str, fetch: &str) -> String {
    let get = if fetch == "wget" {
        format!("wget -qO- {}", sh_quote(url))
    } else {
        format!("curl -fsSL {}", sh_quote(url))
    };
    format!("export FORGE_DESKTOP=0\n{get} | sh\necho \"FORGE_BIN=$HOME/.local/bin/forge\"\n")
}

pub(crate) fn service_script(bin: &str, lan: bool, port: Option<u16>) -> String {
    let exposure = if lan { "--lan" } else { "--local" };
    let port = port.map(|p| format!(" --port {p}")).unwrap_or_default();
    let bin = sh_quote(bin);
    format!("{bin} service install {exposure}{port} && {bin} service status\n")
}

pub(crate) fn pair_script(bin: &str, name: Option<&str>) -> String {
    let name = name
        .map(|n| format!(" --name {}", sh_quote(n)))
        .unwrap_or_default();
    format!("{} anywhere setup{name}\n", sh_quote(bin))
}

pub(crate) fn kv<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    line.strip_prefix(key)?.strip_prefix('=').map(str::trim)
}

/// The challenge from the remote's `forge anywhere approve '<challenge>'` instruction.
pub(crate) fn parse_challenge(line: &str) -> Option<&str> {
    let rest = line.split_once("forge anywhere approve '")?.1;
    let end = rest.find('\'')?;
    let challenge = &rest[..end];
    (!challenge.is_empty()).then_some(challenge)
}

pub(crate) fn parse_safety_code(line: &str) -> Option<&str> {
    let code = line.trim().strip_prefix("Safety code:")?.trim();
    (!code.is_empty()).then_some(code)
}

/// The caller records it with `hosts::upsert`: the engine stays free of the filesystem.
pub(crate) struct AddOutcome {
    pub(crate) record: HostRecord,
}

pub(crate) fn add_host(
    options: &AddOptions,
    remote: &dyn Remote,
    approver: &dyn Approver,
    progress: &mut dyn FnMut(Event),
) -> Result<AddOutcome> {
    let target = validate_target(&options.target)?;
    if let Some(name) = options.name.as_deref() {
        validate_name(name)?;
    }
    let now_ms = || {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    };

    progress(Event::Step(format!("Connecting to {target}")));
    let mut bin = String::new();
    let mut version = String::new();
    let mut fetch = String::from("none");
    let mut host_name = String::new();
    let mut tail = Vec::<String>::new();
    let code = remote.run(&probe_script(), None, &mut |line| {
        if let Some(v) = kv(line, "FORGE_BIN") {
            bin = v.to_string();
        } else if let Some(v) = kv(line, "FORGE_VERSION") {
            version = v.to_string();
        } else if let Some(v) = kv(line, "FORGE_FETCH") {
            fetch = v.to_string();
        } else if let Some(v) = kv(line, "FORGE_HOST") {
            host_name = v.to_string();
        } else if let Some(v) = kv(line, "FORGE_OS") {
            progress(Event::Remote(format!("system: {v}")));
        } else {
            tail.push(line.to_string());
        }
    })?;
    if code != 0 {
        bail!(
            "could not run commands on {target} over ssh (exit {code}): {}. Check that `ssh {target}` works without a password prompt.",
            tail.last().map(String::as_str).unwrap_or("no output")
        );
    }

    if bin.is_empty() {
        progress(Event::Step("Installing Forge".into()));
        let mut lines = Vec::<String>::new();
        let code = match options.installer.as_deref() {
            Some(path) => {
                let bytes = std::fs::read(path).with_context(|| format!("reading {path}"))?;
                let script =
                    "export FORGE_DESKTOP=0\nsh -s\necho \"FORGE_BIN=$HOME/.local/bin/forge\"\n";
                remote.run(script, Some(&bytes), &mut |l| lines.push(l.to_string()))?
            }
            None => {
                if fetch == "none" {
                    bail!("{target} has neither curl nor wget; rerun with --installer <install.sh> to push a local copy");
                }
                remote.run(
                    &install_script(&options.install_url, &fetch),
                    None,
                    &mut |l| lines.push(l.to_string()),
                )?
            }
        };
        for line in &lines {
            match kv(line, "FORGE_BIN") {
                Some(v) => bin = v.to_string(),
                None => progress(Event::Remote(line.clone())),
            }
        }
        if code != 0 || bin.is_empty() {
            bail!("installing Forge on {target} failed (exit {code})");
        }
    } else {
        progress(Event::Step(format!("Forge already installed ({version})")));
    }

    progress(Event::Step("Starting the Forge service".into()));
    let mut failed = Vec::<String>::new();
    let code = remote.run(
        &service_script(&bin, options.lan, options.port),
        None,
        &mut |l| {
            failed.push(l.to_string());
            progress(Event::Remote(l.to_string()));
        },
    )?;
    if code != 0 {
        bail!(
            "`forge service install` failed on {target} (exit {code}): {}",
            failed.last().map(String::as_str).unwrap_or("no output")
        );
    }

    let display = options
        .name
        .clone()
        .filter(|n| !n.trim().is_empty())
        .or_else(|| (!host_name.is_empty()).then(|| host_name.clone()))
        .unwrap_or_else(|| target.to_string());

    let mut paired = false;
    if options.pair {
        progress(Event::Step("Pairing to Forge Anywhere".into()));
        let mut challenge: Option<String> = None;
        let mut safety: Option<String> = None;
        let mut approve_error: Option<anyhow::Error> = None;
        let mut approved = false;
        let mut approval_started = false;
        let code = remote.run(
            &pair_script(&bin, options.name.as_deref()),
            None,
            &mut |line| {
                progress(Event::Remote(line.to_string()));
                if let Some(code) = parse_safety_code(line) {
                    safety = Some(code.to_string());
                }
                if let Some(c) = parse_challenge(line) {
                    challenge = Some(c.to_string());
                }
                // The safety code is printed after the challenge; approve once both are known.
                let ready = challenge.is_some()
                    && (safety.is_some() || line.contains("Waiting for approval"));
                if ready && !approval_started {
                    approval_started = true;
                    progress(Event::AwaitingApproval {
                        safety_code: safety.clone(),
                    });
                    match approver
                        .approve(challenge.as_deref().unwrap_or_default(), safety.as_deref())
                    {
                        Ok(true) => approved = true,
                        Ok(false) => {}
                        Err(e) => approve_error = Some(e),
                    }
                }
            },
        )?;
        if let Some(e) = approve_error {
            return Err(e);
        }
        if approval_started && !approved {
            bail!("approval was declined; {display} is installed but not paired");
        }
        if code != 0 {
            return Err(anyhow!(
                "`forge anywhere setup` failed on {target} (exit {code}); the service is installed, rerun `forge host add {target}` to resume"
            ));
        }
        paired = true;
    }

    let record = HostRecord {
        name: display,
        target: target.to_string(),
        added_ms: now_ms(),
        paired,
        version,
    };
    progress(Event::Step(if paired {
        "Done: running and paired to your fleet".into()
    } else {
        "Done: service running".into()
    }));
    Ok(AddOutcome { record })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    struct Fake {
        calls: RefCell<Vec<String>>,
        installed: bool,
        pair_lines: Vec<&'static str>,
        service_code: i32,
    }

    impl Remote for Fake {
        fn run(
            &self,
            script: &str,
            _stdin: Option<&[u8]>,
            on_line: &mut dyn FnMut(&str),
        ) -> Result<i32> {
            self.calls.borrow_mut().push(script.to_string());
            if script.contains("FORGE_FETCH") {
                on_line("FORGE_OS=Linux-x86_64");
                on_line("FORGE_HOST=vm1");
                on_line(&format!(
                    "FORGE_BIN={}",
                    if self.installed {
                        "/h/.local/bin/forge"
                    } else {
                        ""
                    }
                ));
                on_line("FORGE_FETCH=curl");
                return Ok(0);
            }
            if script.contains("| sh") {
                on_line("downloading");
                on_line("FORGE_BIN=/h/.local/bin/forge");
                return Ok(0);
            }
            if script.contains("service install") {
                on_line("service installed");
                return Ok(self.service_code);
            }
            for l in &self.pair_lines {
                on_line(l);
            }
            Ok(0)
        }
    }

    struct Yes(RefCell<Vec<String>>);
    impl Approver for Yes {
        fn approve(&self, challenge: &str, _s: Option<&str>) -> Result<bool> {
            self.0.borrow_mut().push(challenge.to_string());
            Ok(true)
        }
    }

    fn options() -> AddOptions {
        AddOptions {
            target: "dev@vm1".into(),
            name: Some("vm one".into()),
            lan: false,
            port: None,
            pair: true,
            installer: None,
            install_url: DEFAULT_INSTALL_URL.into(),
        }
    }

    fn with_data_dir<T>(f: impl FnOnce() -> T) -> T {
        f()
    }

    #[test]
    fn parses_the_remote_instructions() {
        let line = "  forge anywhere approve 'abc_DEF-123'";
        assert_eq!(parse_challenge(line), Some("abc_DEF-123"));
        assert_eq!(parse_challenge("forge anywhere approve ''"), None);
        assert_eq!(parse_safety_code("Safety code: 12-34"), Some("12-34"));
        assert_eq!(kv("FORGE_BIN=/x", "FORGE_BIN"), Some("/x"));
    }

    #[test]
    fn scripts_quote_every_operator_supplied_value() {
        let script = pair_script("/h/it's/forge", Some("a b'c"));
        assert!(script.contains("'/h/it'\\''s/forge' anywhere setup --name 'a b'\\''c'"));
        assert!(service_script("/f", false, Some(7795)).contains("--local --port 7795"));
        assert!(service_script("/f", true, None).contains("--lan"));
    }

    #[test]
    fn a_fresh_machine_is_installed_serviced_and_paired() {
        with_data_dir(|| {
            let fake = Fake {
                calls: RefCell::new(vec![]),
                installed: false,
                pair_lines: vec![
                    "Approve this device",
                    "  forge anywhere approve 'CHAL1'",
                    "Safety code: 11-22",
                    "Waiting for approval",
                ],
                service_code: 0,
            };
            let approver = Yes(RefCell::new(vec![]));
            let mut events = Vec::new();
            let out = add_host(&options(), &fake, &approver, &mut |e| events.push(e)).unwrap();
            assert!(out.record.paired);
            assert_eq!(out.record.name, "vm one");
            assert_eq!(approver.0.borrow().as_slice(), ["CHAL1"]);
            assert!(events.contains(&Event::AwaitingApproval {
                safety_code: Some("11-22".into())
            }));
            let calls = fake.calls.borrow();
            assert_eq!(calls.len(), 4, "probe, install, service, pair");
        });
    }

    #[test]
    fn an_installed_machine_skips_the_installer_and_no_pair_skips_pairing() {
        with_data_dir(|| {
            let fake = Fake {
                calls: RefCell::new(vec![]),
                installed: true,
                pair_lines: vec![],
                service_code: 0,
            };
            let mut o = options();
            o.pair = false;
            let out = add_host(&o, &fake, &Yes(RefCell::new(vec![])), &mut |_| {}).unwrap();
            assert!(!out.record.paired);
            assert_eq!(fake.calls.borrow().len(), 2, "probe and service only");
        });
    }

    #[test]
    fn a_failed_service_install_stops_before_pairing() {
        with_data_dir(|| {
            let fake = Fake {
                calls: RefCell::new(vec![]),
                installed: true,
                pair_lines: vec![],
                service_code: 3,
            };
            let err = add_host(&options(), &fake, &Yes(RefCell::new(vec![])), &mut |_| {})
                .err()
                .unwrap()
                .to_string();
            assert!(err.contains("service install"), "{err}");
            assert_eq!(fake.calls.borrow().len(), 2);
        });
    }

    #[test]
    fn a_hostile_target_never_reaches_the_remote() {
        let fake = Fake {
            calls: RefCell::new(vec![]),
            installed: true,
            pair_lines: vec![],
            service_code: 0,
        };
        let mut o = options();
        o.target = "-oProxyCommand=evil".into();
        assert!(add_host(&o, &fake, &Yes(RefCell::new(vec![])), &mut |_| {}).is_err());
        assert!(fake.calls.borrow().is_empty());
    }
}
