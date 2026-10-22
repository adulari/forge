//! Built-in review commands, named as in Claude Code: `/review` (alias `/code-review`) and
//! `/security-review`. Like the other builtins, a user or project command of the same name
//! replaces them, so an imported Claude Code skill keeps working as it did.

use std::path::PathBuf;

use forge_types::TaskTier;

use crate::{Command, Invocation, Scope};

const REVIEW_BODY: &str = "\
Review code changes for correctness bugs. Report findings; do not edit files unless the target \
below contains `--fix`.

Target: $ARGUMENTS
- Nothing → the uncommitted changes (`git diff HEAD` plus untracked files); when the tree is \
clean, the current branch against its upstream or the default branch.
- A number → that pull request (`gh pr view <n>`, `gh pr diff <n>`).
- A branch → `git diff <default-branch>...<branch>`.
- A path → the changes under that path.

Method:
1. Get the diff, then read the surrounding code each change touches (callers, types, tests). \
Never review from the diff alone.
2. Hunt for real defects: wrong logic, broken edge cases (empty input, None, overflow, \
off-by-one), error paths that swallow or misreport failures, races, leaks, API misuse, behaviour \
changes callers or tests do not expect, and missing tests for changed behaviour.
3. Verify every candidate before reporting it: trace the path, and run a focused test or command \
when that is cheap. Drop what you cannot substantiate.
4. Report the findings most severe first, one per line: `file:line`, what is wrong, the concrete \
input or state that breaks it, and the fix. Say plainly when nothing is worth fixing. With \
`--fix`, apply the verified fixes afterwards and rerun the relevant tests.";

const SECURITY_REVIEW_BODY: &str = "\
Security review of the changes on the current branch. Report findings; do not edit files.

Scope: the diff between this branch and the remote default branch (`git merge-base HEAD \
origin/HEAD`, falling back to origin/main or origin/master, then to the uncommitted changes when \
there is no remote). Narrow it to: $ARGUMENTS

Check for injection (SQL, shell, path traversal, templates, deserialization), authentication and \
authorization mistakes, secrets or tokens in code, logs or config, unvalidated untrusted input \
(SSRF, open redirects, XSS), crypto misuse, insecure defaults or file permissions, risky \
dependency changes, and data exposed through errors or telemetry.

For each candidate, trace the data from its untrusted source to the sink and report only what is \
exploitable or clearly risky: severity (critical/high/medium/low), `file:line`, the attack \
scenario, and the fix. Say \"no security findings\" when none survive that check.";

fn builtin(name: &str, description: &str, body: &str) -> Command {
    Command {
        name: name.to_string(),
        description: description.to_string(),
        args: Vec::new(),
        tier: Some(TaskTier::Complex),
        model: None,
        body: body.to_string(),
        scope: Scope::Builtin,
        path: PathBuf::from(format!("<builtin>/commands/{name}.md")),
        invocation: Invocation::default(),
    }
}

/// `/review`, `/code-review` and `/security-review`.
pub(crate) fn builtin_review_commands() -> Vec<Command> {
    let review = "Review the uncommitted diff, a PR number, a branch or a path for correctness bugs (add --fix to apply them)";
    vec![
        builtin("review", review, REVIEW_BODY),
        builtin("code-review", review, REVIEW_BODY),
        builtin(
            "security-review",
            "Review this branch's changes against the default branch for security vulnerabilities",
            SECURITY_REVIEW_BODY,
        ),
    ]
}
