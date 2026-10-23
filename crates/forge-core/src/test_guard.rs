//! The existing-tests-are-spec guard: recognise a request that keeps the existing tests as the
//! specification, detect a turn that rewrote them, and put the originals back.
//!
//! Two strengths, because the two kinds of request mean different things:
//!
//! * [`Level::AnyEdit`] — an explicit prohibition ("don't edit the existing tests") or an
//!   `expect_code_change` run (`bench swe`). Any modification of a tracked test file counts, an
//!   appended test included.
//! * [`Level::Rewrites`] — "keep the existing tests green", in unattended runs. Appending tests,
//!   or adding a new test file, is ordinary practice and stays allowed; what counts is existing content that is no
//!   longer a byte prefix of the file it came from (a changed line, a removed or reordered test,
//!   a deleted file).
//!
//! Detection compares the working tree with the commit the turn started on, not with `HEAD`:
//! the harness asks the model to commit each verified unit, so by the end of the turn a rewritten
//! test is usually committed and `git status` is clean.

use std::path::{Path, PathBuf};

use forge_types::{Message, MessageKind, Role};

use crate::working_tree::git_output;

/// How strictly the existing tests are protected for the current session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Level {
    Off,
    /// Existing content must stay a prefix of the file; appends and new files are fine.
    Rewrites,
    /// Any modification of a tracked test file.
    AnyEdit,
}

/// The pushed-back instruction for an `expect_code_change` run: the hidden evaluation restores the
/// original tests, so a fix that only passes rewritten expectations fails.
const SPEC_HEAD: &str = "Your change edits existing test expectations. Hidden evaluation uses \
the ORIGINAL tests. Re-verify your core change against the pristine tests (they have been \
restored); if they fail, shrink your fix rather than editing tests.";

/// The pushed-back instruction when a person asked for the existing tests to be left alone.
const REQUESTED_HEAD: &str = "The request asked for the existing tests to be kept, but your \
change rewrote or removed content of an existing test file. Those files are back at the state \
they had when this turn started. Re-run the original tests against your change; if they fail, \
fix the production code rather than the tests. New tests belong in a new test file or at the end \
of an existing one.";

/// Proactive counterpart for an explicit prohibition. Makes the recoverable post-hoc guard
/// exceptional rather than the normal path: coverage goes into a new file, so the original
/// contract stays byte-identical and no restore/re-verify loop is needed.
pub(crate) const PRISTINE_TEST_GUIDANCE: &str =
    "Existing tracked test files are immutable for this session. \
Read and run them, but do not edit, delete, or rename them—even to add coverage. Put useful new \
coverage in a new test file, and fix production code so the original pristine tests still pass.";

/// Proactive counterpart for "keep the existing tests green": says what the guard will check.
pub(crate) const KEEP_TESTS_GUIDANCE: &str = "The existing tests are the specification of current \
behavior. Do not rewrite, reorder, rename or delete what they already contain to make them pass; \
fix the production code instead. Put new tests in a new test file, or append them to the end of \
an existing one.";

/// Whether `path` looks like a test file — the small, extensible pattern list the
/// existing-tests-are-spec guard keys on. Matches by basename (`test_*.py`, `*_test.py`,
/// `*_tests.rs`, `*_test.rs`, `*.test.js/ts`, `*.spec.js/ts`, `test_*.rs`) or by living under a
/// `tests/` / `test/` / `testing/` directory component. Paths use `/` (git output).
pub fn is_test_path(path: &str) -> bool {
    let base = path.rsplit('/').next().unwrap_or(path);
    let by_name = (base.starts_with("test_") && (base.ends_with(".py") || base.ends_with(".rs")))
        || base.ends_with("_test.py")
        || base.ends_with("_test.rs")
        || base.ends_with("_tests.rs")
        || base.ends_with("_test.go")
        || [
            ".test.js",
            ".test.ts",
            ".test.jsx",
            ".test.tsx",
            ".spec.js",
            ".spec.ts",
        ]
        .iter()
        .any(|s| base.ends_with(s));
    let by_dir = path
        .split('/')
        .rev()
        .skip(1) // the basename is not a directory component
        .any(|c| c == "tests" || c == "test" || c == "testing");
    by_name || by_dir
}

/// Sentences of a lowercased prompt. A `.` ends a sentence only before whitespace or at the end,
/// so `go test ./...` and `main.rs` stay in one piece.
pub(crate) fn sentences(prompt: &str) -> Vec<String> {
    let prompt = prompt.replace('\u{2019}', "'").to_lowercase();
    let chars: Vec<char> = prompt.chars().collect();
    let mut out = Vec::new();
    let mut current = String::new();
    for (i, &c) in chars.iter().enumerate() {
        let ends = matches!(c, '!' | '?' | '\n' | ';')
            || (c == '.' && chars.get(i + 1).is_none_or(|next| next.is_whitespace()));
        if ends {
            out.push(std::mem::take(&mut current));
        } else {
            current.push(c);
        }
    }
    out.push(current);
    out
}

fn tokens(sentence: &str) -> impl Iterator<Item = &str> {
    sentence
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
}

/// A word that is a file path or a name with an extension (`test/helpers.ts`, `package.json`),
/// not prose. Naming a file under `test/` is not naming the tests.
fn is_path_like(word: &str) -> bool {
    let word = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '/' && c != '.');
    word.contains('/')
        || word
            .split_once('.')
            .is_some_and(|(stem, ext)| !stem.is_empty() && ext.chars().any(char::is_alphabetic))
}

/// The prose word `test`/`tests` (also `unit-tests`, `testing`), not a path under a `test/` folder.
fn names_tests(words: &str) -> bool {
    words
        .split_whitespace()
        .any(|word| word.contains("test") && !is_path_like(word))
}

/// Where a prohibition's object ends: a new clause ("..., and keep the existing tests") or a
/// parenthetical starts a different statement, whatever it says about tests.
fn prohibited_object(rest: &str) -> &str {
    const CLAUSE_BREAKS: [&str; 7] = [", and ", ", but ", " but ", " then ", " while ", " (", ": "];
    let end = CLAUSE_BREAKS
        .iter()
        .filter_map(|brk| rest.find(brk))
        .min()
        .unwrap_or(rest.len());
    &rest[..end]
}

/// "Don't change or weaken the existing tests", "never touch the tests", ... — a negation, then a
/// modifying verb, then "test" as the thing it applies to, all inside one clause. Files that merely
/// live under `test/` do not count: "Don't touch `package.json` or `test/helpers.ts`" leaves the
/// tests to be updated.
fn forbids_touching_tests(prompt: &str) -> bool {
    const NEGATIONS: [&str; 8] = [
        "do not",
        "don't",
        "dont",
        "never",
        "must not",
        "mustn't",
        "should not",
        "shouldn't",
    ];
    const VERBS: [&str; 7] = [
        "modify", "change", "weaken", "edit", "touch", "alter", "rewrite",
    ];
    sentences(prompt).iter().any(|sentence| {
        NEGATIONS.iter().any(|neg| {
            let Some(at) = sentence.find(neg) else {
                return false;
            };
            let after = &sentence[at + neg.len()..];
            VERBS.iter().any(|verb| {
                after
                    .find(verb)
                    .is_some_and(|v| names_tests(prohibited_object(&after[v + verb.len()..])))
            })
        })
    })
}

/// The request itself tells the model to change tests: "update every caller, the tests, the README",
/// "adjust the tests to match", "keep the existing tests (updated for the new signature)". That
/// instruction wins over a prohibition the guard inferred from the same text, because the two
/// cannot both hold: a signature refactor has to change the call sites in the tests. A change verb
/// followed by the tests in the same clause counts; a verb of another action in between ("update
/// the README, add tests") or a negation does not.
pub(crate) fn prompt_asks_to_change_tests(prompt: &str) -> bool {
    const CHANGE: [&str; 13] = [
        "update", "updated", "updating", "adjust", "fix", "rewrite", "edit", "modify", "migrate",
        "port", "rename", "refactor", "amend",
    ];
    const OTHER_ACTIONS: [&str; 18] = [
        "add",
        "write",
        "create",
        "implement",
        "keep",
        "make",
        "ensure",
        "run",
        "check",
        "verify",
        "use",
        "put",
        "remove",
        "delete",
        "leave",
        "preserve",
        "retain",
        "respect",
    ];
    // "existing tests as is", "tests untouched": the tests are being kept, not changed.
    const STAY_PUT: [&str; 7] = [
        "as",
        "untouched",
        "alone",
        "unchanged",
        "intact",
        "pristine",
        "unmodified",
    ];
    const NEGATIONS: [&str; 9] = [
        "not",
        "don't",
        "dont",
        "never",
        "no",
        "without",
        "mustn't",
        "shouldn't",
        "cannot",
    ];
    const PAST: [&str; 8] = [
        "updated",
        "adjusted",
        "migrated",
        "rewritten",
        "edited",
        "modified",
        "changed",
        "fixed",
    ];
    const REACH: usize = 10;
    let lowered = prompt.replace('\u{2019}', "'").to_lowercase();
    let word = |w: &str| {
        w.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'' && c != '/' && c != '.')
            .to_string()
    };
    sentences(&lowered).iter().any(|sentence| {
        let words: Vec<String> = sentence.split_whitespace().map(word).collect();
        let is_test_word = |w: &String| w.contains("test") && !is_path_like(w);
        let negated = |from: usize, to: usize| {
            words[from..to]
                .iter()
                .any(|w| NEGATIONS.contains(&w.as_str()))
        };
        let stays_put = |at: usize| {
            words[at + 1..words.len().min(at + 4)]
                .iter()
                .any(|w| STAY_PUT.contains(&w.as_str()))
        };
        // "<change verb> ... tests"
        let verb_then_tests = words.iter().enumerate().any(|(i, w)| {
            CHANGE.contains(&w.as_str())
                && !negated(i.saturating_sub(3), i)
                && words[i + 1..words.len().min(i + 1 + REACH)]
                    .iter()
                    .enumerate()
                    .map(|(k, w)| (i + 1 + k, w))
                    .take_while(|(_, w)| {
                        !OTHER_ACTIONS.contains(&w.as_str()) && !NEGATIONS.contains(&w.as_str())
                    })
                    .any(|(at, w)| is_test_word(w) && !stays_put(at))
        });
        // "tests (updated for ...)"
        let tests_then_participle = words.iter().enumerate().any(|(j, w)| {
            is_test_word(w)
                && !negated(j.saturating_sub(3), j)
                && !stays_put(j)
                && words[j + 1..words.len().min(j + 1 + REACH)]
                    .iter()
                    .take_while(|w| !NEGATIONS.contains(&w.as_str()))
                    .any(|w| PAST.contains(&w.as_str()))
        });
        verb_then_tests || tests_then_participle
    })
}

/// An explicit request to leave the existing tests untouched.
pub(crate) fn prompt_requires_pristine_existing_tests(prompt: &str) -> bool {
    const PHRASES: [&str; 13] = [
        "do not weaken",
        "no tests weakened",
        "do not modify existing tests",
        "keep existing tests unchanged",
        "keep the existing tests unchanged",
        "leave the existing tests",
        "leave existing tests",
        "leave the tests alone",
        "leave the tests untouched",
        "leave tests alone",
        "existing tests untouched",
        "existing tests as they are",
        "existing tests as is",
    ];
    let prompt = prompt.replace('\u{2019}', "'").to_lowercase();
    prompt.contains("test")
        && !prompt_asks_to_change_tests(&prompt)
        && (forbids_touching_tests(&prompt)
            || PHRASES.iter().any(|phrase| prompt.contains(phrase))
            || sentences(&prompt).iter().any(|s| {
                s.contains("without")
                    && ["changing", "modifying", "editing", "touching"]
                        .iter()
                        .any(|verb| s.contains(verb))
                    && tokens(s).any(|t| t == "tests" || t == "test")
            }))
}

/// Requests that expect test files to change: a green-tests clause in such a prompt does not mean
/// the existing tests are the spec ("update the tests ... and make sure they pass").
const TESTS_EXPECTED_TO_CHANGE: [&str; 21] = [
    "update the test",
    "update test",
    "update existing test",
    "updating the test",
    "fix the test",
    "fix the failing test",
    "fix failing test",
    "fix the broken test",
    "fix broken test",
    "adjust the test",
    "adjust test",
    "rewrite the test",
    "rewrite test",
    "edit the test",
    "modify the test",
    "change the test",
    "migrate",
    "port the test",
    "refactor",
    "rename",
    "tests to match",
];

/// Words that make a following "pass" a noun, a round of work ("final pass", "one more pass over
/// the diff", "a quick pass"), not tests going green. "Final pass: rerun everything and confirm each
/// change is covered" asks for a review; read as "tests must pass" it armed the whole
/// guard — contract text, restore and push-back round — on a turn that never promised green tests.
/// "passes" is only a noun after a count ("two passes"): "it passes" is the verb.
const PASS_NOUN_LEADS: [&str; 21] = [
    "a", "an", "the", "this", "one", "another", "more", "extra", "final", "first", "second",
    "third", "last", "next", "quick", "full", "review", "cleanup", "up", "careful", "thorough",
];
const PASSES_NOUN_LEADS: [&str; 7] = [
    "two", "three", "several", "multiple", "few", "couple", "more",
];

/// A word of `words[at]` that says the tests must be green: `green`, `passing`, a verb "pass".
fn wants_green(words: &[&str], at: usize) -> bool {
    const GREEN_WORDS: [&str; 7] = [
        "green", "passing", "pass", "passes", "intact", "succeed", "succeeds",
    ];
    let word = words[at];
    if !GREEN_WORDS.contains(&word) {
        return false;
    }
    let lead = at.checked_sub(1).map(|i| words[i]);
    let noun_leads: &[&str] = match word {
        "pass" => &PASS_NOUN_LEADS,
        "passes" => &PASSES_NOUN_LEADS,
        _ => &[],
    };
    !lead.is_some_and(|lead| noun_leads.contains(&lead))
}

/// "The existing tests must keep passing", "keep the tests green", "keep `go test -race ./...`
/// green", "get `cargo test` green": one sentence that names tests and wants them green. Skipped
/// for requests that expect tests to change (see [`TESTS_EXPECTED_TO_CHANGE`]) and for
/// identifier migrations, where the tests are renamed along with the code.
pub(crate) fn prompt_expects_green_existing_tests(prompt: &str) -> bool {
    const TEST_WORDS: [&str; 10] = [
        "test", "tests", "testing", "pytest", "unittest", "jest", "vitest", "mocha", "rspec",
        "phpunit",
    ];
    let lowered = prompt.replace('\u{2019}', "'").to_lowercase();
    if TESTS_EXPECTED_TO_CHANGE.iter().any(|m| lowered.contains(m))
        || crate::completeness::direct_completeness_is_identifier_migration(&lowered)
    {
        return false;
    }
    sentences(&lowered).iter().any(|s| {
        let words: Vec<&str> = tokens(s).collect();
        words.iter().any(|t| TEST_WORDS.contains(t))
            && (0..words.len()).any(|at| wants_green(&words, at))
    })
}

/// What the person typed. A harness nudge is stored as a user message too, and its wording is the
/// harness's: the push-back after a restore says "fix the production code rather than the tests",
/// which read as a request to change tests and lifted the protection it had just enforced.
fn user_messages(messages: &[Message]) -> impl Iterator<Item = &str> {
    messages
        .iter()
        .filter(|m| m.role == Role::User && m.kind == MessageKind::Normal)
        .map(|m| m.content.as_str())
}

/// Whether the session still protects the existing tests: the latest request that speaks to it
/// decides, so "update the tests" after an earlier "don't touch the tests" lifts the protection.
pub(crate) fn session_requires_pristine_existing_tests(messages: &[Message]) -> bool {
    user_messages(messages).fold(false, |pristine, prompt| {
        if prompt_requires_pristine_existing_tests(prompt) {
            true
        } else if prompt_asks_to_change_tests(prompt) {
            false
        } else {
            pristine
        }
    })
}

fn session_expects_green_existing_tests(messages: &[Message]) -> bool {
    user_messages(messages).any(prompt_expects_green_existing_tests)
}

/// How strictly this session protects the existing tests. "Keep the tests green" is enforced
/// only where nobody is watching (`forge run`, bench, scripted runs): a person in a chat who says
/// "make sure the tests pass" is not asking for an extra restore-and-retry round whenever a test
/// case is edited, and sees the diff anyway.
pub(crate) fn level(expect_code_change: bool, transcript: &[Message], attended: bool) -> Level {
    if expect_code_change || session_requires_pristine_existing_tests(transcript) {
        Level::AnyEdit
    } else if !attended && session_expects_green_existing_tests(transcript) {
        Level::Rewrites
    } else {
        Level::Off
    }
}

/// The proactive guidance (label, text) the prompt or session calls for, if any.
pub(crate) fn guidance(
    prompt: &str,
    transcript: &[Message],
) -> Option<(&'static str, &'static str)> {
    if prompt_requires_pristine_existing_tests(prompt)
        || (!prompt_asks_to_change_tests(prompt)
            && session_requires_pristine_existing_tests(transcript))
    {
        Some(("pristine existing-test contract", PRISTINE_TEST_GUIDANCE))
    } else if prompt_expects_green_existing_tests(prompt)
        || session_expects_green_existing_tests(transcript)
    {
        Some(("existing-tests-stay-green contract", KEEP_TESTS_GUIDANCE))
    } else {
        None
    }
}

/// `git diff --name-status -z --no-renames` output as `(status, path)` pairs. A rename is
/// reported as a delete plus an add, so moving a test file away counts as removing it.
pub(crate) fn parse_name_status(raw: &[u8]) -> Vec<(char, String)> {
    let text = String::from_utf8_lossy(raw);
    let mut fields = text.split('\0');
    let mut out = Vec::new();
    while let (Some(status), Some(path)) = (fields.next(), fields.next()) {
        if let Some(status) = status.chars().next() {
            out.push((status, path.to_string()));
        }
    }
    out
}

/// Whether `path` at `base` is a byte prefix of the working copy: the edit only appended.
fn only_appended(root: Option<&Path>, base: &str, path: &str) -> bool {
    let Some(original) = git_output(root, &["show", &format!("{base}:./{path}")]) else {
        return false;
    };
    let current = match root {
        Some(root) => std::fs::read(root.join(path)),
        None => std::fs::read(path),
    };
    current.is_ok_and(|current| current.starts_with(&original))
}

/// Existing tracked test files the working tree changes relative to `base` (the commit the turn
/// started on; `HEAD` when unknown), committed or not. New files never count: writing a fresh
/// reproduction test is normal practice. Empty outside a repository.
pub(crate) fn modified_existing_tests(
    root: Option<&Path>,
    base: Option<&str>,
    level: Level,
) -> Vec<String> {
    if level == Level::Off {
        return Vec::new();
    }
    let base = base.unwrap_or("HEAD");
    let Some(raw) = git_output(
        root,
        &[
            "diff",
            "--name-status",
            "-z",
            "--no-renames",
            "--relative",
            base,
            "--",
        ],
    ) else {
        return Vec::new();
    };
    parse_name_status(&raw)
        .into_iter()
        .filter(|(status, path)| matches!(status, 'M' | 'D' | 'T') && is_test_path(path))
        .filter(|(status, path)| {
            level == Level::AnyEdit || *status != 'M' || !only_appended(root, base, path)
        })
        .map(|(_, path)| path)
        .collect()
}

/// Put the tests in `paths` back to their state at `base` (`HEAD` when unknown) and keep the
/// model's edits as a patch in Forge's state folder for the workspace. Returns that patch, or
/// `None` when the tree was left untouched (nothing differed, or git or the disk refused): the
/// guard only fires when the originals are really back, since claiming "tests restored" over an
/// untouched tree would be a lie.
///
/// The edits are deliberately not stashed. `git stash push` writes `refs/stash` and pushes onto
/// the user's own stash stack, so the user's next `git stash pop` would hand them Forge's test
/// edits, and a benchmark's strict repo check sees a ref nobody asked for. A patch file outside
/// the repository leaves no ref, no object and nothing under `.git/`; it also covers edits the
/// model committed, which a stash cannot hold.
pub(crate) fn restore_originals(
    root: Option<&Path>,
    base: Option<&str>,
    paths: &[String],
) -> Option<PathBuf> {
    #[cfg(test)]
    crate::test_state::isolate();
    let base = base.unwrap_or("HEAD");
    let mut diff = vec!["diff", "--binary", "--relative", base, "--"];
    diff.extend(paths.iter().map(String::as_str));
    let patch = git_output(root, &diff).filter(|patch| !patch.is_empty())?;
    let workspace = root.unwrap_or(Path::new("."));
    forge_config::prepare_workspace_state(workspace).ok()?;
    let dir = forge_config::state_subdir(workspace, "test-edits");
    std::fs::create_dir_all(&dir).ok()?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let file = dir.join(format!("{stamp}.patch"));
    std::fs::write(&file, patch).ok()?;
    let mut checkout = vec!["checkout", base, "--"];
    checkout.extend(paths.iter().map(String::as_str));
    if git_output(root, &checkout).is_none() {
        let _ = std::fs::remove_file(&file);
        return None;
    }
    Some(file)
}

/// The instruction pushed back at the model after its test edits were taken out.
pub(crate) fn push_back(expect_code_change: bool, edits: &[String], patch: &Path) -> String {
    let recover = format!(
        "Your test edits are saved as a patch (`git apply {}` re-applies them)",
        patch.display()
    );
    let (head, tail) = if expect_code_change {
        (
            SPEC_HEAD,
            "re-apply only if genuinely justified by the issue text.",
        )
    } else {
        (
            REQUESTED_HEAD,
            "re-apply only if the task genuinely requires changing what a test expects, and say \
             so in your answer.",
        )
    };
    format!(
        "{head} {recover}; {tail}\n\nRestored test files:\n- {}",
        edits.join("\n- ")
    )
}
