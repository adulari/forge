//! Whether the git repository a command would run in runs anything of its own when asked to look.
//!
//! `git status`, `git diff` and `git log` are read-only by their own account, but git runs the
//! helpers a repository names: `core.fsmonitor` on `git status`, a `textconv` or external diff
//! driver on `git diff`, `git show` and `git blame`, `core.pager`, the `post-index-change` hook
//! when a refresh rewrites the index, and a promisor remote (`extensions.partialClone`) that
//! fetches a missing object over the network. All of it comes from the repository's own files, and
//! a repository can arrive from somewhere else: an unpacked archive with a `.git` directory, a
//! committed directory that looks like a bare repository (git uses it as the repository as soon as
//! the command's `cwd` is inside it), or a nested repository behind a gitlink, whose own config
//! `git status` runs.
//!
//! Finding that repository and reading its config is git's own job, with rules (which directory
//! counts, which `HEAD` is valid, where whitespace is trimmed, where discovery stops at a mount
//! point, which index entries are submodules) a second implementation here got wrong six ways. So
//! this asks git, with the plumbing that only reads: `rev-parse` for the work tree and the hook
//! paths git would use, `config --list --show-scope` for the keys of the repository's own config
//! as git parsed them, and `ls-files --stage` for the gitlinks, each of which is inspected the same
//! way. The repository's config (local and worktree scope) must hold only keys known to run
//! nothing; an unknown key, section or `include` is a refusal. Nothing here runs before the config
//! it would read is known to be inert, and none of these commands runs a hook, a helper or a pager.
//!
//! The user's own global and system config is trusted, as is the process environment; this is
//! about what a checkout can bring with it. Anything git cannot answer in time (no `git`, a refusal
//! to open the repository, a dubious owner, too much output) is a refusal.
//!
//! Outside a repository git is refused too, because it stops being a repository tool there:
//! `git diff A B` is `git diff --no-index A B`, which reads plain files and whole trees, hidden ones
//! included. The same happens inside one when a `git diff` operand lies outside the work tree,
//! which is why the caller needs [`Repo::work_tree`].

use std::collections::HashSet;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Everything the probes may take, summed over all of them: a repository with a huge index, or a
/// git that hangs on a FIFO named by an include, must not stall the broker.
const BUDGET: Duration = Duration::from_secs(5);
/// Most output read from a probe that lists config or paths.
const MAX_LISTING_BYTES: u64 = 1 << 20;
/// Most index listing read while looking for gitlinks.
const MAX_INDEX_BYTES: u64 = 64 << 20;
/// Repositories examined for one call (the one asked about and the nested ones under it).
const MAX_REPOS: usize = 64;
const MAX_DEPTH: usize = 5;

/// Hooks git runs while only looking: when an index refresh writes the index, and when a ref
/// transaction commits.
const LOOKING_HOOKS: &[&str] = &["post-index-change", "reference-transaction"];

/// Variables that point git at one repository. A repository nested under another is found by the
/// directory it sits in, which is how `git status` runs inside a submodule.
const LOCAL_REPO_ENV: &[&str] = &[
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_COMMON_DIR",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_PREFIX",
    "GIT_IMPLICIT_WORK_TREE",
];

/// Config keys known to run nothing and reach nothing when git only reads, as `(section, key)`.
/// The subsection is ignored (`remote.origin.url` is `("remote", "url")`). Everything else is
/// refused, so a key git gains later is refused until someone has read what it does.
/// `core.hooksPath` is among them because what matters is where hooks are looked for, which is
/// asked of git rather than read from the value (husky sets one).
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
    ("core", "hookspath"),
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
    /// The top of the work tree, resolved. `None` for a bare repository, or when the working
    /// directory is inside the git directory, which have none.
    pub(super) work_tree: Option<PathBuf>,
}

/// The repository a read-only `git` command would use from `cwd`, if it is one that runs nothing
/// the checkout supplied. `None` is a refusal: no repository, one git will not open, one that
/// names a helper, or one that cannot be judged in time.
pub(super) fn inspect(cwd: &Path) -> Option<Repo> {
    let cwd = std::fs::canonicalize(cwd).ok()?;
    let mut probe = Probe {
        git: locate_git(&cwd)?,
        deadline: Instant::now() + BUDGET,
        seen: HashSet::new(),
    };
    Some(Repo {
        work_tree: probe.inspect_repo(&cwd, 0)?,
    })
}

/// The `git` the shell would run, as an absolute path. A relative or empty `PATH` entry means a
/// checkout can supply the binary, so one that holds a `git` makes the answer "none".
fn locate_git(cwd: &Path) -> Option<PathBuf> {
    locate_git_in(&std::env::var_os("PATH")?, cwd)
}

fn locate_git_in(path: &std::ffi::OsStr, cwd: &Path) -> Option<PathBuf> {
    let name = if cfg!(windows) { "git.exe" } else { "git" };
    for dir in std::env::split_paths(path) {
        if dir.is_absolute() {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        } else if cwd.join(&dir).join(name).symlink_metadata().is_ok() {
            return None;
        }
    }
    None
}

struct Probe {
    git: PathBuf,
    deadline: Instant,
    /// The git directories examined so far. Where a nested directory has no repository of its own,
    /// git finds the one above it again; and the same submodule can be named twice.
    seen: HashSet<PathBuf>,
}

impl Probe {
    /// The work tree of the repository `git` finds from `dir` when that repository, and every
    /// repository nested under it, runs nothing. `None` is a refusal; `Some(None)` is a repository
    /// without a work tree, or one already judged.
    fn inspect_repo(&mut self, dir: &Path, depth: usize) -> Option<Option<PathBuf>> {
        if depth > MAX_DEPTH {
            return None;
        }
        let nested = depth > 0;

        // Whether this is a work tree, which repository it is, and where git looks for the hooks
        // it runs while looking.
        let mut args = vec![
            "rev-parse",
            "--is-inside-work-tree",
            "--path-format=absolute",
            "--git-dir",
        ];
        let hook_paths: Vec<String> = LOOKING_HOOKS
            .iter()
            .map(|hook| format!("hooks/{hook}"))
            .collect();
        for path in &hook_paths {
            args.extend(["--git-path", path.as_str()]);
        }
        let facts = self.output(dir, nested, &args)?;
        let mut lines = facts.strip_suffix('\n')?.split('\n');
        let inside = match lines.next()? {
            "true" => true,
            "false" => false,
            _ => return None,
        };
        let git_dir = std::fs::canonicalize(lines.next()?).ok()?;
        if !self.seen.insert(git_dir) {
            return Some(None);
        }
        if self.seen.len() > MAX_REPOS {
            return None;
        }
        for _ in LOOKING_HOOKS {
            let hook = lines.next()?;
            match std::fs::symlink_metadata(hook) {
                Err(err)
                    if matches!(
                        err.kind(),
                        std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                    ) => {}
                _ => return None,
            }
        }
        if lines.next().is_some() {
            return None;
        }

        // `--no-includes`: an include is a refusal anyway, and is not read to find that out.
        let config = self.output(
            dir,
            nested,
            &["config", "--list", "--no-includes", "-z", "--show-scope"],
        )?;
        if !config_is_inert(&config, nested) {
            return None;
        }

        if !inside {
            return Some(None);
        }
        let top = self.output(dir, nested, &["rev-parse", "--show-toplevel"])?;
        let top = std::fs::canonicalize(top.strip_suffix('\n')?).ok()?;
        for link in self.gitlinks(&top, nested)? {
            // Where the directory has no repository of its own, git finds the one above again,
            // which `seen` knows. Where it has, that one is judged whatever its work tree is: a
            // `core.worktree` of its own moves the work tree but not where `git status` runs.
            self.inspect_repo(&top.join(link), depth + 1)?;
        }
        Some(Some(top))
    }

    /// Paths of the gitlinks (submodule entries) in the index of the repository whose work tree is
    /// `top` that have a `.git` to run in; one that is not checked out gives `git status` no
    /// repository. Run from the top, because `ls-files` lists only what is under its working
    /// directory.
    fn gitlinks(&self, top: &Path, nested: bool) -> Option<Vec<String>> {
        let checkout = top.to_path_buf();
        self.run(
            top,
            nested,
            // The config was read and holds no `core.fsmonitor`; this makes sure of it for the one
            // probe that reads the index.
            &["-c", "core.fsmonitor=false", "ls-files", "--stage", "-z"],
            move |stdout| {
                let mut reader = BufReader::new(stdout.take(MAX_INDEX_BYTES + 1));
                let mut entry = Vec::new();
                let mut read = 0u64;
                let mut links: Vec<String> = Vec::new();
                loop {
                    entry.clear();
                    let n = reader.read_until(0, &mut entry).ok()?;
                    if n == 0 {
                        return Some(links);
                    }
                    read += n as u64;
                    if read > MAX_INDEX_BYTES {
                        return None;
                    }
                    // `<mode> <object> <stage>\t<path>`
                    if !entry.starts_with(b"160000 ") {
                        continue;
                    }
                    let path = entry.splitn(2, |byte| *byte == b'\t').nth(1)?;
                    let path =
                        String::from_utf8(path.strip_suffix(&[0]).unwrap_or(path).to_vec()).ok()?;
                    let checked_out = checkout.join(&path).join(".git").symlink_metadata().is_ok();
                    if checked_out && links.last() != Some(&path) {
                        links.push(path);
                    }
                    if links.len() > MAX_REPOS {
                        return None;
                    }
                }
            },
        )
    }

    /// The text a probe prints, if it succeeds in time with a bounded amount of valid UTF-8.
    fn output(&self, dir: &Path, nested: bool, args: &[&str]) -> Option<String> {
        self.run(dir, nested, args, |stdout| {
            let mut text = String::new();
            let read = stdout
                .take(MAX_LISTING_BYTES + 1)
                .read_to_string(&mut text)
                .ok()?;
            (read as u64 <= MAX_LISTING_BYTES).then_some(text)
        })
    }

    /// Run `git <args>` in `dir` and hand its standard output to `read`, which must read to the
    /// end for the result to count. A non-zero exit, a spawn failure and the end of the budget all
    /// give `None`, and the process is killed.
    fn run<T: Send + 'static>(
        &self,
        dir: &Path,
        nested: bool,
        args: &[&str],
        read: impl FnOnce(ChildStdout) -> Option<T> + Send + 'static,
    ) -> Option<T> {
        let left = self.deadline.checked_duration_since(Instant::now())?;
        let mut command = Command::new(&self.git);
        command
            .arg("--no-pager")
            .args(args)
            .current_dir(dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("LC_ALL", "C");
        // Tests must not see the developer's own repository variables or global config (a commit
        // hook runs with `GIT_DIR` set); production keeps them, because the command being judged
        // runs with them.
        if nested || cfg!(test) {
            for var in LOCAL_REPO_ENV {
                command.env_remove(var);
            }
        }
        if cfg!(test) {
            command
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1");
        }
        let mut child = command.spawn().ok()?;
        let stdout = child.stdout.take()?;
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(read(stdout));
        });
        let value = receiver.recv_timeout(left).ok().flatten();
        let clean = value.is_some() && exited_cleanly(&mut child, self.deadline);
        if !clean {
            let _ = child.kill();
        }
        let _ = child.wait();
        value.filter(|_| clean)
    }
}

fn exited_cleanly(child: &mut Child, deadline: Instant) -> bool {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(2));
            }
            _ => return false,
        }
    }
}

/// Every key the repository itself supplies is on [`INERT_KEYS`], in `git config --list -z
/// --show-scope` form: `scope NUL key [LF value] NUL` per entry, keys with the section and
/// variable name lower-cased by git. System, global and command-line entries are the user's own.
/// A scope this does not know (an older git prints none at all) is a refusal.
fn config_is_inert(listing: &str, nested: bool) -> bool {
    if listing.is_empty() {
        return true;
    }
    let Some(body) = listing.strip_suffix('\0') else {
        return false;
    };
    let fields: Vec<&str> = body.split('\0').collect();
    fields.len().is_multiple_of(2)
        && fields.chunks(2).all(|pair| match pair[0] {
            "system" | "global" | "command" => true,
            "local" | "worktree" => {
                let (key, value) = match pair[1].split_once('\n') {
                    Some((key, value)) => (key, Some(value)),
                    None => (pair[1], None),
                };
                key_is_inert(key, value, nested)
            }
            _ => false,
        })
}

fn key_is_inert(key: &str, value: Option<&str>, nested: bool) -> bool {
    let Some((section, _)) = key.split_once('.') else {
        return false;
    };
    let name = key.rsplit('.').next().unwrap_or(key);
    let (section, name) = (section.to_ascii_lowercase(), name.to_ascii_lowercase());
    match (section.as_str(), name.as_str()) {
        // Anything but an explicit `false` starts a program: a command, or git's own fsmonitor
        // daemon, which outlives the call.
        ("core", "fsmonitor") => value.is_some_and(|v| v.eq_ignore_ascii_case("false")),
        // Moving the work tree makes `git diff` read whatever the index names from elsewhere. Git
        // itself writes it into the config of a submodule's repository, which is judged only as a
        // nested one: `git status` there prints names, never contents.
        ("core", "worktree") => nested,
        pair => INERT_KEYS.contains(&pair),
    }
}

#[cfg(test)]
mod tests;
