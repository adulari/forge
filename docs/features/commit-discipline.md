# Feature: commit discipline — the model commits the work it does

> Status: **SHIPPED** (2026-09-08). Code: `crates/forge-core/src/git_hygiene.rs`, the
> `Version control` section of `FORGE_SYSTEM`, `/commit` in `crates/forge-tui/src/commands.rs`.

## 1. Problem

A model left alone in a repository edits for hours and never commits. Measured on one real
session before this existed (six days, 24k messages, 1.7 B input tokens):

| | |
|---|---|
| successful edits (`edit_file` / `write_file` / `multi_edit` / `apply_patch`) | 2,600 |
| `git commit` calls | 49 |
| `git push` calls | 2 |
| days with **zero** commits | 2 (410 edits on those days) |
| dirty files when inspected | 48, plus 9 unpushed commits |

Nothing in the harness ever mentioned the working tree to the model, so nothing ever prompted the
commit. Claude Code gets this "for free" because its models are trained to commit and its
harness re-injects git status; Forge runs many models that are not, so the harness has to carry
the habit.

## 2. What ships

Three layers, cheapest first. None of them commit or push on the model's behalf: the model runs
`git` through the ordinary shell tool and the same permission broker as any other command.

1. **System prompt.** `FORGE_SYSTEM` gained a `Version control` section: commit each verified
   unit of work as you finish it, stage the specific files (never `git add -A`), conventional
   commit messages, never end a long task uncommitted, never push / force-push / rewrite history
   unasked, and when the branch is ahead of its upstream ask the user (via `ask_user`) whether to
   push.
2. **Turn-start reminder.** While any file *this session* edited is still uncommitted, the turn's
   context carries one system line naming them (`[git] 3 files you edited in this session are
   still uncommitted: …`). Bundled with the prompt, so it costs no extra model call. If the
   branch is also `push_nudge_ahead` or more commits ahead of its upstream, the same line asks
   the model to offer the user a push. With nothing of ours dirty but the branch ahead, a
   one-per-head push reminder fires instead.
3. **Mid-turn reminder.** Every `commit_nudge_edits` (default 10) successful edits since the last
   commit or reminder, one system hint lands right after the tool result. The loop was going to
   continue anyway, so this is also free. A shell command that can move `HEAD` (`git commit`,
   `git reset`, `gh pr merge`, …) resets the counter and forgets whatever is no longer dirty.

Plus `/commit [hint]`: one turn that asks the model to `git status` / `git diff`, group the
changes into focused conventional commits, stage the specific files, and report the hashes — and
not push. It is in the command palette under *Review & ship* and in the mobile command list.

Rate limiting. The turn-start reminder names the same dirty set at most every third turn; a
changed set is announced at once. Each mid-turn reminder carries the edits since the last commit
(`(20 edits since your last commit)`), which also keeps two reminders for the same file from
being byte-identical: the request view drops an exact repeat of a system message, so the
edit-20 reminder for a single file used to vanish.

`/commit` tells the model to name the paths it stages (never `git add -A`, `git add .` or
`git commit -a`) and never to pass `--no-verify`/`-n`: a failing pre-commit hook is fixed, not
skipped.

## Commit policy (`[git] commit_policy`)

`auto` (the default) is `unit` in an interactive session and `end` in a headless one-shot run
(`forge run`, `forge run -p`, `forge bench`). Nobody reviews per-unit commits there, and each one is
a model round trip: the baseline parity run saw a commit-only call in 25 to 27 of 26 tasks per
bridged haiku seed (50 calls, a median 1.2 s each), where Claude Code committed nothing. `end` keeps
one commit at the end of the task; `never` leaves the tree dirty. An explicit `unit`, `end`, `never`
or `request` in the config always wins over `auto`. A run handed to the daemon (`forge run` without
`--no-publish-to-fleet`) is a daemon session someone may follow, so it stays interactive.

An interactive `auto` session is `unit` only while committing is free for the user. When the
permission broker would stop at the model's own `git add && git commit` (the Ask posture with no
allow rule for them), `auto` resolves to **`request`**: the system prompt tells the model not to
commit on its own initiative, the reminders stay quiet, and `/commit` or "commit this" still
commits. Measured on `forge chat` with a fresh permission config, a one-line fix through the
claude-cli bridge raised three prompts (the edit, the model's own check, the commit); Claude Code
raised one. The commit was the one prompt Forge added by policy. An allow rule for both commands, "Allow
shell for this session", or a posture that runs shell unasked (auto-edit, auto, full) restores `unit`;
the choice is re-read every request, so switching mode mid-session switches the wording at once.
`commit_policy = "unit"` keeps the old behaviour in every posture.

## Commit attribution (`[git] coauthor = true`)

The system prompt promises "a Co-Authored-By: Forge trailer is added automatically by a git hook"
on every surface, so every surface installs it: `forge_core::git_attribution` installs the
`prepare-commit-msg` hook for the session's workspace before the first model request and keeps
`<git dir>/forge-model` current (TUI, headless `forge run` and daemon sessions alike). The hook is
written to `git rev-parse --git-path hooks`, which is the shared hooks directory of a linked
worktree and works in a submodule (whose `.git` is a pointer file). An existing
`prepare-commit-msg` hook is kept as `prepare-commit-msg.pre-forge` and run first, never
overwritten. With `core.hooksPath` set, git ignores `.git/hooks`: Forge installs nothing, says so,
and `forge git hook` prints the script to call from your own hook. The script strips Claude, Codex
and Anthropic co-author lines in any casing, adds the Forge trailer once (an `--amend` does not
stack another), and leaves an empty message empty so git aborts instead of committing a bare
trailer.

### What is deliberately NOT nagged about

- Files the user dirtied themselves. The tracker only knows paths that went through Forge's write
  tools in this session, and it re-checks `git status` before every reminder so a commit made
  outside Forge, a revert, or a `/rewind` that restored the files silences it.
- Repositories without git, or workspaces where `git` is missing: the probe returns nothing and
  the feature is inert.
- Pushing. Forge never pushes; the reminders say "ask the user".

## 3. Configuration

```toml
[git]
commit_nudge = true        # both reminders + the push reminder (default true)
commit_nudge_edits = 10    # mid-turn reminder cadence; 0 = turn-start reminder only
push_nudge_ahead = 3       # commits ahead of upstream before "offer a push"; 0 = never
```

The system-prompt guidance stays on regardless; it is part of what Forge is. A model on the
`minimal` harness profile gets neither reminder (see `harness-profile.md`).

## 4. Behaviour at the seams

- **Resume / daemon restart.** The touched-file set is in-memory; after a restart the turn-start
  reminder is silent until the session edits again. Uncommitted files from before the restart are
  the user's to notice (they show in `git status`), by design — the harness cannot tell them from
  the user's own work.
- **Worktree sessions.** The tracker asks `git rev-parse --show-toplevel` once, so a workspace
  that is a subdirectory of the repository still maps edits to repo-relative paths.
- **Rewind.** `/rewind` restores files from checkpoints; the next reminder re-probes and drops
  anything that is clean again.
- **Cost.** One `git status` per turn start, one per mid-turn reminder, one `git rev-parse HEAD`
  after git-ish shell commands. All run off the async executor (`spawn_blocking`) at turn start.

## 5. Tests

- `git_hygiene::tests` — porcelain parsing (branch, ahead count, renames), the every-N counter,
  commit detection via `HEAD`, unrelated dirty files ignored, push reminder once per head, 0 =
  off, file-list capping.
- `tests::commit_nudge_tests` (`crates/forge-core/src/tests/commit_nudge.rs`) — a real session in
  a temp repository: the mid-turn reminder lands right after the edit that made it due; the next
  turn opens by naming the uncommitted file until it is committed outside Forge; `commit_nudge =
  false` silences everything.
- `tests::commit_nudge_tests::auto_is_unit_until_a_one_shot_run_says_otherwise_and_an_explicit_choice_always_wins`
  and `a_one_shot_default_silences_the_reminders_that_an_interactive_auto_session_gets` — the
  resolved policy decides the system prompt's version-control paragraph and the reminders.
