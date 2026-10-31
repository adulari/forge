//! What surrounds a worktree's life beyond `git worktree add/remove` (`[worktree]` in the user
//! config): where it lives, which ignored files follow the checkout into it, the `setup` command
//! that runs once it exists, and the `cleanup` command that runs before it goes away.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use forge_config::worktree::{WorktreeConfig, WorktreeRoot};

const HOOK_TIMEOUT: Duration = Duration::from_secs(600);
const OUTPUT_TAIL: usize = 2000;
/// How long to keep collecting output after the shell itself has exited. A backgrounded child
/// holds the pipes open indefinitely, so this is the cap on waiting for EOF.
const DRAIN_GRACE: Duration = Duration::from_secs(2);
const CAPTURE_LIMIT: usize = 256 * 1024;
const WALK_VISIT_LIMIT: usize = 50_000;
const COPY_BUDGET_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct HookResult {
    pub ran: bool,
    pub ok: bool,
    pub output: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct Provisioned {
    /// Repository-relative paths copied from the main checkout.
    pub copied: Vec<String>,
    pub skipped: Vec<String>,
    pub setup: HookResult,
}

impl Provisioned {
    /// One line for a tool result or toast; empty when nothing happened.
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        if !self.copied.is_empty() {
            parts.push(format!("copied {}", self.copied.join(", ")));
        }
        if self.setup.ran {
            parts.push(if self.setup.ok {
                "setup ran".to_string()
            } else {
                format!("setup FAILED: {}", self.setup.output.trim())
            });
        }
        parts.join("; ")
    }
}

/// Where a new worktree called `name` goes.
pub fn worktree_path(repo_root: &Path, cfg: &WorktreeConfig, name: &str) -> PathBuf {
    match cfg.root {
        WorktreeRoot::Project => repo_root.join(".forge").join("worktrees").join(name),
        WorktreeRoot::Global => global_root(repo_root).join(name),
    }
}

/// The directory that holds every worktree of `repo_root` under `root = "global"`.
pub fn global_root(repo_root: &Path) -> PathBuf {
    let canonical = repo_root
        .canonicalize()
        .unwrap_or_else(|_| repo_root.to_path_buf());
    let digest = {
        use sha2::{Digest, Sha256};
        hex::encode(Sha256::digest(canonical.to_string_lossy().as_bytes()))
    };
    let repo = canonical
        .file_name()
        .and_then(|n| n.to_str())
        .map(crate::worktree_ops::slug)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "repo".to_string());
    forge_config::data_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("worktrees")
        .join(format!("{repo}-{}", &digest[..8]))
}

/// Lets the owner of a background provision stop it: the session was archived or deleted, or the
/// daemon is going down. Cancelling kills the hook's whole process group.
#[derive(Default)]
pub struct SetupControl {
    cancelled: AtomicBool,
    running: AtomicBool,
    pgid: AtomicI32,
}

impl SetupControl {
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    pub fn set_running(&self, running: bool) {
        self.running.store(running, Ordering::Release);
    }

    /// Kill the hook's process group (if one is up) and make provisioning stop at its next step.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        #[cfg(unix)]
        {
            let pgid = self.pgid.load(Ordering::Acquire);
            if pgid > 0 {
                // SAFETY: signals the process group this hook created.
                unsafe { libc::killpg(pgid, libc::SIGKILL) };
            }
        }
    }

    /// [`Self::cancel`], then wait up to `timeout` for the provision to stop touching the tree.
    pub fn cancel_and_wait(&self, timeout: Duration) {
        self.cancel();
        let until = Instant::now() + timeout;
        while self.running.load(Ordering::Acquire) && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

/// Copy the configured git-ignored files and run `setup`. Never fails the caller: a problem is
/// reported in the result so the worktree stays usable.
pub fn provision(
    repo_root: &Path,
    worktree: &Path,
    branch: &str,
    cfg: &WorktreeConfig,
) -> Provisioned {
    provision_with(repo_root, worktree, branch, cfg, None)
}

/// [`provision`] that its owner can cancel through `control`.
pub fn provision_with(
    repo_root: &Path,
    worktree: &Path,
    branch: &str,
    cfg: &WorktreeConfig,
    control: Option<&SetupControl>,
) -> Provisioned {
    let mut out = Provisioned::default();
    let cancelled = || control.is_some_and(SetupControl::is_cancelled);
    if !cfg.copy_ignored.is_empty() && !cancelled() {
        copy_ignored(repo_root, worktree, &cfg.copy_ignored, &mut out, control);
    }
    if let Some(cmd) = cfg
        .setup
        .as_deref()
        .filter(|c| !c.trim().is_empty() && !cancelled())
    {
        out.setup = run_hook_within(cmd, repo_root, worktree, branch, HOOK_TIMEOUT, control);
    }
    out
}

/// Run `cleanup` in `worktree` before it is removed.
pub fn teardown(
    repo_root: &Path,
    worktree: &Path,
    branch: &str,
    cfg: &WorktreeConfig,
) -> HookResult {
    match cfg.cleanup.as_deref().filter(|c| !c.trim().is_empty()) {
        Some(cmd) if worktree.is_dir() => run_hook(cmd, repo_root, worktree, branch),
        _ => HookResult::default(),
    }
}

fn run_hook(cmd: &str, repo_root: &Path, worktree: &Path, branch: &str) -> HookResult {
    run_hook_within(cmd, repo_root, worktree, branch, HOOK_TIMEOUT, None)
}

fn run_hook_within(
    cmd: &str,
    repo_root: &Path,
    worktree: &Path,
    branch: &str,
    timeout: Duration,
    control: Option<&SetupControl>,
) -> HookResult {
    let mut command = if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.args(["/C", cmd]);
        c
    } else {
        let mut c = Command::new("sh");
        c.args(["-c", cmd]);
        c
    };
    command
        .current_dir(worktree)
        .env("FORGE_WORKTREE_PATH", worktree)
        .env("FORGE_WORKTREE_BRANCH", branch)
        .env("FORGE_REPO_ROOT", repo_root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Own process group, so a timeout can take down everything the shell started.
        command.process_group(0);
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(e) => {
            return HookResult {
                ran: true,
                ok: false,
                output: format!("could not start: {e}"),
            }
        }
    };
    if let (Some(control), Ok(pid)) = (control, i32::try_from(child.id())) {
        control.pgid.store(pid, Ordering::Release);
        if control.is_cancelled() {
            control.cancel();
        }
    }
    let readers: Vec<OutputReader> = [
        child
            .stdout
            .take()
            .map(|s| Box::new(s) as Box<dyn Read + Send>),
        child
            .stderr
            .take()
            .map(|s| Box::new(s) as Box<dyn Read + Send>),
    ]
    .into_iter()
    .flatten()
    .map(OutputReader::spawn)
    .collect();
    let started = Instant::now();
    let mut cancelled = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if started.elapsed() > timeout => {
                kill_group(&mut child);
                let _ = child.wait();
                break None;
            }
            Ok(None) if control.is_some_and(SetupControl::is_cancelled) => {
                kill_group(&mut child);
                let _ = child.wait();
                cancelled = true;
                break None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(40)),
            Err(_) => break None,
        }
    };
    if let Some(control) = control {
        control.pgid.store(0, Ordering::Release);
    }
    let drain_until = Instant::now() + DRAIN_GRACE;
    let mut text = String::new();
    for reader in readers {
        text.push_str(&reader.collect(drain_until));
    }
    let text = text.trim();
    let tail: String = {
        let chars: Vec<char> = text.chars().collect();
        chars[chars.len().saturating_sub(OUTPUT_TAIL)..]
            .iter()
            .collect()
    };
    match status {
        Some(status) => HookResult {
            ran: true,
            ok: status.success(),
            output: tail,
        },
        None if cancelled => HookResult {
            ran: true,
            ok: false,
            output: format!("cancelled\n{tail}"),
        },
        None => HookResult {
            ran: true,
            ok: false,
            output: format!("timed out after {}s\n{tail}", timeout.as_secs().max(1)),
        },
    }
}

fn kill_group(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        if let Ok(pid) = i32::try_from(child.id()) {
            // SAFETY: plain signal delivery to the process group this hook created.
            unsafe { libc::killpg(pid, libc::SIGKILL) };
        }
    }
    let _ = child.kill();
}

/// Collects a pipe on its own thread. The buffer is shared, so output read so far survives when
/// the caller stops waiting for an EOF that a leftover grandchild never delivers.
struct OutputReader {
    buf: Arc<Mutex<Vec<u8>>>,
    done: mpsc::Receiver<()>,
}

impl OutputReader {
    fn spawn(mut stream: Box<dyn Read + Send>) -> Self {
        let buf = Arc::new(Mutex::new(Vec::new()));
        let (tx, done) = mpsc::channel();
        let shared = Arc::clone(&buf);
        std::thread::spawn(move || {
            let mut chunk = [0u8; 4096];
            while let Ok(n) = stream.read(&mut chunk) {
                if n == 0 {
                    break;
                }
                let mut guard = shared.lock().unwrap_or_else(|e| e.into_inner());
                guard.extend_from_slice(&chunk[..n]);
                if guard.len() > CAPTURE_LIMIT {
                    let excess = guard.len() - CAPTURE_LIMIT;
                    guard.drain(..excess);
                }
            }
            let _ = tx.send(());
        });
        Self { buf, done }
    }

    fn collect(self, deadline: Instant) -> String {
        let wait = deadline.saturating_duration_since(Instant::now());
        let _ = self.done.recv_timeout(wait);
        let guard = self.buf.lock().unwrap_or_else(|e| e.into_inner());
        String::from_utf8_lossy(&guard).into_owned()
    }
}

fn copy_ignored(
    repo_root: &Path,
    worktree: &Path,
    patterns: &[String],
    out: &mut Provisioned,
    control: Option<&SetupControl>,
) {
    let mut builder = globset::GlobSetBuilder::new();
    for pattern in patterns {
        let pattern = pattern.trim().trim_start_matches("./");
        if pattern.is_empty() || pattern.starts_with('/') || pattern.contains("..") {
            out.skipped
                .push(format!("{pattern} (not a repository-relative pattern)"));
            continue;
        }
        // A bare name matches at any depth, like a gitignore line.
        let anywhere = if pattern.contains('/') {
            pattern.to_string()
        } else {
            format!("**/{pattern}")
        };
        for p in [pattern.to_string(), anywhere] {
            if let Ok(glob) = globset::Glob::new(&p) {
                builder.add(glob);
            }
        }
    }
    let Ok(set) = builder.build() else { return };
    // `--directory` reports a wholly ignored directory once instead of listing every file in a
    // `node_modules` or `target`; a pattern aimed at a file inside one is found by walking it.
    let Ok(listing) = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args([
            "ls-files",
            "-z",
            "--others",
            "--ignored",
            "--exclude-standard",
            "--directory",
        ])
        .output()
    else {
        return;
    };
    if !listing.status.success() {
        return;
    }
    let mut budget = COPY_BUDGET_BYTES;
    let mut visits = WALK_VISIT_LIMIT;
    let mut chosen: Vec<String> = Vec::new();
    for raw in String::from_utf8_lossy(&listing.stdout).split('\0') {
        let rel = raw.trim_end_matches('/');
        if rel.is_empty()
            || rel == ".git"
            || rel.starts_with(".git/")
            || rel == ".forge"
            || rel.starts_with(".forge/")
        {
            continue;
        }
        if set.is_match(rel) {
            chosen.push(rel.to_string());
        } else if raw.ends_with('/') {
            collect_matches(repo_root, rel, &set, &mut visits, &mut chosen);
        }
    }
    for rel in chosen {
        if control.is_some_and(SetupControl::is_cancelled) {
            break;
        }
        let src = repo_root.join(&rel);
        let dest = worktree.join(&rel);
        let existed = dest.symlink_metadata().is_ok();
        let mut size_visits = WALK_VISIT_LIMIT;
        let result = match tree_size(&src, &mut size_visits) {
            Some(size) if size > budget => Err("larger than the copy size limit".to_string()),
            Some(size) => {
                budget -= size;
                copy_path(&src, &dest, worktree, control)
            }
            None => Err("too many files to copy".to_string()),
        };
        match result {
            Ok(()) => out.copied.push(rel),
            Err(e) => {
                if !existed {
                    let _ = std::fs::remove_dir_all(&dest).or_else(|_| std::fs::remove_file(&dest));
                }
                out.skipped.push(format!("{rel} ({e})"));
            }
        }
    }
}

/// Files under the ignored directory `rel` that match `set`.
fn collect_matches(
    repo_root: &Path,
    rel: &str,
    set: &globset::GlobSet,
    visits: &mut usize,
    chosen: &mut Vec<String>,
) {
    let Ok(entries) = std::fs::read_dir(repo_root.join(rel)) else {
        return;
    };
    for entry in entries.flatten() {
        if *visits == 0 {
            return;
        }
        *visits -= 1;
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        let child = format!("{rel}/{name}");
        if kind.is_symlink() || (kind.is_dir() && is_nested_checkout(&entry.path())) {
            continue;
        }
        if set.is_match(&child) {
            chosen.push(child);
        } else if kind.is_dir() {
            collect_matches(repo_root, &child, set, visits, chosen);
        }
    }
}

/// Another checkout (a sibling worktree, a vendored repo) is not part of this one's ignored files.
fn is_nested_checkout(dir: &Path) -> bool {
    dir.join(".git").exists()
}

/// Bytes a copy of `src` would write; `None` when it holds more entries than a copy walks.
fn tree_size(src: &Path, visits: &mut usize) -> Option<u64> {
    let meta = std::fs::symlink_metadata(src).ok()?;
    if meta.file_type().is_symlink() {
        return Some(0);
    }
    if !meta.is_dir() {
        return Some(meta.len());
    }
    let mut total = 0u64;
    for entry in std::fs::read_dir(src).ok()?.flatten() {
        *visits = visits.checked_sub(1)?;
        total = total.saturating_add(tree_size(&entry.path(), visits)?);
    }
    Some(total)
}

/// Refuses a destination that is, or sits below, a symlink inside the worktree: a branch can track
/// a link that points anywhere, and the copy would write the secret through it.
fn dest_is_safe(worktree: &Path, dest: &Path) -> Result<(), String> {
    let rel = dest.strip_prefix(worktree).map_err(|e| e.to_string())?;
    let mut cur = worktree.to_path_buf();
    for part in rel.components() {
        cur.push(part);
        match std::fs::symlink_metadata(&cur) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err("destination is a symlink".to_string())
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    Ok(())
}

fn copy_path(
    src: &Path,
    dest: &Path,
    worktree: &Path,
    control: Option<&SetupControl>,
) -> Result<(), String> {
    if control.is_some_and(SetupControl::is_cancelled) {
        return Err("cancelled".to_string());
    }
    let meta = std::fs::symlink_metadata(src).map_err(|e| e.to_string())?;
    if meta.file_type().is_symlink() {
        return Err("symlink".to_string());
    }
    dest_is_safe(worktree, dest)?;
    if meta.is_dir() {
        std::fs::create_dir_all(dest).map_err(|e| e.to_string())?;
        for entry in std::fs::read_dir(src).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_symlink() || (kind.is_dir() && is_nested_checkout(&entry.path())) {
                continue;
            }
            copy_path(
                &entry.path(),
                &dest.join(entry.file_name()),
                worktree,
                control,
            )?;
        }
        return Ok(());
    }
    if dest.symlink_metadata().is_ok() {
        return Err("already exists in the worktree".to_string());
    }
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::copy(src, dest)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

const ADJECTIVES: [&str; 24] = [
    "amber", "brisk", "calm", "clever", "crisp", "dapper", "eager", "gentle", "golden", "hardy",
    "jolly", "keen", "lucid", "mellow", "nimble", "plucky", "quiet", "rapid", "sunny", "steady",
    "swift", "tidy", "vivid", "witty",
];
const NOUNS: [&str; 24] = [
    "otter", "falcon", "maple", "harbor", "comet", "willow", "badger", "ember", "heron", "lantern",
    "meadow", "orchid", "pebble", "quartz", "raven", "sparrow", "thistle", "violet", "walnut",
    "yarrow", "beacon", "cedar", "delta", "fjord",
];

/// A readable branch/worktree name such as `calm-otter-3f9a`. Callers still have to check that
/// the branch is free: the space is large, not unique.
pub fn friendly_name() -> String {
    let raw = hex::decode(forge_types::new_id().replace('-', "")).unwrap_or_default();
    let byte = |i: usize| raw.get(i).copied().unwrap_or(0) as usize;
    format!(
        "{}-{}-{}",
        ADJECTIVES[byte(0) % ADJECTIVES.len()],
        NOUNS[byte(1) % NOUNS.len()],
        hex::encode(raw.get(2..4).unwrap_or(&[0, 0]))
    )
}

#[cfg(test)]
#[path = "tests/worktree_setup.rs"]
mod tests;
