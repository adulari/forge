//! The path guard behind Ask's read-only auto-allow: which files a command's arguments name, once
//! `~`, the working directory and wildcards are applied, and whether any of them is a secret.
//!
//! The built-in deny rules match secret READS by command verb (`cat *.env`). A read-only command
//! the verbs do not list (`wc -c .env`, `git show HEAD:.env`, `cat .e*`, `grep -r KEY .`) used to
//! prompt for want of any auto-allow; with one it must not become a way around the secret rules,
//! so the same secret globs are applied here to the paths the arguments reach, and a doubt is a
//! refusal. The globs are the `read_file` deny list (so `.env.example` stays readable), the
//! configured `read_file` denies, and a few credential locations the list does not name.

use std::cell::{Cell, OnceCell, RefCell};
use std::collections::{BTreeSet, HashSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use forge_types::{is_dotenv_template, PermissionDecision, PermissionRule, RuleSource};
use globset::{Glob, GlobBuilder, GlobSet, GlobSetBuilder};

use super::super::{lexical_clean, path_candidates};
use super::path_env::{Found, SearchPath};
use super::{git_repo, Host};

/// Paths the secret rules do not name but that hold credentials all the same.
const EXTRA_SECRET_GLOBS: &[&str] = &[
    "/proc/**/environ",
    "/proc/**/cmdline",
    "**/.aws/**",
    "**/.kube/**",
    "**/.docker/**",
];

/// The dotenv glob whose template files (`.env.example`) are readable, mirroring `narrowing`.
const DOTENV_VARIANTS: &str = "**/.env.*";

const MAX_COLONS_IN_WORD: usize = 4;
const MAX_WILDCARD_COMPONENTS: usize = 3;
/// Most files one wildcard may expand to before the line is treated as unprovable.
pub(super) const MAX_GLOB_MATCHES: usize = 1024;
/// Most directory entries a recursive search may have under it before it is treated as unprovable.
const MAX_WALK_ENTRIES: usize = 50_000;
/// How long path checking may take. A wildcard or a walk over a huge tree must not stall the
/// broker; past the budget the line asks.
const BUDGET: Duration = Duration::from_millis(250);

/// How arguments are judged. Built once per call; it owns the time budget.
pub(super) struct Guard<'a> {
    cwd: PathBuf,
    /// The session root, resolved. A recursive search may not leave it.
    workspace: PathBuf,
    home: Option<PathBuf>,
    path: Option<OsString>,
    trusted: &'a dyn Fn(&Path) -> bool,
    /// The repository `git` would use here, once worked out.
    repo: OnceCell<Option<git_repo::Repo>>,
    /// Secret globs configured through `read_file` deny rules, on top of the built-in ones.
    configured: Option<GlobSet>,
    started: Instant,
    /// What the statements judged so far would start, kept for [`Guard::finish`].
    programs: RefCell<BTreeSet<String>>,
    needs_trust: Cell<bool>,
    uses_git: Cell<bool>,
    /// The operands of every `git diff`, which [`Guard::finish`] checks against the work tree.
    git_diffs: RefCell<Vec<Vec<String>>>,
}

enum Target {
    Literal(String),
    /// A wildcard word. `relative_to` is the directory a relative word is written against, so a
    /// match can be seen as the word the command will actually receive.
    Pattern {
        pattern: String,
        relative_to: Option<String>,
    },
}

impl<'a> Guard<'a> {
    pub(super) fn new(
        rules: &[PermissionRule],
        cwd: PathBuf,
        workspace: PathBuf,
        host: Host<'a>,
    ) -> Self {
        let mut builder = GlobSetBuilder::new();
        let mut any = false;
        for rule in rules {
            if rule.tool == "read_file"
                && rule.decision == PermissionDecision::Deny
                && rule.source != RuleSource::Builtin
            {
                for glob in rule.patterns.iter().filter_map(|p| insensitive(p)) {
                    builder.add(glob);
                    any = true;
                }
            }
        }
        Self {
            cwd,
            workspace: std::fs::canonicalize(&workspace).unwrap_or(workspace),
            home: host.home.map(|h| std::fs::canonicalize(&h).unwrap_or(h)),
            path: host.path,
            trusted: host.trusted,
            repo: OnceCell::new(),
            configured: any.then(|| builder.build().ok()).flatten(),
            started: Instant::now(),
            programs: RefCell::new(BTreeSet::new()),
            needs_trust: Cell::new(false),
            uses_git: Cell::new(false),
            git_diffs: RefCell::new(Vec::new()),
        }
    }

    /// Record that the call starts `program`, which [`Guard::finish`] looks up in `PATH`.
    pub(super) fn note_program(&self, program: &str) {
        self.programs.borrow_mut().insert(program.to_string());
    }

    /// Record that the call uses a binary only a trusted project may run unasked.
    pub(super) fn note_needs_trust(&self) {
        self.needs_trust.set(true);
    }

    /// Record a `git` statement; `diff_operands` is `Some` for `git diff`, whose operands must lie
    /// inside the work tree.
    pub(super) fn note_git(&self, diff_operands: Option<Vec<String>>) {
        self.uses_git.set(true);
        self.needs_trust.set(true);
        if let Some(operands) = diff_operands {
            self.git_diffs.borrow_mut().push(operands);
        }
    }

    /// The checks that look at the machine instead of the text, run once every statement of every
    /// line has passed so that a call that asks anyway costs none of them. In order: a binary that
    /// needs trust needs a trusted project, which is a read of one small file; the programs the call
    /// starts must be the system's and not the checkout's; and git runs only in a repository that
    /// runs nothing of its own, found by running git itself.
    pub(super) fn finish(&self) -> bool {
        if self.needs_trust.get() && !self.project_is_trusted() {
            return false;
        }
        self.programs_are_the_systems() && (!self.uses_git.get() || self.git_is_safe())
    }

    /// The workspace and the directory the command runs in, both: either can be somewhere the user
    /// never vouched for, and a trusted workspace can be given a `cwd` that leads out of it.
    fn project_is_trusted(&self) -> bool {
        (self.trusted)(&self.workspace) && (self.trusted)(&self.cwd)
    }

    fn search_path(&self) -> Option<SearchPath> {
        let path = self.path.as_deref()?;
        Some(SearchPath::new(
            path,
            &self.cwd,
            &self.workspace,
            self.home.as_deref(),
        ))
    }

    /// `sh` starts every call, and `rtk` may be put in front of the command (token savings), so
    /// both count with the programs the statements name.
    fn programs_are_the_systems(&self) -> bool {
        let Some(search) = self.search_path() else {
            return true;
        };
        let programs = self.programs.borrow();
        let planted = ["sh", "rtk"]
            .into_iter()
            .chain(programs.iter().map(String::as_str))
            .any(|name| search.find(name) == Found::Planted);
        !planted
    }

    fn git_is_safe(&self) -> bool {
        let Some(repo) = self.git_repo() else {
            return false;
        };
        self.git_diffs.borrow().iter().all(|operands| {
            let words: Vec<&str> = operands.iter().map(String::as_str).collect();
            repo.work_tree
                .as_deref()
                .is_some_and(|tree| self.all_inside(tree, &words))
        })
    }

    /// The repository `git` would use from the working directory, if it runs nothing of its own
    /// when asked to look (see [`git_repo`]). Judged once per call, with the `git` the system has.
    fn git_repo(&self) -> Option<&git_repo::Repo> {
        self.repo
            .get_or_init(|| {
                let Found::System(git) = self.search_path()?.find("git") else {
                    return None;
                };
                git_repo::inspect(&self.cwd, &git, self.trusted)
            })
            .as_ref()
    }

    fn expired(&self) -> bool {
        self.started.elapsed() > BUDGET
    }

    /// A word may name a path directly, as the value of `--flag=path`, glued to a short flag
    /// (`-f.env`), or after a colon (`HEAD:.env`, `:(top).env`); each reading is checked.
    pub(super) fn word_is_safe(&self, word: &str) -> bool {
        if word.matches(':').count() > MAX_COLONS_IN_WORD {
            return false;
        }
        let mut readings = vec![word];
        if word.starts_with('-') {
            if let Some((_, value)) = word.split_once('=') {
                readings.push(value);
            }
            if !word.starts_with("--") && word.len() > 2 && word.is_char_boundary(2) {
                readings.push(&word[2..]);
            }
        }
        for (at, _) in word.match_indices(':') {
            let after = &word[at + 1..];
            readings.push(after);
            // A git pathspec may open with magic: `:(top)path`, `:(icase)path`.
            if let Some((_, path)) = after
                .strip_prefix('(')
                .and_then(|magic| magic.split_once(')'))
            {
                readings.push(path);
            }
        }
        readings
            .into_iter()
            .all(|reading| self.path_is_safe(reading))
    }

    fn path_is_safe(&self, path: &str) -> bool {
        if path.is_empty() {
            return true;
        }
        match self.target(path) {
            None => false,
            Some(Target::Literal(literal)) => self.is_not_secret(&literal),
            Some(Target::Pattern {
                pattern,
                relative_to,
            }) => self
                .expand(&pattern, relative_to.as_deref())
                .is_some_and(|paths| paths.iter().all(|p| self.is_not_secret(p))),
        }
    }

    /// Where `path` points once `~` and the working directory are applied. `~user` and the other
    /// tilde forms are refused (`None`) rather than guessed at.
    fn target(&self, path: &str) -> Option<Target> {
        let (base, rest): (Option<&Path>, &str) = if path == "~" {
            (Some(self.home.as_deref()?), "")
        } else if let Some(rest) = path.strip_prefix("~/") {
            (Some(self.home.as_deref()?), rest)
        } else if path.starts_with('~') {
            return None;
        } else if path.starts_with('/') {
            (None, path)
        } else {
            (Some(self.cwd.as_path()), path)
        };
        let wildcard = rest.contains(['*', '?', '[']);
        // The expansion below is the `glob` crate's, which reads some bracket expressions
        // differently from `sh`: a pattern it matches to nothing can name `.env` to the shell.
        if wildcard && !wildcards_mean_the_same(rest) {
            return None;
        }
        // Each wildcard component multiplies the directories the expansion must read; a glob is
        // not interruptible once started, so deep fan-out is refused up front.
        let fan_out = rest
            .split('/')
            .filter(|part| part.contains(['*', '?', '[']))
            .count();
        if fan_out > MAX_WILDCARD_COMPONENTS {
            return None;
        }
        Some(match (base, wildcard) {
            (None, true) => Target::Pattern {
                pattern: rest.to_string(),
                relative_to: None,
            },
            (None, false) => Target::Literal(rest.to_string()),
            (Some(base), true) => Target::Pattern {
                pattern: format!("{}/{rest}", glob::Pattern::escape(&base.to_string_lossy())),
                relative_to: (base == self.cwd.as_path())
                    .then(|| format!("{}/", base.to_string_lossy().trim_end_matches('/'))),
            },
            (Some(base), false) => Target::Literal(if rest.is_empty() {
                base.to_string_lossy().into_owned()
            } else {
                format!("{}/{rest}", base.to_string_lossy())
            }),
        })
    }

    /// The files `sh` would expand `pattern` to. `None` is "cannot tell": more than
    /// [`MAX_GLOB_MATCHES`], an unreadable entry, `**`, an expired budget, or a match the command
    /// would take for an option. (A file named `--output=x` in the working directory makes
    /// `sort *` write `x`; a repository can contain such a file.)
    fn expand(&self, pattern: &str, relative_to: Option<&str>) -> Option<Vec<String>> {
        // `sh` reads `**` as `*`; the glob crate would recurse through the whole tree.
        if pattern.contains("**") {
            return None;
        }
        // `require_literal_leading_dot` is left off: in glob 0.3 it also hides `.env.production`
        // from the pattern `.env.*`, which is the secret this exists to find. The dot rule is
        // applied below instead, and a looser expansion can only add files to check.
        let options = glob::MatchOptions {
            case_sensitive: true,
            require_literal_separator: true,
            require_literal_leading_dot: false,
        };
        let Ok(paths) = glob::glob_with(pattern, options) else {
            // Not a valid pattern: the shell passes the word through unexpanded.
            return Some(vec![pattern.to_string()]);
        };
        let mut out = Vec::new();
        for entry in paths {
            let path = entry.ok()?.to_string_lossy().into_owned();
            if !shell_expands_to(pattern, &path) {
                continue;
            }
            let as_written = relative_to.and_then(|base| path.strip_prefix(base));
            if as_written.is_some_and(|word| word.starts_with('-')) {
                return None;
            }
            if out.len() >= MAX_GLOB_MATCHES || self.expired() {
                return None;
            }
            out.push(path);
        }
        Some(out)
    }

    /// Not a secret in any form the path can take: as written, normalised, or resolved through
    /// symlinks. A directory counts when something under it would be.
    fn is_not_secret(&self, path: &str) -> bool {
        path_candidates(path).iter().all(|candidate| {
            let inside = format!("{}/x", candidate.trim_end_matches('/'));
            !self.is_secret(candidate, candidate) && !self.is_secret(&inside, candidate)
        })
    }

    /// Whether `probe` matches a secret glob. `subject` is the path being judged, which is what
    /// the dotenv-template exception looks at: a probe under `.env.example` is still a template.
    fn is_secret(&self, probe: &str, subject: &str) -> bool {
        let Some((set, patterns)) = builtin_secrets() else {
            return true;
        };
        let builtin = set.matches(probe).into_iter().any(|at| {
            // A committed template shares the dotenv glob but holds no secret.
            !(patterns[at] == DOTENV_VARIANTS && is_dotenv_template(subject))
        });
        builtin
            || self
                .configured
                .as_ref()
                .is_some_and(|set| set.is_match(probe))
    }

    /// Whether every word that could name a path lies inside `root`. Wildcards are expanded and
    /// symlinks resolved; a word that cannot be resolved is not inside.
    pub(super) fn all_inside(&self, root: &Path, words: &[&str]) -> bool {
        words.iter().all(|word| match self.target(word) {
            None => false,
            Some(Target::Literal(path)) => within(Path::new(&path), root),
            Some(Target::Pattern {
                pattern,
                relative_to,
            }) => self
                .expand(&pattern, relative_to.as_deref())
                .is_some_and(|paths| paths.iter().all(|p| within(Path::new(p), root))),
        })
    }

    /// Whether `word` names a directory, or could: `diff DIR DIR` compares the files of two
    /// directories and prints how they differ, which reads them the way a recursive search does.
    pub(super) fn names_directory(&self, word: &str) -> bool {
        match self.target(word) {
            None => true,
            Some(Target::Literal(path)) => Path::new(&path).is_dir(),
            Some(Target::Pattern {
                pattern,
                relative_to,
            }) => self
                .expand(&pattern, relative_to.as_deref())
                .is_none_or(|paths| paths.iter().any(|p| Path::new(p).is_dir())),
        }
    }

    /// Whether a command that reads every file under the directories `operands` name (the working
    /// directory when none is one) may run unasked. Every root must lie inside the workspace and
    /// outside the way to the home directory, and no file under it may be a secret. A search reads
    /// what it meets, not what it was told to read, and no list of secret names is complete, so
    /// the sweep itself is confined rather than only screened. `skip_hidden` is set for a search
    /// that never opens a hidden file or directory below its roots (plain `rg`).
    pub(super) fn search_is_safe(&self, operands: &[&str], skip_hidden: bool) -> bool {
        // Without a home directory there is nothing to compare the search roots against.
        if self.home.is_none() {
            return false;
        }
        let Some(roots) = self.search_roots(operands) else {
            return false;
        };
        roots.iter().all(|root| {
            self.inside_workspace(root)
                && !self.covers_home(root)
                && !self.tree_holds_secret(root, skip_hidden)
        })
    }

    /// The directories a search walks. `None` when an operand cannot be resolved (`~user`, a
    /// wildcard that cannot be expanded), which could be anyone's directory.
    fn search_roots(&self, operands: &[&str]) -> Option<Vec<PathBuf>> {
        let mut roots: Vec<PathBuf> = Vec::new();
        for operand in operands {
            match self.target(operand)? {
                Target::Literal(path) => roots.push(PathBuf::from(path)),
                Target::Pattern {
                    pattern,
                    relative_to,
                } => roots.extend(
                    self.expand(&pattern, relative_to.as_deref())?
                        .into_iter()
                        .map(PathBuf::from),
                ),
            }
        }
        roots.retain(|root| root.is_dir());
        if roots.is_empty() {
            roots.push(self.cwd.clone());
        }
        Some(roots)
    }

    /// Whether `path` is the home directory or one of its ancestors. `a/link/..` is read the way
    /// the filesystem reads it as well as the way the text does.
    fn covers_home(&self, path: &Path) -> bool {
        let Some(home) = &self.home else {
            return false;
        };
        let clean = PathBuf::from(lexical_clean(&path.to_string_lossy()));
        [
            Some(clean.clone()),
            std::fs::canonicalize(path).ok(),
            std::fs::canonicalize(&clean).ok(),
        ]
        .into_iter()
        .flatten()
        .any(|candidate| home.starts_with(candidate))
    }

    /// Resolved through symlinks first: lexically `link/..` is the directory holding `link`, but on
    /// disk it is the parent of wherever `link` points.
    fn inside_workspace(&self, path: &Path) -> bool {
        within(path, &self.workspace)
    }

    /// Walk `root`, judging every entry. A symlink is judged by where it leads and followed when
    /// it leads to a directory (`grep -R`, `rg -L`, `diff -r` may all do so), and one that leaves
    /// the workspace is unprovable. Too many entries or too long is "cannot tell", which counts as
    /// a secret.
    fn tree_holds_secret(&self, root: &Path, skip_hidden: bool) -> bool {
        let mut pending = vec![root.to_path_buf()];
        let mut visited: HashSet<PathBuf> = HashSet::new();
        let mut seen = 0usize;
        while let Some(dir) = pending.pop() {
            if !visited.insert(std::fs::canonicalize(&dir).unwrap_or_else(|_| dir.clone())) {
                continue;
            }
            // A directory the search cannot read is one it reads nothing from.
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                seen += 1;
                if seen > MAX_WALK_ENTRIES || self.expired() {
                    return true;
                }
                if skip_hidden && entry.file_name().to_string_lossy().starts_with('.') {
                    continue;
                }
                let path = entry.path();
                let text = path.to_string_lossy();
                if self.is_secret(&text, &text) {
                    return true;
                }
                match entry.file_type() {
                    Ok(kind) if kind.is_dir() => pending.push(path),
                    Ok(kind) if kind.is_symlink() => {
                        if !self.is_not_secret(&text) || !self.inside_workspace(&path) {
                            return true;
                        }
                        if path.is_dir() {
                            pending.push(path);
                        }
                    }
                    _ => {}
                }
            }
        }
        false
    }
}

/// Whether `path` is `root` or below it, resolved through symlinks first: lexically `link/..` is the
/// directory holding `link`, but on disk it is the parent of wherever `link` points. `root` is
/// already resolved.
fn within(path: &Path, root: &Path) -> bool {
    std::fs::canonicalize(path)
        .unwrap_or_else(|_| PathBuf::from(lexical_clean(&path.to_string_lossy())))
        .starts_with(root)
}

/// Whether the wildcards in `pattern` mean to `sh` what they mean to the `glob` crate, so that the
/// files the crate lists are the files the shell would. `*` and `?` do. A bracket expression is
/// trusted only as a plain set of letters, digits, `_` and `.`, negated with `!` or not, with digit
/// ranges; everything else disagrees somewhere: `[^a]` negates in `sh` and is a literal caret to
/// the crate (`cat .[^a]nv` reads `.env`), `[[:lower:]]`, `[[=e=]]` and `[[.e.]]` are classes the
/// crate does not know, a range of letters follows the locale's collation in some shells, and a
/// backslash escapes in `sh`. `[` with no `]` after it is an ordinary character to both.
fn wildcards_mean_the_same(pattern: &str) -> bool {
    let chars: Vec<char> = pattern.chars().collect();
    if chars.contains(&'\\') {
        return false;
    }
    let mut at = 0;
    while at < chars.len() {
        if chars[at] != '[' {
            at += 1;
            continue;
        }
        let inner = at + 1;
        let first = inner + usize::from(chars.get(inner) == Some(&'!'));
        // A `]` right after the opening is a member of the set in `sh`; too odd to vouch for.
        if chars.get(first) == Some(&']') {
            return false;
        }
        let Some(len) = chars[first..].iter().position(|c| *c == ']') else {
            at += 1;
            continue;
        };
        let set = &chars[first..first + len];
        let mut i = 0;
        while i < set.len() {
            let plain = set[i].is_ascii_alphanumeric() || matches!(set[i], '_' | '.');
            let digit_range = set[i].is_ascii_digit()
                && set.get(i + 1) == Some(&'-')
                && set.get(i + 2).is_some_and(char::is_ascii_digit);
            if digit_range {
                i += 3;
            } else if plain {
                i += 1;
            } else {
                return false;
            }
        }
        if set.is_empty() {
            return false;
        }
        at = first + len + 1;
    }
    true
}

/// Whether `sh` itself would list `path` for `pattern`: a component beginning with `.` is matched
/// only by a pattern component that begins with a literal `.` (`*` and `[.]` do not count).
fn shell_expands_to(pattern: &str, path: &str) -> bool {
    pattern
        .split('/')
        .zip(path.split('/'))
        .all(|(wanted, found)| !found.starts_with('.') || wanted.starts_with('.'))
}

/// A secret glob that also matches `.ENV` and `ID_RSA`, since macOS and Windows volumes do not
/// tell those files apart from the lower-case ones.
fn insensitive(pattern: &str) -> Option<Glob> {
    GlobBuilder::new(pattern)
        .case_insensitive(true)
        .build()
        .ok()
}

/// The built-in secret globs: the `read_file` deny list plus [`EXTRA_SECRET_GLOBS`], compiled once.
/// `None` when they cannot be compiled, which the caller treats as "everything is a secret".
fn builtin_secrets() -> Option<&'static (GlobSet, Vec<String>)> {
    static SECRETS: OnceLock<Option<(GlobSet, Vec<String>)>> = OnceLock::new();
    SECRETS
        .get_or_init(|| {
            let mut patterns: Vec<String> = forge_config::builtin_deny_rules()
                .into_iter()
                .filter(|rule| rule.tool == "read_file")
                .flat_map(|rule| rule.patterns)
                .collect();
            patterns.extend(EXTRA_SECRET_GLOBS.iter().map(|g| (*g).to_string()));
            let mut builder = GlobSetBuilder::new();
            for pattern in &patterns {
                builder.add(insensitive(pattern)?);
            }
            Some((builder.build().ok()?, patterns))
        })
        .as_ref()
}
