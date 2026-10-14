//! Finding the previous host's layout in a session's own history, and local checkouts on disk.
//!
//! A session from before host tracking has no recorded fingerprint, so the only evidence of a move
//! is the history itself: absolute paths it used that do not exist here. What counts is a missing
//! DIRECTORY with content below it (`.../API/ss-ultimate-wt-x/src/main.rs` → `ss-ultimate-wt-x`),
//! outside the workspace. A missing leaf file is ordinary (deleted, never created, a typo) and says
//! nothing about the machine.

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::path::{Path, PathBuf};

const MAX_DISTINCT_PATHS: usize = 4_000;
const MAX_STALE_ROOTS: usize = 40;
const SKIP_DIRS: &[&str] = &[
    "node_modules",
    "target",
    "vendor",
    "build",
    "dist",
    "Library",
    "snap",
    "go",
];

/// Absolute paths under one of `prefixes` mentioned in `text`.
pub(crate) fn absolute_paths<'a>(
    text: &'a str,
    prefixes: &'a [String],
) -> impl Iterator<Item = String> + 'a {
    text.split(|c: char| {
        c.is_whitespace()
            || matches!(
                c,
                '"' | '\''
                    | '`'
                    | '('
                    | ')'
                    | '['
                    | ']'
                    | '{'
                    | '}'
                    | '<'
                    | '>'
                    | ','
                    | ';'
                    | '|'
                    | '='
            )
    })
    .flat_map(|token| token.split("\\n"))
    .map(|token| token.trim_end_matches(['.', ':', '\\', '/']))
    .filter(|token| {
        prefixes
            .iter()
            .any(|p| token.len() > p.len() && token.starts_with(p.as_str()))
    })
    .filter(|token| !token.contains("..") && !token.contains(['*', '$', '~']))
    .map(str::to_string)
}

/// Distinct missing directory roots (see the module doc) among `paths`, outside `workspace`.
pub(crate) fn stale_roots(
    paths: impl IntoIterator<Item = String>,
    workspace: &Path,
    exists: impl Fn(&Path) -> bool,
) -> Vec<PathBuf> {
    let distinct: BTreeSet<String> = paths.into_iter().take(MAX_DISTINCT_PATHS * 4).collect();
    let mut cache: HashMap<PathBuf, bool> = HashMap::new();
    let mut known = |p: &Path| *cache.entry(p.to_path_buf()).or_insert_with(|| exists(p));
    let mut roots = BTreeSet::new();
    for path in distinct.iter().take(MAX_DISTINCT_PATHS) {
        let path = Path::new(path);
        if roots.iter().any(|r: &PathBuf| path.starts_with(r)) || known(path) {
            continue;
        }
        let Some(root) = path
            .ancestors()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .find(|a| !known(a))
        else {
            continue;
        };
        let dir_level = root != path;
        let foreign = !root.starts_with(workspace) && !root.to_string_lossy().contains("/.forge/");
        if dir_level && foreign {
            roots.insert(root.to_path_buf());
        }
        if roots.len() >= MAX_STALE_ROOTS {
            break;
        }
    }
    roots.into_iter().collect()
}

/// The first missing ancestor of `path` when it is a directory with content below it.
pub(crate) fn missing_dir_root(path: &Path) -> Option<PathBuf> {
    let root = path
        .ancestors()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .find(|a| !a.exists())?;
    (root != path).then(|| root.to_path_buf())
}

/// Git checkouts under `roots`, breadth-first, without descending into a checkout, bounded by
/// `max_depth` and a visited-directory `budget` so a huge home directory stays cheap.
pub(crate) fn find_repos(roots: &[PathBuf], max_depth: usize, budget: usize) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut queue: VecDeque<(PathBuf, usize)> = roots.iter().map(|r| (r.clone(), 0)).collect();
    let mut seen = BTreeSet::new();
    let mut visited = 0;
    while let Some((dir, depth)) = queue.pop_front() {
        if visited >= budget || !seen.insert(dir.clone()) {
            continue;
        }
        visited += 1;
        if super::fingerprint::is_repo(&dir) {
            found.push(dir);
            continue;
        }
        if depth >= max_depth {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut children: Vec<PathBuf> = entries
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .filter(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                !name.starts_with('.') && !SKIP_DIRS.contains(&name.as_str())
            })
            .map(|e| e.path())
            .collect();
        children.sort();
        queue.extend(children.into_iter().map(|c| (c, depth + 1)));
    }
    found.sort();
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_absolute_paths_from_prose_json_and_commands() {
        let prefixes = vec!["/home/u/".to_string()];
        let text = r#"edited `/home/u/API/app-wt-1/src/main.rs`. {"path":"/home/u/sg-pro/a.ts"} cd /home/u/src && ls\n/home/u/x/y: done /etc/passwd ../rel /home/u/$VAR/z"#;
        let found: Vec<_> = absolute_paths(text, &prefixes).collect();
        assert_eq!(
            found,
            vec![
                "/home/u/API/app-wt-1/src/main.rs",
                "/home/u/sg-pro/a.ts",
                "/home/u/src",
                "/home/u/x/y",
            ]
        );
    }

    #[test]
    fn stale_roots_keep_missing_directories_outside_the_workspace_only() {
        let existing: BTreeSet<&str> = ["/", "/home", "/home/u", "/home/u/API", "/home/u/API/app"]
            .into_iter()
            .collect();
        let exists = |p: &Path| existing.contains(p.to_str().unwrap());
        let paths = [
            "/home/u/API/app-wt-1/src/main.rs",
            "/home/u/API/app-wt-1/Cargo.toml",
            "/home/u/API/app/src/gone/mod.rs",
            "/home/u/API/app/new_file.rs",
            "/home/u/sg-pro/a.ts",
            "/home/u/API/never.rs",
            "/home/u/API/app/.forge/worktrees/x/y.rs",
        ]
        .map(String::from);
        let roots = stale_roots(paths, Path::new("/home/u/API/app"), exists);
        assert_eq!(
            roots,
            vec![
                PathBuf::from("/home/u/API/app-wt-1"),
                PathBuf::from("/home/u/sg-pro"),
            ]
        );
    }

    #[test]
    fn find_repos_stops_at_checkouts_and_respects_depth() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        std::fs::create_dir_all(root.join("a/b/repo/.git")).unwrap();
        std::fs::create_dir_all(root.join("a/b/repo/nested/.git")).unwrap();
        std::fs::create_dir_all(root.join("deep/1/2/3/4/repo/.git")).unwrap();
        std::fs::create_dir_all(root.join("node_modules/pkg/.git")).unwrap();
        let found = find_repos(std::slice::from_ref(&root), 3, 1_000);
        assert_eq!(found, vec![root.join("a/b/repo")]);
    }
}
