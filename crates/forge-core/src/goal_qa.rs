//! The QA runner for behaviour assertions (docs/features/goal-contract.md, "Behaviour assertions
//! (QA)"). The worker never runs these: after the read-only checks pass, the validator starts the
//! thing under test in its own process group, waits for it, probes it, keeps the output tail (and
//! a screenshot when the probe asks for one) as proof, then always runs the cleanup and kills the
//! group.

use super::*;
use forge_types::goal_contract::behaviour::Behaviour;
use forge_types::goal_contract::{tail, Assertion, AssertionResult, PROOF_CHARS};
use forge_types::SideEffect;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;

const POLL: Duration = Duration::from_millis(250);
const PROBE_SECS: u64 = 10;
const CLEANUP_SECS: u64 = 20;

/// The scratch directory, port and process group of one behaviour run. Dropping it kills the
/// group and removes the directory, so an aborted validation leaks nothing.
struct QaRun {
    dir: PathBuf,
    port: u16,
    pgid: Option<u32>,
}

impl QaRun {
    fn log(&self) -> PathBuf {
        self.dir.join("run.log")
    }

    fn expand(&self, text: &str) -> String {
        text.replace("${FORGE_QA_PORT}", &self.port.to_string())
            .replace("$FORGE_QA_PORT", &self.port.to_string())
            .replace("${FORGE_QA_DIR}", &self.dir.to_string_lossy())
            .replace("$FORGE_QA_DIR", &self.dir.to_string_lossy())
    }

    fn envs(&self) -> [(&'static str, String); 2] {
        [
            ("FORGE_QA_PORT", self.port.to_string()),
            ("FORGE_QA_DIR", self.dir.to_string_lossy().into_owned()),
        ]
    }
}

impl Drop for QaRun {
    fn drop(&mut self) {
        kill_group(self.pgid);
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn kill_group(pgid: Option<u32>) {
    #[cfg(unix)]
    if let Some(pg) = pgid {
        // SAFETY: signalling a process group we created; ESRCH (already gone) is harmless.
        unsafe { libc::kill(-(pg as i32), libc::SIGKILL) };
    }
    #[cfg(not(unix))]
    let _ = pgid;
}

fn own_group(cmd: &mut Command) {
    #[cfg(unix)]
    cmd.process_group(0);
    #[cfg(not(unix))]
    let _ = cmd;
}

fn shell(line: &str, cwd: &Path, envs: &[(&'static str, String)]) -> Command {
    let mut cmd = Command::new("sh");
    cmd.arg("-c")
        .arg(line)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .kill_on_drop(true);
    for (k, v) in envs {
        cmd.env(k, v);
    }
    own_group(&mut cmd);
    cmd
}

/// Run `line` to completion: `(exit code, combined output)`, or the reason it did not finish.
async fn run_line(
    line: &str,
    cwd: &Path,
    envs: &[(&'static str, String)],
    secs: u64,
) -> Result<(Option<i32>, String), String> {
    let mut cmd = shell(line, cwd, envs);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let child = cmd.spawn().map_err(|e| format!("could not run: {e}"))?;
    let pgid = child.id();
    let waited = tokio::time::timeout(Duration::from_secs(secs), child.wait_with_output()).await;
    kill_group(pgid);
    match waited {
        Ok(Ok(out)) => {
            let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
            text.push_str(&String::from_utf8_lossy(&out.stderr));
            Ok((out.status.code(), text))
        }
        Ok(Err(e)) => Err(format!("could not run: {e}")),
        Err(_) => Err(format!("timed out after {secs}s")),
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .and_then(|l| l.local_addr())
        .map_or(0, |a| a.port())
}

fn read_log(run: &QaRun) -> String {
    std::fs::read_to_string(run.log()).unwrap_or_default()
}

/// What one probe attempt produced.
enum Probe {
    /// The thing did not answer (connection refused, non-zero exit): worth retrying.
    NotReady(String),
    Answered(String),
}

async fn http_probe(url: &str) -> Probe {
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
    {
        Ok(c) => c,
        Err(e) => return Probe::NotReady(e.to_string()),
    };
    match client.get(url).send().await {
        Ok(resp) => {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            Probe::Answered(format!("HTTP {}\n{}", status.as_u16(), body.trim()))
        }
        Err(e) => Probe::NotReady(format!("GET {url}: {e}")),
    }
}

/// A `screenshot <url>` probe drives the browser tool: open headless, shoot, report the page.
/// `Err` is the reason it was skipped.
async fn browser_probe(
    session: &mut Session,
    url: &str,
) -> Result<(String, Option<PathBuf>), String> {
    let Some(tool) = session.tools.get("browser") else {
        return Err("the browser tool is not available".to_string());
    };
    let args = serde_json::json!({"action": "batch", "steps": [
        {"action": "open", "url": url, "headless": true},
        {"action": "screenshot", "caption": format!("QA: {url}")},
        {"action": "close"},
    ]});
    let out = tokio::time::timeout(Duration::from_secs(60), tool.run(&args))
        .await
        .map_err(|_| "the browser probe timed out".to_string())?
        .map_err(|e| format!("the browser tool failed: {e}"))?;
    let shot = out
        .split("screenshot written to ")
        .nth(1)
        .and_then(|rest| rest.split(" (").next())
        .map(PathBuf::from)
        .filter(|p| p.is_file());
    Ok((out, shot))
}

fn port_open(port: u16) -> bool {
    std::net::TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
        Duration::from_millis(200),
    )
    .is_ok()
}

fn is_ready(ready: &str, run: &QaRun) -> bool {
    if let Some(text) = ready.strip_prefix("line:") {
        return read_log(run).contains(text.trim());
    }
    let port = ready
        .strip_prefix("port:")
        .and_then(|p| run.expand(p.trim()).parse().ok())
        .unwrap_or(run.port);
    port_open(port)
}

impl Session {
    fn qa_allowed(&self, line: &str) -> bool {
        let args = serde_json::json!({"command": line, "cwd": self.workspace.root()});
        let workspace = self.workspace_binding.read().ok().map(|g| g.clone());
        // The approved contract card showed both commands; what still binds them is the safety
        // floor and the deny rules, which `Bypass` leaves in force.
        crate::permission::decide_in(
            PermissionMode::Bypass,
            SideEffect::Shell,
            "shell",
            &args,
            &self.rules,
            workspace.as_deref(),
        ) == PermissionDecision::Allow
    }

    /// Run one behaviour assertion end to end. Never leaves a process or a scratch directory.
    pub(crate) async fn run_behaviour_check(
        &mut self,
        assertion: &Assertion,
        timeout: u64,
    ) -> AssertionResult {
        let b: &Behaviour = assertion
            .behaviour
            .as_ref()
            .expect("run_behaviour_check needs a behaviour assertion");
        let result = |passed: bool, reason: String, proof: String, artifact: Option<String>| {
            AssertionResult {
                id: assertion.id,
                passed,
                proof,
                reason,
                artifact,
            }
        };
        for line in [b.run.as_str(), b.probe.as_str(), b.cleanup.as_str()] {
            let is_shell =
                !line.is_empty() && !line.starts_with("GET ") && !line.starts_with("screenshot ");
            if is_shell && !self.qa_allowed(line) {
                return result(
                    false,
                    format!(
                        "refused: `{}` is blocked by a deny rule",
                        forge_types::goal_contract::clip(line, 60)
                    ),
                    String::new(),
                    None,
                );
            }
        }
        let dir = std::env::temp_dir().join(format!(
            "forge-qa-{}-{}-{}",
            std::process::id(),
            assertion.id,
            free_port()
        ));
        if let Err(e) = std::fs::create_dir_all(&dir) {
            return result(
                false,
                format!("could not create the QA directory: {e}"),
                String::new(),
                None,
            );
        }
        let mut run = QaRun {
            dir,
            port: free_port(),
            pgid: None,
        };
        let root = self.workspace.root().to_path_buf();
        let envs = run.envs();
        let Ok(err) = std::fs::File::create(run.log()) else {
            return result(
                false,
                "could not open the QA log".to_string(),
                String::new(),
                None,
            );
        };
        let Ok(out) = err.try_clone() else {
            return result(
                false,
                "could not open the QA log".to_string(),
                String::new(),
                None,
            );
        };
        let mut cmd = shell(&run.expand(&b.run), &root, &envs);
        cmd.stdout(Stdio::from(out)).stderr(Stdio::from(err));
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => return result(false, format!("could not start: {e}"), String::new(), None),
        };
        run.pgid = child.id();

        let outcome = self
            .drive_probe(&mut child, &run, b, &root, &envs, timeout)
            .await;

        if !b.cleanup.trim().is_empty() {
            let _ = run_line(&run.expand(&b.cleanup), &root, &envs, CLEANUP_SECS).await;
        }
        kill_group(run.pgid);
        let _ = child.wait().await;

        let (verdict, output, shot) = match outcome {
            Ok(v) => v,
            Err(skip) => {
                return result(
                    true,
                    format!("{} {skip}", forge_types::goal_contract::UNVERIFIED),
                    format!("skipped: {skip}"),
                    None,
                );
            }
        };
        let proof = tail(
            &format!("probe: {}\n{output}", run.expand(&b.probe)),
            PROOF_CHARS,
        );
        let artifact = shot.map(|p| p.to_string_lossy().into_owned());
        match verdict {
            Ok(()) => result(true, String::new(), proof, artifact),
            Err(reason) => result(false, reason, proof, artifact),
        }
    }

    /// Wait for readiness, probe, and judge. The outer `Err` means "skipped, and why"; the
    /// verdict inside is the assertion's pass or fail with the probe output.
    #[allow(clippy::type_complexity)]
    async fn drive_probe(
        &mut self,
        child: &mut tokio::process::Child,
        run: &QaRun,
        b: &Behaviour,
        root: &Path,
        envs: &[(&'static str, String)],
        timeout: u64,
    ) -> Result<(Result<(), String>, String, Option<PathBuf>), String> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout);
        let probe = run.expand(&b.probe);
        let gated = !b.ready.trim().is_empty();
        let mut last = String::new();
        loop {
            // A run that finished cleanly was a setup step (the probe is the CLI under test);
            // one that died is the failure, with its log as proof.
            if let Ok(Some(status)) = child.try_wait() {
                if !status.success() {
                    let code = status
                        .code()
                        .map_or("a signal".to_string(), |c| format!("exit {c}"));
                    return Ok((
                        Err(format!(
                            "the process ended ({code}) before the probe passed"
                        )),
                        read_log(run),
                        None,
                    ));
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return Ok((
                    Err(format!("not ready after {timeout}s")),
                    format!("{last}\n--- process log ---\n{}", read_log(run)),
                    None,
                ));
            }
            if gated && !is_ready(&run.expand(&b.ready), run) {
                tokio::time::sleep(POLL).await;
                continue;
            }
            let mut shot = None;
            let attempt = if let Some(url) = probe.strip_prefix("GET ") {
                http_probe(url.trim()).await
            } else if let Some(url) = probe.strip_prefix("screenshot ") {
                let (text, path) = browser_probe(self, url.trim()).await?;
                shot = path;
                Probe::Answered(text)
            } else {
                match run_line(&probe, root, envs, PROBE_SECS).await {
                    Ok((Some(0), text)) => Probe::Answered(text),
                    Ok((code, text)) => Probe::NotReady(format!(
                        "exit {}\n{text}",
                        code.map_or("signal".to_string(), |c| c.to_string())
                    )),
                    Err(why) => Probe::NotReady(why),
                }
            };
            match attempt {
                Probe::Answered(text) => {
                    let shot = self.keep_screenshot(shot);
                    let verdict = if b.expect.is_empty() || text.contains(&b.expect) {
                        Ok(())
                    } else {
                        Err(format!("probe output does not contain \"{}\"", b.expect))
                    };
                    return Ok((verdict, text, shot));
                }
                Probe::NotReady(text) => last = text,
            }
            tokio::time::sleep(POLL).await;
        }
    }

    /// File a screenshot the browser tool left outside the evidence folder into it.
    fn keep_screenshot(&self, shot: Option<PathBuf>) -> Option<PathBuf> {
        let path = shot?;
        let ledger = forge_tools::evidence::Ledger::new(self.workspace.root(), &self.id)?;
        if path.starts_with(ledger.dir()) {
            return Some(path);
        }
        let bytes = std::fs::read(&path).ok()?;
        let capture = forge_tools::evidence::Capture {
            caption: Some("QA behaviour check"),
            ..Default::default()
        };
        ledger
            .record(
                forge_tools::evidence::EvidenceKind::Screenshot,
                "qa",
                "png",
                &bytes,
                capture,
            )
            .ok()
            .map(|(p, _)| p)
    }
}
