//! Presentation metadata the fleet list carries per session: which provider drives it, what it
//! descends from, and how much its working tree has changed.
//!
//! The git figures come from a cache that refreshes in the background. `GET /api/sessions` is
//! polled constantly and a `git diff` in a large checkout can take a while, so a stale or missing
//! entry answers immediately (with the last known figures, or none) and the next poll sees the
//! fresh ones.

use std::collections::HashMap;
use std::process::Command;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use forge_store::Store;

use super::SessionRow;

const REFRESH_AFTER: Duration = Duration::from_secs(10);
const FORGET_AFTER: Duration = Duration::from_secs(600);
const MAX_ENTRIES: usize = 256;

/// Additive keys on a `/api/sessions` row. An older client never reads them.
#[derive(Debug, Default, Clone, PartialEq, serde::Serialize)]
pub(super) struct SessionMetaFields {
    /// The model's namespace (`claude-cli`, `inferhub`, ...), so a client can pick a provider mark
    /// without parsing model ids. `None` when the model id has no namespace.
    pub(super) provider: Option<String>,
    /// The session this one was forked from, or was spawned by.
    pub(super) parent_id: Option<String>,
    /// Lines added / removed in the working tree relative to HEAD. `None` until measured.
    pub(super) diff_added: Option<u32>,
    pub(super) diff_removed: Option<u32>,
    /// The checked-out branch, `None` outside a git checkout or on a detached HEAD.
    pub(super) branch: Option<String>,
    /// Pinned to the top of every list.
    pub(super) starred: bool,
    /// Unix seconds the session was marked done, `None` while it is not.
    pub(super) done_at: Option<i64>,
    /// `fix` / `merge` while PR Watch is armed on this session.
    pub(super) pr_watch: Option<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GitFigures {
    added: u32,
    removed: u32,
    branch: Option<String>,
}

struct CacheEntry {
    measured_at: Option<Instant>,
    figures: Option<GitFigures>,
    refreshing: bool,
}

static CACHE: LazyLock<Mutex<HashMap<String, CacheEntry>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub(super) fn provider_of(model: &str) -> Option<String> {
    let (namespace, _) = model.split_once("::")?;
    let namespace = namespace.trim();
    (!namespace.is_empty()).then(|| namespace.to_ascii_lowercase())
}

/// Sums `git diff --numstat` output. Binary files report `-` and count as nothing.
fn parse_numstat(raw: &str) -> (u32, u32) {
    raw.lines().fold((0, 0), |(added, removed), line| {
        let mut parts = line.split('\t');
        let a = parts.next().and_then(|v| v.parse::<u32>().ok());
        let r = parts.next().and_then(|v| v.parse::<u32>().ok());
        (added + a.unwrap_or(0), removed + r.unwrap_or(0))
    })
}

fn git_output(cwd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

fn measure(cwd: &str) -> Option<GitFigures> {
    git_output(cwd, &["rev-parse", "--is-inside-work-tree"])?;
    let numstat = git_output(cwd, &["diff", "--numstat", "HEAD"])
        .or_else(|| git_output(cwd, &["diff", "--numstat"]))
        .unwrap_or_default();
    let (added, removed) = parse_numstat(&numstat);
    let branch = git_output(cwd, &["rev-parse", "--abbrev-ref", "HEAD"])
        .map(|b| b.trim().to_string())
        .filter(|b| !b.is_empty() && b != "HEAD");
    Some(GitFigures {
        added,
        removed,
        branch,
    })
}

fn figures_for(cwd: &str) -> Option<GitFigures> {
    let mut cache = CACHE.lock().ok()?;
    if cache.len() > MAX_ENTRIES {
        cache.retain(|_, entry| {
            entry
                .measured_at
                .is_none_or(|at| at.elapsed() < FORGET_AFTER)
        });
    }
    let entry = cache.entry(cwd.to_string()).or_insert(CacheEntry {
        measured_at: None,
        figures: None,
        refreshing: false,
    });
    let known = entry.figures.clone();
    if entry.refreshing
        || entry
            .measured_at
            .is_some_and(|at| at.elapsed() < REFRESH_AFTER)
    {
        return known;
    }
    let Ok(runtime) = tokio::runtime::Handle::try_current() else {
        return known;
    };
    entry.refreshing = true;
    let key = cwd.to_string();
    runtime.spawn_blocking(move || {
        let figures = measure(&key);
        if let Ok(mut cache) = CACHE.lock() {
            cache.insert(
                key,
                CacheEntry {
                    measured_at: Some(Instant::now()),
                    figures,
                    refreshing: false,
                },
            );
        }
    });
    known
}

/// Fills `meta` on every row. One store query covers all the parents.
pub(super) async fn annotate(store: &std::sync::Arc<Store>, rows: &mut [SessionRow]) {
    let ids: Vec<String> = rows.iter().map(|row| row.id.clone()).collect();
    let lookup = store.clone();
    let (parents, lifecycle) = tokio::task::spawn_blocking(move || {
        (
            lookup.session_parents(&ids).unwrap_or_default(),
            lookup.session_lifecycle(&ids).unwrap_or_default(),
        )
    })
    .await
    .unwrap_or_default();
    for row in rows {
        let figures = figures_for(&row.cwd);
        row.meta = SessionMetaFields {
            provider: provider_of(&row.model),
            parent_id: parents.get(&row.id).cloned(),
            diff_added: figures.as_ref().map(|f| f.added),
            diff_removed: figures.as_ref().map(|f| f.removed),
            branch: figures.and_then(|f| f.branch),
            starred: lifecycle.get(&row.id).is_some_and(|l| l.0),
            done_at: lifecycle.get(&row.id).and_then(|l| l.1),
            pr_watch: crate::serve_pr_watch::mode_of(&row.id),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(dir: &std::path::Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    #[test]
    fn provider_is_the_model_namespace() {
        assert_eq!(
            provider_of("claude-cli::sonnet").as_deref(),
            Some("claude-cli")
        );
        assert_eq!(
            provider_of("InferHub::cb/deepseek-v4.1-flash").as_deref(),
            Some("inferhub")
        );
        assert_eq!(provider_of("mock"), None);
        assert_eq!(provider_of("::oops"), None);
    }

    #[test]
    fn numstat_sums_text_files_and_skips_binary_ones() {
        assert_eq!(
            parse_numstat("3\t1\ta.rs\n-\t-\timg.png\n10\t0\tb.rs\n"),
            (13, 1)
        );
        assert_eq!(parse_numstat(""), (0, 0));
    }

    #[test]
    fn measures_a_dirty_checkout_and_ignores_a_plain_directory() {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-q", "-b", "main"]);
        std::fs::write(dir.path().join("a.txt"), "one\ntwo\n").unwrap();
        git(dir.path(), &["add", "a.txt"]);
        git(dir.path(), &["commit", "-q", "-m", "init"]);
        std::fs::write(dir.path().join("a.txt"), "one\nthree\nfour\n").unwrap();
        let figures = measure(dir.path().to_str().unwrap()).unwrap();
        assert_eq!((figures.added, figures.removed), (2, 1));
        assert_eq!(figures.branch.as_deref(), Some("main"));

        let plain = tempfile::tempdir().unwrap();
        assert_eq!(measure(plain.path().to_str().unwrap()), None);
    }

    #[tokio::test]
    async fn a_first_request_answers_empty_and_a_later_one_sees_the_figures() {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-q", "-b", "main"]);
        std::fs::write(dir.path().join("a.txt"), "x\n").unwrap();
        git(dir.path(), &["add", "a.txt"]);
        git(dir.path(), &["commit", "-q", "-m", "init"]);
        std::fs::write(dir.path().join("a.txt"), "x\ny\n").unwrap();
        let cwd = dir.path().to_str().unwrap();
        assert_eq!(figures_for(cwd), None);
        let mut seen = None;
        for _ in 0..100 {
            tokio::time::sleep(Duration::from_millis(50)).await;
            seen = figures_for(cwd);
            if seen.is_some() {
                break;
            }
        }
        let seen = seen.expect("background refresh finished");
        assert_eq!((seen.added, seen.removed), (1, 0));
    }
}
