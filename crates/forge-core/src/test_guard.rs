//! The existing-tests guard: recognise a request that keeps the existing tests as the
//! specification, detect a turn that changed them, and tell an edit that makes a failure
//! disappear from an edit that follows a behaviour change the request asked for.
//!
//! What a turn may do to an existing test depends on who asked for what ([`Level`]):
//!
//! * [`Level::Frozen`] — a person said to leave the existing tests alone. Any change to a tracked
//!   test file is put back, an appended test included.
//! * [`Level::Guarded`] — an `expect_code_change` run (`bench swe`, a worktree build session) or
//!   "keep the existing tests green" in an unattended run. Appending tests or adding a test file is
//!   ordinary practice. An edit that hides a failure is put back: a deleted test file, removed
//!   assertions, an added skip/xfail marker, an assertion that cannot fail ([`Why`]). Changed lines
//!   of an existing test stay when a code change stands behind them, and the model is asked once
//!   to justify them; with no code change behind them they are put back.
//! * [`Level::Concealing`] — the request itself says to change the tests ("update the tests",
//!   "fix the failing test"). Only the edits that hide a failure are put back.
//!
//! Nothing here says how the work will be judged: the guard cannot know whether the tests that
//! count are the ones in the tree, and a task whose behaviour change contradicts an existing
//! assertion must be allowed to update that assertion.
//!
//! Detection compares the working tree with the commit the turn started on, not with `HEAD`:
//! the harness asks the model to commit each verified unit, so by the end of the turn a rewritten
//! test is usually committed and `git status` is clean.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use forge_types::{Message, MessageKind, Role};
use regex::Regex;

use crate::working_tree::{git_output, untracked_work_files};

/// How an existing test may be changed in the current session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Level {
    Off,
    /// The request asks for test changes: only edits that hide a failure are put back.
    Concealing,
    /// Hiding edits are put back; a changed test with a code change behind it is questioned.
    Guarded,
    /// A person asked for the existing tests to be left alone: any change is put back.
    Frozen,
}

/// The pushed-back instruction when a person asked for the existing tests to be left alone.
const REQUESTED_HEAD: &str = "The request asked for the existing tests to be left as they are, \
but your change touched an existing test file. Those files are back at the state they had when \
this turn started. Re-run the tests as they were against your change; if they fail, fix the \
production code rather than the tests. If the request cannot be met without changing a test, say \
which test and why in your answer instead of editing it.";

/// The pushed-back instruction when test edits that hide a failure were taken out.
const HIDING_HEAD: &str = "Some of your test edits would make a failure disappear instead of \
fixing it, so those files are back at the state they had when this turn started (the reason for \
each is below). Make the code satisfy the tests as they were. If the request itself changes what \
one of them expects, re-apply only that changed expectation, not a removal, a skip or a weaker check, \
and say which test and why in your answer.";

/// The question asked about changed test lines that stay in the tree.
const JUSTIFY_HEAD: &str = "These existing tests have changed lines, and the edits are still in \
place:";

/// Proactive counterpart for an explicit prohibition. Makes the recoverable post-hoc guard
/// exceptional rather than the normal path: coverage goes into a new file, so the original
/// contract stays byte-identical and no restore/re-verify loop is needed.
pub(crate) const PRISTINE_TEST_GUIDANCE: &str =
    "Existing tracked test files are immutable for this session. \
Read and run them, but do not edit, delete, or rename them—even to add coverage. Put useful new \
coverage in a new test file, and fix production code so the existing tests still pass as they \
are. If the request cannot be met without changing one of them, stop and say which test and why.";

/// Proactive counterpart for "keep the existing tests green": says what the guard will check.
pub(crate) const KEEP_TESTS_GUIDANCE: &str = "The existing tests describe the current behavior. \
If your change makes one fail, decide which side is wrong. When the request itself changes that \
behavior, update that test's expected value minimally and say which test and why in your final \
answer. Otherwise fix the code, not the test. Never delete a test or an assertion, skip or xfail \
one, or loosen one so that a failure disappears. Put new tests in a new test file where you can.";

const SKIP_MARKER_PATTERN: &str = r"@pytest\.mark\.(?:skip|skipif|xfail)\b|\bpytest\.(?:skip|xfail|importorskip)\s*\(|@unittest\.(?:skip|skipIf|skipUnless|expectedFailure)\b|\bself\.skipTest\s*\(|\braise\s+(?:unittest\.)?SkipTest\b|#\[ignore\b|\b(?:it|test|describe)\.(?:skip|todo)\s*\(|\bx(?:it|describe|test)\s*\(|\bt\.Skip(?:Now|f)?\s*\(";

const ASSERTION_PATTERN: &str = r"^\s*assert\b|\bself\.assert[A-Z]\w*\s*\(|\bassert(?:_eq|_ne|_matches)?!\s*\(|\bexpect\s*\(|\bpytest\.raises\s*\(|\bassertThat\s*\(|\bt\.(?:Error|Fatal)f?\s*\(";

const ALWAYS_TRUE_PATTERN: &str = r"^\s*assert\s+(?:True|1)\s*(?:,.*)?$|\bassertTrue\(\s*True\s*\)|\bassert!\(\s*true\s*\)|\bexpect\(\s*(?:true|1)\s*\)\s*\.\s*(?:toBe|toEqual)\(\s*(?:true|1)\s*\)|\bassert\w*\b.*(?:\bor\s+True\b|\|\|\s*true\b)";

static SKIP_MARKER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(SKIP_MARKER_PATTERN).expect("skip marker regex"));
static ASSERTION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(ASSERTION_PATTERN).expect("assertion regex"));
static ALWAYS_TRUE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(ALWAYS_TRUE_PATTERN).expect("always-true regex"));

/// A line that keeps a test from running: a skip, xfail, ignore or todo marker.
pub fn is_skip_marker_line(line: &str) -> bool {
    SKIP_MARKER.is_match(line)
}

/// A line that checks something: an assert, an `expect(...)`, `pytest.raises`, `t.Errorf`, ...
pub fn is_assertion_line(line: &str) -> bool {
    ASSERTION.is_match(line)
}

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

/// What the session's requests say about the existing tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stance {
    Silent,
    /// "Don't edit the existing tests".
    Frozen,
    /// "Update the tests", "fix the failing test".
    Revisable,
}

/// The latest request that speaks to the tests decides, so "update the tests" after an earlier
/// "don't touch the tests" lifts the protection, and a later "leave them alone" restores it.
fn session_stance(messages: &[Message]) -> Stance {
    user_messages(messages).fold(Stance::Silent, |stance, prompt| {
        if prompt_requires_pristine_existing_tests(prompt) {
            Stance::Frozen
        } else if prompt_asks_to_change_tests(prompt) {
            Stance::Revisable
        } else {
            stance
        }
    })
}

/// Whether the session still protects the existing tests: the latest request that speaks to it
/// decides, so "update the tests" after an earlier "don't touch the tests" lifts the protection.
pub(crate) fn session_requires_pristine_existing_tests(messages: &[Message]) -> bool {
    session_stance(messages) == Stance::Frozen
}

fn session_expects_green_existing_tests(messages: &[Message]) -> bool {
    user_messages(messages).any(prompt_expects_green_existing_tests)
}

/// How this session treats edits to the existing tests. A run is guarded when it expects a code
/// change, or when it says the tests must stay green and nobody is watching (`forge run`, bench,
/// scripted runs): a person in a chat who says "make sure the tests pass" is not asking for an
/// extra round whenever a test case is edited, and sees the diff anyway. A person's explicit
/// "don't edit the tests" freezes them in any run.
pub(crate) fn level(expect_code_change: bool, transcript: &[Message], attended: bool) -> Level {
    let guarded_run =
        expect_code_change || (!attended && session_expects_green_existing_tests(transcript));
    match session_stance(transcript) {
        Stance::Frozen => Level::Frozen,
        Stance::Revisable if guarded_run => Level::Concealing,
        Stance::Silent if guarded_run => Level::Guarded,
        Stance::Revisable | Stance::Silent => Level::Off,
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

/// Why an edit to an existing test is taken out of the tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Why {
    /// A person asked for the existing tests to be left alone.
    Requested,
    Deleted,
    AssertionsRemoved(usize),
    SkipAdded(usize),
    AlwaysTrue,
    /// Lines the test already had were changed, and no code outside the tests was.
    CodeUnchanged,
}

impl Why {
    fn text(&self) -> String {
        match self {
            Why::Requested => "the request said to leave it alone".to_string(),
            Why::Deleted => "the file was deleted or replaced".to_string(),
            Why::AssertionsRemoved(n) => format!("{n} assertion(s) were removed"),
            Why::SkipAdded(n) => format!("{n} skip/xfail marker(s) were added"),
            Why::AlwaysTrue => "an assertion that cannot fail was added".to_string(),
            Why::CodeUnchanged => {
                "lines it already had were changed while no code outside the tests was".to_string()
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Restore {
    pub path: String,
    pub why: Vec<Why>,
}

/// What a turn did to the existing test files, sorted by what the guard does about it.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct TestEdits {
    /// Files to put back, with the reasons.
    pub restore: Vec<Restore>,
    /// Existing tests whose lines changed with a code change behind them: they stay, and the
    /// model is asked once whether the request changes what they pin down.
    pub revised: Vec<String>,
    pub(crate) excerpt: String,
}

impl TestEdits {
    pub(crate) fn is_empty(&self) -> bool {
        self.restore.is_empty() && self.revised.is_empty()
    }

    pub(crate) fn restore_paths(&self) -> Vec<String> {
        self.restore.iter().map(|r| r.path.clone()).collect()
    }
}

enum Edit {
    Same,
    Added,
    Hides(Vec<Why>),
    Revises,
}

/// Lines (trimmed, non-blank) of `text` that `other` does not have as often. Comparing as
/// multisets keeps a moved or re-indented line from counting as a change.
fn lines_missing_from<'a>(text: &'a str, other: &str) -> Vec<&'a str> {
    let mut available: HashMap<&str, usize> = HashMap::new();
    for line in other.lines().map(str::trim).filter(|l| !l.is_empty()) {
        *available.entry(line).or_default() += 1;
    }
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .filter(|line| match available.get_mut(line) {
            Some(left) if *left > 0 => {
                *left -= 1;
                false
            }
            _ => true,
        })
        .collect()
}

fn classify(before: &str, after: &str) -> Edit {
    let removed = lines_missing_from(before, after);
    let added = lines_missing_from(after, before);
    let count = |lines: &[&str], matches: fn(&str) -> bool| {
        lines.iter().filter(|line| matches(line)).count()
    };
    let always_true = |line: &str| ALWAYS_TRUE.is_match(line);
    let mut why = Vec::new();
    let skips =
        count(&added, is_skip_marker_line).saturating_sub(count(&removed, is_skip_marker_line));
    if skips > 0 {
        why.push(Why::SkipAdded(skips));
    }
    let lost = count(&removed, is_assertion_line).saturating_sub(count(&added, is_assertion_line));
    if lost > 0 {
        why.push(Why::AssertionsRemoved(lost));
    }
    if count(&added, always_true) > count(&removed, always_true) {
        why.push(Why::AlwaysTrue);
    }
    if !why.is_empty() {
        Edit::Hides(why)
    } else if !removed.is_empty() {
        Edit::Revises
    } else if !added.is_empty() {
        Edit::Added
    } else {
        Edit::Same
    }
}

fn is_prose_path(path: &str) -> bool {
    path.starts_with("docs/")
        || [".md", ".rst", ".txt", ".adoc"]
            .iter()
            .any(|ext| path.ends_with(ext))
}

/// Whether the turn changed anything that is neither a test nor prose: the code a changed
/// expectation could be following.
fn code_changed(root: Option<&Path>, changes: &[(char, String)]) -> bool {
    let is_code = |path: &str| !is_test_path(path) && !is_prose_path(path);
    changes.iter().any(|(_, path)| is_code(path))
        || root
            .and_then(untracked_work_files)
            .is_some_and(|files| files.iter().any(|path| is_code(path)))
}

fn file_at(root: Option<&Path>, base: &str, path: &str) -> Option<String> {
    git_output(root, &["show", &format!("{base}:./{path}")])
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
}

fn file_now(root: Option<&Path>, path: &str) -> Option<String> {
    let bytes = match root {
        Some(root) => std::fs::read(root.join(path)),
        None => std::fs::read(path),
    };
    bytes
        .ok()
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
}

const EXCERPT_FILES: usize = 6;
const EXCERPT_LINES_PER_FILE: usize = 24;
const EXCERPT_CHARS: usize = 3_000;

/// The changed lines of `paths` against `base`, bounded: what the model needs to see to say why.
fn excerpt(root: Option<&Path>, base: &str, paths: &[String]) -> String {
    let mut out = String::new();
    for path in paths.iter().take(EXCERPT_FILES) {
        let Some(raw) = git_output(
            root,
            &[
                "diff",
                "--unified=0",
                "--no-color",
                "--relative",
                base,
                "--",
                path,
            ],
        ) else {
            continue;
        };
        let text = String::from_utf8_lossy(&raw);
        let body: Vec<&str> = text
            .lines()
            .filter(|line| {
                line.starts_with("@@")
                    || (line.starts_with(['+', '-'])
                        && !line.starts_with("+++")
                        && !line.starts_with("---"))
            })
            .collect();
        let _ = write!(out, "\n{path}:");
        for line in body.iter().take(EXCERPT_LINES_PER_FILE) {
            let _ = write!(out, "\n  {line}");
        }
        if body.len() > EXCERPT_LINES_PER_FILE {
            let _ = write!(
                out,
                "\n  ... {} more changed lines",
                body.len() - EXCERPT_LINES_PER_FILE
            );
        }
    }
    if paths.len() > EXCERPT_FILES {
        let _ = write!(out, "\n... and {} more files", paths.len() - EXCERPT_FILES);
    }
    if out.len() > EXCERPT_CHARS {
        let cut = (0..=EXCERPT_CHARS)
            .rev()
            .find(|&i| out.is_char_boundary(i))
            .unwrap_or(0);
        out.truncate(cut);
        out.push_str("\n  ...");
    }
    out
}

/// What the turn did to the existing tracked test files, committed or not, relative to `base`
/// (the commit the turn started on; `HEAD` when unknown). New files never count: writing a fresh
/// reproduction test is normal practice. Empty outside a repository.
pub(crate) fn assess(root: Option<&Path>, base: Option<&str>, level: Level) -> TestEdits {
    let mut edits = TestEdits::default();
    if level == Level::Off {
        return edits;
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
        return edits;
    };
    let changes = parse_name_status(&raw);
    let tests: Vec<&(char, String)> = changes
        .iter()
        .filter(|(status, path)| matches!(status, 'M' | 'D' | 'T') && is_test_path(path))
        .collect();
    if tests.is_empty() {
        return edits;
    }
    if level == Level::Frozen {
        edits.restore = tests
            .iter()
            .map(|(_, path)| Restore {
                path: path.clone(),
                why: vec![Why::Requested],
            })
            .collect();
        return edits;
    }
    let mut code: Option<bool> = None;
    for (status, path) in tests {
        if *status != 'M' {
            edits.restore.push(Restore {
                path: path.clone(),
                why: vec![Why::Deleted],
            });
            continue;
        }
        let (Some(before), Some(after)) = (file_at(root, base, path), file_now(root, path)) else {
            continue;
        };
        match classify(&before, &after) {
            Edit::Same | Edit::Added => {}
            Edit::Hides(why) => edits.restore.push(Restore {
                path: path.clone(),
                why,
            }),
            Edit::Revises if level == Level::Guarded => {
                if *code.get_or_insert_with(|| code_changed(root, &changes)) {
                    edits.revised.push(path.clone());
                } else {
                    edits.restore.push(Restore {
                        path: path.clone(),
                        why: vec![Why::CodeUnchanged],
                    });
                }
            }
            Edit::Revises => {}
        }
    }
    if !edits.revised.is_empty() {
        edits.excerpt = excerpt(root, base, &edits.revised);
    }
    edits
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

/// The message pushed at the model after the guard acted. `patch` is where the restored edits
/// were saved (`None` when nothing was restored).
pub(crate) fn push_back(
    level: Level,
    edits: &TestEdits,
    patch: Option<&Path>,
    base: Option<&str>,
) -> String {
    let mut out = String::new();
    if let Some(patch) = patch.filter(|_| !edits.restore.is_empty()) {
        out.push_str(if level == Level::Frozen {
            REQUESTED_HEAD
        } else {
            HIDING_HEAD
        });
        let _ = write!(
            out,
            " Your edits are saved as a patch (`git apply {}` re-applies them).\n\nRestored test files:",
            patch.display()
        );
        for restored in &edits.restore {
            let reasons: Vec<String> = restored.why.iter().map(Why::text).collect();
            if level == Level::Frozen {
                let _ = write!(out, "\n- {}", restored.path);
            } else {
                let _ = write!(out, "\n- {}: {}", restored.path, reasons.join("; "));
            }
        }
    }
    if !edits.revised.is_empty() {
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        let base = base.unwrap_or("HEAD");
        let _ = write!(
            out,
            "{JUSTIFY_HEAD}{}\n\nDoes the request change the behavior these tests pin down? If it \
             does, keep the edit and name the old and the new expectation in your answer. If it \
             does not, the old test is still the specification: put it back with `git checkout \
             {} -- {}` and make the code satisfy it.",
            edits.excerpt,
            &base[..base.len().min(12)],
            edits.revised.join(" ")
        );
    }
    out
}

/// The one-line notice for the person: what the guard did before pushing back.
pub(crate) fn warning(level: Level, edits: &TestEdits) -> String {
    let mut parts = Vec::new();
    if !edits.restore.is_empty() {
        let n = edits.restore.len();
        parts.push(if level == Level::Frozen {
            format!("restored {n} existing test file(s) the request asked to leave alone")
        } else {
            format!("restored {n} existing test file(s) whose edits hide a failure")
        });
    }
    if !edits.revised.is_empty() {
        parts.push(format!(
            "asking whether the request justifies the changed lines in {} existing test file(s)",
            edits.revised.len()
        ));
    }
    format!("{} — pushing back once", parts.join("; "))
}
