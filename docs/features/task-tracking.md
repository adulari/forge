# Feature: Task / todo tracking (`update_tasks`)

Status: built. A structured, live task list the agent maintains during multi-step work,
rendered in the TUI and persisted across resume. Wave-2 P1 (TodoWrite + Task* ≈ 196 uses in
the owner's history).

## Problem (JTBD)

> When the agent tackles a multi-step job, I want to **see the plan and watch it progress** —
> which step it's on, what's done, what's left — instead of a wall of streamed text.

## What it does

- New agent tool **`update_tasks`**: the model passes the full ordered task list (each
  `{title, status: pending|in_progress|done}`); it replaces the previous list. The model
  updates it as work progresses (one item `in_progress`, mark `done` when finished).
- The TUI renders a styled checklist into scrollback on every update — `☐` pending, `◐`
  in-progress (highlighted), `☑` done (dimmed + struck-through), with a `(n/m done)` header.
- The list is **persisted** (`session_tasks` table, one JSON row per session) and **restored
  on resume** (`/resume`), re-emitted so the resumed session shows its progress.
- Cleared on `/new`.

## Both execution paths

`update_tasks` is a **core virtual tool** (like `spawn_agents`/`ask_user`) — it isn't a plain
registry tool because it mutates session state + emits to the UI. It works on both paths:

- **Direct provider** (API models via the mesh): intercepted in `Session::invoke_tool`, which
  sets `self.tasks`, persists, and emits `PresenterEvent::Tasks` live.
- **CLI bridge** (`claude-cli::`/`codex-cli::`): the bridge's tools come from `forge mcp-serve`
  (a separate process), so `mcp_serve` advertises `update_tasks` and writes the list to the
  store under the parent session id (`snapshot::ENV_SESSION`). After the bridge turn,
  `run_turn` reloads the store and emits the list if it changed — so bridge-driven updates
  surface in the TUI too (end-of-turn rather than mid-stream).

This matters because a bridge-only setup (no direct API keys) would otherwise never see it.

## Impact

| Layer | Change |
|------|--------|
| `forge-types` | `TodoItem { title, status }` + `TodoStatus {Pending,InProgress,Done}` (loose parse, glyphs) |
| `forge-store` | `session_tasks` table + `set_tasks`/`tasks` (JSON, replace-wholesale) |
| `forge-core` | `Session.tasks`; `update_tasks` virtual tool (spec + intercept + handler); `parse_tasks`; rehydrate on build/resume, clear on reset; end-of-turn reload+emit for the bridge path |
| `forge-tui` | `PresenterEvent::Tasks`; `render::task_list_lines` (styled checklist); headless plain render |
| `forge-cli` (`mcp_serve`) | advertise + handle `update_tasks` → persist to the store by session id |

## Verification

- Unit: `TodoStatus::parse_loose`; store round-trip + wholesale replace.
- Core integration: a provider that calls `update_tasks` → list set + persisted + `Tasks`
  event emitted; resume restores the list.
- **Live (codex bridge)**: `forge run --model codex-cli::gpt-5.5 "track a 3-step plan…"` →
  codex calls `mcp__forge__update_tasks` repeatedly (pending→in_progress→done), visible in the
  TUI, persisted to the store.
- `cargo test --workspace` + `clippy --workspace --all-targets -D warnings` green.
