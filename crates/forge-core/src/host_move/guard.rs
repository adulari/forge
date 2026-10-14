//! Stale-path guard: a file tool or shell `cwd` aimed at a previous-host path that does not exist
//! here gets an error naming the local equivalent, instead of a bare "not found" that invites the
//! model to recreate the file from memory.

use std::path::{Path, PathBuf};

use super::plan::StaleRoot;

#[derive(Debug, Clone, Default)]
pub(crate) struct HostGuard {
    entries: Vec<StaleRoot>,
    /// Directories that held a previous-host root (`.../API/`): an invented sibling there is the
    /// same confusion as a remembered one.
    layout_parents: Vec<PathBuf>,
    local_repos: Vec<PathBuf>,
}

impl HostGuard {
    pub(crate) fn new(entries: Vec<StaleRoot>, local_repos: &[String]) -> Self {
        let mut layout_parents: Vec<PathBuf> = entries
            .iter()
            .filter_map(|e| Path::new(&e.from).parent().map(Path::to_path_buf))
            .collect();
        layout_parents.sort();
        layout_parents.dedup();
        Self {
            entries,
            layout_parents,
            local_repos: local_repos.iter().map(PathBuf::from).collect(),
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The error for the first target this call names that is a stale previous-host path.
    pub(crate) fn check(&self, args: &serde_json::Value, workspace: &Path) -> Option<String> {
        if self.entries.is_empty() {
            return None;
        }
        forge_tools::path_arg_targets(args, workspace)
            .into_iter()
            .filter(|target| !target.exists())
            .find_map(|target| self.check_target(&target, workspace))
    }

    fn check_target(&self, target: &Path, workspace: &Path) -> Option<String> {
        if let Some(entry) = self
            .entries
            .iter()
            .find(|e| target.starts_with(&e.from) && !Path::new(&e.from).exists())
        {
            return Some(message(target, entry, workspace));
        }
        let root = super::scan::missing_dir_root(target)?;
        let parent = root.parent()?;
        if root.starts_with(workspace) || !self.layout_parents.iter().any(|p| p == parent) {
            return None;
        }
        let name = root.file_name()?.to_string_lossy().to_lowercase();
        let local = self.local_repos.iter().find(|repo| {
            let base = repo
                .file_name()
                .map(|n| n.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            !base.is_empty()
                && (name == base
                    || name
                        .strip_prefix(&base)
                        .is_some_and(|r| r.starts_with(['-', '_', '.'])))
        });
        let entry = StaleRoot {
            from: root.display().to_string(),
            to: local.map(|p| p.display().to_string()),
            via: "same checkout name".to_string(),
            alternates: Vec::new(),
        };
        Some(message(target, &entry, workspace))
    }
}

fn message(target: &Path, entry: &StaleRoot, workspace: &Path) -> String {
    let head = format!(
        "`{}` does not exist on this host. This session moved hosts, and `{}` belongs to the \
         previous host's layout.",
        target.display(),
        entry.from
    );
    match entry.mapped(target) {
        Some(mapped) => {
            let caveat = if mapped.exists() {
                String::new()
            } else {
                " That path does not exist either, so list the directory first.".to_string()
            };
            let also = if entry.alternates.is_empty() {
                String::new()
            } else {
                format!(" Other checkouts of it: {}.", entry.alternates.join(", "))
            };
            format!(
                "{head} The local equivalent is `{}` ({}).{caveat}{also} Verify with list_dir or \
                 read_file before retrying; do not recreate files from memory.",
                mapped.display(),
                entry.via
            )
        }
        None => format!(
            "{head} It has no known equivalent here. Inspect the current layout (list_dir on {}, \
             `git worktree list`) instead of guessing, and do not recreate it from memory.",
            workspace.display()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn stale_targets_name_the_local_equivalent_and_live_paths_pass() {
        let tmp = tempfile::tempdir().unwrap();
        let here = tmp.path().canonicalize().unwrap();
        let app = here.join("app");
        std::fs::create_dir_all(app.join("src")).unwrap();
        std::fs::write(app.join("src/main.rs"), "").unwrap();
        // As on the new host: the parent directory exists, the old worktree beside it does not.
        std::fs::create_dir_all(here.join("gone/API")).unwrap();
        let old = here.join("gone/API/app-wt-x");
        let guard = HostGuard::new(
            vec![
                StaleRoot {
                    from: old.display().to_string(),
                    to: Some(app.display().to_string()),
                    via: "same git remote github.com/acme/app".into(),
                    alternates: vec!["/srv/app".into()],
                },
                StaleRoot {
                    from: here.join("gone/sg-pro").display().to_string(),
                    to: None,
                    via: "no local equivalent found".into(),
                    alternates: Vec::new(),
                },
            ],
            &[app.display().to_string()],
        );
        let err = guard
            .check(&json!({ "path": old.join("src/main.rs") }), &app)
            .unwrap();
        assert!(err.contains(&format!("`{}`", app.join("src/main.rs").display())));
        assert!(err.contains("same git remote"));
        assert!(err.contains("/srv/app"));
        assert!(!err.contains("does not exist either"));

        let err = guard
            .check(&json!({ "cwd": old.join("missing") }), &app)
            .unwrap();
        assert!(err.contains("does not exist either"));

        let err = guard
            .check(&json!({ "path": here.join("gone/sg-pro/a.ts") }), &app)
            .unwrap();
        assert!(err.contains("no known equivalent"));

        let invented = here.join("gone/API/app-wt-invented/src/lib.rs");
        let err = guard.check(&json!({ "path": invented }), &app).unwrap();
        assert!(err.contains(&format!("`{}`", app.join("src/lib.rs").display())));

        assert_eq!(
            guard.check(&json!({ "path": app.join("src/main.rs") }), &app),
            None
        );
        assert_eq!(
            guard.check(&json!({ "path": app.join("new.rs") }), &app),
            None
        );
        assert_eq!(
            guard.check(&json!({ "path": here.join("elsewhere/x/y.rs") }), &app),
            None
        );
    }
}
