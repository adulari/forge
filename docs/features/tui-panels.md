# Feature: Forgeline panels and onboarding

The panel surfaces of the Forge TUI: the welcome state, the tasks and agents band, the wide-terminal
details rail, the `/btw`, `/usage`, `/mesh` and `/voice` overlays, the workflow view, the `/config`
editor and the `forge setup` wizard. They draw with the Forgeline tokens and glyphs from
`docs/features/tui-theme.md`: no colour literals, an ASCII twin for every glyph, and no painted
backgrounds except selected rows.

## Welcome state

`app/render/welcome.rs`. In fullscreen the empty transcript renders a welcome (a render state, not
scrollback, so it vanishes on the first message and is never in history):

- a two-row half-block `FORGE` wordmark in the ember heat ramp when the terminal is 30 rows or
  taller, otherwise a one-row `✦ forge` (plain `FORGE` text in the ASCII tier);
- the tagline;
- a `/init` nudge only while the project is uninitialised. It is state: the launcher calls
  `App::set_project_initialized(false)`, and the welcome re-checks `.forge/` guidance every two
  seconds, so the line clears as soon as `/init` writes it. Fullscreen pushes no scrollback warning;
  inline prints the same one-line warning once;
- three things to try, the key row (`/ commands`, `@ files`, `Ctrl+K actions`, `Shift+Tab mode`,
  `? all keys`; narrow terminals drop `Shift+Tab` first), a `More` row (`forge doctor`, `/mesh`) when
  there is room, and recent sessions when a launcher supplies them with
  `App::set_recent_sessions`.

At most 13 rows at 80x24. Inline prints a compact identity row (`✦ forge v2.x · repo ⑂ branch ·
tagline`) and a keys row once, before the live region (`banner_lines`, `print_banner_direct`, which
honours the colour tier).

## Tasks and agents band

`app/render/panels.rs` and `panels/`. Collapsed, one row: `▸ Tasks 1/3   ◉ Wire retry …` on the left
(the task in progress, else the next open one), running agents and reviews plus `Ctrl+O details`
flush right. With no tasks it reads `▸ Agents N`. `Ctrl+O` opens the lists:

- **Tasks**: `✓ ◉ ○` rows with an elapsed time for running work (frozen when it finishes) and
  `@assignee` for delegated tasks. When the region is short, open work wins the rows and finished
  tasks fold into `… +N completed`.
- **Agents**: the main chat plus every subagent and assay critic, each with a spinner or outcome,
  model, what it is doing, elapsed time and cost. `↑↓` moves the raised `❯` row, `Enter` opens that
  entry's transcript, workflow `phase()` groups get a header.

The protocol carries no timestamps for tasks or agents, so the elapsed counters start when the
panel first sees the item running (`panels/common.rs` `Clock`).

### Details rail

At 150 columns or more, in fullscreen, with details open, the band disappears and the same content
renders as a 38-column right rail beside the transcript: `TASKS d/n`, `AGENTS`, `CHANGED` (files
landed this turn with `+adds −dels`) and `CONTEXT` (gauge and percentage of the window). The
transcript area shrinks, so its hit-test geometry and mouse selection stop at the rail. Inline never
opens a rail.

## Scrollback records

`app/lines/plan.rs` and `agents.rs`. A plan card is a block (`PLAN` label, title, `N steps`, numbered
steps whose wrapped text hangs under the step title, muted `Risks:`), printed in full in both modes.
Subagent groups print as `◆ Agents` with faint `├` branches and a `└ 3 agents · $0.07` footer.
Workflow history lines use the notice grammar (`· workflow name started`, `◆ phase Fix`,
`✓ workflow finished · summary`).

## Overlays

`app/render/overlays/`, `app/render/voice.rs`. Every overlay is a rounded frame with a title, a
context on the right of the top border, its keys in the bottom border and one row / two cells of
padding that a mouse drag does not copy (`overlays::padded_panel`).

- **`/btw`**: see `docs/features/side-questions.md`.
- **`/usage`**: `PERIOD IN OUT COST` columns right-aligned under their headers (`5h`, `Today`, `Week`,
  `Month`, `Session`; caps as `of $5.00 (42%)`), costs as `$0`, `<$0.01` or two decimals, `QUOTAS`
  as `▰▱` bars (Claude's reading says how old it is), pace notes, and `BY MODEL (today)` with an
  explicit empty state. Per-model rows keep `subscription`, `free` and `untracked`.
- **`/mesh`**: the routing inspector in the same frame; quota meters use the `▰▱` glyphs and the
  ok/warn/err roles; the browsing cursor is the raised `❯` row and the routed pick is marked `✓`.
- **`/voice`**: the recording card with a `● REC` indicator that holds still under reduced motion,
  a progress bar for the model download and keys in the border.

## Workflow view

`workflow_view/render.rs`. The takeover is a header row (`✦ workflow name`; phase, agents, elapsed,
cost and `Esc background` right), the phase checklist (`✓ 1  Survey  3 agents  1m 02s`,
`◉ 2  Fix  2 agents  running`), nested agent rows (spinner or outcome, name, model, action, elapsed
and cost, the selected row raised with `❯`), a `LOG` section and a hint row (`↑↓ select agent ·
Enter zoom · Esc background · Ctrl+C stop`). Backgrounded, a one-row band shows
`⠹ workflow name · phase 2 · 2 agents running` with elapsed, cost and `Ctrl+O view`. Enter-zoom is
unchanged.

## Config editor and setup wizard

`config_editor/render.rs`, `init_wizard.rs`. `/config` is a `Settings` takeover: search row with a
`scope: User Project` control (`Tab` toggles), sections with counts beside the current section's
settings from 120 columns, or a `PROVIDERS & KEYS  PgUp/PgDn section 1/7` header below that. Rows are
a caret, label, muted description and a value in one grammar (`○ not set`, `✓ keyring`, `● on` /
`○ off`, `‹ auto ›`, plain numbers and text). The selected setting's full help is wrapped under the
list with its source and key path last. `←→` still change enum values, so sections move with
`PgUp`/`PgDn`. `forge setup` uses the same frame, `❯` rows, `●●○○○` step dots and labels the
recommended choice (`(recommended)`).

## Polish pass

Plan-card rows carry wrap marks (`copy_mark::hang` plus prose) instead of being pre-wrapped, so they re-wrap at the live width and hang under the step text. The shared list view sizes its shortcut column from the visible window, and the config editor sizes its label column from the visible rows.

## Welcome example focus

On the empty-transcript welcome (fullscreen), `Down` from an empty composer moves focus into the three "Try" examples. The focused row shows the `❯` caret on the raised fill with an `Enter use · Esc back` hint. `Up`/`Down` move, `Enter` copies the example into the composer (it is not sent), `Esc` or any other key drops the focus and falls through to the normal handler. Implemented by `App::welcome_key` (`app/state/panels_ui.rs`), called from the idle branch of the key loop ahead of prompt history.
