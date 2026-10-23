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
is the one per-session input every call site already has. The two binaries a repository can
influence, `git` and `file`, additionally need the project to be trusted (`forge trust`; see
"Trust" below): in a project nobody vouched for they keep their prompt.

**What qualifies** (fail closed: a gap costs a prompt, never a permission). Every statement of the
line must be one of a short allow-list of binaries (`ls cat head tail wc pwd echo stat du nl tac
cut tr sort uniq basename dirname realpath readlink which md5sum sha1sum sha256sum b3sum diff cmp
grep egrep fgrep rg find sed git printf column rev fold expand comm paste nproc whoami uname id df
file hostname`) written plainly, with no argument that writes, runs code or reaches the network
(`git` and `file` only in a trusted project):

- `find` without `-delete -exec -execdir -ok -okdir -fprint* -fls -rm`; `sort` without `-o`,
  `--compress-program`, `-T`; `uniq` with at most one operand, where `-` and everything after `--`
  count as operands (`uniq - out` writes `out`); `wc`, `du` and `sort` without `--files0-from`;
  the checksum tools without `--check`; `rg` without `--pre`, `--hostname-bin`, `-z`; `tail`
  without `-f`; `diff` without `-l`, `--from-file`, `--to-file`; `sed` only as
  `sed -n 'N,Mp' [files]`; `file` without `-C -m -f -z -Z`; `hostname` only as a lookup; `printf` only
  with a format first, never an option (bash's builtin `printf -v NAME` assigns a variable,
  evaluates a subscript like `a[$(cmd)]`, and so can replace `PATH` or `HOME` for the rest of the
  line);
- `git` only `status diff log show branch rev-parse ls-files blame grep ls-tree rev-list shortlog
  describe merge-base show-ref for-each-ref name-rev`, plus `tag` in its listing forms, `reflog`
  and `reflog show`, `stash list` and `worktree list`, with `branch` limited to its listing
  forms. Rejected: `-c`, `--output`, `--ext-diff`, `--textconv`, `--open-files-in-pager` and `-O`
  anywhere in a cluster of short flags (`git grep -nOsh x` runs `sh`), `--exec*`, `--config-env`,
  `--no-index`, `--submodule`, `--recurse-submodules`, `--show-signature`, any `%G` placeholder
  and, for `for-each-ref`, any argument naming `signature` (`%(signature)`, `--sort=signature`;
  they run `gpg` over a signature the repository supplied), and any global option before the
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
  file named `--output=x`, asks, as does a pattern with more than three wildcard components, and a
  bracket expression is trusted only as a plain set of letters, digits, `_` and `.`, negated with
  `!` or not, with digit ranges: `[^a]`, `[[:lower:]]`, `[[=e=]]`, letter ranges and backslashes ask,
  because the expansion is the `glob` crate's and `sh` reads those differently). So
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
  The roots are the operands, which include every word after `--` whatever it starts with (a
  committed symlink named `-esc` is a root there). The walk is bounded (50,000 entries, 250 ms);
  past the bound the call asks.
- `git` runs only in a project the user trusted (see "Trust") and, there, only in a repository
  whose own files run nothing (`ask_shell/git_repo.rs`). Git
  runs helpers a repository names: `core.fsmonitor` on `git status`, a `textconv` or external diff
  driver on `git diff`/`show`/`blame`, a pager, the `post-index-change` hook, a promisor remote
  that fetches over the network. Those files can arrive with a checkout (an unpacked archive's
  `.git`, a committed directory shaped like a bare repository, which git adopts as soon as the
  call's `cwd` is inside it, or a nested repository behind a gitlink, whose own config `git
  status` runs). Finding the repository and reading its config is git's own job, with rules a
  second implementation here got wrong six ways (which `HEAD` is valid, where whitespace is
  trimmed, where discovery stops at a mount point, which index entries are submodules), so the
  check asks git, with commands that only read and run no hook, helper or pager: `rev-parse` for
  the repository, its work tree and where it looks for hooks, `config --list --show-scope` for the
  keys of the repository's own config (local and worktree scope) as git parsed them, and
  `ls-files --stage` for gitlinks, each checked-out one of which is judged the same way, judged by
  its git directory. A nested repository must also be a work tree whose top is the directory its
  gitlink names: a `core.worktree` of its own that points elsewhere makes the parent's `git
  status` compare the index of the nested repository with, and print the difference to, whatever
  files that directory holds (an oracle for any file the user can read, which no argument names),
  so it is refused. Those keys must all be on a short list of inert ones (`core.bare`,
  `remote.*.url`, `branch.*.merge`, ...; `core.fsmonitor` only as `false`, since `true` starts
  git's own daemon, which outlives the call). Any other key or section, an `include`, a
  `post-index-change` or `reference-transaction` hook at the path git resolves for it (so a
  `core.hooksPath` is judged by where it leads), output past a bound, a `git` that fails, a probe
  that takes over 1.5 s in all, a git directory outside trusted ground, or more than 32 nested
  repositories is a refusal.
  Outside a repository `git` asks too, because it stops being a repository tool there (`git diff
  A B` is `git diff --no-index A B`), as it does where git will not open the repository (a
  dubious owner, a mount point inside the checkout); inside one, `git diff` asks when an operand
  lies outside the work tree, for the same reason. It needs git 2.31 or later (`rev-parse
  --path-format`); an older git asks.
- **Trust.** The check above runs git inside a repository somebody else wrote, which is itself
  exposure to them: asking git whether the repository is safe must not be the way it runs
  something. Round 3 found no way to make the probe run repository code or reach the network, and
  hardened it anyway (every probe forces `core.fsmonitor=false`, sets `GIT_NO_LAZY_FETCH=1` and
  `GIT_ALLOW_PROTOCOL=none`, and starts without the provider keys Forge holds in its own
  environment, which the shell tool strips from every command too), but the stronger statement is
  that in a project the user has not vouched for the probe never starts a process. `git` and `file`
  are on a second list (`NEEDS_TRUST` in `ask_shell.rs`): they are auto-allowed only when
  `forge_config::project_trust::is_trusted`, the existing mechanism behind `forge trust`, says yes
  for the session workspace and for the directory the command runs in, and (for git) for the git
  directory git found. The store is read on every call and nothing is remembered, so `forge trust
  --revoke` applies to the next call; a linked worktree has the trust of its main checkout;
  `FORGE_TRUST_PROJECT=1` counts, as everywhere in Forge. Trust covers the directories below the
  one trusted, so a repository found above a trusted subdirectory, a gitfile, or a symlinked
  module that leads out of the tree is not covered. A project that ships nothing in `.forge/` is
  never asked at startup, so the first `git status` prompt is the cue to run `forge trust`.
  Every other binary on the list stays auto-allowed in an untrusted project, decided per binary
  (a binary that was doubtful went on the trust list instead; a test makes every new binary pick a
  side):

  | Binary | Needs trust | Why |
  |---|---|---|
  | `ls cat head tail wc nl tac cut tr sort uniq rev fold expand comm paste column printf echo basename dirname realpath readlink which pwd nproc whoami uname id df du stat hostname` | no | filters and lookups: they read their arguments, standard input and the files named as plain data, take no configuration, ignore rule, attributes file, hook or helper path from the repository or from a directory an argument names, and start no program under the accepted flags |
  | `md5sum sha1sum sha256sum b3sum cmp diff` | no | the same, with `--check`, `-l`, `--from-file` and `--to-file` refused |
  | `grep egrep fgrep find sed` | no | no repository-supplied configuration; `find -exec` and the other writers refused, `sed` only as a line-range print |
  | `rg` | no | reads `.gitignore`, `.ignore` and `.rgignore`, which only decide which files it opens, and the guard already screens every file a search could open; `--pre`, `-z`, `--hostname-bin` refused |
  | `git` | yes | runs what the repository's config, attributes and hooks name, and follows its submodules |
  | `file` | yes | libmagic parses arbitrary binary formats, which is exposure to a hostile file that no argument bounds; `-z` (which hands compressed content to external decompressors) is refused everywhere |

- **Programs and `PATH`.** The text rules assume `ls` and `git` are the system's. The decision
  looks up `sh` (which runs every call), `rtk` (which may be put in front of the command) and every
  program the line starts in `PATH` the way the shell does (`egrep` and `fgrep` are scripts that
  run `grep`, which counts too), and asks when the first entry that
  holds the name is one the checkout controls: a relative or empty entry (looked up in the
  directory the command runs in: `bin`, `.`, a trailing colon), or an absolute one that, links
  followed, lies inside the workspace or the working directory (a virtualenv, a direnv `PATH_add`,
  an absolute `node_modules/.bin`). A planted `bin/ls`, `bin/git` or `bin/sh` would otherwise run
  with a line judged read-only. An entry that holds none of those names costs nothing (a
  virtualenv's `python` and `pip`), and a tree that holds the home directory is the user's own, not
  a checkout (a session started in `~`). The probe runs only the `git` this lookup found, never
  one the checkout supplied.
- **Cost.** The checks that look at the machine (trust, `PATH`, and above all running git) run
  once, after every statement of every line has passed the text rules, so a call that asks anyway
  pays for none of them. The decision is made on the thread that runs the turn, so what is left is
  bounded: a git that hangs (a FIFO standing in for `.git/config`) is killed at 1.5 s, a
  legitimate repository takes milliseconds (400,000 index entries under 0.6 s, 30 nested
  repositories about 0.5 s), and past 32 nested repositories, 64 MiB of index or the budget the
  call asks.
- **Blanks.** `sh` splits words on space and tab only. `str::trim` and `char::is_whitespace`
  also take no-break and other Unicode spaces, which the shell keeps inside the word, so both
  were wrong in two places: a redirect ended at any Unicode space (`printf x >&2<nbsp>h/.bashrc`
  was judged `>&2` plus a lone argument while `sh` redirected into a file called `2<nbsp>h/.bashrc`,
  in Read-only mode too), and a line ended at one was trimmed (`cat plain<nbsp>` was judged as
  `cat plain` while the shell read `plain<nbsp>`, which a repository can make a symlink to a
  secret). Both use ASCII blanks now.

**Rationale.** Claude Code does not prompt for these commands, and the prompt buys little: the
calls cannot change anything, and the ones that can still prompt. In the scripted TUI benchmark
(s04, file edit) Claude Code asked once, for the edit; Forge's Ask asked for the edit and then for
one or two shell calls in real-backend runs (2 and 3 prompts, against 1). The rule removes the
shell prompts that only read, and leaves a commit or a test run asking. A user who answers
"yes" to every `git status` learns to answer "yes" to everything, which defeats the prompts that
matter. Auto-allowing the provably harmless class keeps the prompt for the rest.

**Limits.** This is a floor against accidents and lazy prompt injection, not a sandbox, like the
rest of the broker.
- Trust is the user's one explicit statement per project, and it covers everything below the
  directory trusted, nested repositories included. A trusted repository is still judged (a
  hostile `.git/config` in a trusted project asks), but a project the user trusted and then
  cloned something hostile into is the user's call. In a project nobody trusted `git` and `file`
  ask every time; that is the price of never running git there, and `forge trust` (or
  `FORGE_TRUST_PROJECT=1`) removes it.
- The user's own global and system git config and the process environment are trusted: a
  `diff.<driver>.textconv` or `core.fsmonitor` there still runs under `git diff`/`git status`,
  and a driver those files define runs on whatever attributes a repository selects. What a
  checkout brings with it is what `git_repo.rs` asks git about. `git status` may refresh
  `.git/index`. Some repositories ask for want of a key the list does not know (an LFS filter,
  `http.*.extraheader` from CI): that is a prompt, not a hole.
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

**Review findings, round 2** (two more reviewers attacked what round 1 added; all closed).
`printf -v` ran a command (`a[$(cmd)]`) and replaced `PATH`/`HOME` for the rest of the line, in
Read-only mode too; `for-each-ref --format=%(signature)` ran `gpg`; a dash-named operand after `--`
(`diff -r -- -esc src`, a committed symlink out of the workspace) hid the search root from the
confinement check. In the repository check, six disagreements with git, all from reimplementing
it: a decoy `HEAD` hid the bare-shaped repository git adopts, and a plain `git clone` could carry
one; `.trim()` on a `gitdir:` file, on `commondir` and on `core.hooksPath` named a different
directory than git does (any Unicode blank); a nested repository behind a gitlink, or behind a
symlinked `modules/` entry, was never inspected; a mount point inside the checkout made git
stop where the inspection went on (`git diff . ../empty` then dumped untracked files); and
`core.fsmonitor = true` started a persistent daemon. All but the gitlinks went away by asking git
instead of reading its files. One bug of the rewrite itself, found while testing it: judging a
nested repository only when its work tree is the directory it sits in missed one whose own
`core.worktree` points elsewhere, whose config `git status` still runs; they are judged by git
directory now. The tests build the layouts with real git, and a variant of each was run end to end
against the decision, including the mount case in a user namespace.

**Review findings, round 3** (two more reviewers attacked the redesign of round 2; the probe
itself held, the code around it did not). The probe could not be made to run repository code or
touch the network, with every exec-capable key, all hooks and every layout tried. Closed: **(1)**
the probe ran a `git` the checkout supplied when `PATH` held an absolute entry inside the workspace
(a virtualenv, `PATH_add`), handing it the provider keys, and ran it even for a line that asked
anyway; **(2)** the planted-binary check covered only `git`, while a relative or empty `PATH` entry
equally ran a planted `ls` or the `sh` that starts every call (a trailing colon is enough); **(3)**
the probe stalled the turn's thread for five seconds on a FIFO standing in for `.git/config`, also
for lines that asked anyway; **(4)** a nested repository's `core.worktree` was accepted as inert,
which makes the parent's `git status` a one-bit oracle for any readable file; **(5)** `>&2<nbsp>name`
and a trailing no-break space (see "Blanks"); **(6)** followed up from a reviewer's unconfirmed
note about bracket expressions: `cat .[^a]nv`, `.[[:lower:]]nv` and `.e[^x]v` printed `.env`,
because the guard's wildcard expansion (the `glob` crate) lists nothing for patterns `sh` expands
to it. The fix is the trust gate, the `PATH` lookup, the
probe-last ordering with a 1.5 s budget, the work-tree check on nested repositories, and the
hardening of every probe, each with a regression test (`ask_shell/host_tests.rs`,
`ask_shell/path_env.rs`, `ask_shell/git_repo/tests.rs`, `readonly_shell.rs`). Verified to fail
without its fix by reverting each in turn.

**Alternatives rejected.** Reusing the Read-only classifier unchanged (it admits command
substitution and `jq`, which can print the environment); asking the `auto` temper's classifier model
(a model call and latency on every `ls`, and not deterministic); leaving it to per-user `allow`
rules (every user would rediscover the same twenty patterns).
