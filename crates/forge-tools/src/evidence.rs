//! Evidence ledger: what a session captured as proof of its work.
//!
//! Screenshots from `browser`/`device` and the output of passing verification commands land in
//! `<workspace>/.forge/evidence/<session>/` with monotonic names, and every capture appends one
//! line to `manifest.jsonl` there (timestamp, kind, file, caption, page URL without its query
//! string, sha256 of the file). `open_pr` renders the manifest into the PR body, so the proof a
//! reviewer sees is what was recorded, never what the model says it did.
//!
//! The session id reaches tools as a task-local scoped by the direct path's dispatcher; a CLI
//! bridge's `forge mcp-serve` child has no such scope and reads the parent session id from the
//! env the parent sets on it ([`SESSION_ENV`]).

use std::future::Future;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

mod pr_block;
pub use pr_block::PrBlockOptions;

tokio::task_local! {
    static SESSION_SCOPE: (String, PathBuf);
}

/// The parent session id a CLI bridge exports to its `forge mcp-serve` child. Mirrors
/// `forge_core::snapshot::ENV_SESSION` (forge-tools cannot depend on forge-core).
pub const SESSION_ENV: &str = "FORGE_CHECKPOINT_SESSION";

pub const MANIFEST: &str = "manifest.jsonl";

/// Lines of command output kept as test proof.
pub const TEST_TAIL_LINES: usize = 40;

/// Run `fut` with `session` as the session evidence is recorded against, rooted at `workspace`
/// (the root `open_pr` later reads the ledger from).
pub fn scope_session<F: Future>(
    session: String,
    workspace: PathBuf,
    fut: F,
) -> impl Future<Output = F::Output> {
    SESSION_SCOPE.scope((session, workspace), fut)
}

pub fn current_session() -> Option<String> {
    SESSION_SCOPE
        .try_with(|(session, _)| session.clone())
        .ok()
        .or_else(|| std::env::var(SESSION_ENV).ok())
        .filter(|id| valid_session_id(id))
}

/// A session id becomes a directory name, so anything that could escape the evidence root is
/// refused rather than sanitized into a different, colliding name.
fn valid_session_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    Screenshot,
    TestOutput,
    /// A public preview URL a `shell_job` expose opened for a dev server.
    Preview,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceItem {
    pub ts: String,
    pub kind: EvidenceKind,
    /// File name inside the session's evidence directory.
    pub file: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caption: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub sha256: String,
    /// `browser`, `device` or `shell`.
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
}

/// What a capture records beyond its bytes.
#[derive(Debug, Default, Clone)]
pub struct Capture<'a> {
    pub caption: Option<&'a str>,
    pub url: Option<&'a str>,
    pub command: Option<&'a str>,
    pub exit_code: Option<i32>,
}

/// One session's evidence directory.
#[derive(Debug, Clone)]
pub struct Ledger {
    root: PathBuf,
    dir: PathBuf,
}

impl Ledger {
    pub fn new(workspace: &Path, session: &str) -> Option<Self> {
        if !valid_session_id(session) {
            return None;
        }
        let root = workspace.join(".forge").join("evidence");
        Some(Self {
            dir: root.join(session),
            root,
        })
    }

    /// The ledger of the session this tool call runs for, rooted at its workspace.
    pub fn current() -> Option<Self> {
        if let Ok((session, workspace)) = SESSION_SCOPE.try_with(Clone::clone) {
            return Self::new(&workspace, &session);
        }
        let session = current_session()?;
        let workspace = crate::SESSION_WORKSPACE
            .try_with(Clone::clone)
            .ok()
            .or_else(|| std::env::current_dir().ok())?;
        Self::new(&workspace, &session)
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Write `bytes` under the next monotonic name and append its manifest line.
    pub fn record(
        &self,
        kind: EvidenceKind,
        source: &str,
        extension: &str,
        bytes: &[u8],
        capture: Capture<'_>,
    ) -> std::io::Result<(PathBuf, EvidenceItem)> {
        std::fs::create_dir_all(&self.dir)?;
        ensure_git_ignored(&self.root);
        let (path, mut file) = self.create_next(source, extension)?;
        file.write_all(bytes)?;
        let item = EvidenceItem {
            ts: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            kind,
            file: path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
            caption: capture
                .caption
                .map(str::trim)
                .filter(|c| !c.is_empty())
                .map(str::to_string),
            url: capture.url.map(url_sans_query),
            sha256: sha256_hex(bytes),
            source: source.to_string(),
            command: capture.command.map(str::to_string),
            exit_code: capture.exit_code,
        };
        let mut line = serde_json::to_string(&item).map_err(std::io::Error::other)?;
        line.push('\n');
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.dir.join(MANIFEST))?
            .write_all(line.as_bytes())?;
        Ok((path, item))
    }

    /// Record a passing verification command: the command, its exit status and the last
    /// [`TEST_TAIL_LINES`] lines of what it printed.
    pub fn record_test_output(
        &self,
        command: &str,
        exit_code: i32,
        output: &str,
    ) -> std::io::Result<(PathBuf, EvidenceItem)> {
        let body = format!(
            "$ {command}\nexit {exit_code}\n\n{}\n",
            tail_lines(output, TEST_TAIL_LINES)
        );
        self.record(
            EvidenceKind::TestOutput,
            "shell",
            "txt",
            body.as_bytes(),
            Capture {
                command: Some(command),
                exit_code: Some(exit_code),
                ..Capture::default()
            },
        )
    }

    /// Record a preview URL `shell_job` expose opened for background job `job` on `port`. The URL
    /// is kept whole: a tunnel URL has no query string, and its host is the whole point.
    pub fn record_preview(
        &self,
        url: &str,
        job: u32,
        port: u16,
    ) -> std::io::Result<(PathBuf, EvidenceItem)> {
        let body = format!("{url}\njob {job} port {port}\n");
        let caption = format!("job {job} port {port}");
        self.record(
            EvidenceKind::Preview,
            "preview",
            "txt",
            body.as_bytes(),
            Capture {
                caption: Some(&caption),
                url: Some(url),
                ..Capture::default()
            },
        )
    }

    /// Every manifest entry, in capture order. Unreadable lines are skipped: a torn final line
    /// from a crash must not hide everything recorded before it.
    pub fn items(&self) -> Vec<EvidenceItem> {
        std::fs::read_to_string(self.dir.join(MANIFEST))
            .map(|text| {
                text.lines()
                    .filter_map(|line| serde_json::from_str(line).ok())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// `create_new` makes the name claim atomic, so a bridge child and the parent process writing
    /// into one session never overwrite each other's file.
    fn create_next(
        &self,
        source: &str,
        extension: &str,
    ) -> std::io::Result<(PathBuf, std::fs::File)> {
        let mut index = self.highest_index() + 1;
        loop {
            let path = self.dir.join(format!("{index:04}-{source}.{extension}"));
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(file) => return Ok((path, file)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => index += 1,
                Err(error) => return Err(error),
            }
        }
    }

    fn highest_index(&self) -> u32 {
        std::fs::read_dir(&self.dir)
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .filter_map(|entry| {
                        let name = entry.file_name();
                        let name = name.to_str()?;
                        name.split_once('-')?.0.parse::<u32>().ok()
                    })
                    .max()
                    .unwrap_or(0)
            })
            .unwrap_or(0)
    }
}

/// Keep evidence out of `git status`, the way checkpoints are: a `*` ignore file inside the
/// directory ignores everything under it without touching the user's own `.gitignore`.
fn ensure_git_ignored(root: &Path) {
    let ignore = root.join(".gitignore");
    if !ignore.exists() {
        let _ = std::fs::write(ignore, "*\n");
    }
}

/// Query strings and fragments carry tokens and session ids; credentials in the authority do too.
pub fn url_sans_query(url: &str) -> String {
    let base = url.split(['?', '#']).next().unwrap_or(url);
    match base.split_once("://") {
        Some((scheme, rest)) => {
            let (authority, path) = match rest.find('/') {
                Some(slash) => rest.split_at(slash),
                None => (rest, ""),
            };
            let host = authority
                .rsplit_once('@')
                .map_or(authority, |(_, host)| host);
            format!("{scheme}://{host}{path}")
        }
        None => base.to_string(),
    }
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn tail_lines(text: &str, lines: usize) -> String {
    let all: Vec<&str> = text.trim_end().lines().collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}

/// Where a screenshot goes when no session is known (a bare tool call outside any session): a
/// per-process temp name that still never reuses a path within the process.
pub(crate) fn fallback_capture_path(prefix: &str) -> PathBuf {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    std::env::temp_dir().join(format!("{prefix}-{}-{n:04}.png", std::process::id()))
}

/// Save a screenshot into the current session's ledger (or a temp file outside any session) and
/// describe where it went.
pub(crate) fn save_screenshot(
    source: &str,
    fallback_prefix: &str,
    bytes: &[u8],
    caption: Option<&str>,
    url: Option<&str>,
) -> Result<String, crate::ToolError> {
    let write_error =
        |e: std::io::Error| crate::ToolError::Failed(format!("write screenshot: {e}"));
    if let Some(ledger) = Ledger::current() {
        let capture = Capture {
            caption,
            url,
            ..Capture::default()
        };
        let (path, _) = ledger
            .record(EvidenceKind::Screenshot, source, "png", bytes, capture)
            .map_err(write_error)?;
        return Ok(format!(
            "screenshot written to {} ({} bytes; recorded as session evidence)",
            path.display(),
            bytes.len()
        ));
    }
    let path = fallback_capture_path(fallback_prefix);
    std::fs::write(&path, bytes).map_err(write_error)?;
    Ok(format!(
        "screenshot written to {} ({} bytes)",
        path.display(),
        bytes.len()
    ))
}

/// The PR-body section listing what this session captured, or `None` when it captured nothing.
/// Deterministic: built from the manifest alone, in capture order. Labelled "captured" because
/// nobody — the model included — has looked at the images.
pub fn pr_evidence_block(workspace: &Path, session: &str) -> Option<String> {
    pr_block::render(workspace, session, &PrBlockOptions::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ledger() -> (tempfile::TempDir, Ledger) {
        let dir = tempfile::tempdir().unwrap();
        let ledger = Ledger::new(dir.path(), "sess-1").unwrap();
        (dir, ledger)
    }

    #[test]
    fn record_appends_a_manifest_line_with_the_file_hash() {
        let (_dir, ledger) = ledger();
        let (path, item) = ledger
            .record(
                EvidenceKind::Screenshot,
                "browser",
                "png",
                b"abc",
                Capture {
                    caption: Some("login page"),
                    url: Some("https://u:p@example.com/login?token=secret#frag"),
                    ..Capture::default()
                },
            )
            .unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"abc");
        assert_eq!(
            item.sha256,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(item.url.as_deref(), Some("https://example.com/login"));
        assert_eq!(item.caption.as_deref(), Some("login page"));
        assert_eq!(ledger.items(), vec![item]);
        let manifest = std::fs::read_to_string(ledger.dir().join(MANIFEST)).unwrap();
        assert_eq!(manifest.lines().count(), 1);
        assert!(!manifest.contains("secret"));
    }

    #[test]
    fn names_are_monotonic_and_never_reused() {
        let (_dir, ledger) = ledger();
        let names: Vec<String> = (0..3)
            .map(|_| {
                ledger
                    .record(
                        EvidenceKind::Screenshot,
                        "browser",
                        "png",
                        b"x",
                        Capture::default(),
                    )
                    .unwrap()
                    .1
                    .file
            })
            .collect();
        assert_eq!(
            names,
            ["0001-browser.png", "0002-browser.png", "0003-browser.png"]
        );
        std::fs::remove_file(ledger.dir().join("0002-browser.png")).unwrap();
        let (_, next) = ledger
            .record(
                EvidenceKind::Screenshot,
                "device",
                "png",
                b"y",
                Capture::default(),
            )
            .unwrap();
        assert_eq!(next.file, "0004-device.png");
        assert_eq!(ledger.items().len(), 4);
    }

    #[test]
    fn evidence_root_is_git_ignored() {
        let (dir, ledger) = ledger();
        ledger
            .record(
                EvidenceKind::Screenshot,
                "browser",
                "png",
                b"x",
                Capture::default(),
            )
            .unwrap();
        let ignore = dir.path().join(".forge/evidence/.gitignore");
        assert_eq!(std::fs::read_to_string(ignore).unwrap(), "*\n");
    }

    #[test]
    fn test_output_keeps_the_command_exit_and_last_lines() {
        let (_dir, ledger) = ledger();
        let output: String = (1..=100).map(|n| format!("line {n}\n")).collect();
        let (path, item) = ledger.record_test_output("cargo test", 0, &output).unwrap();
        let body = std::fs::read_to_string(path).unwrap();
        assert!(body.starts_with("$ cargo test\nexit 0\n\nline 61\n"));
        assert!(body.ends_with("line 100\n"));
        assert!(!body.contains("line 60\n"));
        assert_eq!(item.kind, EvidenceKind::TestOutput);
        assert_eq!(item.command.as_deref(), Some("cargo test"));
        assert_eq!(item.exit_code, Some(0));
        assert_eq!(item.file, "0001-shell.txt");
    }

    #[test]
    fn unsafe_session_ids_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        for id in ["", "../x", "a/b", "a b", "."] {
            assert!(Ledger::new(dir.path(), id).is_none(), "{id:?}");
        }
    }

    #[test]
    fn a_torn_manifest_line_hides_nothing_before_it() {
        let (_dir, ledger) = ledger();
        ledger
            .record(
                EvidenceKind::Screenshot,
                "browser",
                "png",
                b"x",
                Capture::default(),
            )
            .unwrap();
        std::fs::OpenOptions::new()
            .append(true)
            .open(ledger.dir().join(MANIFEST))
            .unwrap()
            .write_all(b"{\"ts\":\"20")
            .unwrap();
        assert_eq!(ledger.items().len(), 1);
    }

    #[test]
    fn pr_block_over_a_fake_manifest_is_golden() {
        let dir = tempfile::tempdir().unwrap();
        let session_dir = dir.path().join(".forge/evidence/abc123");
        std::fs::create_dir_all(&session_dir).unwrap();
        let manifest = [
            r#"{"ts":"2026-10-14T10:00:00Z","kind":"screenshot","file":"0001-browser.png","caption":"empty cart","url":"http://localhost:3000/cart","sha256":"0123456789abcdef0123","source":"browser"}"#,
            r#"{"ts":"2026-10-14T10:01:00Z","kind":"test_output","file":"0002-shell.txt","sha256":"fedcba9876543210fedc","source":"shell","command":"cargo test -p shop","exit_code":0}"#,
            r#"{"ts":"2026-10-14T10:02:00Z","kind":"screenshot","file":"0003-device.png","sha256":"aaaaaaaaaaaaaaaaaaaa","source":"device"}"#,
        ]
        .join("\n");
        std::fs::write(session_dir.join(MANIFEST), manifest + "\n").unwrap();
        let block = pr_evidence_block(dir.path(), "abc123").unwrap();
        assert_eq!(
            block,
            "### Evidence (captured, not reviewed)\n\
             Checks that passed during the session:\n\
             - `cargo test -p shop` — exit 0 at 2026-10-14T10:01:00Z (`.forge/evidence/abc123/0002-shell.txt`, sha256 `fedcba987654`)\n\
             Screenshots:\n\
             - `.forge/evidence/abc123/0001-browser.png` (browser, http://localhost:3000/cart, sha256 `0123456789ab`) — empty cart\n\
             - `.forge/evidence/abc123/0003-device.png` (device, sha256 `aaaaaaaaaaaa`)\n"
        );
        assert!(!block.contains("verified"));
    }

    #[test]
    fn pr_block_is_absent_without_evidence() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(pr_evidence_block(dir.path(), "none"), None);
    }

    #[tokio::test]
    async fn screenshots_land_in_the_scoped_session_ledger() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().to_path_buf();
        let message = crate::SESSION_WORKSPACE
            .scope(
                std::env::temp_dir(),
                scope_session("scoped-1".to_string(), workspace.clone(), async {
                    save_screenshot(
                        "browser",
                        "forge-browser",
                        b"png",
                        Some("home"),
                        Some("http://x/?a=1"),
                    )
                }),
            )
            .await
            .unwrap();
        assert!(
            message.contains("recorded as session evidence"),
            "{message}"
        );
        let items = Ledger::new(&workspace, "scoped-1").unwrap().items();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].url.as_deref(), Some("http://x/"));
        assert_eq!(items[0].caption.as_deref(), Some("home"));
    }

    #[test]
    fn pr_block_masks_env_assignment_values() {
        let (dir, ledger) = ledger();
        ledger
            .record_test_output("API_KEY=hunter2 RUST_LOG=debug cargo test a=b", 0, "ok")
            .unwrap();
        let block = pr_evidence_block(dir.path(), "sess-1").unwrap();
        assert!(!block.contains("hunter2"), "{block}");
        assert!(
            block.contains("`API_KEY=*** RUST_LOG=*** cargo test a=b`"),
            "{block}"
        );
    }

    #[test]
    fn fallback_paths_never_repeat_within_a_process() {
        assert_ne!(fallback_capture_path("p"), fallback_capture_path("p"));
    }
}
