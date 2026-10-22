# Feature: the bridge's completeness review runs only when it can find something

> Status: **SHIPPED** (2026-10-22). Code: `crates/forge-core/src/review_gate.rs` (the decision),
> `crates/forge-core/src/completeness.rs` (`completeness_nudge`),
> `crates/forge-core/src/model_loop.rs` (the bridge-yield branch that fires it),
> `crates/forge-core/src/completion.rs` (`VerificationLedger::checked_since_edit`). Tests:
> `review_gate::tests`, `tests::completeness_gate`.

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
| large change | three or more files or 60 changed lines since the turn's starting commit, untracked files included |

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

## 3. Evidence and limits

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
