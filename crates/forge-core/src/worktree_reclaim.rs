//! Reclaiming git worktrees Forge can no longer justify keeping, and reporting what they cost.
//!
//! [`WorktreeGuard`](crate::worktree::WorktreeGuard) removes a subagent worktree in `Drop`, which
//! covers the ordinary exit. It does NOT cover the process being killed, and it never had anything
//! to say about the worktrees an operator created by hand. Left alone, a repository accumulates
//! registrations whose directory is gone, worktrees whose branch merged months ago, and a `target/`
//! dir per worktree — the case this module was written for filled a 1.8 TB disk to zero bytes free
//! and killed a linker with a bus error, on a machine whose CI runners share that disk.
//!
//! The module is deliberately conservative. [`survey`] only reads; [`classify`] declares a worktree
//! reclaimable ONLY when its registration is already prunable, or when its branch is fully merged
//! into the default branch AND the working tree is clean AND it has no unpushed commits AND no live
//! session references it. Everything else comes back as [`Verdict::Skip`] carrying the reason, and
//! `remove` refuses anything `classify` did not clear — regardless of who created the worktree.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::worktree::WorktreeError;

/// Where a Forge-created worktree's ownership record lives, relative to the repo root. Kept
/// OUTSIDE the worktree itself so it survives the worktree directory being deleted by hand, and so
/// `commit_worktree`'s `git add -A` can never sweep it into a child's snapshot.
const OWNERS_DIR: &[&str] = &[".forge", "worktree-owners"];

/// Everything needed to judge one worktree, as observed on disk and in git.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeFacts {
    pub path: PathBuf,
    /// `None` for a detached-HEAD worktree.
    pub branch: Option<String>,
    /// git considers the registration stale — the directory is gone.
    pub prunable: bool,
    /// The branch is an ancestor of the default branch (nothing would be lost).
    pub merged: bool,
    /// `git status --porcelain` reported something (including untracked files).
    pub dirty: bool,
    /// The branch tracks an upstream and is ahead of it.
    pub unpushed: bool,
    /// git has the worktree locked (`git worktree lock`).
    pub locked: bool,
    /// A live session's cwd or worktree path is inside this worktree.
    pub live: bool,
    /// Forge created it: it lives under `.forge/worktrees` or `.claude/worktrees`, or an ownership
    /// record names it.
    pub forge_owned: bool,
    /// Forge created it and the process that owned it is gone — `Drop` never ran.
    pub orphaned: bool,
    /// Size on disk of the whole worktree, `target/` included.
    pub size_bytes: u64,
    /// Size of the regenerable Cargo build artifacts under `target/`.
    pub artifact_bytes: u64,
    /// A process currently has its cwd or an open file below `target/`.
    pub artifact_in_use: bool,
    /// Seconds since anything under `target/` was modified.
    pub artifact_age_secs: Option<u64>,
    /// Seconds since the worktree directory was last modified; `None` when it is gone.
    pub age_secs: Option<u64>,
}

/// What [`classify`] decided about one worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Provably safe to reclaim, with the reason it qualified.
    Reclaim(&'static str),
    /// Kept, with the reason — always shown to the user rather than silently dropped.
    Skip(String),
}

impl Verdict {
    pub fn is_reclaim(&self) -> bool {
        matches!(self, Verdict::Reclaim(_))
    }

    pub fn reason(&self) -> &str {
        match self {
            Verdict::Reclaim(r) => r,
            Verdict::Skip(r) => r,
        }
    }
}

/// A worktree plus the decision made about it.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub facts: WorktreeFacts,
    pub verdict: Verdict,
    pub artifact_verdict: Verdict,
}

/// The whole picture: every worktree, its verdict, and the totals the user needs to see BEFORE the
/// disk fills rather than after a build dies.
#[derive(Debug, Clone, Default)]
pub struct ReclaimReport {
    pub candidates: Vec<Candidate>,
}

impl ReclaimReport {
    pub fn total_bytes(&self) -> u64 {
        self.candidates.iter().map(|c| c.facts.size_bytes).sum()
    }

    pub fn reclaimable_bytes(&self) -> u64 {
        self.candidates
            .iter()
            .map(|c| {
                if c.verdict.is_reclaim() {
                    c.facts.size_bytes
                } else if c.artifact_verdict.is_reclaim() {
                    c.facts.artifact_bytes
                } else {
                    0
                }
            })
            .sum()
    }

    pub fn artifact_bytes(&self) -> u64 {
        self.candidates.iter().map(|c| c.facts.artifact_bytes).sum()
    }

    pub fn artifact_reclaimable_bytes(&self) -> u64 {
        self.candidates
            .iter()
            .filter(|c| c.artifact_verdict.is_reclaim())
            .map(|c| c.facts.artifact_bytes)
            .sum()
    }

    pub fn reclaimable(&self) -> impl Iterator<Item = &Candidate> {
        self.candidates.iter().filter(|c| c.verdict.is_reclaim())
    }

    pub fn skipped(&self) -> impl Iterator<Item = &Candidate> {
        self.candidates.iter().filter(|c| !c.verdict.is_reclaim())
    }
}

/// Survey every worktree registered in `repo_root`, classify each, and sort by reclaimable size so
/// the worst offender is first. `live_paths` are the cwd / worktree paths of sessions that are
/// currently alive; anything inside one of them is never reclaimed.
pub fn survey(repo_root: &Path, live_paths: &[PathBuf]) -> Result<ReclaimReport, WorktreeError> {
    survey_with_artifact_age(repo_root, live_paths, 7 * 24 * 60 * 60)
}

pub fn survey_with_artifact_age(
    repo_root: &Path,
    live_paths: &[PathBuf],
    artifact_min_age_secs: u64,
) -> Result<ReclaimReport, WorktreeError> {
    let main = canonical(repo_root);
    let default_branch = default_branch(repo_root);
    let owners = read_owners(repo_root);
    let entries = list_worktrees(repo_root)?;

    let mut candidates: Vec<Candidate> = entries
        .into_iter()
        // The main checkout is never a candidate. Comparing against `repo_root` alone is NOT
        // enough: run from a linked worktree, `git rev-parse --show-toplevel` names THAT worktree,
        // and the main checkout (branch `main`, merged, clean) would classify as reclaimable —
        // caught the first time this was pointed at a real repository. `git worktree list` always
        // emits the main worktree first, so that flag is the authority.
        .filter(|e| !e.bare && !e.is_main && canonical(&e.path) != main)
        .map(|entry| {
            let facts = facts_for(
                repo_root,
                &entry,
                default_branch.as_deref(),
                &owners,
                live_paths,
            );
            let verdict = classify(&facts);
            let artifact_verdict = classify_artifacts(&facts, artifact_min_age_secs);
            Candidate {
                facts,
                verdict,
                artifact_verdict,
            }
        })
        .collect();

    candidates.sort_by(|a, b| {
        let key = |c: &Candidate| (c.verdict.is_reclaim(), c.facts.size_bytes);
        key(b)
            .cmp(&key(a))
            .then_with(|| a.facts.path.cmp(&b.facts.path))
    });

    Ok(ReclaimReport { candidates })
}

/// Build output has a separate lifecycle from its worktree. It is safe to regenerate even when
/// source is dirty or unmerged, but only after an explicit quiet period and when no live session
/// points into the worktree.
pub fn classify_artifacts(f: &WorktreeFacts, min_age_secs: u64) -> Verdict {
    if f.artifact_bytes == 0 {
        return Verdict::Skip("no build artifacts".into());
    }
    if f.live {
        return Verdict::Skip("a live session is using this worktree".into());
    }
    if f.artifact_in_use {
        return Verdict::Skip("a build is using these artifacts".into());
    }
    let Some(age) = f.artifact_age_secs else {
        return Verdict::Skip("artifact modification time is unavailable".into());
    };
    if age < min_age_secs {
        return Verdict::Skip(format!(
            "build artifacts are not stale ({} old, minimum {})",
            human_age(Some(age)),
            human_age(Some(min_age_secs))
        ));
    }
    Verdict::Reclaim("stale build artifacts")
}

/// Delete only `target/`, after observing its size, latest modification, and live-session state
/// again. The second observation matters because a dry-run report can sit on screen while a build
/// starts; stale facts must never authorize a later deletion.
pub fn prune_artifacts(
    worktree: &Path,
    live_paths: &[PathBuf],
    min_age_secs: u64,
) -> Result<u64, WorktreeError> {
    let target = worktree.join("target");
    let (bytes, age) = artifact_facts(&target);
    let facts = WorktreeFacts {
        path: worktree.to_path_buf(),
        branch: None,
        prunable: false,
        merged: false,
        dirty: false,
        unpushed: false,
        locked: false,
        live: live_paths.iter().any(|p| is_within(p, worktree)),
        forge_owned: false,
        orphaned: false,
        size_bytes: bytes,
        artifact_bytes: bytes,
        artifact_in_use: path_in_use(&target),
        artifact_age_secs: age,
        age_secs: modified_age_secs(worktree),
    };
    if let Verdict::Skip(reason) = classify_artifacts(&facts, min_age_secs) {
        return Err(WorktreeError::NonZeroExit {
            cmd: "reclaim artifacts".into(),
            stderr: format!("refusing to prune {}: {reason}", target.display()),
        });
    }
    std::fs::remove_dir_all(&target).map_err(|e| WorktreeError::NonZeroExit {
        cmd: "reclaim artifacts".into(),
        stderr: format!("removing {}: {e}", target.display()),
    })?;
    Ok(bytes)
}

/// Decide whether one worktree is provably safe to reclaim. The refusals come first on purpose:
/// uncommitted or unpushed work outranks every reason to delete, whoever created the worktree.
pub fn classify(f: &WorktreeFacts) -> Verdict {
    if f.prunable {
        // The directory is already gone — only the registration is left, and pruning it cannot
        // lose work that no longer exists on disk.
        return Verdict::Reclaim("stale registration (directory is gone)");
    }
    if f.dirty {
        return Verdict::Skip("uncommitted changes".into());
    }
    if f.unpushed {
        return Verdict::Skip("unpushed commits".into());
    }
    if f.live {
        return Verdict::Skip("a live session is using it".into());
    }
    if f.locked {
        return Verdict::Skip("locked by git".into());
    }
    let Some(branch) = &f.branch else {
        return Verdict::Skip("detached HEAD — no branch to check for merge".into());
    };
    if !f.merged {
        return Verdict::Skip(format!(
            "branch {branch} is not merged into the default branch"
        ));
    }
    if f.orphaned {
        return Verdict::Reclaim("orphaned Forge worktree, merged and clean");
    }
    Verdict::Reclaim("branch merged, working tree clean")
}

/// Remove a worktree that [`classify`] cleared. Re-checks the verdict rather than trusting the
/// caller: nothing here deletes on a `Skip`. A prunable registration is pruned instead of removed
/// (its directory is already gone). The branch is deleted with `git branch -d`, which git itself
/// refuses for an unmerged branch — a second, independent guard on top of `classify`.
pub fn remove(repo_root: &Path, facts: &WorktreeFacts) -> Result<(), WorktreeError> {
    let verdict = classify(facts);
    if let Verdict::Skip(reason) = verdict {
        return Err(WorktreeError::NonZeroExit {
            cmd: "reclaim".into(),
            stderr: format!("refusing to remove {}: {reason}", facts.path.display()),
        });
    }

    if facts.prunable {
        git(repo_root, &["worktree", "prune"])?;
    } else {
        let path = facts.path.to_string_lossy().to_string();
        git(repo_root, &["worktree", "remove", &path])?;
    }
    if let Some(branch) = &facts.branch {
        // Best-effort: a shared branch someone else still wants, or one git considers unmerged
        // against its upstream, is left alone. The disk win is the worktree, not the ref.
        let _ = git(repo_root, &["branch", "-d", branch]);
    }
    clear_ownership_for_path(repo_root, &facts.path);
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Ownership records — how a killed session's worktree stays recognisable
// ---------------------------------------------------------------------------------------------

/// What Forge recorded about a worktree it created, so a later reclaim can recognise an orphan
/// whose owning process is gone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerRecord {
    pub path: PathBuf,
    pub branch: String,
    pub pid: u32,
    pub created_at: u64,
}

/// Record that this process owns `path`. Written on worktree creation and removed by the guard's
/// `Drop`; a record still present with a dead pid is exactly the signature of a session that was
/// killed before `Drop` could run. Best-effort — failing to write it must never fail the worktree.
pub fn record_ownership(repo_root: &Path, id: &str, path: &Path, branch: &str) {
    let dir = owners_dir(repo_root);
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let created_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default();
    let body = format!(
        "path\t{}\nbranch\t{branch}\npid\t{}\ncreated_at\t{created_at}\n",
        path.display(),
        std::process::id(),
    );
    let _ = std::fs::write(dir.join(format!("{}.record", sanitize(id))), body);
}

/// Drop the ownership record for `id` (clean shutdown path).
pub fn clear_ownership(repo_root: &Path, id: &str) {
    let _ = std::fs::remove_file(owners_dir(repo_root).join(format!("{}.record", sanitize(id))));
}

/// Every ownership record currently on disk, keyed by canonical worktree path.
pub fn read_owners(repo_root: &Path) -> BTreeMap<PathBuf, OwnerRecord> {
    let mut out = BTreeMap::new();
    let Ok(entries) = std::fs::read_dir(owners_dir(repo_root)) else {
        return out;
    };
    for entry in entries.flatten() {
        let Ok(body) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        if let Some(record) = parse_owner(&body) {
            out.insert(canonical(&record.path), record);
        }
    }
    out
}

fn parse_owner(body: &str) -> Option<OwnerRecord> {
    let mut fields: BTreeMap<&str, &str> = BTreeMap::new();
    for line in body.lines() {
        if let Some((k, v)) = line.split_once('\t') {
            fields.insert(k, v);
        }
    }
    Some(OwnerRecord {
        path: PathBuf::from(fields.get("path")?),
        branch: fields
            .get("branch")
            .copied()
            .unwrap_or_default()
            .to_string(),
        pid: fields.get("pid")?.parse().ok()?,
        created_at: fields
            .get("created_at")
            .and_then(|v| v.parse().ok())
            .unwrap_or_default(),
    })
}

fn clear_ownership_for_path(repo_root: &Path, path: &Path) {
    let target = canonical(path);
    let Ok(entries) = std::fs::read_dir(owners_dir(repo_root)) else {
        return;
    };
    for entry in entries.flatten() {
        let matches = std::fs::read_to_string(entry.path())
            .ok()
            .and_then(|b| parse_owner(&b))
            .is_some_and(|r| canonical(&r.path) == target);
        if matches {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

fn owners_dir(repo_root: &Path) -> PathBuf {
    OWNERS_DIR
        .iter()
        .fold(repo_root.to_path_buf(), |p, s| p.join(s))
}

fn sanitize(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Whether a process is still running. A record whose pid is gone means `Drop` never ran.
pub fn pid_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        // Signal 0 checks existence/permission without delivering. errno comes from std, not
        // libc's per-platform accessor (`__errno_location`/`__error`), which breaks macOS builds.
        let delivered = unsafe { libc::kill(pid as i32, 0) } == 0;
        delivered || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
    #[cfg(not(unix))]
    {
        // Without a cheap portable probe, assume alive: over-keeping is the safe direction.
        let _ = pid;
        true
    }
}

// ---------------------------------------------------------------------------------------------
// git observation
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Default, Clone)]
struct WorktreeEntry {
    path: PathBuf,
    branch: Option<String>,
    prunable: bool,
    locked: bool,
    bare: bool,
    /// The repository's main checkout — the first record git emits.
    is_main: bool,
}

/// Parse `git worktree list --porcelain`. Records are separated by blank lines; the first is the
/// main worktree.
fn list_worktrees(repo_root: &Path) -> Result<Vec<WorktreeEntry>, WorktreeError> {
    let out = git(repo_root, &["worktree", "list", "--porcelain"])?;
    Ok(parse_worktree_list(&String::from_utf8_lossy(&out)))
}

fn parse_worktree_list(text: &str) -> Vec<WorktreeEntry> {
    let mut out = Vec::new();
    let mut current: Option<WorktreeEntry> = None;
    for line in text.lines() {
        if line.trim().is_empty() {
            out.extend(current.take());
            continue;
        }
        let (key, value) = line.split_once(' ').unwrap_or((line, ""));
        match key {
            "worktree" => {
                out.extend(current.take());
                let first = out.is_empty();
                current = Some(WorktreeEntry {
                    path: PathBuf::from(value),
                    is_main: first,
                    ..Default::default()
                });
            }
            "branch" => {
                if let Some(entry) = current.as_mut() {
                    entry.branch = Some(value.trim_start_matches("refs/heads/").to_string());
                }
            }
            "prunable" => {
                if let Some(entry) = current.as_mut() {
                    entry.prunable = true;
                }
            }
            "locked" => {
                if let Some(entry) = current.as_mut() {
                    entry.locked = true;
                }
            }
            "bare" => {
                if let Some(entry) = current.as_mut() {
                    entry.bare = true;
                }
            }
            _ => {}
        }
    }
    out.extend(current);
    out
}

/// The branch a merge check should be made against: `origin/HEAD` when it resolves, else the first
/// of the usual suspects that exists. `None` when the repository has none of them, in which case
/// nothing is ever considered merged (and so nothing is reclaimed on that basis).
fn default_branch(repo_root: &Path) -> Option<String> {
    if let Ok(out) = git(
        repo_root,
        &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
    ) {
        let name = String::from_utf8_lossy(&out).trim().to_string();
        if !name.is_empty() {
            return Some(name);
        }
    }
    ["origin/main", "origin/master", "main", "master"]
        .into_iter()
        .find(|candidate| git(repo_root, &["rev-parse", "--verify", "--quiet", candidate]).is_ok())
        .map(str::to_string)
}

fn facts_for(
    repo_root: &Path,
    entry: &WorktreeEntry,
    default_branch: Option<&str>,
    owners: &BTreeMap<PathBuf, OwnerRecord>,
    live_paths: &[PathBuf],
) -> WorktreeFacts {
    let exists = entry.path.exists();
    let owner = owners.get(&canonical(&entry.path));
    let forge_owned = owner.is_some() || is_forge_path(&entry.path);
    let merged = match (&entry.branch, default_branch) {
        (Some(branch), Some(default)) => is_ancestor(repo_root, branch, default),
        _ => false,
    };

    let target = entry.path.join("target");
    let (artifact_bytes, artifact_age_secs) = artifact_facts(&target);
    let artifact_in_use = path_in_use(&target);
    WorktreeFacts {
        branch: entry.branch.clone(),
        prunable: entry.prunable || !exists,
        merged,
        dirty: exists && is_dirty(&entry.path),
        unpushed: exists && has_unpushed(&entry.path),
        locked: entry.locked,
        live: live_paths.iter().any(|p| is_within(p, &entry.path)),
        forge_owned,
        orphaned: owner.is_some_and(|o| !pid_alive(o.pid)),
        size_bytes: if exists { dir_size(&entry.path) } else { 0 },
        artifact_bytes,
        artifact_in_use,
        artifact_age_secs,
        age_secs: modified_age_secs(&entry.path),
        path: entry.path.clone(),
    }
}

/// A worktree Forge created lives under `.forge/worktrees` or (for the bridged CLIs)
/// `.claude/worktrees`.
fn is_forge_path(path: &Path) -> bool {
    let text = path.to_string_lossy().replace('\\', "/");
    text.contains("/.forge/worktrees/") || text.contains("/.claude/worktrees/")
}

fn is_dirty(worktree: &Path) -> bool {
    Command::new("git")
        .args(["-C", &worktree.to_string_lossy(), "status", "--porcelain"])
        .output()
        .map(|o| !o.stdout.is_empty())
        .unwrap_or(true) // unreadable → treat as dirty, i.e. keep it
}

/// Commits on the worktree's branch that its upstream does not have. A branch with no upstream is
/// NOT reported here — its safety is established by the merge check instead.
fn has_unpushed(worktree: &Path) -> bool {
    let wt = worktree.to_string_lossy().to_string();
    let upstream = Command::new("git")
        .args([
            "-C",
            &wt,
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            "@{u}",
        ])
        .output();
    let Ok(upstream) = upstream else {
        return true; // could not ask git → keep it
    };
    if !upstream.status.success() {
        return false;
    }
    let upstream = String::from_utf8_lossy(&upstream.stdout).trim().to_string();
    Command::new("git")
        .args([
            "-C",
            &wt,
            "rev-list",
            "--count",
            &format!("{upstream}..HEAD"),
        ])
        .output()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .trim()
                .parse::<u64>()
                .unwrap_or(1)
                > 0
        })
        .unwrap_or(true)
}

fn is_ancestor(repo_root: &Path, branch: &str, default_branch: &str) -> bool {
    git(
        repo_root,
        &["merge-base", "--is-ancestor", branch, default_branch],
    )
    .is_ok()
}

fn is_within(candidate: &Path, root: &Path) -> bool {
    let (candidate, root) = (canonical(candidate), canonical(root));
    candidate == root || candidate.starts_with(&root)
}

fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn modified_age_secs(path: &Path) -> Option<u64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    SystemTime::now()
        .duration_since(modified)
        .ok()
        .map(|d| d.as_secs())
}

/// Measure artifacts and use the newest descendant mtime as the staleness signal. Looking only at
/// `target/` itself is unsafe because modifying an existing file does not update its parent.
fn artifact_facts(target: &Path) -> (u64, Option<u64>) {
    if !target.is_dir() {
        return (0, None);
    }
    let mut total = 0u64;
    let mut newest = std::fs::metadata(target)
        .ok()
        .and_then(|m| m.modified().ok());
    let mut stack = vec![target.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(dir) else {
            // An unreadable subtree means the age evidence is incomplete, so refuse pruning.
            return (total, None);
        };
        for entry in entries.flatten() {
            let Ok(meta) = entry.metadata() else {
                return (total, None);
            };
            if let Ok(modified) = meta.modified() {
                newest = Some(newest.map_or(modified, |seen| seen.max(modified)));
            } else {
                return (total, None);
            }
            if meta.is_dir() {
                stack.push(entry.path());
            } else if meta.is_file() {
                total = total.saturating_add(meta.len());
            }
        }
    }
    let age = newest.and_then(|modified| {
        SystemTime::now()
            .duration_since(modified)
            .ok()
            .map(|d| d.as_secs())
    });
    (total, age)
}

/// Cargo holds files and lockfiles open below `target/` while it builds. Linux exposes those
/// references in procfs, giving us a direct in-use signal instead of trying to infer activity from
/// size or process names. On platforms without procfs the conservative mtime gate still applies.
fn path_in_use(path: &Path) -> bool {
    #[cfg(target_os = "linux")]
    {
        let root = canonical(path);
        let Ok(processes) = std::fs::read_dir("/proc") else {
            return false;
        };
        for process in processes.flatten().filter(|e| {
            e.file_name()
                .to_string_lossy()
                .bytes()
                .all(|b| b.is_ascii_digit())
        }) {
            let proc_path = process.path();
            if std::fs::read_link(proc_path.join("cwd"))
                .ok()
                .is_some_and(|p| is_within(&p, &root))
            {
                return true;
            }
            let Ok(fds) = std::fs::read_dir(proc_path.join("fd")) else {
                continue;
            };
            if fds.flatten().any(|fd| {
                std::fs::read_link(fd.path())
                    .ok()
                    .is_some_and(|p| is_within(&p, &root))
            }) {
                return true;
            }
        }
    }
    false
}

/// Recursive size of a directory, `target/` included — the whole point of the measurement is that
/// build artifacts are where the hundreds of gigabytes actually are. Symlinks are not followed, so
/// a link out of the tree cannot inflate the number or cause a cycle.
fn dir_size(path: &Path) -> u64 {
    let mut total = 0u64;
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                stack.push(entry.path());
            } else if meta.is_file() {
                total = total.saturating_add(meta.len());
            }
        }
    }
    total
}

/// Human-readable byte size, used by every surface that reports worktree cost.
pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Compact age rendering ("3d", "5h", "12m").
pub fn human_age(secs: Option<u64>) -> String {
    let Some(secs) = secs else {
        return "-".into();
    };
    match secs {
        s if s >= 86_400 => format!("{}d", s / 86_400),
        s if s >= 3_600 => format!("{}h", s / 3_600),
        s if s >= 60 => format!("{}m", s / 60),
        s => format!("{s}s"),
    }
}

fn git(repo_root: &Path, args: &[&str]) -> Result<Vec<u8>, WorktreeError> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(args)
        .output()
        .map_err(|e| WorktreeError::SpawnFailed(e.to_string()))?;
    if out.status.success() {
        Ok(out.stdout)
    } else {
        Err(WorktreeError::NonZeroExit {
            cmd: args.join(" "),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }
}
