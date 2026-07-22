# Feature: TUI ASCII header & statusline

> Right-sized spec for a TUI cosmetic + UX feature. Implemented in `crates/forge-tui`
> immediately after.

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

### Statusline (bottom bar, replaces the plain footer)
Segments, separated by ` · `, left→right by priority; right side reserved for hints:

```
 ⠙ model reasoning   [complex] qwen3.8-max-preview   $0.0033         esc stop · Ctrl↑ escalate
```
- phase/spinner segment only when `busy`.
- `[tier] model` from `app.routing`.
- `$cost` from `app.cost_usd`.
- `mode` (permission) — optional v1.
- hints right-aligned; dropped first when width is tight.

### Live turn activity heartbeat

While a turn is busy, a dedicated row above the input reports event-backed progress rather than a
generic `working…` label:

```text
 ⠹ model reasoning  ·  qwen3.8-max-preview  ·  thought 42.8k chars  ·  event now
```

The phase follows real presenter events: preparing context, provider request/first-event wait,
reasoning, response streaming, tool execution, tool-result processing, provider recovery,
subagent/workflow coordination, compaction, and finalization. Hidden reasoning still increments a
visible character counter. The row shows `quiet Ns` after 15 seconds without an event and an amber
`⚠ no events for …` warning after 60 seconds. This is deliberately a heartbeat, not fabricated
percentage progress: a provider that is still computing is distinguishable from an actively
streaming model, but Forge does not claim how close either one is to completion.

Width handling: compute available cols; build segments in priority order
(model+cost, then tier, then mode, then hints) and stop adding when they won't fit.

### Narrow-terminal fallback
- If `area.width < WORDMARK_WIDTH (≈44)` in welcome state → render single-line
  `⚒ FORGE — model-mesh coding agent`.
- Statusline: drop hints → mode → tier until it fits; never drop model/cost.

### Edge cases
| Edge case | Behaviour |
|-----------|-----------|
| Terminal < wordmark width | compact single-line wordmark |
| Very short terminal height | banner still drawn; conversation panel just smaller (banner only in welcome state, so no conflict) |
| No routing yet (before first turn) | statusline shows brand + hints; model/cost show `—` / `$0.0000` |
| Extremely narrow (<24 cols) | statusline shows just `$cost`; banner shows `FORGE` |
| busy but no tokens yet | spinner segment animates (tick-driven, already wired) |

## 6. Definition of done
- [ ] Welcome banner renders in brand orange on empty session; tagline shown
- [ ] Compact header shown once conversation has content
- [ ] Statusline shows spinner/tier/model/cost/hints with priority-based width handling
- [ ] Narrow-terminal fallbacks render without wrapping garbage (TestBackend at 30 + 80 cols)
- [ ] `cargo fmt` + `clippy -D warnings` clean; all existing tests pass
- [ ] New render tests: banner-on-empty, compact-on-nonempty, statusline contents, narrow fallback
- [ ] Verified live in the TUI against Ollama
