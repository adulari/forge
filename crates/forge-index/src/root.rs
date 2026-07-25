//! Index-root policy: which directories the Lattice is allowed to take as a repo root, and how
//! large an index it may build without being asked.
//!
//! Motivation. The walker used to index whatever root it was handed. `forge run`/`forge chat`
//! launched from `$HOME` therefore background-indexed the entire home directory: on one real
//! install that produced a single root holding 80,117 files / 1.44M symbols / 8.4M references —
//! roughly 2.2 GB of SQLite, 73,000 of those files being the Go module cache and the Android SDK.
//! The functional damage is worse than the disk: `lattice_ref` lookups are scoped by `repo_root`,
//! so a bogus root does not slow other projects down, but any turn *running inside that root* gets
//! retrieval dominated by third-party toolchain symbols. It is also a privacy problem — symbols
//! from every unrelated private project under `$HOME` end up in Forge's database.
//!
//! The guard lives here, and is enforced inside [`crate::Lattice::update`] rather than at a call
//! site, because there are three independent entry points that index (`forge lattice update`, the
//! session's background auto-index, and `Session::transition_workspace`) and a guard on one of them
//! is trivially bypassed by the others. The watcher already refused `$HOME`
//! ([`crate::resolve_watch_root`]) while the indexer did not — that exact asymmetry is what let the
//! home directory in.

use std::path::Path;

use forge_store::Store;

use crate::LatticeError;

/// Ceiling on indexable source files for a root that carries a project marker (see
/// [`has_project_marker`]). Counted after the ignore rules, so it is a count of files the index
/// would actually parse.
///
/// 25,000 is chosen to sit above every ordinary single-project repository and below every
/// aggregate tree. For calibration: Forge itself tracks ~1,000 files; CPython ~3,000; the Rust
/// compiler ~15,000; the Linux kernel ~55,000. A repo genuinely larger than this is rare enough
/// that asking once (`--force`) is better than silently writing a multi-hundred-MB index — at the
/// ~27 KB of SQLite per indexed file this database averages, 25,000 files is already ~650 MB.
pub const MAX_FILES_MARKED_ROOT: usize = 25_000;

/// Ceiling for a root with **no** project marker. A marker is the only evidence we have that the
/// user meant this directory as a codebase; without one, allow an index only while it is small
/// enough to obviously be a single ad-hoc folder of source. Two orders of magnitude below the
/// failure mode (80,117 files), and far above any hand-made directory.
pub const MAX_FILES_UNMARKED_ROOT: usize = 2_000;

/// Files whose presence in a directory marks it as a project the user chose, rather than a
/// container that merely happens to sit above some code. VCS metadata first, then the common
/// single-language manifests.
const PROJECT_MARKERS: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    ".jj",
    ".forge",
    "AGENTS.md",
    "FORGE.md",
    "Cargo.toml",
    "go.mod",
    "package.json",
    "pyproject.toml",
    "setup.py",
    "pom.xml",
    "build.gradle",
    "build.gradle.kts",
    "Gemfile",
    "composer.json",
    "CMakeLists.txt",
    "mix.exs",
];

/// Why a directory was refused as an index root.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RootRefusal {
    /// The root is the user's home directory, an ancestor of it, or a filesystem root. Never a
    /// project; refused unconditionally (no `--force`), matching what the watcher has always done.
    #[error(
        "refusing to index {root} — that is the home/system root, not a project. \
         Run Forge from a project directory instead."
    )]
    HomeOrSystemRoot { root: String },
    /// The root holds more indexable source files than the policy allows unattended.
    #[error(
        "refusing to index {root} — {found}+ indexable source files exceeds the {limit} limit for \
         a root {marker_note}. Index a subdirectory, or re-run with --force to index it anyway."
    )]
    TooManyFiles {
        root: String,
        found: usize,
        limit: usize,
        marker_note: &'static str,
    },
}

impl RootRefusal {
    /// Build the over-size refusal for `root`, phrasing the limit in terms of whether the root
    /// carries a project marker.
    pub(crate) fn too_many(root: &str, found: usize, limit: usize, marked: bool) -> Self {
        RootRefusal::TooManyFiles {
            root: root.to_string(),
            found,
            limit,
            marker_note: if marked {
                "with a project marker"
            } else {
                "with no project marker (.git, Cargo.toml, package.json, …)"
            },
        }
    }
}

/// Whether `root` is the home directory itself, an ancestor of it (`/home`, `/`, `C:\Users`), or a
/// filesystem root. `home` is the user's home directory when known; when it is `None` only the
/// filesystem-root check applies (fail open rather than guess).
///
/// Comparison is on the paths as given plus their canonical forms, so `/home/floris/` and a
/// symlinked home both match.
pub fn is_home_or_system_root(root: &Path, home: Option<&Path>) -> bool {
    if root.parent().is_none() {
        return true; // `/`, or a bare Windows prefix like `C:\`
    }
    let Some(home) = home else { return false };
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let (root_c, home_c) = (canon(root), canon(home));
    // The home directory itself, or anything above it (`/home`, `/Users`, `/`).
    root_c == home_c || home_c.starts_with(&root_c)
}

/// Whether `root` looks like a project the user deliberately opened — see [`PROJECT_MARKERS`].
pub fn has_project_marker(root: &Path) -> bool {
    PROJECT_MARKERS.iter().any(|m| root.join(m).exists())
}

/// The indexable-file ceiling that applies to `root`.
pub fn file_limit_for(root: &Path) -> usize {
    if has_project_marker(root) {
        MAX_FILES_MARKED_ROOT
    } else {
        MAX_FILES_UNMARKED_ROOT
    }
}

/// Third-party toolchain and SDK trees, matched on trailing path components rather than a bare
/// directory name so a project's own `pkg/` or `sdk/` source directory is never caught.
///
/// Deliberately narrow: every entry here is a location whose contents are *installed artifacts*,
/// never hand-written source in the user's repository. `~/go/pkg/mod` (the Go module cache) and
/// `~/Android/Sdk` alone accounted for 73,086 of the 80,117 files in the runaway home index.
const TOOLCHAIN_DIR_SUFFIXES: &[&[&str]] = &[
    // Go module cache + checksum DB (`$GOPATH/pkg/mod`, default `~/go`).
    &["go", "pkg", "mod"],
    &["go", "pkg", "sumdb"],
    // Android SDK, in its Linux/Windows and macOS default locations.
    &["Android", "Sdk"],
    &["Android", "sdk"],
    &["Library", "Android", "sdk"],
    // Installed Python packages (a `venv` without the leading dot isn't caught by the dotdir rule).
    &["site-packages"],
    &["dist-packages"],
];

/// Whether `path` is a toolchain/SDK directory that is never worth indexing.
pub fn is_toolchain_dir(path: &Path) -> bool {
    let components: Vec<&str> = path
        .components()
        .filter_map(|c| c.as_os_str().to_str())
        .collect();
    TOOLCHAIN_DIR_SUFFIXES.iter().any(|suffix| {
        components.len() >= suffix.len()
            && components[components.len() - suffix.len()..] == **suffix
    })
}

/// The identity half of the policy: refuse `root` outright if it is the home/system root. The
/// magnitude half needs a walk and lives in [`crate::Lattice::update`].
pub fn classify_root(root: &Path, home: Option<&Path>) -> Option<RootRefusal> {
    is_home_or_system_root(root, home).then(|| RootRefusal::HomeOrSystemRoot {
        root: root.display().to_string(),
    })
}

/// Every `repo_root` that currently has rows in the index.
pub fn indexed_roots(store: &Store) -> Result<Vec<String>, LatticeError> {
    Ok(store.lattice_repo_roots()?)
}

/// Indexed roots whose directory no longer exists on disk — safe to drop unattended.
pub fn stale_roots(store: &Store) -> Result<Vec<String>, LatticeError> {
    Ok(indexed_roots(store)?
        .into_iter()
        .filter(|r| !Path::new(r).is_dir())
        .collect())
}

/// Delete every row belonging to `root`: its files, and — through `lattice_file`'s
/// `ON DELETE CASCADE` — their nodes, edges and references. Returns the number of files removed.
///
/// The cascade is load-bearing, so it is worth stating why it can be trusted: SQLite enforces
/// foreign keys per *connection* and defaults them OFF, but `forge-store` sets
/// `PRAGMA foreign_keys = ON` in its connection manager for every connection it hands out, and
/// `prune_removes_nodes_refs_and_edges_for_the_root` asserts the child rows actually disappear.
pub fn prune_root(store: &Store, root: &str) -> Result<usize, LatticeError> {
    Ok(store.prune_lattice_repo(root)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static N: AtomicUsize = AtomicUsize::new(0);

    fn tmp(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "forge-root-policy-{}-{name}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn refuses_home_itself_and_anything_above_it() {
        let home = tmp("home");
        assert!(is_home_or_system_root(&home, Some(&home)), "home itself");
        assert!(
            is_home_or_system_root(home.parent().unwrap(), Some(&home)),
            "an ancestor of home"
        );
        assert!(
            is_home_or_system_root(Path::new("/"), Some(&home)),
            "filesystem root"
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn allows_a_project_under_home() {
        let home = tmp("home-ok");
        let project = home.join("code/app");
        std::fs::create_dir_all(&project).unwrap();
        assert!(!is_home_or_system_root(&project, Some(&home)));
        assert!(classify_root(&project, Some(&home)).is_none());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn home_check_is_inert_when_home_is_unknown() {
        let dir = tmp("nohome");
        assert!(
            !is_home_or_system_root(&dir, None),
            "unknown home must fail open, not refuse everything"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn marker_presence_selects_the_file_limit() {
        let dir = tmp("marker");
        assert!(!has_project_marker(&dir));
        assert_eq!(file_limit_for(&dir), MAX_FILES_UNMARKED_ROOT);
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        assert!(has_project_marker(&dir));
        assert_eq!(file_limit_for(&dir), MAX_FILES_MARKED_ROOT);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn toolchain_dirs_match_on_path_not_bare_name() {
        assert!(is_toolchain_dir(Path::new("/home/u/go/pkg/mod")));
        assert!(is_toolchain_dir(Path::new("/home/u/go/pkg/sumdb")));
        assert!(is_toolchain_dir(Path::new("/home/u/Android/Sdk")));
        assert!(is_toolchain_dir(Path::new("/home/u/Library/Android/sdk")));
        assert!(is_toolchain_dir(Path::new(
            "/proj/venv/lib/python3.13/site-packages"
        )));
        // A project's own directories must survive: these are real source locations.
        assert!(!is_toolchain_dir(Path::new("/proj/pkg")));
        assert!(!is_toolchain_dir(Path::new("/proj/pkg/mod")));
        assert!(!is_toolchain_dir(Path::new("/proj/go")));
        assert!(!is_toolchain_dir(Path::new("/proj/sdk")));
        assert!(!is_toolchain_dir(Path::new("/proj/android/app/src")));
    }
}
