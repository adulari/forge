//! The path guard behind Ask's read-only auto-allow: which files a command's arguments name, once
//! `~`, the working directory and wildcards are applied, and whether any of them is a secret.
//!
//! The built-in deny rules match secret READS by command verb (`cat *.env`). A read-only command
//! the verbs do not list (`wc -c .env`, `git show HEAD:.env`, `cat .e*`, `grep -r KEY .`) used to
//! prompt for want of any auto-allow; with one it must not become a way around the secret rules,
//! so the same secret globs are applied here to the paths the arguments reach, and a doubt is a
//! refusal. The globs are the `read_file` deny list (so `.env.example` stays readable), the
//! configured `read_file` denies, and a few credential locations the list does not name.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use forge_types::{is_dotenv_template, PermissionDecision, PermissionRule, RuleSource};
use globset::{Glob, GlobBuilder, GlobSet, GlobSetBuilder};

use super::super::{lexical_clean, path_candidates};

/// Paths the secret rules do not name but that hold credentials all the same.
const EXTRA_SECRET_GLOBS: &[&str] = &[
    "/proc/**/environ",
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
pub(super) struct Guard {
    cwd: PathBuf,
    home: Option<PathBuf>,
    /// Secret globs configured through `read_file` deny rules, on top of the built-in ones.
    configured: Option<GlobSet>,
    started: Instant,
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

impl Guard {
    pub(super) fn new(rules: &[PermissionRule], cwd: PathBuf, home: Option<PathBuf>) -> Self {
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
            home: home.map(|h| std::fs::canonicalize(&h).unwrap_or(h)),
            configured: any.then(|| builder.build().ok()).flatten(),
            started: Instant::now(),
        }
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

    /// A recursive search must not be able to reach the home directory: the working directory is
    /// home or above it, or an operand is. Home holds every credential file the user has.
    pub(super) fn recursive_root_is_safe(&self, operands: &[&str]) -> bool {
        // Without a home directory there is nothing to compare the search roots against.
        self.home.is_some()
            && !self.covers_home(&self.cwd)
            && !operands.iter().any(|word| self.names_home_or_above(word))
    }

    fn names_home_or_above(&self, word: &str) -> bool {
        match self.target(word) {
            Some(Target::Literal(path)) => self.covers_home(Path::new(&path)),
            Some(Target::Pattern { .. }) => false,
            // An unresolvable tilde form could be anyone's home.
            None => true,
        }
    }

    fn covers_home(&self, path: &Path) -> bool {
        let Some(home) = &self.home else {
            return false;
        };
        let clean = PathBuf::from(lexical_clean(&path.to_string_lossy()));
        let resolved = std::fs::canonicalize(&clean).unwrap_or_else(|_| clean.clone());
        home.starts_with(&clean) || home.starts_with(&resolved)
    }

    /// No file under the directories `operands` name (the working directory when none is one) is
    /// a secret. A recursive `grep` prints matching lines from every file it meets, `.env` too.
    pub(super) fn trees_hold_no_secret(&self, operands: &[&str]) -> bool {
        let mut roots: Vec<PathBuf> = Vec::new();
        for operand in operands {
            match self.target(operand) {
                None => return false,
                Some(Target::Literal(path)) => roots.push(PathBuf::from(path)),
                Some(Target::Pattern {
                    pattern,
                    relative_to,
                }) => {
                    let Some(paths) = self.expand(&pattern, relative_to.as_deref()) else {
                        return false;
                    };
                    roots.extend(paths.into_iter().map(PathBuf::from));
                }
            }
        }
        roots.retain(|root| root.is_dir());
        if roots.is_empty() {
            roots.push(self.cwd.clone());
        }
        roots.iter().all(|root| !self.tree_holds_secret(root))
    }

    /// Walk `root` without following symlinks (each one is judged by where it leads). Too many
    /// entries or too long is "cannot tell", which counts as a secret.
    fn tree_holds_secret(&self, root: &Path) -> bool {
        let mut pending = vec![root.to_path_buf()];
        let mut seen = 0usize;
        while let Some(dir) = pending.pop() {
            // A directory the search cannot read is one it reads nothing from.
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                seen += 1;
                if seen > MAX_WALK_ENTRIES || self.expired() {
                    return true;
                }
                let path = entry.path();
                let text = path.to_string_lossy();
                if self.is_secret(&text, &text) {
                    return true;
                }
                match entry.file_type() {
                    Ok(kind) if kind.is_dir() => pending.push(path),
                    Ok(kind) if kind.is_symlink() && !self.is_not_secret(&text) => return true,
                    _ => {}
                }
            }
        }
        false
    }
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
