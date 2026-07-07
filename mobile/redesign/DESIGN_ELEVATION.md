# Forge App — DESIGN ELEVATION (Emberline v2)

Addendum to DESIGN_SYSTEM.md after first-build review. The base is clean but reads like a
generic dark app: too many identical rounded cards, flat hierarchy, the ember used as a mere
"accent" rather than as meaning. This document makes three specific, opinionated moves. It is
BINDING and overrides DESIGN_SYSTEM.md where they differ. Keep everything not mentioned here
exactly as specified in DESIGN_SYSTEM.md. Restraint is the rule — spend boldness only on move 1.

---

## Move 1 — Thermal identity (the one bold idea): **state is temperature**

Forge is a forge. The ember (`#FF913C`) stops being decoration and becomes *heat = alive*.
A session's activity is rendered as temperature, cooling to graphite when idle. This is the
signature no competitor has. Apply it precisely, never gaudily.

- **Heat edge** (new): a live/working session shows a 3px leading-left vertical bar with a
  vertical gradient `ember400 → ember500`, plus a soft outward glow
  `shadow: color-mix(ember400 22%) blur 16 offsetX 0`. Idle/done sessions have NO heat edge
  (cool). This replaces the "one identical card per session" look — see Move 2.
- **Heat dot**: `StatusDot` busy state gains a radial ember glow halo (12px, `ember400` at 18%
  opacity, no hard edge) behind the 9px dot; the dot itself keeps the 1s pulse. Waiting keeps the
  danger beacon (it's an *alarm*, hotter than busy — danger, not ember). Idle/done: flat, no glow.
- **Overheat gauge**: `ContextGauge` fill gains a faint same-color glow as it passes 70% (warn)
  and 90% (danger) — the context window visibly "runs hot". Below 70%: accent, no glow.
- **Forge ambient**: the app background (`Screen` on bg1) gets ONE very subtle top ambient ember
  wash: `radial-gradient(1100px 420px at 50% -8%, color-mix(ember400 5%, transparent), transparent 62%)`
  on dark; `4%` on light. Barely perceptible — it makes the whole surface feel lit from a forge,
  not flat. Implement once in `Screen` behind content; never repeat per-card.

New tokens (add to `src/theme/tokens.ts`, both themes):
`heatEdgeFrom = ember400`, `heatEdgeTo = ember500`,
`heatGlow = 'rgba(255,145,60,0.22)'` (dark) / `'rgba(199,93,16,0.20)'` (light),
`dotGlow = 'rgba(255,145,60,0.18)'` (dark) / `'rgba(199,93,16,0.16)'` (light),
`forgeWash` (the radial string above per theme). Keep them semantic; no raw hex elsewhere.

## Move 2 — De-box: ink hierarchy over containers

The Fleet showed five identical rounded boxes — the exact "rounded card everywhere" default to
avoid. Fix: **list items are hairline-separated rows, not cards.** Cards are reserved for
*content that is genuinely a discrete object*: plan, diff, permission, question, agent, the
Settings groups, the New-Session form. Not for list rows.

- **Fleet / Inbox / History rows**: a `ListRow`-style layout separated by 1 hairline (inset 16),
  NO per-row border/box/fill. Live rows carry the Move-1 heat edge at the far left (bleeding to
  the screen edge) as the ONLY container-ish affordance; idle rows are pure type on bg1. Selected
  (waiting) row: `selection` tint wash + heat/alarm edge. This alone reads far more editorial.
- Stronger vertical rhythm: row min-height 72 for Fleet (was cramped), 16pt internal gaps, the
  title on its own line at `heading` weight, the metadata line in `sub`/mono, the gauge+cost line
  quiet. Let whitespace separate, not borders.
- Keep the Fleet aggregate header, but as an airy 3-up of *type* (big tabular number + tiny
  uppercase label), hairline-separated, NOT three bordered tiles.

## Move 3 — Editorial, instrument-grade type & detail

Make the type itself memorable and the details sharp; this is where "beautiful" comes from once
the boxes are gone.

- **Titles**: screen titles `title` weight 700 at `letter-spacing: -0.4`, tighter and more
  confident. Add the ⚒ forge mark (small, `ink3`) beside the Fleet title only — the one identity
  moment. Nowhere else inline.
- **Metrics are instrument readouts**: ALL numbers — cost, tokens, context, times, session-id
  tails, ports — in JetBrains Mono with `tabular-nums`. The mono is the "gauge cluster" texture.
  Cost animates with the existing count-up.
- **Section labels** (`SectionHeader`): prepend a 6px ember tick (2px tall ember bar) before the
  uppercase label, then the label, then a hairline rule filling the row. Signature, quiet.
- **Tab bar**: active tab gains a 2px ember "heat" underline (top of the tab) + the icon's 4%
  scale tick; inactive stays `ink3`. The bar itself is bg2 with a single top hairline — refined,
  not heavy.
- **Segmented**: thumb slides on the `press` spring (already specced) — verify it actually
  animates; add a 1px inset hairline on the thumb for a "machined" edge.
- **Hairlines & radii**: hairlines use `StyleSheet.hairlineWidth`; content-card radius stays 12,
  feature cards (plan/diff/permission) 16; buttons/inputs 10 (slightly softer than 8). Depth:
  dark = elevation + hairline only (no drop shadows except sheets/palette); light = one whisper
  ambient shadow on true cards only.
- **Disabled buttons**: the current 0.4-opacity ember reads muddy-brown. Change disabled fill to
  `bg3` with `ink4` label (not a dimmed accent) so primary CTAs never look like mud.

---

## Where each move lands (for the workers)
- Tokens + `Screen` forge-wash + `StatusDot` heat + `ContextGauge` overheat + `SectionHeader`
  tick + `Segmented` machined thumb + `Button` disabled fix + a new `HeatEdge` ds primitive →
  **elevation wave EW1** (foundation; safe to run against theme + ds/).
- `SessionCard`→de-boxed heat-row, Fleet/Inbox/History list restyle, Fleet header airy 3-up + ⚒,
  tab-bar heat underline, Connect micro-polish → **EW2** (screens).
- All B3+ content cards (Plan/Diff/Permission/Question/Agent) honor Move 1 (heat where the object
  is "live"/pending) + Move 3 (feature-card radius 16, mono metrics, refined detail).

Taste guardrail: this is a *precision instrument*, not a light show. If a screen has more than the
heat edge + one or two ember dots + the active tab visible, it's too hot — cool it down.
