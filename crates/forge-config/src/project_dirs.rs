//! Which directories contribute project resources (docs/features/command-skill-system.md).
//!
//! Skills, commands and agent definitions used to load from the working directory only, so a
//! session started in `repo/pkg/` did not see `repo/.forge/commands`. They now resolve from the
//! working directory up to the repository top (the nearest ancestor holding `.git`, a directory or
//! a linked worktree's pointer file). Project `config.toml` stays cwd-only: settings can widen
//! permissions, and only the directory Forge was started in is reviewed for that.
//!
//! Trust is honoured: when the working directory is trusted, a parent that is not trusted
//! contributes nothing, so trusting `repo/pkg` never brings in `repo/.forge`. An untrusted working
//! directory has no trusted parent (trust covers a directory and everything below it), and its
//! parents' resources load exactly as its own do, behind the same first-use confirmation.

use std::path::{Path, PathBuf};

/// The directories whose project resources apply to a session in `cwd`, nearest first: `cwd`
/// itself (as given), then each parent up to and including the repository top. Only `cwd` when it
/// is not inside a repository.
pub fn project_resource_dirs(cwd: &Path) -> Vec<PathBuf> {
    resource_dirs_with(cwd, crate::project_trust::is_trusted)
}

/// The nearest ancestor of `dir` (itself included) that holds `.git`.
pub fn repo_top(dir: &Path) -> Option<PathBuf> {
    dir.ancestors()
        .find(|ancestor| ancestor.join(".git").exists())
        .map(Path::to_path_buf)
}

fn resource_dirs_with(cwd: &Path, is_trusted: impl Fn(&Path) -> bool) -> Vec<PathBuf> {
    let mut dirs = vec![cwd.to_path_buf()];
    let Ok(canonical) = cwd.canonicalize() else {
        return dirs;
    };
    let Some(top) = repo_top(&canonical) else {
        return dirs;
    };
    if top == canonical {
        return dirs;
    }
    let cwd_trusted = is_trusted(&canonical);
    for dir in canonical.ancestors().skip(1) {
        if !cwd_trusted || is_trusted(dir) {
            dirs.push(dir.to_path_buf());
        }
        if dir == top {
            break;
        }
    }
    dirs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap().join("repo");
        let deep = root.join("pkg/sub");
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::create_dir_all(&deep).unwrap();
        (tmp, root, deep)
    }

    #[test]
    fn a_subdirectory_reaches_up_to_the_repository_top_nearest_first() {
        let (_tmp, root, deep) = repo();
        assert_eq!(
            resource_dirs_with(&deep, |_| false),
            [deep.clone(), root.join("pkg"), root.clone()]
        );
        assert_eq!(resource_dirs_with(&root, |_| false), vec![root.clone()]);
        // Nothing above the repository top.
        assert!(!resource_dirs_with(&deep, |_| false).contains(&root.parent().unwrap().into()));
    }

    #[test]
    fn outside_a_repository_only_the_working_directory_counts() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("a/b");
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(resource_dirs_with(&dir, |_| true), [dir]);
    }

    #[test]
    fn a_trusted_subdirectory_skips_untrusted_parents() {
        let (_tmp, root, deep) = repo();
        let pkg = root.join("pkg");
        let trusted = |dir: &Path| dir.starts_with(&pkg);
        assert_eq!(resource_dirs_with(&deep, trusted), [deep.clone(), pkg]);
        let all = |dir: &Path| dir.starts_with(&root);
        assert_eq!(resource_dirs_with(&deep, all).len(), 3);
    }

    #[test]
    fn commands_skills_and_setup_resolve_from_a_subdirectory() {
        let (_tmp, root, deep) = repo();
        assert!(!crate::project_initialization(&deep).initialized);
        std::fs::write(root.join("AGENTS.md"), "guidance").unwrap();
        assert!(crate::project_initialization(&deep).initialized);

        let project_skills: Vec<PathBuf> = crate::command_sources_in(&deep)
            .skills
            .into_iter()
            .filter(|d| d.scope == forge_skills::Scope::Project)
            .map(|d| d.path)
            .collect();
        assert_eq!(
            project_skills,
            [
                deep.join(".forge/skills"),
                root.join("pkg/.forge/skills"),
                root.join(".forge/skills")
            ]
        );
    }

    #[test]
    fn a_linked_worktree_pointer_file_marks_the_top() {
        let tmp = tempfile::tempdir().unwrap();
        let wt = tmp.path().canonicalize().unwrap().join("wt");
        std::fs::create_dir_all(wt.join("src")).unwrap();
        std::fs::write(wt.join(".git"), "gitdir: /elsewhere/.git/worktrees/wt\n").unwrap();
        assert_eq!(
            resource_dirs_with(&wt.join("src"), |_| false),
            [wt.join("src"), wt.clone()]
        );
    }
}
