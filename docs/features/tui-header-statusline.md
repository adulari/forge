# Feature: TUI header, composer border & statusline

> Originally a right-sized spec for the ASCII welcome banner and a statusline. The chrome has since
> been redesigned as part of the Forgeline identity (see [tui-theme.md](tui-theme.md)): sections 5
> onwards describe the current behaviour. The welcome banner is owned by the panels surface.

## 0. Current design: four regions, one job each

| Region | Answers | Content |
|---|---|---|
| Header (fullscreen only, 1 row) | where am I | `✦ forge`, `repo ⑂ branch` (`worktree` marker, `@sha` in warn for a detached HEAD), `● remote` while remote control is on, version from 120 columns |
| Composer border | what is happening now, how my next message runs, which keys work | top-left live line, top-right chips, bottom-right hints |
| Status line (1 row) | engine and meters | model · tier, cost, turn tokens, throughput on the left; context gauge and mesh effort on the right |
| `/status` | everything the others drop | model, mode, location, context, turn, session, quotas, MCP, version |

Row budget at 80x24 fullscreen: header 1 + composer 3 + status 1 = 5 rows of chrome, 19 for the
transcript. Inline has no header (the region stays `LIVE_H` = 14 rows) and keeps a `● remote` and
a `⑂ branch` item in the status line instead.

## 1. Problem (JTBD)

> When I open `forge chat`, I want an instantly recognizable, beautiful first impression
> and a clear at-a-glance status bar, so I trust the tool, know what it's doing, and can
> read its live state (model, cost, mode) without hunting.

Today the TUI opens straight into an empty conversation panel with a thin 1-row header.
It works but it's plain — no brand moment, and live state is scattered (model/cost in the
header, hints in the footer). Reference bar: Claude Code's welcome banner + Antigravity's
splash and persistent status line.

Who's affected: every interactive user. Why it matters: first impression + readability of
the live Model-Mesh state (the product's differentiator) is the daily experience.

## 2. Scope (MoSCoW)

**Must have**
- As a user, I see a striking ASCII-art **FORGE** wordmark (brand orange) as a welcome
  banner when a chat session opens (empty conversation), so the tool feels polished.
- As a user, I see a dedicated **statusline** (bottom bar) with: concrete live phase/spinner,
  mesh tier + model, running cost, permission mode, and key hints — clear hierarchy.
- As a user on a narrow terminal, the banner and statusline **degrade gracefully** (no
  overflow/wrap garbage): banner falls back to a compact wordmark; statusline drops
  lower-priority segments.

**Should have**
- The big banner gives way to a compact 1-row brand header once the conversation has
  content (so it doesn't waste vertical space mid-session).
- A subtle tagline under the banner ("model-mesh coding agent").

**Could have**
- A color gradient across the wordmark.

**Won't have (this iteration)**
- Animated/typed banner reveal; configurable themes; multiple logo variants.

## Non-goals
- This feature does **not** change the conversation/input rendering or the agent loop.
- It does **not** add new session state — only displays existing state.

## 3. Acceptance criteria

```
Given a fresh `forge chat` session with no messages yet
When the TUI renders
Then the conversation area shows the multi-line ASCII "FORGE" wordmark in brand orange
And a tagline line is shown beneath it

Given a session that has at least one message
When the TUI renders
Then the big banner is replaced by a compact 1-row brand header (⚒ FORGE)
And the conversation transcript is shown

Given any session
When the TUI renders
Then a statusline row shows: [live phase + spinner when busy] · [tier] model · $cost · mode · hints
And the model + cost segments are always present (highest priority)

Given a terminal narrower than the wordmark (< ~46 cols)
When the TUI renders the welcome state
Then it shows a single-line compact wordmark instead of the multi-line art (no wrapping)

Given a terminal too narrow for all statusline segments
When the TUI renders
Then lower-priority segments (hints, mode, session) are dropped before model/cost
```

## 4. Impact analysis

The original banner/statusline was a single-crate rendering change. The live turn heartbeat also
uses one additive core event (`ProviderRequest`) so the UI can distinguish context assembly from
the exact provider-request boundary.

| Layer | Insertion point |
|-------|-----------------|
| TUI render | `crates/forge-tui/src/app.rs` — new `render_banner`, rework `render_header`/`render_footer` into a `render_statusline`; `render` chooses banner vs compact header by `app.lines.is_empty() && app.streaming.is_empty()` |
| Brand asset | a `const FORGE_WORDMARK: &[&str]` (ANSI-Shadow block letters) in `app.rs` |

No new dependencies. `App` already carries everything the statusline needs (routing,
cost_usd, busy/tick, permission mode is implied by config — add a `mode: String` field to
`App` if we want to show it; otherwise omit mode for v1).

## 5. Technical design

### Welcome state (empty conversation)
Conversation area renders, centered, the wordmark + tagline instead of an empty panel:

```
   ███████╗ ██████╗ ██████╗  ██████╗ ███████╗
   ██╔════╝██╔═══██╗██╔══██╗██╔════╝ ██╔════╝
   █████╗  ██║   ██║██████╔╝██║  ███╗█████╗
   ██╔══╝  ██║   ██║██╔══██╗██║   ██║██╔══╝
   ██║     ╚██████╔╝██║  ██║╚██████╔╝███████╗
   ╚═╝      ╚═════╝ ╚═╝  ╚═╝ ╚═════╝ ╚══════╝
            model-mesh coding agent · type a task to begin
```

Non-empty conversation → compact header row: ` ⚒ FORGE   <session>`.

### Composer border

The composer is the only permanent frame. Its border carries three zones and nothing is drawn
twice (`app/render/chrome.rs`):

- **Top-left, the live line.** One of: `⠹ Starting session…`; `▲ Esc again to rewind` or
  `▲ Ctrl+C again to quit` while armed; `● Waiting for you · your approval`; `⠼ Finding a model`;
  the busy line `⠼ Writing · 14s · ↓1.2k tok · 38 tok/s`; or, after a turn that stopped for a reason,
  `▲ Step limit reached · send `continue` to resume` (`✗` for a budget cap, unfinished tasks or no
  output). The verb follows real presenter events (Preparing, Routing, Thinking, Writing,
  Running <tool>, Waiting, Working, Retrying, Compacting, Finishing). `last event 41s ago` joins the
  line after 15 s of silence and turns warn after 60 s, so a provider stall looks different from
  hidden reasoning. On narrow borders a stop reason shrinks to a short phrase and then to the bare
  recovery.
- **Top-right, chips.** `N lines` while multi-line, the loop and goal chips, and the mode
  chip last in its temper colour (Read-only `user`, Ask `fg_muted`, Auto-edit `brand`, Full `err`).
  Queued messages are not a chip: they are listed in a band directly above the composer, so the
  count and the text are in one place ([steering.md](steering.md)).
  The border itself takes the temper colour, and `warn` while the app waits for you.
- **Bottom-right, hints.** At most five `key label` pairs for the current state: idle, typing,
  suggestion pending, busy, stopped, palette, picker, permission, question, armed. When the border
  is narrow they drop from the second-to-last backwards so the first and `? keys` survive longest.
  Decision blocks and popups supply their own set through their `hints` hook.

The counters the old turn-activity row printed (hidden-reasoning characters, buffered provider
events, tool-call count, the model at the provider boundary) are listed under `ACTIVITY` in
`/status` while a turn runs.

### Status line

One row (plus one per configured `extra_rows` entry and one for a `[statusline] command`). Left:
the model (`llama3.2`), the `⟨med⟩` rung marker when a rung is set, cost (`$0`, `<$0.01`, `$0.42`,
`free`, `subscription`), the running turn's `↑72 ↓30` tokens while it runs, and live throughput.
Right: the context gauge (`ctx ▱▱▱▱▱▱▱▱ 0% of 32k`, muted below 70 %, warn from 70, err from 90)
and the mesh effort chip when you pinned one (`mesh high`).

Each fact is on screen once, and a cell that would only say "nothing chosen" is not drawn:

- the routing tier is not in the row (the answer's label names the model that took the turn, and
  `/status` and `/mesh` keep the router's own names; a model you pinned shows no tier at all). The
  `Tier` widget still draws it when configured;
- the turn's tokens leave the row when the turn ends, because the turn's footer in the transcript
  (`1.7s · ↑12.3k ↓12 · subscription`) carries the same numbers;
- `⟨auto⟩` (the provider chose the rung), `mesh auto` (nothing pinned), `ctx` before any number
  exists and `untracked` (a model without a price) are left out; `/status` names each of them.

When the row is too narrow widgets drop in this order, and a faint `+N` counts them: throughput,
extras (quotas, MCP, branch, custom, session tokens), tokens, mesh, rung marker, tier, cost, then the
model. The gauge never drops; it shrinks to a short bar and then to `ctx 12%`. Configured widgets
still render: `Mode` lives on the composer border, and every widget is listed by `/status`.

### `/status`

`/status` (also in the palette, Ctrl+K under Session, and Help) opens a modal inspector; Esc closes
it. It lists MODEL (provider-qualified id, tier, mesh effort, model rung), MODE (temper, thinking,
goal, loop, queue), WHERE (directory, branch, worktree, remote control), CONTEXT (bar, tokens,
window, auto-compact), TURN, ACTIVITY (while busy), SESSION (id, turns, cost, tokens with cache),
QUOTAS, MCP and VERSION, plus a line saying how many status-line widgets were dropped at the
current width.

### Edge cases
| Edge case | Behaviour |
|-----------|-----------|
| Terminal < wordmark width | compact single-line wordmark |
| Very short terminal height | banner still drawn; conversation panel just smaller (banner only in welcome state, so no conflict) |
| No routing yet (before first turn) | status line shows `no model yet`, `ctx —` and `mesh auto`; no cost |
| Extremely narrow (<24 cols) | statusline shows just `$cost`; banner shows `FORGE` |
| busy but no tokens yet | the live line's spinner animates (tick-driven); under reduced motion it is a static `…` and the elapsed counter carries liveness |

## 6. Definition of done
- [ ] Welcome banner renders in brand orange on empty session; tagline shown
- [ ] Compact header shown once conversation has content
- [ ] Statusline shows spinner/tier/model/cost/hints with priority-based width handling
- [ ] Narrow-terminal fallbacks render without wrapping garbage (TestBackend at 30 + 80 cols)
- [ ] `cargo fmt` + `clippy -D warnings` clean; all existing tests pass
- [ ] New render tests: banner-on-empty, compact-on-nonempty, statusline contents, narrow fallback
- [ ] Verified live in the TUI against Ollama
