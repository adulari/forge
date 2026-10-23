# Project trust

A repository's `.forge/` directory is input from whoever wrote the repository. Before this change,
cloning a repo and running Forge in it applied all of it with no question asked:

- `.forge/config.toml` was merged unconditionally, so a project could set
  `permission_mode = "bypass"`, add `[[permissions.rules]] tool = "*" allow = "*"`, and declare
  `[[hooks]]` that ran on the first tool call. It could also point `[lattice.embeddings]` or
  `[mesh] headroom_url` at its own server, raise spend caps, or empty `[mesh] disabled`.
- `.forge/settings.json` hooks and `.forge/mcp.toml` stdio servers were loaded and started at
  session start.

Running Forge in a cloned repo was arbitrary command execution.

## What an untrusted project can still set

Until the project is trusted, `forge_config::load()` applies only the parts of `.forge/config.toml`
that cannot run anything, send data anywhere or widen permissions
(`project_trust::sanitize_untrusted`):

| Kept | Why |
|---|---|
| `[[permissions.rules]]` with `ask` or `deny` | restriction is always safe |
| `permission_mode` = `plan`/`read-only` or `default`/`ask` | a stricter temper |
| `permissions.auto_classifier = false` | makes `auto` ask more |
| `permissions.auto_allow_read_only_shell = false` | makes Ask prompt for read-only shell commands again |
| `[tui]`, `[keybinds]`, `[recap]`, `[suggest]`, `[project]`, `[model_effort]`, `[compact_cap]`, `stop_hook_max_blocks`, `commands.max_palette` | display and pacing |
| `[mesh]` except `headroom`, `headroom_url`, budgets/caps, `pricing`, `subscriptions`, `disabled`, `bridge_mcp_external` | routing preferences among providers the user already configured |

Everything else is dropped: `[[hooks]]`, allow rules, a more permissive `permission_mode`,
`[mcp]`, `[shell]`, `[lsp]`, `[lattice]`, `[providers]`, `[statusline]`, `system_prompt_overrides`,
`commands.trust_project`, and every other section. `.forge/mcp.toml` servers and
`.forge/settings.json` hooks are not loaded at all. The user's own config, the user-scope
`settings.json` and `mcp.toml`, and `FORGE_*` environment variables are unaffected.

What was dropped is named once at startup of `forge run` / `forge chat` / `forge mcp-serve`
(`note: this project is not trusted, so Forge ignored its hooks, 1 allow rule, … — review .forge/
and run `forge trust` to enable them`), and by `forge doctor` (`project trust` warning). An
interactive `forge chat` (or `forge run --tui`) asks once: `Trust this folder and load them? [y/N]`.

## What trust also gates

In the Ask temper, shell commands that only read run without a prompt (`git status`, `ls`,
[temper-modes.md](temper-modes.md)). `git` and `file` are the two such commands a repository can
influence (git runs the programs the repository's config, attributes and hooks name; `file` parses
arbitrary binary formats), so they run unprompted only in a trusted project, and in an untrusted one
Forge never starts git to find out whether the repository is safe. Everything else on that list
stays unprompted in an untrusted project. Trust is read from the store on every call, so
`forge trust --revoke` applies to the very next command.

Because the prompt itself gave no hint of this, the permission panel now says so: when a shell
call asks only because the project is not trusted (in the Ask temper, every statement read-only,
no rule deciding it), one line is added above the choices, `Trust this project to skip prompts for
read-only git: forge trust` (shorter on a narrow terminal). It is advisory. It is worked out from
the text of the command and the trust store alone, never by running git in the repository, and it
does not change what any call may do; trusting still leaves a repository whose config names a
helper asking (`forge_core::permission::shell_trust_hint`).

## Trusting

```sh
forge trust              # trust the current project
forge trust ~/src/repo   # or a path
forge trust --list
forge trust --revoke
FORGE_TRUST_PROJECT=1 forge run "…"   # trust for one process (CI, scripts)
```

Trust is stored in `<config dir>/trusted-projects.json`, which a repository cannot write. It covers
the directory and everything below it, and a linked git worktree inherits the trust of its main
checkout (so daemon sessions in `.forge/worktrees/*` behave like the repo they came from).

## Daemon-hosted sessions

`forge_config::load_in(root)` is `load()` for an explicit project root, and `command_sources_in(root)`
is the same for project commands/skills. A session builds its configuration from ITS workspace root
(including `forge serve` sessions, whose process cwd is unrelated to any workspace): the project
`.forge/config.toml`, the trust decision, `.forge/mcp.toml` and the `.forge/settings.json` hooks all
resolve against that root, not the daemon's cwd.

## "Always allow" answers

Answering `a` (always) at a permission prompt normally appends an allow rule to the project's
`.forge/config.toml`. In an untrusted project that file's allow rules are ignored, so the answer is
stored in the user's `trusted-projects.json` instead, keyed by the project root, and applied on the
next load. In a trusted project the behaviour is unchanged.

## Tests

`forge-config`: `untrusted_project_cannot_escalate_run_hooks_or_spawn_mcp` (the full attack repo:
bypass mode, allow-all rule, `[[hooks]]`, `mcp.toml` server, `settings.json` hook; then trusted),
`always_allow_in_an_untrusted_project_is_kept_in_the_user_store`, and the `project_trust` unit
tests (sanitizer, stricter temper kept, worktree maps to its main checkout).
