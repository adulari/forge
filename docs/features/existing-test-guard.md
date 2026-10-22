# Existing-tests guard and migration guidance

Two quality guards keep a turn from "fixing" the wrong thing: one stops it from rewriting the
tests it was told to keep, the other stops a rename from turning into compatibility work nobody
asked for. Code: `crates/forge-core/src/test_guard.rs`, `crates/forge-core/src/completeness.rs`.

## The existing-tests guard

A turn that rewrites an existing test to match its own behavior fails any evaluation that runs
the original tests, and breaks the promise in "keep the existing tests green". Before a turn
completes, the guard checks the tree against the commit the turn started on, puts the originals
back, and pushes back once ("re-verify against the originals; fix the production code rather
than the tests").

### What arms it

| Signal | Level | What counts |
|---|---|---|
| `expect_code_change` (`bench swe`), or an explicit prohibition: "don't edit/change/weaken the existing tests", "leave the existing tests alone/untouched", "without changing the tests", "keep the existing tests unchanged" | `AnyEdit` | any modified or deleted tracked test file, an appended test included |
| One sentence that names tests and wants them green: "the existing tests must keep passing", "keep the tests green", "keep `go test -race ./...` green", "get `cargo test` green", "make sure `cargo test` passes" | `Rewrites` | existing content that is no longer a byte prefix of the file (a changed line, a test inserted before another, a removed test, a deleted or moved file); a test appended to the end of a file and a new test file pass |

A prompt that expects tests to change ("update the tests ...", "fix the failing test", a refactor,
a rename, a deprecation) does not arm `Rewrites`: there the tests are renamed along with the
code. The strongest signal in any user message of the session applies to the whole session.

`mesh.guard_test_edits = false` turns the guard off. It is skipped past the soft turn deadline.

### Why the commit matters

The harness asks the model to commit each verified unit, so by the end of a turn a rewritten test
is usually committed and `git status` is clean. Detection runs `git diff --name-status <turn-start
HEAD>` (the turn's `TaskScope.base_head`), which sees staged, unstaged and committed changes.

### Restoring

The model's edits, committed or not, are written as a patch (`git diff --binary <turn-start HEAD>`
over the test paths) to `<state>/test-edits/<n>.patch` in the workspace's folder under Forge's data
directory (see [workspace-state.md](workspace-state.md)). The files are then checked out from the
turn-start commit, and the push-back says `git apply <patch>` re-applies them. Nothing is written
into the repository: no `refs/stash`, no entry on the user's own stash stack (an earlier version
used `git stash push`, which left `refs/stash` in 8 of 10 benchmark repos and would have handed the
user Forge's test edits at their next `git stash pop`), no object and nothing under `.git/`. If the
patch cannot be written or git refuses the checkout, the tree is left untouched and the guard does
not fire.

The push-back for `expect_code_change` keeps the "hidden evaluation uses the original tests"
wording. For a person's request it says the request asked for the tests to be kept and does not
mention an evaluation.

### Proactive guidance

When the prompt arms the guard, one system message is added once per session: for `AnyEdit` the
existing "tracked test files are immutable, put coverage in a new file" text, for `Rewrites` a
shorter text that says appending or a new file is fine and rewriting is not.

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
