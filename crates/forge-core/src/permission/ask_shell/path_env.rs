//! Which program a name in `PATH` becomes, for the programs a read-only call starts.
//!
//! The Ask auto-allow judges `git status` or `ls` by the arguments written, on the understanding
//! that `git` and `ls` are the system's. They are not when `PATH` names a directory the checkout
//! controls: a relative entry (`bin`, `.`, or the empty entry a trailing colon makes) is looked up
//! in the directory the command runs in, and an absolute one can lie inside the workspace (a
//! virtualenv, a direnv `PATH_add`, an absolute `node_modules/.bin`). A repository that ships a
//! `bin/ls`, `bin/git` or `bin/sh` then runs it with a line this file's caller called read-only.
//!
//! The same lookup tells the git probe which `git` to run, so it never runs one the checkout
//! brought. The process environment is otherwise trusted, as everywhere in the broker.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// Where a name in `PATH` leads.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Found {
    /// No entry of `PATH` holds it, so the call fails and runs nothing.
    Missing,
    /// The first entry that holds it is one the checkout controls.
    Planted,
    /// A file outside every directory the checkout controls.
    System(PathBuf),
}

pub(super) struct SearchPath {
    entries: Vec<PathBuf>,
    /// Where a relative entry is looked up: the directory the command runs in, resolved.
    cwd: PathBuf,
    /// Directories whose contents the checkout controls: the workspace and the working directory.
    checkout: Vec<PathBuf>,
}

impl SearchPath {
    /// `workspace` is already resolved. A directory that holds the home directory is the user's
    /// own tree rather than a checkout (a session started in `~` finds `~/.cargo/bin` below its
    /// workspace), so it does not count as one.
    pub(super) fn new(
        path: &OsStr,
        cwd: &Path,
        workspace: &Path,
        home: Option<&Path>,
    ) -> SearchPath {
        let cwd = std::fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf());
        let mut checkout = vec![workspace.to_path_buf(), cwd.clone()];
        checkout.retain(|dir| !home.is_some_and(|home| home.starts_with(dir)));
        SearchPath {
            entries: std::env::split_paths(path).collect(),
            cwd,
            checkout,
        }
    }

    /// What `name` resolves to the way `execvp` resolves it: the first entry that holds it wins.
    /// A relative or empty entry that holds anything of that name counts as planting, whatever it
    /// is, because the checkout's contents are not ours to vet.
    pub(super) fn find(&self, name: &str) -> Found {
        for entry in &self.entries {
            if !entry.is_absolute() {
                if self.cwd.join(entry).join(name).symlink_metadata().is_ok() {
                    return Found::Planted;
                }
                continue;
            }
            let candidate = entry.join(name);
            if !candidate.is_file() {
                continue;
            }
            let real = std::fs::canonicalize(&candidate).unwrap_or_else(|_| candidate.clone());
            return if self.checkout.iter().any(|dir| real.starts_with(dir)) {
                Found::Planted
            } else {
                Found::System(candidate)
            };
        }
        Found::Missing
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::symlink;

    use tempfile::TempDir;

    use super::*;

    struct Tree {
        _dir: TempDir,
        root: PathBuf,
        workspace: PathBuf,
        system: PathBuf,
    }

    /// A workspace and a system directory (`git`, `ls`, `sh`) side by side.
    fn tree() -> Tree {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let workspace = root.join("ws");
        let system = root.join("usr-bin");
        for name in ["git", "ls", "sh"] {
            std::fs::create_dir_all(&system).unwrap();
            std::fs::write(system.join(name), "").unwrap();
        }
        std::fs::create_dir_all(workspace.join("bin")).unwrap();
        Tree {
            _dir: dir,
            root,
            workspace,
            system,
        }
    }

    fn path(dirs: &[&Path]) -> std::ffi::OsString {
        std::env::join_paths(dirs).unwrap()
    }

    fn find(t: &Tree, dirs: &[&Path], name: &str) -> Found {
        SearchPath::new(&path(dirs), &t.workspace, &t.workspace, None).find(name)
    }

    #[test]
    fn a_system_directory_is_the_systems() {
        let t = tree();
        assert_eq!(
            find(&t, &[&t.system], "git"),
            Found::System(t.system.join("git"))
        );
        assert_eq!(find(&t, &[&t.system], "nope"), Found::Missing);
        assert_eq!(find(&t, &[], "git"), Found::Missing);
    }

    #[test]
    fn a_relative_or_empty_entry_that_holds_the_name_is_planted_for_every_program() {
        let t = tree();
        for name in ["git", "ls", "sh"] {
            std::fs::write(t.workspace.join("bin").join(name), "").unwrap();
            for entry in ["bin", "./bin", ""] {
                let cwd = if entry.is_empty() {
                    t.workspace.join("bin")
                } else {
                    t.workspace.clone()
                };
                let search = SearchPath::new(
                    &path(&[Path::new(entry), &t.system]),
                    &cwd,
                    &t.workspace,
                    None,
                );
                assert_eq!(search.find(name), Found::Planted, "{name} via {entry:?}");
            }
        }
        // `.` resolves against where the command runs, so it is the planted `ls` itself.
        std::fs::write(t.workspace.join("ls"), "").unwrap();
        assert_eq!(
            SearchPath::new(
                &path(&[Path::new("."), &t.system]),
                &t.workspace,
                &t.workspace,
                None
            )
            .find("ls"),
            Found::Planted
        );
    }

    #[test]
    fn a_relative_entry_that_holds_nothing_of_that_name_is_skipped() {
        let t = tree();
        std::fs::write(t.workspace.join("bin/other"), "").unwrap();
        assert_eq!(
            find(&t, &[Path::new("bin"), &t.system], "ls"),
            Found::System(t.system.join("ls"))
        );
        assert_eq!(
            find(&t, &[Path::new("missing-dir"), &t.system], "ls"),
            Found::System(t.system.join("ls"))
        );
    }

    #[test]
    fn an_absolute_entry_inside_the_workspace_or_the_working_directory_is_planted() {
        let t = tree();
        let venv = t.workspace.join("bin");
        std::fs::write(venv.join("git"), "").unwrap();
        assert_eq!(find(&t, &[&venv, &t.system], "git"), Found::Planted);
        // Only the names it holds: the system `ls` behind it is still found.
        assert_eq!(
            find(&t, &[&venv, &t.system], "ls"),
            Found::System(t.system.join("ls"))
        );
        // The working directory counts when it is outside the workspace.
        let elsewhere = t.root.join("elsewhere");
        std::fs::create_dir_all(elsewhere.join("tools")).unwrap();
        std::fs::write(elsewhere.join("tools/ls"), "").unwrap();
        let search = SearchPath::new(
            &path(&[&elsewhere.join("tools"), &t.system]),
            &elsewhere,
            &t.workspace,
            None,
        );
        assert_eq!(search.find("ls"), Found::Planted);
    }

    #[test]
    fn a_link_into_the_checkout_is_planted_and_a_link_out_of_it_is_not() {
        let t = tree();
        let tools = t.root.join("tools");
        std::fs::create_dir_all(&tools).unwrap();
        std::fs::write(t.workspace.join("bin/git"), "").unwrap();
        symlink(t.workspace.join("bin/git"), tools.join("git")).unwrap();
        assert_eq!(find(&t, &[&tools, &t.system], "git"), Found::Planted);
        // A virtualenv's `ls -> /usr/bin/ls` leads to the system's program.
        symlink(t.system.join("ls"), t.workspace.join("bin/ls")).unwrap();
        assert!(matches!(
            find(&t, &[&t.workspace.join("bin"), &t.system], "ls"),
            Found::System(_)
        ));
        // A directory entry that is itself a link into the workspace.
        symlink(t.workspace.join("bin"), t.root.join("bin-link")).unwrap();
        assert_eq!(
            find(&t, &[&t.root.join("bin-link"), &t.system], "git"),
            Found::Planted
        );
    }

    #[test]
    fn a_tree_that_holds_the_home_directory_is_not_a_checkout() {
        let t = tree();
        let home = t.workspace.join("home");
        let cargo = home.join(".cargo/bin");
        std::fs::create_dir_all(&cargo).unwrap();
        std::fs::write(cargo.join("rtk"), "").unwrap();
        let search = |workspace: &Path, home: Option<&Path>| {
            SearchPath::new(&path(&[&cargo, &t.system]), workspace, workspace, home)
        };
        assert_eq!(search(&t.workspace, None).find("rtk"), Found::Planted);
        assert_eq!(
            search(&t.workspace, Some(&home)).find("rtk"),
            Found::System(cargo.join("rtk")),
            "a session started in the home directory"
        );
        // A project below the home directory is still a checkout.
        let project = home.join("project");
        std::fs::create_dir_all(project.join("bin")).unwrap();
        std::fs::write(project.join("bin/ls"), "").unwrap();
        let search = SearchPath::new(
            &path(&[&project.join("bin"), &t.system]),
            &project,
            &project,
            Some(&home),
        );
        assert_eq!(search.find("ls"), Found::Planted);
    }
}
