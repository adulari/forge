//! Moved-session grounding (docs/features/session-host-move.md).
//!
//! A long session moved from a laptop to a server kept describing the laptop: sibling worktrees,
//! directory names, file contents. Same user, same home directory, so every remembered absolute
//! path looked plausible and pointed at nothing, and the model invented files to fill the gap.
//! Nothing on resume, import or handoff told it the machine had changed.
//!
//! Each session records the host it was last grounded on ([`HostFingerprint`]). Resume compares it
//! with the current host; when they differ (or, for a session from before tracking, when its own
//! history names directories that do not exist here) one system notice is appended and persisted
//! with the layout and a path map, and the [`guard::HostGuard`] turns file-tool calls into stale
//! paths into an error naming the local equivalent. The notice is decided by an atomic swap of the
//! stored fingerprint, so it is sent once per move, never per turn.

mod fingerprint;
mod guard;
mod plan;
mod scan;

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub(crate) use fingerprint::{remotes as repo_remotes, repo_root};
pub use fingerprint::{HostFingerprint, RepoRef};
pub(crate) use guard::HostGuard;
pub use plan::StaleRoot;

use crate::{CoreError, Session};
use forge_types::{Message, PresenterEvent, Role};

/// Legacy sessions (no recorded fingerprint) count as moved at this many missing directories.
const LEGACY_MIN_STALE_ROOTS: usize = 2;
const HOME_SCAN_DEPTH: usize = 5;
const HOME_SCAN_BUDGET: usize = 4_000;

/// What `session.host_state` holds.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostState {
    pub fingerprint: HostFingerprint,
    /// Previous-host paths that are gone here, with their local equivalents (the guard's table).
    #[serde(default)]
    pub stale: Vec<StaleRoot>,
    /// Local checkouts known when the move was detected (for the guard's name matching).
    #[serde(default)]
    pub local_repos: Vec<String>,
}

impl HostState {
    fn parse(raw: Option<&str>) -> Option<Self> {
        serde_json::from_str(raw?).ok()
    }

    fn guard(&self) -> HostGuard {
        HostGuard::new(self.stale.clone(), &self.local_repos)
    }
}

/// The outcome of one detection pass.
struct Detection {
    notice: Option<String>,
    state: HostState,
}

fn home_prefixes(fingerprints: &[Option<&HostFingerprint>]) -> Vec<String> {
    let mut prefixes: Vec<String> = fingerprints
        .iter()
        .flatten()
        .filter_map(|f| f.home.clone())
        .chain(std::env::var("HOME").ok())
        .filter(|h| h.len() > 1)
        .map(|h| format!("{}/", h.trim_end_matches('/')))
        .collect();
    prefixes.sort();
    prefixes.dedup();
    prefixes
}

/// Every local checkout worth matching against: the layout around the workspace, plus (only when
/// something is still unmapped) a bounded scan of the home directory and the old roots' parents.
fn local_repos(
    current: &HostFingerprint,
    extra_roots: &[PathBuf],
    scan_home: bool,
) -> Vec<RepoRef> {
    let mut repos = current.repos.clone();
    let mut found = scan::find_repos(extra_roots, 1, HOME_SCAN_BUDGET);
    if scan_home {
        let home: Vec<PathBuf> = current.home.iter().map(PathBuf::from).collect();
        found.extend(scan::find_repos(&home, HOME_SCAN_DEPTH, HOME_SCAN_BUDGET));
    }
    for path in found {
        let path_text = path.display().to_string();
        if !repos.iter().any(|r| r.path == path_text) {
            repos.push(RepoRef {
                remotes: fingerprint::remotes(&path),
                path: path_text,
            });
        }
    }
    repos
}

/// Decide whether the session moved and build the notice + new state. Pure over its inputs apart
/// from filesystem reads, so it is testable without a `Session`.
fn detect<'a>(
    stored: Option<&HostState>,
    current: HostFingerprint,
    history: impl Iterator<Item = &'a str>,
    history_empty: bool,
    force: bool,
    scan_home: bool,
) -> Detection {
    let previous = stored.map(|s| &s.fingerprint);
    let moved = previous.and_then(|p| p.moved_reason(&current));
    let unknown_history = stored.is_none() && !history_empty;
    if moved.is_none() && !force && !unknown_history {
        let state = match stored {
            Some(s) => HostState {
                fingerprint: current,
                ..s.clone()
            },
            None => HostState {
                fingerprint: current,
                ..Default::default()
            },
        };
        return Detection {
            notice: None,
            state,
        };
    }
    let prefixes = home_prefixes(&[previous, Some(&current)]);
    let workspace = PathBuf::from(current.repo_root.as_deref().unwrap_or(&current.cwd));
    let paths = history.flat_map(|text| scan::absolute_paths(text, &prefixes).collect::<Vec<_>>());
    let stale = scan::stale_roots(paths, &workspace, Path::exists);
    let reason = moved.or_else(|| {
        (unknown_history && stale.len() >= LEGACY_MIN_STALE_ROOTS).then(|| {
            format!(
                "its history names {} directories that do not exist on this host (`{}`)",
                stale.len(),
                current.host
            )
        })
    });
    let reason = match (reason, force) {
        (Some(reason), _) => reason,
        (None, true) => "a re-grounding was requested (/reground)".to_string(),
        (None, false) => {
            return Detection {
                notice: None,
                state: HostState {
                    fingerprint: current,
                    ..stored.cloned().unwrap_or_default()
                },
            }
        }
    };
    let old_parents: Vec<PathBuf> = stale
        .iter()
        .filter_map(|r| r.parent().filter(|p| p.is_dir()).map(Path::to_path_buf))
        .collect();
    let mut local = local_repos(&current, &old_parents, false);
    let mut map = plan::map_stale(&stale, previous, &current, &local);
    if scan_home && map.iter().any(|e| e.to.is_none()) {
        local = local_repos(&current, &old_parents, true);
        map = plan::map_stale(&stale, previous, &current, &local);
    }
    // Entries from earlier moves stay while their source is still gone.
    for old in stored.map(|s| s.stale.as_slice()).unwrap_or_default() {
        if !Path::new(&old.from).exists() && !map.iter().any(|e| e.from == old.from) {
            map.push(old.clone());
        }
    }
    let notice = plan::notice(previous, &current, &reason, &map);
    Detection {
        notice: Some(notice),
        state: HostState {
            fingerprint: current,
            stale: map,
            local_repos: local.into_iter().map(|r| r.path).collect(),
        },
    }
}

/// The recorded cwd of a resumed session, relocated when it does not exist on this host: the local
/// checkout with the same git remote (or checkout name) as the session's recorded repo, keeping the
/// cwd's position inside the repo. Persists the new cwd and drops a recorded worktree that is gone.
/// Returns `recorded` unchanged when it exists or nothing matches.
pub fn resolve_resume_cwd(store: &forge_store::Store, session_id: &str, recorded: &str) -> String {
    if Path::new(recorded).is_dir() {
        return recorded.to_string();
    }
    let raw = store.session_host_state(session_id).ok().flatten();
    let state = HostState::parse(raw.as_deref());
    let old_root = state
        .as_ref()
        .and_then(|s| s.fingerprint.repo_root.clone())
        .unwrap_or_else(|| recorded.to_string());
    let remotes: Vec<String> = state
        .iter()
        .flat_map(|s| s.fingerprint.repos.iter())
        .filter(|r| r.path == old_root)
        .flat_map(|r| r.remotes.clone())
        .collect();
    let name = Path::new(&old_root).file_name().map(|n| n.to_os_string());
    let by_remote = |p: &PathBuf| {
        !remotes.is_empty() && fingerprint::remotes(p).iter().any(|r| remotes.contains(r))
    };
    let by_name =
        |p: &PathBuf| remotes.is_empty() && name.is_some() && p.file_name() == name.as_deref();
    let invoked = std::env::current_dir()
        .ok()
        .and_then(|d| fingerprint::repo_root(&d));
    let pick = invoked.filter(|p| by_remote(p) || by_name(p)).or_else(|| {
        let home: Vec<PathBuf> = std::env::var("HOME")
            .ok()
            .map(PathBuf::from)
            .into_iter()
            .collect();
        let candidates = scan::find_repos(&home, HOME_SCAN_DEPTH, HOME_SCAN_BUDGET);
        let found = candidates.iter().find(|p| by_remote(p));
        found
            .or_else(|| candidates.iter().find(|p| by_name(p)))
            .cloned()
    });
    let Some(new_root) = pick else {
        return recorded.to_string();
    };
    let inner = Path::new(recorded).strip_prefix(&old_root).ok();
    let relocated = inner
        .map(|rest| new_root.join(rest))
        .filter(|p| p.is_dir())
        .unwrap_or(new_root);
    let relocated = relocated.display().to_string();
    tracing::info!(session_id, from = recorded, to = %relocated, "relocated a moved session's workspace");
    let _ = store.set_session_cwd(session_id, &relocated);
    if store
        .session_worktree(session_id)
        .ok()
        .flatten()
        .is_some_and(|wt| !Path::new(&wt).exists())
    {
        let _ = store.clear_session_worktree(session_id);
    }
    relocated
}

/// The session's workspace moved on purpose on this host (the `worktree` tool entering or leaving
/// a worktree): re-record the fingerprint at the new root, keeping any stale-path map, so the next
/// resume does not mistake the switch for a host move.
pub fn record_workspace_switch(store: &forge_store::Store, session_id: &str, root: &Path) {
    let raw = store.session_host_state(session_id).ok().flatten();
    let state = HostState {
        fingerprint: HostFingerprint::capture(root),
        ..HostState::parse(raw.as_deref()).unwrap_or_default()
    };
    if let Ok(json) = serde_json::to_string(&state) {
        let _ = store.swap_session_host_state(session_id, raw.as_deref(), &json);
    }
}

impl Session {
    /// Record the host a fresh session starts on, so a later move is detected exactly.
    pub(crate) fn record_host_on_start(&mut self) {
        let state = HostState {
            fingerprint: HostFingerprint::capture(self.workspace.root()),
            ..Default::default()
        };
        if let Ok(json) = serde_json::to_string(&state) {
            let _ = self.store.swap_session_host_state(&self.id, None, &json);
        }
    }

    /// Resume-time detection: append the moved-session notice once when the host or layout
    /// changed, and arm the stale-path guard. Best-effort: a detection failure never blocks resume.
    pub(crate) fn reconcile_host_on_resume(&mut self) {
        let _ = self.reconcile_host(false);
    }

    /// `/reground`: re-run detection unconditionally (the notice is appended even when the
    /// fingerprint matches), and arm the next compaction to drop host-specific facts. Returns a
    /// one-line report for the UI.
    pub fn prepare_reground(&mut self) -> Result<String, CoreError> {
        let notice = self.reconcile_host(true)?;
        let current = HostFingerprint::capture(self.workspace.root());
        self.compact_directive = Some(plan::reground_directive(&current));
        let mapped = self.host_guard_entries();
        Ok(match notice {
            Some(_) => format!(
                "⟲ re-grounded on host `{}`: layout notice added ({mapped} stale path root(s) \
                 guarded); re-summarizing history without host-specific facts…",
                current.host
            ),
            None => {
                "⟲ another Forge process just re-grounded this session; re-summarizing…".to_string()
            }
        })
    }

    fn host_guard_entries(&self) -> usize {
        HostState::parse(
            self.store
                .session_host_state(&self.id)
                .ok()
                .flatten()
                .as_deref(),
        )
        .map_or(0, |s| s.stale.len())
    }

    fn reconcile_host(&mut self, force: bool) -> Result<Option<String>, CoreError> {
        let raw = self.store.session_host_state(&self.id)?;
        let stored = HostState::parse(raw.as_deref());
        let current = HostFingerprint::capture(self.workspace.root());
        let history_empty = self.transcript.iter().all(|m| m.role == Role::System);
        let args: Vec<String> = self
            .transcript
            .iter()
            .flat_map(|m| m.tool_calls.iter().map(|c| c.args.to_string()))
            .collect();
        let history = self
            .transcript
            .iter()
            .map(|m| m.content.as_str())
            .chain(args.iter().map(String::as_str));
        let detection = detect(
            stored.as_ref(),
            current,
            history,
            history_empty,
            force,
            true,
        );
        let json = serde_json::to_string(&detection.state)?;
        if raw.as_deref() == Some(json.as_str()) && detection.notice.is_none() {
            self.host_guard = detection.state.guard();
            return Ok(None);
        }
        if !self
            .store
            .swap_session_host_state(&self.id, raw.as_deref(), &json)?
        {
            let now = HostState::parse(self.store.session_host_state(&self.id)?.as_deref());
            self.host_guard = now.map(|s| s.guard()).unwrap_or_default();
            return Ok(None);
        }
        self.host_guard = detection.state.guard();
        let Some(notice) = detection.notice else {
            return Ok(None);
        };
        let seq = self.next_seq();
        self.store
            .add_message(&self.id, seq, Role::System, &notice, None)?;
        self.transcript.push(Message::system(&notice));
        let summary = notice.lines().next().unwrap_or_default().to_string();
        self.presenter.emit(PresenterEvent::Warning(format!(
            "{summary} — told the model, mapped {} stale path root(s)",
            detection.state.stale.len()
        )));
        Ok(Some(notice))
    }

    /// The stale-path guard's verdict for a tool call (`None` = allowed).
    pub(crate) fn stale_host_path(&self, args: &serde_json::Value) -> Option<String> {
        if self.host_guard.is_empty() {
            return None;
        }
        self.host_guard.check(args, self.workspace.root())
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::path::Path;

    pub(crate) fn fake_repo(dir: &Path, remote: &str) {
        std::fs::create_dir_all(dir.join(".git/refs/heads")).unwrap();
        std::fs::write(dir.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(dir.join(".git/refs/heads/main"), "abc123\n").unwrap();
        std::fs::write(
            dir.join(".git/config"),
            format!("[core]\n\tbare = false\n[remote \"origin\"]\n\turl = {remote}\n"),
        )
        .unwrap();
    }
}

#[cfg(test)]
mod tests;
