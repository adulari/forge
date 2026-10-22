# ADR-0008: Tool safety via switchable permission modes + per-tool rules

- **Status:** Accepted
- **Date:** 2026-06-14
- **Deciders:** Floris Voskamp

## Context

Forge tools can write files and execute shell commands on the user's machine driven by a
model's output. This is the single largest security/safety surface (threat model: an LLM
or a prompt-injected file can attempt destructive or exfiltrating actions). FR-10 (with
confirmed A-5) requires **switchable permission modes** plus fine-grained per-tool /
per-project allow-ask-deny rules, defaulting to safe behaviour, and configurable to bypass
for trusted flows.

## Options considered

1. **Single fixed policy (always ask)** — safe but unusably noisy for bulk edits.
2. **Per-tool allow/ask/deny rules only** — flexible but no fast global switch; users must
   reconfigure rules to change posture for a session.
3. **Permission *modes* (global posture) layered over per-tool rules** — a session-level
   mode sets the default posture; per-tool/per-project rules refine it. Mirrors the
   well-understood Claude Code model the user referenced.

## Decision

A **central permission broker** (in the session core) gates every side-effecting tool call.
It resolves a decision from two layers:

1. **Mode** (session-level posture, switchable at runtime and as a configured default):
   - `default` — ask before any side effect (file write/edit, shell).
   - `accept-edits` — auto-allow workspace edits and ordinary shell commands; ask for writes beyond the
  workspace and for shell that deletes, moves or writes outside it (amended 2026-10; the original
  "still ask for shell" never matched the code, which allowed every shell command).
- In every mode, including `bypass`, a structural floor refuses catastrophic deletes of home/root
  and privilege escalation (`sudo` and friends), see `docs/features/temper-modes.md`.
   - `bypass` — auto-allow all tool actions (explicit, deliberate opt-in).
   - `plan` — read-only; deny all side effects (planning/analysis sessions).
2. **Rules** (per-tool / per-project allow | ask | deny) that refine the mode, with an
   explicit precedence: an explicit `deny` rule always wins; otherwise the mode decides;
   `plan` mode overrides any `allow`.

Read-only tools (read/search/list) never prompt. Every gated decision is recorded with the
session (observability) and surfaced through the presenter as a permission event (ADR-0004).

## Rationale

Modes give a one-switch safety posture for different workflows; rules give precision; the
`deny`-wins / `plan`-overrides precedence makes "safe by default, dangerous only on
explicit opt-in" a structural guarantee rather than a convention — directly serving the
security NFR and FR-10. Centralising the check in one broker means no tool can bypass it.

## Consequences

- **Positive:** Safe out of the box; fast posture switching; auditable decisions; a single
  chokepoint to reason about for the threat model.
- **Negative / trade-offs accepted:** `bypass` mode is genuinely dangerous — it must be a
  conscious, clearly-signalled choice (UI warning), never a silent default.
- **Follow-ups:** Define the `Tool` trait's `side_effect` classification, the rule schema
  in `forge-config`, and the path-scoping for rules (e.g. allow writes only within CWD).
  Feed this into the Phase 3 threat model.

## Amendment (2026-10): Ask runs provably read-only shell without a prompt

**Rule.** In the `default` mode (the **Ask** temper), a `shell` call whose every part is provably
read-only runs without a prompt: `git status`, `git diff`, `ls`, `cat file`, `grep -rn x src`,
`rg`, `wc -l`, and pipelines or `&&`/`;` chains of such commands. Everything else in Ask still
asks. `[permissions] auto_allow_read_only_shell = false` turns the rule off (default `true`).

**Where it sits.** It replaces only the *mode fallback* of Ask, the last step of the precedence
above, so everything that outranks the fallback still does: the safety floor, every deny rule (the
secret-read verbs included), the `plan` contract, a published port, and every `ask` or `allow`
rule on a shell segment. `PreToolUse` hooks run before the broker is asked and can still block.
`plan` (Read-only), `accept-edits` (Auto-edit), `auto` and `bypass` (Full) are unchanged. The
rule lives in one place (`permission::mode_fallback`, qualifying logic in
`permission/ask_shell.rs`), which the native loop, the CLI bridge (`forge mcp-serve`), subagents
and the concurrent-batch check all call, so every execution path reaches the same decision. The
setting travels as an inert marker rule from `Config::permission_rules()` for the same reason: it
is the one per-session input every call site already has.

**What qualifies** (fail closed: a gap costs a prompt, never a permission). Every statement of the
line must be one of a short allow-list of binaries (`ls cat head tail wc pwd echo stat du nl tac
cut tr sort uniq basename dirname realpath readlink which md5sum sha1sum sha256sum b3sum diff cmp
grep egrep fgrep rg find sed git`) written plainly, with no argument that writes, runs code or
reaches the network:

- `find` without `-delete -exec -execdir -ok -okdir -fprint* -fls`; `sort` without `-o`,
  `--compress-program`, `-T`; `uniq` with at most one operand; `rg` without `--pre`,
  `--hostname-bin`, `-z`; `tail` without `-f`; `diff` without `-l`; `sed` only as
  `sed -n 'N,Mp' [files]`; `git` only `status diff log show branch rev-parse ls-files blame grep
  ls-tree rev-list shortlog describe merge-base show-ref`, with `branch` limited to its listing
  forms and no `-c`, `--output`, `--ext-diff`, `--textconv`, `-O`, `--open-files-in-pager`,
  `--exec*`, `--config-env`, nor a global option before the subcommand (`git -C dir diff` asks).
  `git remote -v` asks too, since a remote URL often carries a token. Long options are matched as
  getopt-style abbreviations as well, since `sort --out=x` is `sort --output=x`.
- No redirect except `2>&1`-style descriptor copies and `>/dev/null`; no `<`, here-documents or
  here-strings; no command, process or parameter substitution (`$`, backtick, `<(`, `>(`); no
  subshell, group or brace expansion; no backgrounding; no environment-assignment prefix (the
  first word of a statement must be an allow-listed binary); no wrapper binary (`env`, `command`,
  `nohup`, `time`, `exec`), no binary named by a path, no quoted command word; no control
  character, so a command with a newline in it asks (a `commands` batch is judged entry by
  entry). `background`, `poll_until_exit_zero` and `pty` runs keep their prompt.
- Pipelines and chains qualify only when each stage does.
- A path guard over the arguments applies the secret globs of the `read_file` deny list (with its
  `.env.example` exception), the configured `read_file` denies and `/proc/*/environ`, `.aws`,
  `.kube`, `.docker`, to every path an argument names: through `~`, the working directory,
  symlinks, `--flag=path`, `-fpath`, `rev:path`, and wildcards expanded against the file system
  the way `sh` would (a match that the command would read as an option, such as a file named
  `--output=x`, asks, as does a pattern with more than three wildcard components). A recursive `grep`, `diff -r`, `rg --hidden/-g/-u` or `git grep --untracked`
  asks if its tree holds a secret file, or if it could reach the home directory. So `cat .env` is
  still denied by the built-in rule, and `wc -c .env`, which no deny verb lists, asks.

**Rationale.** Claude Code does not prompt for these commands, and the prompt buys little: the
calls cannot change anything, and the ones that can still prompt. In the scripted TUI benchmark
(s04, file edit) Claude Code asked once, for the edit; Forge's Ask asked for the edit and then for
one or two shell calls in real-backend runs (2 and 3 prompts, against 1). The rule removes the
shell prompts that only read, and leaves a commit or a test run asking. A user who answers
"yes" to every `git status` learns to answer "yes" to everything, which defeats the prompts that
matter. Auto-allowing the provably harmless class keeps the prompt for the rest.

**Limits.** This is a floor against accidents and lazy prompt injection, not a sandbox, like the
rest of the broker.
- Configuration is trusted: a `diff.<driver>.textconv`, `core.fsmonitor` or `diff.external` in the
  user's or repository's git config still runs under `git diff`/`git status`. `git status` may
  refresh `.git/index`. A `git` command with `-c`/`-C` asks, but one that merely relies on config
  does not.
- Secrets are judged by the paths a command names or walks, not by what `git` can print from
  history: `git log -p` and `git grep` over tracked files can show a file that was once committed
  (`.env` named on the line, or any `-- path` pathspec, is checked). The recursive-search walk is
  bounded (50,000 entries, 250 ms); past the bound the call asks, so `grep -rn x .` in a repository
  with a large untracked tree asks.
- Reading is not confined to the workspace: a path argument may point anywhere readable, subject to
  the secret guard.
- Wildcards, `~` and symlinks are resolved when the call is judged, not when it runs.
- The Read-only temper keeps its own, looser classifier (`permission/readonly_shell.rs`), which
  accepts for instance `echo $(pwd)`. The two share their per-binary argument rules, which now
  also reject abbreviated forms of the hostile options (this tightened Read-only too).

**Alternatives rejected.** Reusing the Read-only classifier unchanged (it admits command
substitution and `jq`, which can print the environment); asking the `auto` temper's classifier model
(a model call and latency on every `ls`, and not deterministic); leaving it to per-user `allow`
rules (every user would rediscover the same twenty patterns).
