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
