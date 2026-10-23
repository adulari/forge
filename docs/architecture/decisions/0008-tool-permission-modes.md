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
grep egrep fgrep rg find sed git printf column rev fold expand comm paste nproc whoami uname id df
file hostname`) written plainly, with no argument that writes, runs code or reaches the network:

- `find` without `-delete -exec -execdir -ok -okdir -fprint* -fls -rm`; `sort` without `-o`,
  `--compress-program`, `-T`; `uniq` with at most one operand, where `-` and everything after `--`
  count as operands (`uniq - out` writes `out`); `wc`, `du` and `sort` without `--files0-from`;
  the checksum tools without `--check`; `rg` without `--pre`, `--hostname-bin`, `-z`; `tail`
  without `-f`; `diff` without `-l`, `--from-file`, `--to-file`; `sed` only as
  `sed -n 'N,Mp' [files]`; `file` without `-C -m -f`; `hostname` only as a lookup;
- `git` only `status diff log show branch rev-parse ls-files blame grep ls-tree rev-list shortlog
  describe merge-base show-ref for-each-ref name-rev`, plus `tag` in its listing forms, `reflog`
  and `reflog show`, `stash list` and `worktree list`, with `branch` limited to its listing
  forms. Rejected: `-c`, `--output`, `--ext-diff`, `--textconv`, `--open-files-in-pager` and `-O`
  anywhere in a cluster of short flags (`git grep -nOsh x` runs `sh`), `--exec*`, `--config-env`,
  `--no-index`, `--submodule`, `--recurse-submodules`, `--show-signature` and any `%G` placeholder
  (they run `gpg` over a signature the repository supplied), and any global option before the
  subcommand (`git -C dir diff` asks). `git remote -v` asks too, since a remote URL often carries
  a token. Long options are matched as getopt-style abbreviations as well, since `sort --out=x` is
  `sort --output=x`, except for the exact options that merely start like one (`git diff --text`).
- No redirect except `2>&1`-style descriptor copies (`>&1file` is a redirect into a file and
  asks) and `>/dev/null`; no `<`, here-documents or
  here-strings; no command, process or parameter substitution (`$`, backtick, `<(`, `>(`); no
  subshell, group or brace expansion; no backgrounding; no environment-assignment prefix (the
  first word of a statement must be an allow-listed binary); no wrapper binary (`env`, `command`,
  `nohup`, `time`, `exec`), no binary named by a path, no quoted command word; no control
  character, so a command with a newline in it asks (a `commands` batch is judged entry by
  entry). `background`, `poll_until_exit_zero` and `pty` runs keep their prompt.
- Pipelines and chains qualify only when each stage does.
- A path guard over the arguments applies the secret globs of the `read_file` deny list (with its
  `.env.example` exception), the configured `read_file` denies and `/proc/*/environ`,
  `/proc/*/cmdline`, `.aws`, `.kube`, `.docker`, to every path an argument names: through `~`, the
  working directory, symlinks, `--flag=path`, `-fpath`, `rev:path`, and wildcards expanded against
  the file system the way `sh` would (a match that the command would read as an option, such as a
  file named `--output=x`, asks, as does a pattern with more than three wildcard components). So
  `cat .env` is still denied by the built-in rule, and `wc -c .env`, which no deny verb lists,
  asks.
- A command that reads every file under a directory is confined, not just screened, because it
  reads what it meets rather than what it was told to read and no list of secret names is
  complete. `grep -r/-R/-d recurse` (and abbreviations of the action, `-d rec`), `diff -r` and
  `diff DIR DIR` (which compares two directories file by file without `-r`), `rg` of any kind and
  `git grep --untracked` qualify only when every root they search lies inside the workspace,
  not above or at the home directory, and holds no secret file, symlinks followed and a symlink
  that leaves the workspace unprovable. A plain `rg` skips hidden files and directories, so a
  hidden `.env` does not stop it, but a visible `server.pem`, `*.key` or `id_rsa` does.
  The walk is bounded (50,000 entries, 250 ms); past the bound the call asks.
- `git` runs only in a repository whose own files run nothing (`ask_shell/git_repo.rs`). Git
  runs helpers a repository names: `core.fsmonitor` on `git status`, a `textconv` or external diff
  driver on `git diff`/`show`/`blame`, a pager, the `post-index-change` hook, a promisor remote
  that fetches over the network. Those files can arrive with a checkout (an unpacked archive's
  `.git`, or a committed directory shaped like a bare repository, which git adopts as soon as the
  call's `cwd` is inside it). So the repository git would find from the working directory is
  located the way git locates it (`.git` directory or `gitdir:` file, else a bare-shaped
  directory), and its config, worktree config, and the config of every submodule repository must
  contain only keys on a short list of inert ones (`core.bare`, `remote.*.url`, `branch.*.merge`,
  ...; `core.fsmonitor` only as `true`/`false`). Any other key or section, an `include`, an
  oversized or unreadable file, or a `post-index-change` or `reference-transaction` hook is a
  refusal. Outside a repository `git` asks too, because it stops being a repository tool there
  (`git grep` is `--no-index`, `git diff A B` is `git diff --no-index A B`); and inside one,
  `git diff` asks when an operand lies outside the work tree, for the same reason.

**Rationale.** Claude Code does not prompt for these commands, and the prompt buys little: the
calls cannot change anything, and the ones that can still prompt. In the scripted TUI benchmark
(s04, file edit) Claude Code asked once, for the edit; Forge's Ask asked for the edit and then for
one or two shell calls in real-backend runs (2 and 3 prompts, against 1). The rule removes the
shell prompts that only read, and leaves a commit or a test run asking. A user who answers
"yes" to every `git status` learns to answer "yes" to everything, which defeats the prompts that
matter. Auto-allowing the provably harmless class keeps the prompt for the rest.

**Limits.** This is a floor against accidents and lazy prompt injection, not a sandbox, like the
rest of the broker.
- The user's own global and system git config and the process environment are trusted: a
  `diff.<driver>.textconv` or `core.fsmonitor` there still runs under `git diff`/`git status`,
  and a driver those files define runs on whatever attributes a repository selects. What a
  checkout brings with it is what `git_repo.rs` reads. `git status` may refresh `.git/index`.
  Some repositories ask for want of a key the list does not know (`core.hooksPath` from husky,
  an LFS filter, `http.*.extraheader` from CI): that is a prompt, not a hole.
- Secrets are judged by the paths a command names or walks, not by what `git` can print from the
  repository's history. `git log -p`, `git show`, `git diff` and `git grep` over tracked files can
  show a file that was once committed, or a tracked `.env` that was modified: `.env` named on the
  line, or any `-- path` pathspec, is checked, but a pathless command is not. Those bytes are
  repository content that every clone already holds; the guard exists for the untracked local
  files (`.env`, keys) that only this machine has, which `git` does not print.
- Reading a named file is not confined to the workspace: a path argument may point anywhere
  readable, subject to the secret guard, as with `read_file`. Only commands that sweep trees are
  confined.
- Wildcards, `~` and symlinks are resolved when the call is judged, not when it runs.
- The Read-only temper keeps its own, looser classifier (`permission/readonly_shell.rs`), which
  accepts for instance `echo $(pwd)`, has no path guard, and does not judge a repository's config.
  The two share their per-binary argument rules and the redirect stripper, so the rules fixed here
  (`uniq` operands, clustered `-O`, `%G`, abbreviated hostile options, `>&1file`) tightened
  Read-only too.

**Review findings, 2026-10** (three independent reviewers attacked the first version; all closed).
`uniq - out` wrote `out` because `-` was not counted as an operand; `git grep -nOsh x` ran `sh`
because `-O` was only recognised first in a word; a committed directory shaped like a bare
repository ran a `textconv` command and could trigger a network fetch; `git diff --no-index` and
`git diff DIR DIR` printed arbitrary trees, hidden files included, and a plain `rg` read visible
secret files (`*.pem`, `id_rsa`) and, with `-L`, followed symlinks to `.env`; `grep -d rec`
recursed without being recognised; a recursive search rooted below `$HOME` outside the workspace
passed the home check; `.git/config` could carry `core.fsmonitor`. A further hole found while
fixing them: `>&1file` passed the redirect stripper as `>&1` and wrote `1file`. Each has a
regression test in `ask_shell/tests.rs`, `ask_shell/git_repo/tests.rs` and `readonly_shell.rs`.
Not closed, by design: the history content above.

**Alternatives rejected.** Reusing the Read-only classifier unchanged (it admits command
substitution and `jq`, which can print the environment); asking the `auto` temper's classifier model
(a model call and latency on every `ls`, and not deterministic); leaving it to per-user `allow`
rules (every user would rediscover the same twenty patterns).
