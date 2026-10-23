# Feature: Temper — SHIFT+TAB operating-mode switcher

> Status: **DESIGN + BUILD** (2026-06-15).
>
> An interactive switcher that cycles Forge's operating **temper** — how forward-leaning the
> agent is allowed to act — live in the TUI with **SHIFT+TAB**, shown in the statusline.
> "Temper" is the forge/metallurgy framing (heat-treatment hardness *and* disposition/restraint),
> and it gives Forge a distinct, unique vocabulary instead of Claude Code's "permission mode".

---

## 1. Problem (JTBD)

> **As** a developer mid-session, **I want** to flip between "just look / plan", "ask me first",
> and "go ahead and edit" **with one keystroke**, **so that** I can tighten or loosen the leash
> as the task changes without restarting Forge or editing config.

Today the permission mode is fixed at session start (`config.permission_mode` / `--mode`). The
four modes already exist in the engine (`PermissionMode::{Default, AcceptEdits, Bypass, Plan}`,
ADR-0008) but there is **no way to change it during a session** and **no on-screen indicator**
of which is active. Power users expect Claude-Code-style SHIFT+TAB cycling.

---

## 2. The temper vocabulary

Internal enum variants and config/CLI keys are **unchanged** (back-compat; security-critical
names stay stable). Temper is a **display + UX layer** plus runtime switching. Canonical key
(accepted by `--mode`/config) → temper label → meaning:

| Canonical key | Temper label | Behaviour | In SHIFT+TAB cycle? |
|---------------|--------------|-----------|---------------------|
| `plan` | **Read-only** | Investigate & propose, **no** side effects (hard contract). | yes |
| `default` | **Ask** | Ask before each side effect, except shell commands that are provably read-only (see "Ask and read-only shell"). | yes |
| `accept-edits` | **Auto-edit** | Auto-apply workspace edits and ordinary shell; ask for writes beyond the workspace and for shell that deletes, moves or writes outside it (see "Auto-edit and shell"). | yes |
| `auto` | **Auto** | Proceed on safe calls, ask on risky or unrecognised ones (see "Auto mode classification"). | yes |
| `bypass` | **Full** | All permission guards off (the unoverridable safety denylist still applies). | **no — explicit opt-in only** |

The dimension is themed **temper**; the values are **descriptive** so the active permission is
obvious at a glance (the prior themed value-names hid which mode was which). `--mode` and
`[permission] mode = …` accept **either** the canonical key **or** the label
(e.g. `--mode read-only` == `--mode plan`, `--mode auto-edit` == `--mode accept-edits`).

**Cycle order** (SHIFT+TAB): `Ask → Auto-edit → Auto → Read-only → Ask …` (Auto added later, see claude-code-parity.md). **Full** is deliberately
**excluded** from the cycle: landing on "all guards off" by tapping a key is a footgun, so it is
reachable only via `--mode full` / config. (Claude Code likewise never cycles into bypass.)

### File tools outside the workspace

The structured file tools (`read_file`, `write_file`, `edit_file`, `multi_edit`, `apply_patch`,
`append_file`, `delete_file`, `list_dir`, `search`, `glob`, `notebook_edit`) accept paths outside
the session workspace. Reads proceed in every mode. A write goes through the broker like any other
side effect: **Full** allows it, **Ask** and **Auto** ask, **Auto-edit** asks too (it trusts edits
to the project, not the whole disk; paths under `[tools] extra_roots` count as project), and
**Read-only** denies it. The secret denylist (`.env*`, SSH keys, cloud credentials, ...) applies in
every mode, Full included. One write is refused outright: a linked-worktree session writing into
another Forge-managed worktree of the same repo (`<repo>/.forge/worktrees/<other>`), which would
clobber a sibling agent's isolated work. Writes aimed at the main checkout of a linked worktree are
still redirected into the worktree.

`[tools] confine_to_workspace = true` restores the old hard confinement (workspace plus
`extra_roots` only, refused in every mode). It was the default until the refusals proved to buy no
safety, since `shell` was never confined, while pushing models to write files through `cat >` and
heredocs instead of the reviewable file tools.

### Ask and read-only shell

Ask used to prompt for every shell call, `git status` and `cat calc.py` included. It now runs a
shell command without a prompt when the broker can prove it only reads (`permission/ask_shell.rs`,
ADR-0008): every statement is one of `ls cat head tail wc pwd echo stat du nl tac cut tr sort uniq
basename dirname realpath readlink which md5sum sha1sum sha256sum b3sum diff cmp grep egrep fgrep
rg find sed git printf column rev fold expand comm paste nproc whoami uname id df file hostname`,
written plainly, with arguments that cannot write, run code or reach the network. Pipelines and
`&&`/`;` chains qualify when every stage does. The call still shows in the transcript as an
ordinary tool row, just with no prompt.

It does not qualify, so it asks as before, when it has a redirect (other than `2>&1` and
`>/dev/null`), a `$`/backtick/`<(` substitution, a subshell or brace, a `&`, a `VAR=value`
prefix, a wrapper (`env`, `command`, `nohup`, `time`), a binary named by path or in quotes, a newline,
or a flag such as `find -exec/-delete`, `sort -o`, `rg --pre`, `tail -f`, `printf -v`; for `sed` only
`sed -n 'N,Mp'` qualifies, for `git` only the read subcommands (`status diff log show branch`
listing, `rev-parse ls-files blame grep ls-tree rev-list shortlog describe merge-base show-ref
for-each-ref name-rev`, `tag` and `reflog` for listing, `stash list`, `worktree list`) with no
`-c`, `-C`, `--output`, `--ext-diff`, `--textconv`, `-O`, `--no-index` or `--show-signature`.

`git` and `file` run without a prompt only in a project you trusted (`forge trust`,
[project-trust.md](project-trust.md)), because a repository can influence them: git runs the
programs a repository's config, attributes and hooks name, and `file` parses whatever binary
format it is handed. In a project you did not trust they ask every time and Forge never starts git
there to find out more; every other command on the list stays unprompted there, since nothing a
repository holds changes what it does. A project that ships nothing in `.forge/` is not asked at
startup, so the first `git status` prompt is the cue to run `forge trust` once. Trust is read on
every call (`forge trust --revoke` applies to the next one), covers the directory the session
started in and the one the command runs in, and a linked worktree has the trust of its main
checkout.

Even in a trusted project `git` also asks outside a repository, in one whose own config (or a
nested repository's, behind a submodule entry) names a program or anything else the broker does
not know to be inert (`core.fsmonitor` other than `false`, a `textconv` driver, a promisor remote,
an `include`), where a `post-index-change` or `reference-transaction` hook is, with a nested
repository whose work tree is not the directory it sits in, with a git directory outside the
trusted tree, with more than 32 nested repositories, or when the check takes over 1.5 s; and
`git diff` asks when an operand lies outside the repository's work tree. Git runs those programs
on `git status` and `git diff`, and a checkout can bring its own. The check asks git itself
(`rev-parse`, `config --list`, `ls-files`) rather than reading `.git`, with `core.fsmonitor`
forced off, lazy fetching and every transport forbidden and none of Forge's provider keys in its
environment, so it needs git 2.31 or later; an older git asks. It runs last, once per call, and
only for a line that passed every other check.

The programs themselves must be the system's: when `PATH` holds a relative or empty entry (`bin`,
`.`, a trailing colon) or one that lies inside the workspace, and that entry holds a program the
call starts (`ls`, `git`, the `sh` that runs it), the call asks, because a repository could ship
that file. A virtualenv's `python` and `pip` in `PATH` cost nothing.

Secrets are not a way around the secret rules. The built-in deny verbs (`cat .env`, ...) refuse as
before, and the paths a read-only command names are also checked against the same secret globs
(`.env*` but not `.env.example`, `*.pem`, `id_rsa`, `~/.ssh/**`, `/proc/*/environ`, ...), through
symlinks, `~` and wildcards: `wc -c .env`, `git show HEAD:.env` and `cat .e*` ask. A command that
reads every file below a directory (`grep -r`, `diff` of two directories, `rg`, `git grep
--untracked`) asks unless the directory is inside the workspace and holds no such file; a plain
`rg` ignores hidden files, so a hidden `.env` does not stop it, but a visible `server.pem` does.

What outranks it still does: the safety floor, deny rules, ask rules, hooks (`PreToolUse` runs
first and may block), and the Read-only temper (which has its own classifier). Auto-edit, Auto and
Full are unchanged. The native loop, the CLI bridge (`forge mcp-serve`) and subagents all use the
same decision.

```toml
[permissions]
auto_allow_read_only_shell = true   # default; false makes Ask prompt for every shell command again
```

An untrusted project may set it to `false` (more prompts) but not to `true`
([project-trust.md](project-trust.md)).

### Auto-edit and shell

Auto-edit used to allow every shell command, so `rm -fr ~` or `rm -rf "$HOME"` ran unprompted in
the default temper while these docs said it asked. It now runs ordinary commands (builds, tests,
git, in-workspace deletes such as `rm -rf target`) without a prompt and asks, with the reason in an
`auto-edit asks: ...` warning, when a line (`permission/auto.rs`, `accept_edits_risk`):

- deletes recursively or forcibly outside the workspace (`rm -rf ../x`, `rm -rf $VAR`, `xargs rm -rf`,
  or any relative delete after `cd` leaves the workspace), or `find … -delete`s there;
- copies, moves, links, touches, `tee`s, `sed -i`s or redirects (`>`/`>>`) to a path outside the
  workspace (`/tmp` and `$TMPDIR` are scratch and never count);
- touches a raw disk (`dd of=`, `mkfs`, `wipefs`, `fdisk`, …), pipes into a shell, or runs inline
  interpreter code that deletes files (`python -c 'shutil.rmtree(…)'`).

The CLI bridge (`mcp-serve`) applies the same rule, and also asks for file-tool writes beyond the
workspace exactly like the direct path (it used to allow them).

### Safety floor (every temper, Full included)

Before any temper or rule is consulted, the broker refuses two classes of shell line
(`permission/catastrophic.rs`); no allow rule, project setting or `bypass` overrides them:

- **Catastrophic deletes.** A recursive delete, move-away, wipe or recursive permission change of
  the home directory, `/`, or a top-level system directory, in any spelling: flag order (`rm -fr`,
  `rm -r -f`, `--recursive --force`), `--`, `~/`, `~/*`, `$HOME`, `"${HOME}"`, an absolute `rm`
  path, `timeout`/`nice`/`command`/`env` wrappers, `bash -c`/`eval` bodies, `X=~; rm -rf $X`,
  `cd ~ && rm -rf .`, `find ~ -delete`, `echo ~ | xargs rm -rf`, `mv ~ elsewhere`,
  `rsync --delete … ~/`, and inline `shutil.rmtree(os.path.expanduser("~"))`. The literal globs in
  the builtin denylist stay as a second layer.
- **Privilege escalation.** `sudo`, `su`, `doas`, `pkexec`, `run0`, `passwd`, `sudoedit`, wherever
  they appear in the line. Forge runs commands without a terminal, so these can only fail, and
  PAM records each failure toward `pam_faillock`'s lockout (three lock the account, screen unlock
  included). The tool result tells the model to stop and hand the exact command to the user, and
  the user sees a `refused ... needs root` warning.

The floor is a guard against accidents and lazy attacks, not a sandbox: indirection it cannot see
(a script written to disk and then run, `eval "$(curl …)"`) still needs `shell.sandbox` or a
container.

### Auto mode classification

`auto` resolves symlinks when it decides whether a write leaves the workspace, so a link inside the
repo that points elsewhere (including one the model just made with `ln -s`) counts as outside.

`auto` sorts every shell command three ways (`permission/auto.rs`, `permission/auto_safe.rs`):

- **Safe**: known build/test/lint/format tools, read-only utilities, local git, edits inside the
  workspace. Runs without asking.
- **Risky**: recursive or forced deletes, force-push, writes outside the workspace, credential
  paths, pipe-to-shell, uploads, `sudo`, and similar. Always asks, with the reason in the
  `auto mode asks: ...` warning. Nothing downstream can override this.
- **Unknown**: everything else: unfamiliar binaries, `./deploy.sh`, `make release`,
  `npm run <x>`, `python x.py`, `base64 -d` payloads, any command named by a non-system path.

An Unknown command is judged by one bounded side call (`auto_classifier.rs`): a cheap model, low
effort, 64 output tokens, 4s timeout, given the command, working directory, workspace root, a
snippet of the user's request and the first 2000 characters of up to two workspace files the
command names (a script is judged by what it does, not what it is called), replying `{"verdict":"allow"|"ask","reason":"..."}`. The verdict is
cached per session by whitespace-normalised command. The classifier can turn Unknown into Allow or
Ask and nothing else: deny rules, ask rules and Risky verdicts are decided before it runs. Every
failure (no usable model, timeout, provider error, an answer that is not that JSON) asks.
An allow is logged at debug level as `auto: allowed by classifier`.

```toml
[permissions]
auto_classifier = true   # default; false makes every Unknown command ask, with no model call
```

The CLI-bridge `mcp-serve` runs the same classifier before relaying an Unknown command to the
user's prompt (`mcp_serve/auto_classify.rs`), with its own per-process cache. It tries up to two
keyed, non-subscription trivial-tier models, then, on a claude-cli session, `claude-cli::haiku`
on the user's own plan (the parent passes `FORGE_BRIDGE_CLASSIFIER_MODEL`; 25s limit, since it
starts a process). If none answers, the existing permission relay prompts as before. Unattended
subagents have no classifier surface: an Unknown command there is denied.

**Reserved:** `Assay` (read-only analysis crew) is the next feature and will slot in beside
Read-only as a read-only sibling. Not wired into the cycle this turn.

---

## 3. Scope (MoSCoW)

**Must**
- M1 — SHIFT+TAB in the chat TUI cycles temper through Guarded → Smith → Survey (wrapping).
- M2 — the active temper is shown in the statusline (themed label) at all times.
- M3 — switching takes effect on the **next** turn's tool calls (the permission engine reads the
  session's current mode); a switch mid-flight is only allowed when **idle** (not during a turn).
- M4 — each switch leaves a one-line scrollback note (`temper → Smith`) so history is legible.
- M5 — `--mode` / config accept the temper labels as aliases for the canonical keys.

**Should**
- S1 — persist the new mode to the session row so `forge sessions` / resume reflect it.

**Won't (this turn)**
- Assay temper (next feature). Cycling into Unfettered. Headless/`forge run` switching (no TUI).

---

## 4. Acceptance criteria

```
Given an idle chat-TUI session in Guarded temper
When the user presses SHIFT+TAB
Then the statusline shows "Smith" and a "temper → Smith" line is added to scrollback
And the next turn auto-applies file edits without a permission prompt

Given temper Smith
When the user presses SHIFT+TAB twice
Then temper is Survey (Smith → Survey), and a write tool in the next turn is denied (read-only)

Given temper Survey
When the user presses SHIFT+TAB
Then temper wraps to Guarded (never to Unfettered)

Given a turn is running (busy)
When the user presses SHIFT+TAB
Then the temper does not change (no mid-turn switch); the keystroke is ignored

Given `forge run --mode read-only`  <!-- the `survey` alias was never shipped -->
Then it behaves exactly as `--mode plan` (read-only)
```

---

## 5. Design / impact

| Layer | File | Change |
|-------|------|--------|
| Types | `forge-types/src/lib.rs` | `PermissionMode::label()` (themed), `cycle_next()` (safe 3-cycle, excludes Bypass), serde `alias` for the temper labels |
| Store | `forge-store/src/lib.rs` | `update_session_mode(id, mode)` (S1 persistence) |
| Core | `forge-core/src/lib.rs` | `Session::temper()` getter, `set_mode`, `cycle_temper() -> PermissionMode` (advance + persist) |
| TUI | `forge-tui/src/app.rs` | `KeyKind::CycleTemper`; `App.temper` label; render in `render_statusline`; `set_temper()` (label + scrollback note) |
| TUI | `forge-tui/src/driver.rs` | map `KeyCode::BackTab` → `KeyKind::CycleTemper` |
| CLI | `forge-cli/src/main.rs` | chat-TUI loop: on `CycleTemper` when idle, `session.cycle_temper()` + `app.set_temper()`; `--mode` value aliases |

**Vertical slice:** SHIFT+TAB → driver yields `KeyKind::CycleTemper` → (idle only) main locks the
session, `cycle_temper()` advances the mode, persists it, returns the new `PermissionMode` →
`app.set_temper(mode)` updates the statusline label + queues a scrollback note → next
`run_turn` reads `self.mode` in `permission::decide` as before.

**Why runtime mode lives on `Session`:** the permission engine already reads `self.mode`
(`forge-core/src/lib.rs` `invoke_tool`). Making `mode` mutable + adding a setter is the whole
mechanism; no change to `permission::decide`.

---

## 6. Definition of done

- [ ] All §4 acceptance criteria pass (unit + TUI render tests).
- [ ] `PermissionMode::label()`/`cycle_next()` unit-tested incl. the Bypass exclusion + wrap.
- [ ] `--mode read-only|ask|auto-edit|full` (plus `plan`, `default`, `accept-edits`, `bypass`). The survey/guarded/smith/unfettered aliases are NOT accepted by clap and error out and equal to canonical.
- [ ] Statusline shows the themed temper; SHIFT+TAB note appears in scrollback.
- [ ] Switch persisted to the session row.
- [ ] fmt + clippy `-D warnings` + full workspace green.

The secret denylist judges every path a call names: `paths[]`, `edits[].path`, the targets inside an `apply_patch` body, and the symlink-resolved target of each. Auto temper applies the same set (plus `cwd`) when deciding whether a write leaves the workspace.
