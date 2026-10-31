//! One kernel child process: spawn, handshake, cell execution with timeout/interrupt/kill, and
//! teardown. Cell output rides the child's stderr pipe (stdout and stderr share it), cut into
//! cells by a nonce-bearing sentinel; structured replies come back as JSON lines on stdout.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{mpsc, Notify};

use super::output::HeadTail;
use crate::shell::{kill_process_group, put_in_own_process_group};

const RUNNER: &str = include_str!("runner.py");
pub(super) const INTERRUPT_GRACE: Duration = Duration::from_secs(3);
const SENTINEL_WAIT: Duration = Duration::from_secs(2);
const PRIVILEGE_NAMES: [&str; 7] = ["sudo", "su", "doas", "pkexec", "run0", "passwd", "sudoedit"];

pub(super) enum Piece {
    Data(Vec<u8>),
    CellEnd(u64),
}

/// Splits the stderr byte stream on `\x1eFORGE-END <nonce> <cell>\x1e\n`. A sentinel with another
/// nonce is plain data; a partial sentinel at the end of a chunk is held for the next one.
pub(super) struct Splitter {
    prefix: Vec<u8>,
    pending: Vec<u8>,
}

impl Splitter {
    pub(super) fn new(nonce: &str) -> Self {
        Self {
            prefix: format!("\x1eFORGE-END {nonce} ").into_bytes(),
            pending: Vec::new(),
        }
    }

    pub(super) fn feed(&mut self, bytes: &[u8]) -> Vec<Piece> {
        self.pending.extend_from_slice(bytes);
        let buf = std::mem::take(&mut self.pending);
        let mut out = Vec::new();
        let mut data = Vec::new();
        let mut i = 0;
        'scan: while i < buf.len() {
            let Some(rel) = buf[i..].iter().position(|b| *b == 0x1e) else {
                data.extend_from_slice(&buf[i..]);
                i = buf.len();
                break;
            };
            let at = i + rel;
            data.extend_from_slice(&buf[i..at]);
            let rest = &buf[at..];
            let hold = if rest.len() < self.prefix.len() {
                if self.prefix.starts_with(rest) {
                    true
                } else {
                    data.push(0x1e);
                    i = at + 1;
                    continue 'scan;
                }
            } else if !rest.starts_with(&self.prefix) {
                data.push(0x1e);
                i = at + 1;
                continue 'scan;
            } else {
                let after = &rest[self.prefix.len()..];
                match after.iter().position(|b| *b == 0x1e) {
                    Some(p) if p + 1 < after.len() => {
                        let cell = (after[p + 1] == b'\n')
                            .then(|| std::str::from_utf8(&after[..p]).ok()?.parse::<u64>().ok())
                            .flatten();
                        if let Some(cell) = cell {
                            if !data.is_empty() {
                                out.push(Piece::Data(std::mem::take(&mut data)));
                            }
                            out.push(Piece::CellEnd(cell));
                            i = at + self.prefix.len() + p + 2;
                        } else {
                            data.push(0x1e);
                            i = at + 1;
                        }
                        continue 'scan;
                    }
                    Some(_) => true,
                    None if after.len() > 24 => {
                        data.push(0x1e);
                        i = at + 1;
                        continue 'scan;
                    }
                    None => true,
                }
            };
            if hold {
                i = at;
                break;
            }
        }
        if !data.is_empty() {
            out.push(Piece::Data(data));
        }
        self.pending = buf[i..].to_vec();
        out
    }
}

#[derive(Default)]
struct Shared {
    current: HeadTail,
    done: HashMap<u64, (String, bool)>,
}

struct Capture {
    state: Mutex<Shared>,
    wake: Notify,
}

impl Capture {
    fn lock(&self) -> std::sync::MutexGuard<'_, Shared> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn take_current(&self) -> (String, bool) {
        std::mem::take(&mut self.lock().current).into_text()
    }
}

pub(super) enum Outcome {
    Done {
        reply: Value,
        output: String,
        truncated: bool,
        timed_out_secs: Option<u64>,
    },
    Killed {
        output: String,
        truncated: bool,
    },
    Died {
        status: String,
        output: String,
        truncated: bool,
    },
}

struct InFlight {
    pgid: i32,
    done: bool,
}

impl Drop for InFlight {
    fn drop(&mut self) {
        if !self.done {
            signal_group(self.pgid, libc::SIGINT);
        }
    }
}

fn signal_group(pgid: i32, sig: i32) {
    unsafe { libc::kill(-pgid, sig) };
}

pub(super) struct Kernel {
    child: Child,
    pgid: i32,
    stdin: Option<ChildStdin>,
    replies: mpsc::UnboundedReceiver<Value>,
    capture: Arc<Capture>,
    shim_root: PathBuf,
    pub(super) cwd: PathBuf,
    next_cell: u64,
    busy: Option<u64>,
    killed_by_us: bool,
}

impl Drop for Kernel {
    fn drop(&mut self) {
        kill_process_group(Some(self.pgid));
        let _ = std::fs::remove_dir_all(&self.shim_root);
    }
}

fn random_hex() -> std::io::Result<String> {
    let mut bytes = [0u8; 16];
    std::io::Read::read_exact(&mut std::fs::File::open("/dev/urandom")?, &mut bytes)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// Executables named like the privileged binaries that refuse politely, first on the kernel's
/// PATH, so a dynamically built `subprocess.run(['sudo', ...])` never reaches PAM.
fn make_shim(nonce: &str) -> std::io::Result<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let root = std::env::temp_dir().join(format!("forge-kernel-{nonce}"));
    let bin = root.join("bin");
    std::fs::create_dir_all(&bin)?;
    for name in PRIVILEGE_NAMES {
        let path = bin.join(name);
        std::fs::write(
            &path,
            format!(
                "#!/bin/sh\necho \"Forge does not run privileged commands; ask the user to run: {name} $*\" >&2\nexit 126\n"
            ),
        )?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(root)
}

pub(super) fn describe_status(status: std::process::ExitStatus, killed_by_us: bool) -> String {
    use std::os::unix::process::ExitStatusExt;
    if let Some(sig) = status.signal() {
        let name = match sig {
            libc::SIGKILL => "SIGKILL".to_string(),
            libc::SIGSEGV => "SIGSEGV".to_string(),
            libc::SIGABRT => "SIGABRT".to_string(),
            libc::SIGTERM => "SIGTERM".to_string(),
            other => format!("signal {other}"),
        };
        if sig == libc::SIGKILL && !killed_by_us {
            format!("{name}, likely out of memory")
        } else {
            name
        }
    } else {
        format!("exit {}", status.code().unwrap_or(-1))
    }
}

impl Kernel {
    pub(super) async fn spawn(python: &Path, cwd: &Path) -> Result<Self, String> {
        let nonce = random_hex().map_err(|e| format!("nonce: {e}"))?;
        let shim_root = make_shim(&nonce).map_err(|e| format!("shim: {e}"))?;
        let path = std::env::var_os("PATH").unwrap_or_default();
        let joined = std::env::join_paths(
            std::iter::once(shim_root.join("bin")).chain(std::env::split_paths(&path)),
        )
        .map_err(|e| format!("PATH: {e}"))?;
        let mut cmd = Command::new(python);
        cmd.args(["-u", "-c", RUNNER])
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        put_in_own_process_group(&mut cmd);
        for var in forge_types::child_env::scrubbed_vars() {
            cmd.env_remove(var);
        }
        cmd.env_remove("FORGE_DB")
            .env("PYTHONUNBUFFERED", "1")
            .env("PYTHONIOENCODING", "utf-8")
            .env("MPLBACKEND", "Agg")
            .env("TERM", "dumb")
            .env("NO_COLOR", "1")
            .env("PIP_DISABLE_PIP_VERSION_CHECK", "1")
            .env("PIP_PROGRESS_BAR", "off")
            .env("FORGE_KERNEL_NONCE", &nonce)
            .env("PATH", joined);
        let mut child = cmd.spawn().map_err(|e| {
            let _ = std::fs::remove_dir_all(&shim_root);
            format!("{}: {e}", python.display())
        })?;
        let pgid = child.id().map(|p| p as i32).unwrap_or(0);
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let mut stderr = child.stderr.take().expect("piped stderr");
        let (tx, replies) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if let Ok(value) = serde_json::from_str::<Value>(&line) {
                    if tx.send(value).is_err() {
                        break;
                    }
                }
            }
        });
        let capture = Arc::new(Capture {
            state: Mutex::new(Shared::default()),
            wake: Notify::new(),
        });
        let reader = Arc::clone(&capture);
        tokio::spawn(async move {
            let mut splitter = Splitter::new(&nonce);
            let mut buf = vec![0u8; 16 * 1024];
            loop {
                let n = match stderr.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                let mut state = reader.lock();
                for piece in splitter.feed(&buf[..n]) {
                    match piece {
                        Piece::Data(bytes) => state.current.push(&bytes),
                        Piece::CellEnd(cell) => {
                            let text = std::mem::take(&mut state.current).into_text();
                            state.done.insert(cell, text);
                        }
                    }
                }
                drop(state);
                reader.wake.notify_one();
            }
            reader.wake.notify_one();
        });
        Ok(Self {
            child,
            pgid,
            stdin: Some(stdin),
            replies,
            capture,
            shim_root,
            cwd: cwd.to_path_buf(),
            next_cell: 0,
            busy: None,
            killed_by_us: false,
        })
    }

    /// Returns the interpreter version, or why the handshake failed (with the stderr tail).
    pub(super) async fn hello(&mut self, budget: Duration) -> Result<(u32, u32), String> {
        let line = format!("{}\n", json!({"op": "hello"}));
        self.send(&line).await.map_err(|e| format!("write: {e}"))?;
        let reply = tokio::time::timeout(budget, self.replies.recv()).await;
        match reply {
            Ok(Some(v)) if v["status"] == "ready" => {
                let version = v["python"].as_str().unwrap_or("0.0.0");
                let mut parts = version.split('.').filter_map(|p| p.parse::<u32>().ok());
                let (major, minor) = (parts.next().unwrap_or(0), parts.next().unwrap_or(0));
                if (major, minor) < (3, 8) {
                    return Err(format!("python {version} is older than 3.8"));
                }
                Ok((major, minor))
            }
            Ok(_) => {
                tokio::time::sleep(Duration::from_millis(100)).await;
                let (tail, _) = self.capture.take_current();
                Err(stderr_tail(&tail))
            }
            Err(_) => Err("no handshake reply in time".to_string()),
        }
    }

    async fn send(&mut self, line: &str) -> std::io::Result<()> {
        match self.stdin.as_mut() {
            Some(stdin) => stdin.write_all(line.as_bytes()).await,
            None => Err(std::io::ErrorKind::BrokenPipe.into()),
        }
    }

    #[cfg(test)]
    pub(super) fn close_control(&mut self) {
        self.stdin = None;
    }

    #[cfg(test)]
    pub(super) fn pid(&self) -> i32 {
        self.pgid
    }

    pub(super) fn try_exited(&mut self) -> Option<String> {
        let status = self.child.try_wait().ok().flatten()?;
        Some(describe_status(status, self.killed_by_us))
    }

    async fn recv_reply(&mut self, id: u64) -> Option<Value> {
        loop {
            let value = self.replies.recv().await?;
            if value["id"].as_u64() == Some(id) {
                return Some(value);
            }
        }
    }

    async fn cell_output(&mut self, id: u64) -> (String, bool) {
        let deadline = tokio::time::Instant::now() + SENTINEL_WAIT;
        loop {
            let wake = self.capture.wake.notified();
            if let Some(done) = self.capture.lock().done.remove(&id) {
                return done;
            }
            if tokio::time::timeout_at(deadline, wake).await.is_err() {
                return self
                    .capture
                    .lock()
                    .done
                    .remove(&id)
                    .unwrap_or_else(|| self.capture.take_current());
            }
        }
    }

    /// Finish a cell an aborted turn left running. False means the kernel should be replaced.
    pub(super) async fn settle_busy(&mut self) -> bool {
        let Some(old) = self.busy else { return true };
        let settled = tokio::time::timeout(INTERRUPT_GRACE, self.recv_reply(old)).await;
        if !matches!(settled, Ok(Some(_))) {
            return false;
        }
        self.cell_output(old).await;
        self.busy = None;
        true
    }

    async fn died(&mut self) -> Outcome {
        let status = match tokio::time::timeout(Duration::from_secs(2), self.child.wait()).await {
            Ok(Ok(status)) => describe_status(status, self.killed_by_us),
            _ => {
                kill_process_group(Some(self.pgid));
                "connection lost".to_string()
            }
        };
        let (output, truncated) = self.capture.take_current();
        Outcome::Died {
            status,
            output,
            truncated,
        }
    }

    pub(super) async fn exec(&mut self, code: &str, timeout: Duration) -> Outcome {
        self.next_cell += 1;
        let id = self.next_cell;
        self.busy = Some(id);
        let line = format!("{}\n", json!({"op": "exec", "id": id, "code": code}));
        if self.send(&line).await.is_err() {
            return self.died().await;
        }
        let mut guard = InFlight {
            pgid: self.pgid,
            done: false,
        };
        let first = tokio::select! {
            reply = self.recv_reply(id) => Some(reply),
            _ = tokio::time::sleep(timeout) => None,
        };
        let (reply, timed_out_secs) = match first {
            Some(reply) => (reply, None),
            None => {
                signal_group(self.pgid, libc::SIGINT);
                match tokio::time::timeout(INTERRUPT_GRACE, self.recv_reply(id)).await {
                    Ok(reply) => (reply, Some(timeout.as_secs())),
                    Err(_) => {
                        guard.done = true;
                        self.killed_by_us = true;
                        kill_process_group(Some(self.pgid));
                        let _ =
                            tokio::time::timeout(Duration::from_secs(2), self.child.wait()).await;
                        let (output, truncated) = self.capture.take_current();
                        return Outcome::Killed { output, truncated };
                    }
                }
            }
        };
        guard.done = true;
        let Some(reply) = reply else {
            return self.died().await;
        };
        let (output, truncated) = self.cell_output(id).await;
        self.busy = None;
        Outcome::Done {
            reply,
            output,
            truncated,
            timed_out_secs,
        }
    }
}

fn stderr_tail(text: &str) -> String {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let tail = lines[lines.len().saturating_sub(3)..].join(" | ");
    if tail.is_empty() {
        "no output from the interpreter".to_string()
    } else {
        tail
    }
}
