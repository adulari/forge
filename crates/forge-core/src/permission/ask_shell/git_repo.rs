//! Whether the git repository a command would run in runs anything of its own when asked to look.
//!
//! `git status`, `git diff` and `git log` are read-only by their own account, but git runs the
//! helpers a repository names: `core.fsmonitor` on `git status`, a `textconv` or external diff
//! driver on `git diff`, `git show` and `git blame`, `core.pager`, the `post-index-change` hook
//! when a refresh rewrites the index, and a promisor remote (`extensions.partialClone`) that
//! fetches a missing object over the network. All of it comes from the repository's own files, and
//! a repository can arrive from somewhere else: an unpacked archive with a `.git` directory, or a
//! committed directory that looks like a bare repository, which git uses as the repository as soon
//! as the command's `cwd` is inside it.
//!
//! So the auto-allow reads those files first, the way git finds them: from the working directory
//! upward, the first `.git` (directory or `gitdir:` file) or bare-looking directory wins, and its
//! config, its worktree config and the config of every submodule repository under it must hold only
//! keys known to run nothing. An unknown key, section or `include` is a refusal. The user's own
//! global and system config is trusted, as is the process environment; this is about what a
//! checkout can bring with it.
//!
//! Outside a repository git is refused too, because it stops being a repository tool there:
//! `git grep` behaves as `git grep --no-index` and `git diff A B` as `git diff --no-index A B`, which
//! read plain files and whole trees, hidden ones included. The same happens inside one when a
//! `git diff` operand lies outside the work tree, which is why the caller needs [`Repo::work_tree`].

use std::path::{Path, PathBuf};

/// A config bigger than this is not read.
const MAX_CONFIG_BYTES: u64 = 256 * 1024;
/// Directories examined under `modules/` before the repository is treated as unprovable.
const MAX_MODULE_DIRS: usize = 256;
const MAX_MODULE_DEPTH: usize = 6;

/// Hooks git runs while only looking: when an index refresh writes the index, and when a ref
/// transaction commits.
const LOOKING_HOOKS: &[&str] = &["post-index-change", "reference-transaction"];

/// Config keys known to run nothing and reach nothing when git only reads, as `(section, key)`.
/// The subsection is ignored (`remote.origin.url` is `("remote", "url")`). Everything else is
/// refused, so a key git gains later is refused until someone has read what it does.
const INERT_KEYS: &[(&str, &str)] = &[
    ("core", "repositoryformatversion"),
    ("core", "filemode"),
    ("core", "bare"),
    ("core", "logallrefupdates"),
    ("core", "ignorecase"),
    ("core", "precomposeunicode"),
    ("core", "symlinks"),
    ("core", "autocrlf"),
    ("core", "safecrlf"),
    ("core", "eol"),
    ("core", "sparsecheckout"),
    ("core", "sparsecheckoutcone"),
    ("core", "untrackedcache"),
    ("core", "protectntfs"),
    ("core", "trustctime"),
    ("core", "checkstat"),
    ("core", "quotepath"),
    ("core", "ignorestat"),
    ("core", "preloadindex"),
    ("core", "splitindex"),
    ("core", "whitespace"),
    ("core", "longpaths"),
    ("remote", "url"),
    ("remote", "pushurl"),
    ("remote", "fetch"),
    ("remote", "push"),
    ("remote", "tagopt"),
    ("remote", "prune"),
    ("remote", "prunetags"),
    ("remote", "gh-resolved"),
    ("branch", "remote"),
    ("branch", "merge"),
    ("branch", "rebase"),
    ("branch", "description"),
    ("branch", "pushremote"),
    ("branch", "mergeoptions"),
    ("branch", "vscode-merge-base"),
    ("branch", "github-pr-owner-number"),
    ("user", "name"),
    ("user", "email"),
    ("user", "signingkey"),
    ("user", "useconfigonly"),
    ("init", "defaultbranch"),
    ("submodule", "url"),
    ("submodule", "active"),
    ("submodule", "branch"),
    ("submodule", "ignore"),
    ("extensions", "worktreeconfig"),
    ("extensions", "objectformat"),
    ("extensions", "refstorage"),
    ("push", "default"),
    ("push", "autosetupremote"),
    ("push", "followtags"),
    ("pull", "rebase"),
    ("pull", "ff"),
    ("fetch", "prune"),
    ("fetch", "prunetags"),
    ("merge", "ff"),
    ("rebase", "autostash"),
    ("gc", "auto"),
];

/// A repository whose own files run nothing when git only looks.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Repo {
    /// The directory holding `.git`, resolved. `None` for a bare repository, which has none.
    pub(super) work_tree: Option<PathBuf>,
}

enum Found {
    /// Not inside a repository.
    Nothing,
    Repo {
        git_dir: PathBuf,
        work_tree: Option<PathBuf>,
    },
    /// Something that looks like a repository but cannot be read the way git would read it.
    Unreadable,
}

/// The repository a read-only `git` command would use from `cwd`, if it is one that runs nothing
/// the checkout supplied. `None` is a refusal: no repository, an unreadable one, or one that
/// names a helper.
pub(super) fn inspect(cwd: &Path) -> Option<Repo> {
    let cwd = std::fs::canonicalize(cwd).ok()?;
    let Found::Repo { git_dir, work_tree } = discover(&cwd) else {
        return None;
    };
    let mut scan = Scan::default();
    let inert = git_dir_is_inert(&git_dir, &mut scan) && modules_are_inert(&git_dir);
    // Hooks are run from the work tree, or from the git directory of a bare repository.
    let hooks_base = work_tree.as_deref().unwrap_or(&git_dir);
    (inert
        && scan
            .hooks_paths
            .iter()
            .all(|path| hooks_dir_is_inert(hooks_base, path)))
    .then_some(Repo { work_tree })
}

/// What reading one repository's config files turned up, besides whether they were inert.
#[derive(Default)]
struct Scan {
    /// Set for the repository of a submodule, whose config git itself fills with a
    /// `core.worktree` pointing at the submodule's checkout.
    submodule: bool,
    /// Every `core.hooksPath` seen. It only redirects where hooks are looked for, so it is inert
    /// when the directory it names holds none of [`LOOKING_HOOKS`] (husky sets one).
    hooks_paths: Vec<String>,
}

fn hooks_dir_is_inert(base: &Path, configured: &str) -> bool {
    let dir = match configured.strip_prefix("~/") {
        Some(rest) => match std::env::var_os("HOME") {
            Some(home) => Path::new(&home).join(rest),
            None => return false,
        },
        None if configured.starts_with('~') => return false,
        None => base.join(configured),
    };
    LOOKING_HOOKS
        .iter()
        .all(|hook| std::fs::symlink_metadata(dir.join(hook)).is_err())
}

/// The repository git would use from `start`: for each directory upward, a `.git` directory or
/// `gitdir:` file, else the directory itself when it has the shape of a bare repository.
fn discover(start: &Path) -> Found {
    for dir in start.ancestors() {
        let dot_git = dir.join(".git");
        match std::fs::metadata(&dot_git) {
            Ok(meta) if meta.is_dir() => {
                if looks_like_git_dir(&dot_git) {
                    return Found::Repo {
                        git_dir: dot_git,
                        work_tree: Some(dir.to_path_buf()),
                    };
                }
            }
            Ok(meta) if meta.is_file() => {
                return match gitfile_target(&dot_git, dir) {
                    Some(target) if looks_like_git_dir(&target) => Found::Repo {
                        git_dir: target,
                        work_tree: Some(dir.to_path_buf()),
                    },
                    _ => Found::Unreadable,
                };
            }
            Ok(_) => return Found::Unreadable,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Found::Unreadable,
        }
        if looks_like_git_dir(dir) {
            return Found::Repo {
                git_dir: dir.to_path_buf(),
                work_tree: None,
            };
        }
    }
    Found::Nothing
}

/// The directory a `.git` file (`gitdir: <path>`, relative to the file's directory) points at.
fn gitfile_target(file: &Path, parent: &Path) -> Option<PathBuf> {
    let text = read_small(file)?;
    let target = text.lines().next()?.strip_prefix("gitdir:")?.trim();
    if target.is_empty() {
        return None;
    }
    std::fs::canonicalize(parent.join(target)).ok()
}

/// A repository has a `HEAD` file and, in its common directory, `objects/` and `refs/`.
fn looks_like_git_dir(dir: &Path) -> bool {
    let common = common_dir(dir);
    dir.join("HEAD").is_file() && common.join("objects").is_dir() && common.join("refs").is_dir()
}

/// A linked worktree's git directory names the main repository's in `commondir`.
fn common_dir(git_dir: &Path) -> PathBuf {
    read_small(&git_dir.join("commondir"))
        .and_then(|text| {
            let line = text.lines().next()?.trim();
            (!line.is_empty())
                .then(|| std::fs::canonicalize(git_dir.join(line)).ok())
                .flatten()
        })
        .unwrap_or_else(|| git_dir.to_path_buf())
}

fn git_dir_is_inert(git_dir: &Path, scan: &mut Scan) -> bool {
    let common = common_dir(git_dir);
    let mut homes = vec![git_dir.to_path_buf()];
    if common != git_dir {
        homes.push(common);
    }
    homes.iter().all(|home| {
        ["config", "config.worktree"]
            .iter()
            .all(|name| config_is_inert(&home.join(name), scan))
            && LOOKING_HOOKS
                .iter()
                .all(|hook| std::fs::symlink_metadata(home.join("hooks").join(hook)).is_err())
    })
}

/// `git status` also runs in each submodule, with that submodule's own config.
fn modules_are_inert(git_dir: &Path) -> bool {
    let modules = common_dir(git_dir).join("modules");
    if !modules.exists() {
        return true;
    }
    let mut pending = vec![(modules, 0usize)];
    let mut examined = 0usize;
    while let Some((dir, depth)) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return false;
        };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else {
                return false;
            };
            if !kind.is_dir() {
                continue;
            }
            examined += 1;
            if examined > MAX_MODULE_DIRS || depth > MAX_MODULE_DEPTH {
                return false;
            }
            let path = entry.path();
            if looks_like_git_dir(&path)
                && !git_dir_is_inert(
                    &path,
                    &mut Scan {
                        submodule: true,
                        ..Scan::default()
                    },
                )
            {
                return false;
            }
            pending.push((path, depth + 1));
        }
    }
    true
}

/// A missing file is an empty config; one that cannot be read, or is not text, is not inert.
fn config_is_inert(path: &Path, scan: &mut Scan) -> bool {
    match std::fs::symlink_metadata(path) {
        Err(err) => return err.kind() == std::io::ErrorKind::NotFound,
        Ok(meta) if !meta.is_file() || meta.len() > MAX_CONFIG_BYTES => return false,
        Ok(_) => {}
    }
    read_small(path).is_some_and(|text| config_text_is_inert(&text, scan))
}

fn read_small(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_CONFIG_BYTES {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

/// Every key of a git config file is on [`INERT_KEYS`]. A line continued with a trailing `\` is
/// refused (the value under it is not parsed), and so is anything that is not a plain section
/// header or `key = value`.
fn config_text_is_inert(text: &str, scan: &mut Scan) -> bool {
    let mut section: Option<String> = None;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with(['#', ';']) {
            continue;
        }
        if line.ends_with('\\') {
            return false;
        }
        let mut entry = line;
        if line.starts_with('[') {
            let Some((name, after)) = parse_header(line) else {
                return false;
            };
            section = Some(name);
            entry = after.trim();
            if entry.is_empty() || entry.starts_with(['#', ';']) {
                continue;
            }
        }
        let Some(section) = &section else {
            return false;
        };
        let (key, value) = match entry.split_once('=') {
            Some((key, value)) => (key.trim(), Some(value.trim())),
            None => (entry, None),
        };
        let key = key.to_ascii_lowercase();
        if !key_is_inert(section, &key, value, scan.submodule) {
            return false;
        }
        if (section.as_str(), key.as_str()) == ("core", "hookspath") {
            match value {
                Some(path) => scan.hooks_paths.push(path.to_string()),
                None => return false,
            }
        }
    }
    true
}

fn key_is_inert(section: &str, key: &str, value: Option<&str>, submodule: bool) -> bool {
    match (section, key) {
        // A bare `true` is git's built-in fsmonitor daemon; anything else names a hook program.
        ("core", "fsmonitor") => {
            value.is_none_or(|v| v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("false"))
        }
        // Moving the work tree makes `git diff` read whatever the index names from elsewhere.
        ("core", "worktree") => submodule,
        // Judged by the directory it names, once the whole config has been read.
        ("core", "hookspath") => {
            !submodule && value.is_some_and(|v| !v.is_empty() && !v.contains(['"', '\\', '#', ';']))
        }
        _ => INERT_KEYS.contains(&(section, key)),
    }
}

/// `[section]`, `[section "subsection"]` or the old `[section.subsection]`, returning the
/// lower-cased section name and whatever follows the closing bracket.
fn parse_header(line: &str) -> Option<(String, &str)> {
    let inner = line.strip_prefix('[')?;
    let end = inner.find(|c: char| c.is_whitespace() || matches!(c, '"' | ']' | '.'))?;
    let name = inner[..end].to_ascii_lowercase();
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return None;
    }
    let tail = &inner[end..];
    if let Some(after) = tail.strip_prefix(']') {
        return Some((name, after));
    }
    if tail.starts_with('.') {
        let close = tail.find(']')?;
        return (!tail[..close].contains('"')).then(|| (name, &tail[close + 1..]));
    }
    let quoted = tail.trim_start().strip_prefix('"')?;
    let mut escaped = false;
    for (at, c) in quoted.char_indices() {
        match c {
            _ if escaped => escaped = false,
            '\\' => escaped = true,
            '"' => return Some((name, quoted[at + 1..].strip_prefix(']')?)),
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests;
