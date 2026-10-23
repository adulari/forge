# Feature: the bridge's completeness review runs only when it can find something

> Status: **SHIPPED** (2026-10-22). Code: `crates/forge-core/src/review_gate.rs` (the decision),
> `crates/forge-core/src/completeness.rs` (`completeness_nudge`),
> `crates/forge-core/src/model_loop.rs` (the bridge-yield branch that fires it),
> `crates/forge-core/src/completion.rs` (`VerificationLedger::checked_since_edit`),
> `crates/forge-core/src/working_tree.rs` (what counts as a change),
> `crates/forge-provider/src/cli_provider/cli_stream.rs` (`tool_call_succeeded`). Tests:
> `review_gate::tests`, `tests::completeness_gate`, `completion::tests`, `working_tree::tests`.
> Follow-up of 2026-10-23 (codex bridge): see "What the codex bridge showed" below.

## 1. Problem

When a CLI bridge (claude, codex) finishes a turn that changed files, Forge injects one more user
message ("Before finishing, do ONE final review ...") and re-drives the bridge, which re-sends the
whole conversation and tool surface. The native loop has no such pass. In the baseline parity run
(`docs/benchmarks/claude-code-parity-2026-10.md`) it ran in 71 of 78 bridged turns (48 of 52 haiku)
and cost 8 to 15% of the bridged wall time, 22 to 31% of its input tokens and 18 to 28% of its model
calls. It changed file content in about 2 of the 71, and no pass/fail outcome flipped.

Two causes made it fire almost always:

- The "long request" signal split sentences on every `.`, so `config/limits.conf` and
  `./check-config.sh` each ended a sentence. A two-sentence request counted as four, and the
  threshold was three.
- It looked at `git diff HEAD`. A model that commits its own work (the default `unit` commit
  policy told it to) leaves that diff empty: 44 of 48 haiku turns had committed before the review,
  and in 29 of those 44 the model went to `git show` / `git log` / `HEAD~` instead (5 of 9 for
  sonnet). The first inspection came back completely empty in 11 haiku reviews.

## 2. What it does now

The review still requires that the turn made a net change to the tree since it started. It then
needs one of these signals, checked in this order:

| Signal | Rule |
|---|---|
| failing check | a test, lint, typecheck or build failed and has not passed since |
| hedged answer | the answer says something is untested, unverified, not yet done, a TODO, and so on (phrases, not bare words: "no tests were skipped" is not a hedge) |
| enumerated requirements | the request has three or more bulleted or numbered items |
| unverified multi-requirement | two listed items, or five prose sentences, and no check passed after the last edit |
| large change | three or more files or 60 changed lines this turn changed, measured against the turn's starting commit; untracked files count only when a file tool wrote something, and test-run byproducts (`__pycache__/`, `node_modules/`) never do (section 3) |

Prose is counted outside fenced code, with inline code reduced to a word, and a terminator only
ends a sentence before whitespace, so file names and decimals do not split one. The size of the
change and the text the model is told to inspect are both measured against the commit the turn
started from: the nudge says `git diff <start-sha>`, which includes the model's own commits, and
falls back to `git diff HEAD` only in a repository with no commit at turn start.

`VerificationLedger::checked_since_edit` is stricter than the completion gate's "verified since":
only a recognised check (the same families the gate uses) counts, a read does not, and a later edit
makes the last check stale. An edit made through a shell command (`sed -i`) is not an edit to the
ledger, so a passing check followed by a shell edit still reads as checked.

`lean_context` keeps its own, looser prompt-shape check (`request_lists_multiple_requirements`);
it decides which optional context to inject, not whether to review.

## 3. What the codex bridge showed (2026-10-23)

`claude-cli::` and `codex-cli::` share one branch of the loop (`forge_provider::is_cli_bridge`), so
the gate above was already the codex bridge's gate. It still ran in most runs: 43 completeness
reviews in 52 sessions on `codex-cli::gpt-6-luna`, inside 57 `exec resume` turns that were 64
percent of the arm's input tokens (`results/codex-v1.md` in the benchmark kit; the rest of the
resumes are guard and verification re-drives). Eight live runs with the gate's decision logged
(`RUST_LOG=forge_core::review_gate=debug`) found four defects in what the gate *measured*, none in
what it *decides*:

| Defect | Where it showed | Fix |
|---|---|---|
| Untracked test-run byproducts counted as changed files. A repository without a `.gitignore` lists `__pycache__/*.pyc` as untracked, so a two-line fix plus one pytest run was a four-file change. The review then showed the model the `__pycache__/` in `git status`, and it spent the review and the next verification turn deleting them (10 `delete_file` calls, each one staling the passing test run), then re-ran the tests, which recreated them: a01 took three extra turns for a two-line fix | a01 | `working_tree::is_run_artifact` (`__pycache__`, `.pytest_cache`, `.mypy_cache`, `.ruff_cache`, `.tox`, `node_modules`, `.gradle`, `.forge`, `*.pyc`, a top-level `target/`) is skipped by the size measure and the tree fingerprint, and a `delete_file`/`edit_file` whose every path is one no longer stales a passing check |
| Untracked files a command wrote (a server's `.state/` directory, a token file) counted as three edited files | d05 | untracked files count toward the size only when a file tool (`write_file`, `edit_file`, `apply_patch`, ...) changed something this turn (`VerificationLedger::authored`); tracked changes always count |
| Work an earlier turn left uncommitted counted again. The size is measured against the commit the turn started from, so in a session whose earlier turns did not commit, every later turn started large | multi-turn S1-S3 | `TurnBaseline` records, at turn start, which files differed and by how many lines; only the difference since then counts |
| The evidence was wrong in both directions. Forge's `shell` answers `shell: exit 101 ...` as an ordinary result, so codex marks the MCP call `completed` and claude `is_error: false`: a failing `cargo test` was recorded as a passing check (25 of 189 shell calls in the baseline exited non-zero), and `FailingCheck` could never fire. And a `commands` batch (`["cargo build", "cargo test"]`, the form Forge's own tool description recommends) was read as one multi-line script with an unchecked exit status, so a batched test run never counted as a check, for the direct path as well | b06 (reviewed as "unchecked", its tests ran in a batch) | `tool_call_succeeded` reads the first line of a bridged `shell` result. `classify_tool` classifies each batch entry on its own (they are separate processes) and `VerificationLedger::observe_result` reads the batch header (`(1/2 ok, stopped at #2)`) to say which entry failed: a failing `git diff --check` after a passing test run is not a failed test, and a failure that cannot be attributed (`(2/3 ok)`) blames nothing |

The signals, their order and their thresholds are unchanged. The measured change is in section 5.

## 4. The answer a review leaves

A completeness review or a verification re-drive (the completion gate's "You reported every task
Done. Before this turn can end, prove it ...") adds a model reply after the answer the user asked
for. The reply is the re-drive's own: "Final review: every requirement is handled", "The test
suite passes". The turn's result is settled by `answer_after_review`:

| The re-drive | The turn's answer |
|---|---|
| left the working tree as it found it (`working_tree::worktree_fingerprint`) | the original answer, alone |
| changed the tree | the original answer, a blank line, then the re-drive's reply |
| said nothing, or repeated the answer | the original answer |
| left a check failing | the re-drive's reply (it says what is still broken) |

The fingerprint hashes `HEAD`, `git status`, `git diff HEAD` and the bytes of every untracked work
file, and skips what a test run leaves (`is_run_artifact`). Before, an untracked file counted by
name only (a review that rewrote the new `ANSWER.md` read as "no change") and every
`__pycache__/` read as an edit, so a verification that merely ran the tests replaced the answer.

The gate that starts a verification re-drive is shared by both loops: an all-Done task list is
re-verified only when this turn ran a tool or changed something (`Session::turn_did_work`). The
direct loop had that rule; the bridge loop did not. Measured on the multi-turn session S1 (seven
prompts, then a memory probe "answer purely from our conversation, without opening any files or
running any commands"), `codex-cli::gpt-6-luna`: the probe was answered correctly (the project
name, both ground rules, the feature before the wrap-up), then Forge saw the finished list of the
earlier turns, asked the model to prove it, the model ran the test suite, a mid-intent nudge and
a completeness review followed, and the turn's result was the last reply's one line, "The
requested verification is complete: `python3 -m unittest discover -v` passed all 19 tests." The
probe scored 0.0 (1.0 for Codex's own loop and Forge's native loop). S2's probe (0.67) went the same
way: the correct answer, then a verification and a review, and the review's text as the result. S3's
(0.8) kept its answer, which itself missed a fact, but also ran the needless verification. After, on
S1: one model call, the answer, probe 1.0.

## 5. Measured (2026-10-23, `codex-cli::gpt-6-luna`, ChatGPT Plus)

Eight tasks, one run each, the same task directories, Forge built from `origin/main` (3d9d857b)
against the same tree with this change (99819478; the batch attribution above was added after and
is covered by unit tests only). `results/runs/fxrev-base` and `fxrev-after` in the benchmark kit.

| task | before: pass, input M, agent s, model calls, extra turns | after |
|---|---|---|
| a01 back-to-back conflicts | pass, 0.92, 50, 4, 3 | pass, 0.14, 18, 1, 0 |
| a03 refactor order pricing | pass, 0.53, 50, 2, 1 | pass, 0.58, 63, 2, 1 |
| a05 mailer options object | **fail**, 0.94, 85, 2, 1 | pass, 0.54, 51, 2, 1 |
| a06 answer retry policy | pass, 0.41, 24, 2, 1 | pass, 0.41, 31, 2, 1 |
| b02 ttlcache expiry trap | pass, 0.53, 41, 2, 1 (test guard) | pass, 0.78, 95, 2, 1 (test guard) |
| b06 semver regression | pass, 0.59, 45, 2, 1 | fail, 0.81, 66, 2, 1; rerun pass, 0.56, 43, 2, 1 |
| d02 huge output needle | pass, 0.15, 16, 1, 0 | pass, 0.28, 28, 1, 0 |
| d05 background server roundtrip | pass, 0.40, 38, 2, 1 | pass, 0.14, 26, 1, 0 |
| **total** | 7/8, 4.46M input, 348 s, 17 calls, 9 extra turns | 7/8 (8/8 with the b06 rerun), 3.67M input, 378 s, 13 calls, 5 extra turns |

The two reviews that went (a01, d05) were the two the gate should not have run: 0.92M to 0.14M and
0.40M to 0.14M input tokens, 50 s to 18 s and 38 s to 26 s. The other four reviews (a03, a05, a06,
b06) are the gate working as designed (three or more listed requirements, a large change); a06's
is the one a doc-only deliverable still earns. Wall time over eight tasks is not a result:
the machine ran at load 20-30 and b02's second turn (the existing-tests guard restoring a test the
model had reordered, not a review) took 95 s against 41 s. b06's failure is the model naming the
build-metadata commit instead of the one that broke the ordering (`FAIL: the Unreleased changelog
section does not cite the offending commit (c373db9)`), before any review; the rerun named the
right one. a05 passed because the tests are no longer declared immutable
([existing-test-guard.md](existing-test-guard.md)); on the native loop (`codex-oauth::gpt-6-luna`) it
still passes (0.21M input tokens, 84 s), without the contradicting message. The final head
(eca4eda2) passes a01, a05 and b06 again: 0.20M input tokens and 20 s with no review, 1.16M and
76 s, 0.75M and 47 s.

## 6. Evidence and limits

Replaying the gate over the 71 baseline reviews (from each transcript's tool calls and the request
text; the workspace diff could not be recomputed, so the number of files the model edited stands in
for it) keeps 35 of 71: 13 on size, 13 on three or more listed requirements, 6 on an unchecked
multi-requirement request and 3 on a hedged answer. Of the 24 reviews on the small tasks (a01, b08,
d01 to d03, d05 to d08) it keeps 1. The review turn's own cost there was a median 5.0 s.

The one baseline review that edited a file (haiku d03) is among those dropped, and the task passed
in both seeds anyway; Claude Code solves it with no review. A review that would catch a missed
requirement in a small, checked, single-file change is no longer run: that is the trade.

A live check on two small tasks (d06, d07; `claude-cli::haiku`, one run each, a debug build against
the release baseline binary) ran no review (2 of 2 before) and used 15 model calls against 18 and
292k input tokens against 364k.

## 7. The change travels with the review

When the gate says review, the nudge used to tell the model to run `git status --short` and
`git diff <start>` and read each new file, so the model's first step in the review was a tool call
whose only purpose was to see what Forge could have shown it: a full request (the whole context
again) before the review itself could begin. `review_evidence::gather` now runs the same two
commands, minus the test byproducts, adds the content of each new untracked file, and puts the
output in the nudge inside `<change>` tags, marked as data to check and not instructions. The text
then says "Do not run those commands again". A change too big to carry (more than 12,000 bytes of
evidence, or a new file over 6,000 bytes) keeps the old wording, which sends the model to fetch it.

Measured on `codex-cli::gpt-6-luna` (`results/runs/codex-a2`, 52 tasks, against 13 tasks on the
build with this change; review turns are the `codex exec resume` that carries the nudge):

| | review turns | model calls per review | tool calls per review | reviews that made no tool call |
|---|---|---|---|---|
| before (two seeds) | 17 and 16 | 2.4 and 3.3 | 1.4 and 2.3 | 0 and 0 |
| with the evidence | 10 (9 carried it) | 1.4 | 0.4 | 7 of 10 |

and on `claude-cli::sonnet` (a02, a05, a08 and b04, reviewed in both baseline seeds), the assistant
events of the review turn: a02 3 and 3, a05 2 and 2, a08 3 and 2, b04 3 and 3 before; a02 1, a05 2,
a08 1, b04 2 with the evidence (a05's change was too big to carry and kept the old wording, so it
made its one tool call; the other three made none). The review's own reading of the change is
unchanged; what went is the round trip before it. The gate is untouched: the same turns are
reviewed.
