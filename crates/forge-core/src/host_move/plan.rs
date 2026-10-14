//! The path map from the previous host's layout to this one, and the notice that tells the model.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::fingerprint::{HostFingerprint, RepoRef};

const MAX_LISTED: usize = 24;

/// One previous-host path that does not exist here, and where its content most likely lives now.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StaleRoot {
    pub from: String,
    #[serde(default)]
    pub to: Option<String>,
    pub via: String,
    #[serde(default)]
    pub alternates: Vec<String>,
}

impl StaleRoot {
    /// `target` re-rooted at the local equivalent.
    pub(crate) fn mapped(&self, target: &Path) -> Option<PathBuf> {
        let to = Path::new(self.to.as_deref()?);
        let rest = target.strip_prefix(&self.from).ok()?;
        Some(if rest.as_os_str().is_empty() {
            to.to_path_buf()
        } else {
            to.join(rest)
        })
    }
}

fn basename(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

fn shared_components(a: &Path, b: &Path) -> usize {
    a.components()
        .zip(b.components())
        .take_while(|(x, y)| x == y)
        .count()
}

/// Order candidates: same basename first, then the current workspace repo, then closest in the
/// directory tree to `from`.
fn rank<'a>(
    from: &Path,
    current_root: Option<&str>,
    mut hits: Vec<&'a RepoRef>,
) -> Vec<&'a RepoRef> {
    let name = basename(from);
    hits.sort_by_key(|r| {
        let path = Path::new(&r.path);
        (
            basename(path) != name,
            Some(r.path.as_str()) != current_root,
            usize::MAX - shared_components(from, path),
            r.path.clone(),
        )
    });
    hits
}

fn entry(from: &Path, via: String, ranked: &[&RepoRef]) -> StaleRoot {
    StaleRoot {
        from: from.display().to_string(),
        to: ranked.first().map(|r| r.path.clone()),
        via,
        alternates: ranked
            .iter()
            .skip(1)
            .take(3)
            .map(|r| r.path.clone())
            .collect(),
    }
}

/// `checkout-wt-x` belongs to `checkout`: equal names, or the local name plus a separator.
fn name_matches(stale: &str, local: &str) -> bool {
    !local.is_empty()
        && (stale == local
            || stale
                .strip_prefix(local)
                .is_some_and(|rest| rest.starts_with(['-', '_', '.'])))
}

/// Map every previous-host path that is gone here onto a local checkout: by git remote when the
/// previous fingerprint knows the remote, else by checkout name. Longest `from` first, so the
/// guard matches the most specific entry.
pub(crate) fn map_stale(
    stale: &[PathBuf],
    previous: Option<&HostFingerprint>,
    current: &HostFingerprint,
    local: &[RepoRef],
) -> Vec<StaleRoot> {
    let gone = |p: &str| !Path::new(p).exists();
    let current_root = current.repo_root.as_deref();
    let mut out: Vec<StaleRoot> = Vec::new();
    let covered = |out: &[StaleRoot], p: &Path| out.iter().any(|e| p.starts_with(&e.from));
    if let Some(prev) = previous {
        let old_root = prev.repo_root.as_deref().unwrap_or(&prev.cwd);
        let new_root = current_root.unwrap_or(&current.cwd);
        if old_root != new_root && gone(old_root) {
            out.push(StaleRoot {
                from: old_root.to_string(),
                to: Some(new_root.to_string()),
                via: "this session's workspace".to_string(),
                alternates: Vec::new(),
            });
        }
        for old in prev.repos.iter().filter(|r| gone(&r.path)) {
            let from = Path::new(&old.path);
            if covered(&out, from) || old.remotes.is_empty() {
                continue;
            }
            let hits: Vec<&RepoRef> = local
                .iter()
                .filter(|l| l.remotes.iter().any(|r| old.remotes.contains(r)))
                .collect();
            if hits.is_empty() {
                continue;
            }
            let remote = hits[0]
                .remotes
                .iter()
                .find(|r| old.remotes.contains(r))
                .cloned()
                .unwrap_or_default();
            out.push(entry(
                from,
                format!("same git remote {remote}"),
                &rank(from, current_root, hits),
            ));
        }
    }
    for root in stale {
        if covered(&out, root) {
            continue;
        }
        let name = basename(root);
        let hits: Vec<&RepoRef> = local
            .iter()
            .filter(|l| name_matches(&name, &basename(Path::new(&l.path))))
            .collect();
        let mut ranked = rank(root, current_root, hits);
        ranked.sort_by_key(|r| usize::MAX - basename(Path::new(&r.path)).len());
        let via = if ranked.is_empty() {
            "no local equivalent found".to_string()
        } else {
            "same checkout name".to_string()
        };
        out.push(entry(root, via, &ranked));
    }
    out.sort_by(|a, b| b.from.len().cmp(&a.from.len()).then(a.from.cmp(&b.from)));
    out
}

/// The current layout, as the notice and the `/reground` summarizer see it.
pub(crate) fn layout(current: &HostFingerprint) -> String {
    let mut lines = Vec::new();
    match current.repo_root.as_deref() {
        Some(root) => {
            let branch = current.branch.as_deref().unwrap_or("detached");
            let head = current
                .head
                .as_deref()
                .map(|h| format!(", HEAD {}", &h[..h.len().min(10)]))
                .unwrap_or_default();
            lines.push(format!("- repo root: {root} (branch {branch}{head})"));
            if current.cwd != root {
                lines.push(format!("- workspace: {}", current.cwd));
            }
            let worktrees = super::fingerprint::worktrees(Path::new(root));
            if worktrees.len() > 1 {
                lines.push(format!("- git worktrees: {}", join_paths(&worktrees)));
            }
            let siblings: Vec<String> = super::fingerprint::sibling_dirs(Path::new(root))
                .iter()
                .map(|d| {
                    let git = if super::fingerprint::is_repo(d) {
                        " (git)"
                    } else {
                        ""
                    };
                    format!("{}{git}", d.display())
                })
                .collect();
            if !siblings.is_empty() {
                lines.push(format!("- sibling directories: {}", list(&siblings)));
            }
            lines.push(format!(
                "- top-level entries: {}",
                top_level(Path::new(root))
            ));
        }
        None => {
            lines.push(format!("- workspace: {} (not a git checkout)", current.cwd));
            lines.push(format!(
                "- top-level entries: {}",
                top_level(Path::new(&current.cwd))
            ));
        }
    }
    lines.join("\n")
}

fn join_paths(paths: &[PathBuf]) -> String {
    list(
        &paths
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>(),
    )
}

fn list(items: &[String]) -> String {
    let mut shown = items
        .iter()
        .take(MAX_LISTED)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    if items.len() > MAX_LISTED {
        shown.push_str(&format!(", … ({} more)", items.len() - MAX_LISTED));
    }
    shown
}

fn top_level(dir: &Path) -> String {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_name() != ".git")
        .map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            if e.path().is_dir() {
                format!("{name}/")
            } else {
                name
            }
        })
        .collect();
    names.sort();
    if names.is_empty() {
        "(empty)".to_string()
    } else {
        list(&names)
    }
}

/// The one system notice a moved session gets. Deterministic for a given layout: no timestamps.
pub(crate) fn notice(
    previous: Option<&HostFingerprint>,
    current: &HostFingerprint,
    reason: &str,
    map: &[StaleRoot],
) -> String {
    let from = previous.map_or_else(
        || {
            "an earlier host or layout (not recorded: this session predates host tracking)"
                .to_string()
        },
        HostFingerprint::label,
    );
    let mut text = format!(
        "[host] This session moved: {reason}.\nBefore: {from}.\nNow: {}.\n\n\
         History from before this point was written on the previous host, so paths it names may \
         not exist here. Tool outputs in this session are real and trustworthy: what a tool \
         returns describes this host. Before using a path you remember from that earlier history, \
         check it once (list_dir, read_file, `git worktree list`), and if it is missing here, use \
         the path map below instead of recreating it. There is no need to audit earlier tool \
         calls; the session itself is fine.\n\n\
         Current layout:\n{}",
        current.label(),
        layout(current)
    );
    let mapped: Vec<&StaleRoot> = map.iter().filter(|e| e.to.is_some()).collect();
    if !mapped.is_empty() {
        text.push_str("\n\nPath map (previous → here):");
        for e in mapped.iter().take(MAX_LISTED) {
            let to = e.to.as_deref().unwrap_or_default();
            text.push_str(&format!("\n- {} → {to} ({})", e.from, e.via));
            if !e.alternates.is_empty() {
                text.push_str(&format!(
                    "; also checked out at {}",
                    e.alternates.join(", ")
                ));
            }
        }
    }
    let unmapped: Vec<String> = map
        .iter()
        .filter(|e| e.to.is_none())
        .map(|e| e.from.clone())
        .collect();
    if !unmapped.is_empty() {
        text.push_str(&format!(
            "\n\nPrevious paths with no equivalent here (do not use them): {}",
            list(&unmapped)
        ));
    }
    text
}

/// The extra instruction `/reground` gives the summarizer.
pub(crate) fn reground_directive(current: &HostFingerprint) -> String {
    format!(
        "RE-GROUNDING: this history was written on a different host or workspace layout than the \
         one the session runs on now. In the summary, DROP host-specific facts that no longer \
         hold: absolute paths, worktree and directory names, layouts, file contents and command \
         output tied to the previous host, unless the current layout below confirms them. KEEP \
         goals, decisions, constraints, open tasks and conclusions about how the code behaves, \
         using repo-relative paths. Current layout:\n{}",
        layout(current)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo(path: &str, remote: &str) -> RepoRef {
        RepoRef {
            path: path.into(),
            remotes: vec![remote.into()],
        }
    }

    #[test]
    fn previous_repos_map_by_remote_and_history_roots_by_name() {
        let tmp = tempfile::tempdir().unwrap();
        let here = tmp.path().canonicalize().unwrap();
        let api = here.join("API/app");
        let autopilot = here.join("autopilot/app");
        let gone = "/nonexistent-host-move/laptop/API";
        let previous = HostFingerprint {
            host: "laptop".into(),
            cwd: format!("{gone}/app-wt-feature"),
            repo_root: Some(format!("{gone}/app-wt-feature")),
            repos: vec![
                repo(&format!("{gone}/app-wt-feature"), "github.com/acme/app"),
                repo(&format!("{gone}/app-wt-other"), "github.com/acme/app"),
                repo(&format!("{gone}/dash"), "github.com/acme/dash"),
            ],
            ..Default::default()
        };
        let current = HostFingerprint {
            host: "server".into(),
            cwd: api.display().to_string(),
            repo_root: Some(api.display().to_string()),
            ..Default::default()
        };
        let local = vec![
            repo(&api.display().to_string(), "github.com/acme/app"),
            repo(&autopilot.display().to_string(), "github.com/acme/app"),
        ];
        let stale = vec![
            PathBuf::from(format!("{gone}/app-wt-third")),
            PathBuf::from("/nonexistent-host-move/laptop/sg-pro"),
        ];
        let map = map_stale(&stale, Some(&previous), &current, &local);
        let find = |from: &str| map.iter().find(|e| e.from.ends_with(from)).unwrap();
        assert_eq!(find("app-wt-feature").via, "this session's workspace");
        let other = find("app-wt-other");
        assert_eq!(other.to.as_deref(), Some(api.to_str().unwrap()));
        assert_eq!(other.via, "same git remote github.com/acme/app");
        assert_eq!(other.alternates, vec![autopilot.display().to_string()]);
        assert_eq!(find("app-wt-third").via, "same checkout name");
        assert_eq!(find("sg-pro").to, None);
        assert!(
            map.iter().all(|e| !e.from.ends_with("/dash")),
            "no local dash"
        );
        let notice = notice(Some(&previous), &current, "it moved", &map);
        assert!(notice.contains("Path map (previous → here):"));
        assert!(notice.contains("no equivalent here"));
        assert!(notice.contains(&format!("Now: host `server` (workspace {})", api.display())));
    }

    #[test]
    fn mapped_rewrites_the_prefix_only() {
        let e = StaleRoot {
            from: "/old/app-wt".into(),
            to: Some("/new/app".into()),
            via: String::new(),
            alternates: Vec::new(),
        };
        assert_eq!(
            e.mapped(Path::new("/old/app-wt/src/a.rs")),
            Some(PathBuf::from("/new/app/src/a.rs"))
        );
        assert_eq!(
            e.mapped(Path::new("/old/app-wt")),
            Some(PathBuf::from("/new/app"))
        );
        assert_eq!(e.mapped(Path::new("/old/app-wtx/a")), None);
    }

    #[test]
    fn checkout_names_match_on_a_separator_boundary() {
        assert!(name_matches("ss-ultimate-wt-x", "ss-ultimate"));
        assert!(name_matches("ss-ultimate", "ss-ultimate"));
        assert!(!name_matches("ss-ultimatex", "ss-ultimate"));
        assert!(!name_matches("ss-ultimate-wt-x", "ssu-dashboard"));
    }
}
