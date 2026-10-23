# Existing-tests guard and migration guidance

Two quality guards keep a turn from "fixing" the wrong thing: one stops it from making a failing
test disappear, the other stops a rename from turning into compatibility work nobody asked for.
Code: `crates/forge-core/src/test_guard.rs`, `crates/forge-core/src/completeness.rs`.

## The existing-tests guard

A model that is stuck on a failing test has two ways out that are not fixes: delete or skip the
test, or change what it expects until it agrees with the code. The second one is also what a
correct change looks like when the request itself changes the behavior the test pins down, and
the guard cannot tell the two apart from the diff alone. So it does not try to. It puts back
what only ever hides a failure, and it asks about the rest.

Nothing the guard says to the model claims how the work will be judged. An earlier version told
the model that "hidden evaluation uses the ORIGINAL tests". Forge cannot know that. In SWE-bench
the tests that count are the upstream tests, which are updated when the issue changes behavior,
and the claim sent a model back to an old message that its own correct change had just replaced.

Before a turn completes, the guard compares the tree with the commit the turn started on and sorts
every changed tracked test file.

### What arms it

| Signal | Level | What happens |
|---|---|---|
| A person says to leave the existing tests alone: "don't edit/change/weaken the existing tests", "leave the existing tests alone/untouched", "without changing the tests", "keep the existing tests unchanged" | `Frozen` | any modified or deleted tracked test file is put back, an appended test included |
| `expect_code_change` (`bench swe`, a worktree build session), or one sentence that names tests and wants them green ("the existing tests must keep passing", "keep `go test -race ./...` green", "make sure `cargo test` passes") in an unattended run | `Guarded` | see below |
| The same signals, but the request also asks for test changes ("update the tests ...", "fix the failing test", "keep the existing tests (updated for the new signature)") | `Concealing` | only the edits that hide a failure are put back |

An explicit instruction to change tests wins over a prohibition the guard inferred. The two cannot
both hold (a signature refactor has to change the call sites in the tests). Across a session the
latest request that speaks to the tests decides: "don't edit the existing tests" protects them
through the unrelated messages that follow, a later "now update the tests" lifts that, and a later
"leave the tests alone" restores it. A path or file name (`test/helpers.ts`) is not the tests, and
the object of a prohibition ends at ", and ..." or ", but ...", so "don't touch `package.json`,
and keep the existing tests green" prohibits nothing about the tests. A prompt that expects tests
to change (a rename, a refactor, a deprecation) does not arm the green-tests signal. Only what the
person typed counts: the harness stores its own nudges as user messages too, and the push-back
after a restore ("fix the production code rather than the tests") used to read as a request to
change tests and lift the protection it had just enforced.

**"Pass" has to be the verb.** "Final pass: rerun everything and confirm each change is covered
by a test" is a request for a review round, and the noun "pass" used to count as "the tests must
pass". In an unattended run that armed `Guarded` for the rest of the session and added the
"existing tests describe the current behavior" text to the turn. When the model had widened an
existing import line of a test file in an earlier turn (3 of 5 runs of the multi-turn suite's
session S3 on `cb/deepseek-v4.1-flash`), the end-of-turn check then restored that line from the
turn-start commit, even though the model had just put the original back, and sent it round a second
time: 11 to 14 model calls and 330 to 475 k prompt tokens for that last prompt, against 3 to 6 calls
and 62 to 176 k for the same five seeds with the fix (Claude Code: 4 calls, 73 to 112 k).
A "pass" after an article, an ordinal or a quantity word ("final", "a quick", "one more", "second",
"review", "extra") is a round of work and arms nothing; "the tests pass", "must pass", "it passes",
"keep passing" still do. The same sentence with "passing" or "green" in it still arms the guard.

A turn that is armed measures its edits against the commit it started on, so a test file the model
rewrote in an earlier, unarmed turn is the baseline, not a violation. That also means an armed
turn that puts such a line back to its original text shows a changed line against the turn start
and is sorted like any other edit (below). The guard does not look further back than the turn.

### What `Guarded` does with an edit

| The turn did to an existing test file | Result |
|---|---|
| appended or inserted tests, a new test file, a reorder, a re-indent | nothing |
| deleted or moved the file | put back |
| removed more assertions than it added | put back |
| added a skip, xfail, ignore or todo marker | put back |
| added an assertion that cannot fail (`assert True`, `... or True`, `expect(true).toBe(true)`) | put back |
| changed lines the test already had, **and** changed code outside the tests (prose files do not count) | **kept**; the model is asked once |
| changed lines the test already had, and nothing but tests changed | put back |

The skip-marker and assertion patterns are the ones `bench integrity` scores with
(`forge_core::is_skip_marker_line`, `forge_core::is_assertion_line`), so the live guard and the
scorer agree on what hides a failure.

What a kept edit is asked, once, with the changed lines quoted (at most six files, 24 lines each):

> These existing tests have changed lines, and the edits are still in place: ...
> Does the request change the behavior these tests pin down? If it does, keep the edit and name
> the old and the new expectation in your answer. If it does not, the old test is still the
> specification: put it back with `git checkout <turn start> -- <files>` and make the code satisfy it.

Doing nothing keeps the edit, which is the point: the previous version put the old test back, so a
model that had made a correct change to the message and its test reverted its own fix to satisfy
the restored test.

The message for a put-back says why per file, that the files are back at the state they had when the
turn started, that the edits are saved as a patch (`git apply <patch>`), and that if the request
itself changes what a test expects, only that changed expectation should be re-applied, with the
test and the reason named in the answer. `Frozen` says the request asked for the tests to be left as
they are, and that if it cannot be met without changing a test, the answer should say which and why.

`mesh.guard_test_edits = false` turns the guard off. It is skipped past the soft turn deadline. It
runs once per turn and leaves whatever state the model chose afterwards, except that the completeness
audit (below) puts hiding edits back again before the patch is captured.

### Why the commit matters

The harness asks the model to commit each verified unit, so by the end of a turn a rewritten test
is usually committed and `git status` is clean. Detection runs `git diff --name-status <turn-start
HEAD>` (the turn's `TaskScope.base_head`), which sees staged, unstaged and committed changes. A
changed line is a line that one version has more often than the other (trimmed, as a multiset), so
a moved or re-indented line is not a change.

### Restoring

The model's edits, committed or not, are written as a patch (`git diff --binary <turn-start HEAD>`
over the restored paths) to `<state>/test-edits/<n>.patch` in the workspace's folder under Forge's
data directory (see [workspace-state.md](workspace-state.md)). The files are then checked out from
the turn-start commit. Nothing is written into the repository: no `refs/stash`, no entry on the
user's own stash stack (an earlier version used `git stash push`, which left `refs/stash` in 8 of
10 benchmark repos and would have handed the user Forge's test edits at their next `git stash
pop`), no object and nothing under `.git/`. If the patch cannot be written or git refuses the
checkout, the tree is left untouched and nothing is claimed.

### Proactive guidance

When the prompt arms the guard, one system message is added once per session. `Frozen` gets "the
existing tracked tests are immutable, put coverage in a new file, and if the request cannot be met
without changing one, stop and say which and why". The green-tests signal gets: the existing tests
describe the current behavior; if your change makes one fail, decide which side is wrong; when the
request changes that behavior, update that test's expected value minimally and say which test and why
in the final answer; otherwise fix the code; never delete a test or an assertion, skip or xfail one,
or loosen one so a failure disappears; put new tests in a new test file where you can. The older text
forbade rewriting what a test "already contains", which left a task that loosens one assertion
(a summary that gains a block under a test that compares the whole output) with no way to finish
green.

### What else changed with it

- `MINIMAL_DIFF_BIAS`, appended to every `expect_code_change` turn, no longer says "hidden tests
  assert on unchanged surrounding behavior"; it says callers and tests depend on it. Worktree build
  sessions get it too, and there is no hidden test there.
- The completeness audit prompts said "hidden tests often exercise an omitted sibling path" and
  forbade editing tests outright. They now say the existing tests often do not exercise that path,
  and forbid skipping, deleting or weakening tests.

## Identifier-migration guidance

A prompt that mentions a rename, deprecation or alias adds a bounded production-scope sweep
before editing (two plain-literal searches, open every unedited sibling) and a completeness
audit after it. The sweep is what makes `docs/usage` / `--help` / env-var drift tasks pass and is
unchanged.

The alias rule is narrower than it was:

- **A deprecation**: a path that still consumes the old name is a concrete omission until it
  prefers the replacement and keeps the old name as a fallback (unless removal is explicit).
- **A plain rename**: no alias unless the task asks for compatibility.
- **A frozen or public name** (a compatibility or wire-format document, an API contract,
  migrations, changelog history): not an omission. Leave the name, and never edit the file that
  declares it.

The exemption is one macro (`frozen_contract!`) shared by the first-pass guidance, the audit and
its retry prompts, so the texts cannot drift apart.

A rename that moves a file is not a migration. The atomic-write idiom ("written to a temporary
file, then renamed into place", "renamed over", `os.rename`, `fs::rename`, `rename(2)`) is
removed before the prompt is classified (`FILE_RENAME_IDIOMS` in
`crates/forge-core/src/completeness.rs`). Before, a read-only question about which temporary
file dulwich renames into place got the migration sweep guidance, the completeness audit and its
retry: about 40 s of gate rounds per run on DeepSeek, and a first-pass instruction to sweep for
"old identifiers" that the question did not have.

The audit's evidence readers (`completeness_repository_search_ran`, the primary-sweep matches,
the opened-but-unchanged paths) read top-level tool calls only, so a sweep done inside
`run_code` is invisible to them and the "you skipped the REQUIRED repository search" retry
follows nearly every audit on DeepSeek. That is left as it is on purpose. Making the readers see
script calls (measured 2026-10-23, `inferhub::cb/deepseek-v4.1-flash`) dropped the retry, but
the retry's extra repository-root grep is what caught a stale README line on the docs-drift task
(solved 2/5 with the script-aware readers against 5/5 without), and the unresolved-path rule,
once it could see scripts, told the model twice to rename the same-named unrelated functions
the rename task says to keep. A cheaper replacement for that second look has to come first.
