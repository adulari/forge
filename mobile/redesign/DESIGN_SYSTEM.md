# Forge App — DESIGN SYSTEM ("Machined")

**Machined — supersedes Emberline.** Source of truth for the redesign's visual
language lives in `docs/design/machined/` (`Forge Machined - Desktop.dc.html`,
`Forge Machined - Mobile.dc.html`, `INVENTORY.md` maps every frame to its
implementation file). This document makes the taste decisions so implementation
workers never have to. Where a value appears here, it is the value — no worker
invents colors, sizes, durations, or curves.

## 0. The idea

Forge is a **control surface for a fleet of AI coding agents**. Machined's design
language is **"machined instrument panel"**: dense, hairline-defined graphite
surfaces read like a piece of precision hardware, not a soft consumer app — flat
planes, tight radii (3-4px), zero decorative glow, and technical text set in mono
wherever it names a real thing (a model id, a cost, a path, a count, a timer). One
living element, the ember accent, is reserved for *state* — busy, waiting-on-you,
the primary action, an active tab — never for constant brand decoration.

Machined explicitly retires the previous "thermal identity" pass (Emberline's
ambient forge-wash gradient, HeatEdge running/waiting gradients, StatusDot glow
halos and ring beacons): depth now reads through hairlines and flat color steps,
not through glowing edges. Every retired token/component still exists in code
(zero-alpha tokens, a HeatEdge that renders null) so the 292 files that import
them keep compiling — wave 2 removes the now-dead call sites.

Benchmarks and what we take/beat:

- **Linear / Raycast**: take the sharpness — hairline borders, tight radii,
  command palette, keyboard-first on desktop. Beat them on mobile ergonomics
  (they are desktop-first; we are thumb-first) and on chat-prose generosity.
- **Claude iOS**: take the calm, type-first hierarchy for chat prose specifically
  (body copy stays generous — see §7's "dense chrome, generous prose" rule).
  Beat it with real developer density everywhere else — fleet state, diffs, mesh.
- **A terminal / instrument panel**: take the monospace discipline for anything
  that names a real value. Beat it on approachability — mono is for facts, not
  for every label.

Character rules (apply everywhere):
1. **Ember is scarce and state-only.** Never a constant brand wash, never a
   persistent "selected" fill (Segmented's active state is a neutral overlay,
   not an ember tint) — only things that are alive or need a human get it.
2. **Hairlines, not shadows.** Depth reads through 1px border/hairline steps and
   flat bg0→bg3 color steps. No box-shadow on any card/surface in either theme.
   The one exception is `depth.sheet` (bottom sheets, centered overlays,
   command palette) — a modal genuinely floating above the whole UI still needs
   real lift; nothing else does.
3. **Mono names things, sans says things.** Any text that is a model id, a file
   path, a cost, a count, a timer, a branch/worktree name, or a section label
   renders in Geist Mono with tabular numerals. Everything conversational (chat
   prose, button labels, dialog copy) stays in Geist Sans.
4. **Tab-underline, not pill nav.** Session-scoped navigation (Chat/Tasks/
   Agents/Review/Replay) is a flat text-tab strip with a 2px accent underline on
   the active tab — never a filled pill. Segmented stays for true value pickers
   (READ/ASK/EDIT/FULL, LIGHT/DARK/SYSTEM) where a pill makes sense.
5. **Radius is compressed and uniform.** 3-4px on almost everything (buttons,
   inputs, chips, cards); 6px only on things that float above the whole UI
   (sheet top corners, centered overlay cards); pills stay pills (999).

---

## 1. Color

All values live in `src/theme/tokens.ts` (the ONLY hex file). Both themes ship;
dark is the brand-primary theme, light is a full first-class citizen (default:
follow system) — not an inverted dark theme, a genuinely separate palette.

### 1.1 Ember scale (brand, shared by both themes)

| token | hex | use |
|---|---|---|
| ember100 | `#FFE7D3` | tint text on ember-900 surfaces |
| ember200 | `#FFC9A0` | glow highlights, gauge tips |
| ember300 | `#FFA96B` | hover/active of accent on dark |
| ember400 | `#FF8A3D` | **the brand accent** (dark-theme interactive) |
| ember500 | `#F07A2E` | pressed accent (dark) |
| ember600 | `#C4601F` | light-theme interactive accent |
| ember700 | `#964916` | pressed accent (light) |
| ember900 | `#45210A` | ember-tinted dark wells (selection bg on dark) |

### 1.2 Dark theme (identity: "machined graphite")

| semantic token | hex/value | notes |
|---|---|---|
| bg0 | `#09090B` | deepest — code wells, recessed input fills |
| bg1 | `#0D0D11` | app/page background, sidebar/rail, sheet bg |
| bg2 | `#0E0E12` | cards, composer |
| bg3 | `#101015` | chips (hover), raised rows |
| border | `rgba(244,244,246,0.09)` | default hairlines/dividers/card edges |
| borderStrong | `rgba(244,244,246,0.14)` | inputs (focused, native), stronger dividers |
| hairline | `rgba(244,244,246,0.07)` | de-boxed list row separators (not card edges) |
| ink | `#F4F4F6` | primary text |
| ink2 | `#9A9AA6` | secondary text, inactive tab label |
| ink3 | `#5F5F6B` | tertiary/meta, section-header label, placeholders |
| ink4 | `#45454F` | disabled, footnotes |
| accent | `#FF8A3D` (ember400) | interactive, active, busy, active-tab underline |
| accentPressed | `#F07A2E` (ember500) | |
| onAccent | `#1A0E04` | text/icons on ember fills |
| success | `#5FB97D` | allow, done, diff-add, cost |
| danger | `#E5605C` | deny, destructive, waiting-critical, diff-del |
| warn | `#D9A94E` | plan notes, caution banners |
| info | `#7E9CB8` | diff hunk headers, neutral accents |
| successBg | `#0F1D14` | diff-add line bg |
| dangerBg | `#211012` | diff-del line bg |
| warnBg | `#201808` | banner bg (ink: `warnBgInk` `#EFD9AC`) |
| selection | `rgba(255,138,61,0.14)` | ember-tinted selected-row well (NOT Segmented's active fill — see §6) |
| overlayScrim | `rgba(5,5,6,0.6)` | behind sheets/modals |
| focusRing | `rgba(255,138,61,0.4)` | web keyboard-focus ring |

Retired thermal tokens (kept, zero-alpha, do not render anything): `forgeWashOpacity`
(0), `forgeWash`, `heatEdgeFrom`/`heatEdgeTo` (still resolve to ember400/500 —
harmless, nothing reads them for color since HeatEdge renders null), `heatGlow`,
`dotGlow`, `waitingEdgeFrom`/`waitingEdgeTo`/`waitingGlow`.

### 1.3 Light theme (identity: "machined steel, daylight")

A genuinely separate palette (per `docs/design/machined`'s "M Fleet Light" frame),
not an inverted dark theme.

| semantic token | hex/value | notes |
|---|---|---|
| bg0 | `#F5F4F1` | |
| bg1 | `#EFEDE8` | page background |
| bg2 | `#FFFFFF` | cards, composer |
| bg3 | `#F7F6F3` | chips, raised rows |
| border | `rgba(0,0,0,0.12)` | |
| borderStrong | `rgba(0,0,0,0.22)` | |
| hairline | `rgba(0,0,0,0.09)` | |
| ink | `#1C1B19` | |
| ink2 | `#6E6A61` | |
| ink3 | `#8A867D` | |
| ink4 | `#B0ACA2` | |
| accent | `#D96A1E` | ember400 fails contrast on paper — a distinct light accent |
| accentPressed | `#C25C15` | |
| onAccent | `#FFFFFF` | |
| success | `#4C8A60` | |
| danger | `#C44A42` | |
| warn | `#9A7A2E` | |
| info | `#5B7C94` | |
| successBg | `#E6F0E8` | |
| dangerBg | `#F8E6E3` | |
| warnBg | `#F5EDD8` (ink: `ink` — already legible on paper-toned bg) | |
| selection | `#F6E3D2` | |
| overlayScrim | `rgba(28,27,25,0.35)` | |
| focusRing | `rgba(217,106,30,0.4)` | |

### 1.4 Fixed semantic mapping (never swap)

`accent` = brand / active / busy / primary CTA / active-tab underline ·
`success` = allow / done / cost / diff-add · `danger` = deny / destructive /
**waiting-on-you** / diff-del · `warn` = notes/caution · `info` = diff hunks /
neutral-highlight. Status dots (`statusDotColor`): idle = ink3, busy = accent,
waiting = danger, done/past = ink4 — flat, no pulse-halo (see §6 StatusDot).
Context gauge fill (`gaugeColor`): accent; >70% warn; >90% danger.

---

## 2. Typography

- **Sans**: **Geist**, bundled and custom on every platform (native embeds it via
  the expo-font config plugin, same mechanism as mono; web ships its own
  `@font-face`). RN cannot vary a custom family's weight on native, so every
  `type` token resolves its family per-weight through `sansFamilyFor(400|500|
  600|700)` rather than one constant. Fallback stack on web:
  `"Geist, Inter, system-ui, sans-serif"` (Inter/system-ui only matter during
  the brief pre-load flash or a load failure).
- **Mono**: **Geist Mono** (bundled; Regular/Medium/SemiBold — the design never
  uses mono at true 700, so the `bold` key name maps to SemiBold). Used for
  every technical label: code blocks, diff bodies, model ids, session/worktree
  ids, paths/branches, costs, counts, timers, section headers, Segmented option
  labels, status Badges.
- Tabular numerals (`fontVariant: ["tabular-nums"]`) on every metric: cost,
  tokens, counts, times.

"2a Air" scale — mono floor 10.5px, dense chrome + generous chat prose (size/
line-height/weight — the only allowed combinations):

| token | size/lh | weight | use |
|---|---|---|---|
| display | 26/32 | 700, ls −0.5 | onboarding hero only |
| title | 18/24 | 700, ls −0.3 | screen titles |
| heading | 15/22 | 600 | card titles, row titles |
| headingBold | 15/22 | 700, ls −0.2 | screen-header variant of heading |
| body | 13/21 | 400 | chat text, default |
| bodyBold | 13/21 | 600 | emphasis, button labels |
| sub | 12/17 | 400 | secondary rows, descriptions |
| meta | 11/15 | 500 | status strip, timestamps, badges (sans base) |
| section | 10/14 | 600, ls 1, UPPERCASE | section headers (sans box; `SectionHeader` overrides family to mono — see §6) |
| code | 12/18 mono | 400 | code blocks, transcript code |
| codeSmall | 11/16 mono | 400 | diffs, agent tails, overlay body |
| monoMeta | 10.5/14 mono | 400 | tightest tier — secondary figures beside a bigger number |

Cost format: `$0.0421` (4 dp) under $1, `$12.48` (2 dp) above. Tokens:
`128.4k / 200k`. Relative times: `12s · 4m · 2h · 3d`.

---

## 3. Space, shape, depth, icons

- **Spacing scale (pt)**: 2, 4, 8, 12, 16, 20, 24, 32, 48. Screen gutter: 16
  (compact), 24 (medium+). Card padding: 12×14. List row min-height 56; dense
  rows 44. Tap targets ≥44×44 (dense 32px desktop-only variants land in a later
  wave).
- **Radii** (compressed — key names are historical pixel values, no longer
  literal): `radius4` → 3px (inline code, tiny badges), `radius8` → 4px
  (buttons, inputs, chips-square, most cards), `radius12` → 4px, `radius16` →
  6px (sheets, centered overlay cards — the one place a slightly larger radius
  reads as "floating above everything"), `radiusPill` → 999 (pills, dots),
  Segmented `radiusSegmentOuter` → 4 / `radiusSegmentInner` → 2.
- **Depth**: no shadow on any card/surface, either theme — hairlines only. The
  single exception is `depth.sheet` (`shadowOpacity 0.35, radius 24, offsetY
  8`), reserved for things that float above the whole UI: `Sheet`, centered
  overlays, the command palette. `depth.raised` still exists (dark: null,
  light: `shadowOpacity 0.03` — halved from the old 0.06) for any future
  surface that explicitly opts back into a whisper of lift, but no current ds/
  component uses it.
- **Icons**: lucide, stroke 1.75, sizes 16 (inline/meta), 20 (default), 24 (tab
  bar). Icon color follows the text color beside it. Canonical picks:
  Fleet=`flame`, Inbox=`bell-dot`, History=`history`, Settings=`settings-2`,
  send=`arrow-up` (in a filled accent circle), stop=`square`, attach=
  `paperclip`, worktree=`git-branch`, merge=`git-merge`, discard=`trash-2`,
  archive=`archive`, agents=`bot`, tasks=`list-checks`, review=`file-diff`,
  palette=`command`, scan=`scan-line`, mic=`mic`. The ⚒ mark stays ONLY as the
  app icon/logo, not an inline icon.

---

## 4. Voice & microcopy

Lowercase-calm, specific, human; no exclamation marks; errors say what
happened + what to do ("daemon unreachable — is `forge serve` running?").
Destructive confirms name the object ("Discard branch `forge/subagent/ab12` —
unmerged work is lost."). Empty states are one warm sentence + one action.
Server-sent `{error}` strings render verbatim (they are written for humans).
Unchanged from the previous pass — Machined is a visual/material rework, not a
voice rework.

---

## 5. Motion language

Reanimated v4 on native (UI thread); the same components on web use
Reanimated's JS driver or a CSS twin where noted. **Every** animation checks
`useReducedMotion()` and renders its final state statically when set (pulses
become solid, entrances instant, springs snap). Motion tokens/named patterns
(Strike, Cast, Rise, Kindle, Temper, Bellows, Anvil, Emberdot, Gaugeflow,
Signal, ...) are unchanged from the previous pass and still live in
`src/theme/motion.ts` — Machined is a color/type/shape rework, not a motion
rework. Two motion-adjacent things Machined does retire:

- **StatusDot's glow halo (busy) and ring beacon (waiting)** are gone —
  `useEmberdot`'s own opacity pulse on the dot itself remains (that is the
  dot's state animation, not a decorative glow); the halo/ring Views around it
  do not render.
- **ContextGauge's overheat shadow glow** (>90%) is gone — the color step to
  `danger` alone signals it now.

---

## 6. Component inventory (ds/ — every state specified)

State legend: D default · P pressed (Strike) · F focused (2px accent ring,
web/desktop; native: borderStrong) · L loading (spinner-in-place, label
persists at 0.6) · X disabled (0.4 opacity, no Strike) · E error (danger
border + sub-line) · M empty.

Mechanical rules applied across every component below: borders always come
from the `border`/`hairline` tokens, never a hardcoded literal; radius always
comes from the `radii` scale; no dark-theme card/surface carries a shadow;
anything technical (a count, a cost, a timer, a model id, a path) renders in
Geist Mono.

**Controls**
- `Button` — variants: `primary` (accent fill, onAccent text), `secondary`
  (transparent + 1px border — no longer a filled bg3 surface), `ghost`
  (transparent, ink2, hover bg3 on web), `danger` (transparent, danger-tinted
  35%-alpha border, danger text — the outlined "Deny" look), `allow` (success
  fill, successBg text). States D/P/F/L/X. Radius 4. Min-height 44/48(primary).
- `IconButton` — 44×44 hit area, 20px icon; D/P/F/X; optional badge dot.
- `Input` — a recessed fill (`bg0`, not `bg2` — reads as a well against
  surrounding cards), border, radius 4, 13pt body; label (meta, ink3) above;
  D/F/E/X; mono variant for URLs/paths; `clear` affordance.
- `TaskComposer` — pill radius 999, no shadow (border-only — Machined drops the
  old floating-pill lift); accent send disc.
- `Chip` — pill radius 999, bg2 (unselected) / selection-tinted bg (selected);
  used for command chips and filters. (Deliberately NOT forced into mono/
  uppercase — Chip labels are frequently free-text project/host names, not
  status vocabulary; see Badge below for the mono-status treatment.)
- `Segmented` — rectangular track (radius 4 outer / 2 inner); active segment
  gets a **neutral** low-alpha fill (`rgba(244,244,246,0.09)` dark /
  `rgba(0,0,0,0.08)` light — never an ember tint, per character rule 1) with
  an accent label; option labels render 11px semibold mono uppercase for
  short technical options (READ/ASK/EDIT/FULL, LIGHT/DARK/SYSTEM). No sliding
  thumb.
- `TabStrip` — flat text tabs on a hairline baseline; active = ink + 2px
  accent underline, inactive = ink2; optional mono count suffix or waiting
  dot. This is session-scoped navigation (Chat/Tasks/Agents/Review/Replay) —
  never a pill/fill, per character rule 4.
- `Switch`, `Checkbox` — accent when on; off-track is a fixed neutral
  (`#33333B` dark / `rgba(0,0,0,0.2)` light), not a theme background token.
- `SearchField` — Input + `search` icon + cancel; debounced 150ms.

**Status & data**
- `StatusDot(state: idle|busy|waiting|done)` — 6px, flat (no glow/halo);
  colors from `statusDotColor`.
- `Badge` — tones: neutral, accent, success, danger, warn, `outline`; radius
  2-3 (`small` shape) or pill; mono 10.5px label (all tones — badges are
  technical/status labels), uppercase for status tones.
- `ContextGauge` — 2px track (a fixed neutral overlay, not a border token) +
  animated fill (Gaugeflow), color steps accent→warn(>70%)→danger(>90%),
  `128.4k/200k` mono beside. No overheat glow (see §5).
- `CostMetric` — mono tabular numerals, success color, count-up.
- `KeyValueRow` — settings rows: label ink (sans — generic label/value pairs,
  not always technical), value ink2, optional chevron.
- `RelativeTime` — self-refreshing (30s), mono (a timer).

**Containers**
- `Screen` — safe-area, **bg0** (was bg1 — Machined drops the old ambient
  "forge wash" gradient entirely; the screen is just its flat background
  color now), gutter, optional scroll/keyboard-avoid; ONE per route.
- `Card` — bg2, radius 4 (both `default` and `feature` variants now resolve to
  the same radius), hairline-weight border, **no shadow**. The `heatEdge` prop
  still exists for source compat but renders nothing (`HeatEdge` is gutted).
- `ListRow` — 56pt min, Strike, hairline separator (inset 16), leading/
  trailing slots — de-boxed, radius 0, unchanged from the previous pass.
- `BoundedList` — FlatList wrapper: stable keys, `ListEmptyComponent`
  mandatory, pagination hooks, Bellows refresh (native), memoized rows.
- `Sheet` — radius 6 top corners, **bg1** (was bg2 — matches the app/page
  background so a sheet reads as "the same surface, raised" rather than a
  distinct card), keeps `depth.sheet` (one of the two places a shadow is still
  correct — see §3).
- `Toast` / `Banner` — tones warn/danger/neutral; unchanged from the previous
  pass.
- `EmptyState` — 24-32px lucide icon (ink3), one sub sentence, optional
  secondary Button.
- `Skeleton` — Temper shimmer blocks, radius 3 default.
- `ConfirmDialog` — centered ≤360pt card, radius 6 (overlay-scale radius, not
  the general 4px card radius); destructive variant: primary is Cancel, danger
  button holds "Discard" with a 400ms press-and-hold fill.
- `MasterDetail` — compact/medium stack via route navigation; expanded
  (≥1024) renders a persistent left rail (316pt default) + right detail pane.

Everything above composes into the higher-level chat/session/fleet/anywhere
components (`Composer`, `PermissionCard`, `PlanCard`, `SessionCard`,
`OverlayPanel`, `CommandPalette`, ...) — those are out of wave 1's scope; they
inherit the new tokens/type automatically and get their own Machined pass in
later waves.

---

## 7. Responsive layout (same screens, phone → desktop)

Breakpoints (window width, pt): `compact <640` · `medium 640–1023` ·
`expanded ≥1024`.

- **compact** (phones): bottom tab bar, stack navigation, sheets from bottom,
  composer pinned above keyboard.
- **medium** (tablets/portrait, narrow desktop windows): same tabs, content
  max-width 720 centered, gutters 24, modals become centered cards (≤560).
- **expanded** (web/desktop/tablet-landscape): **master–detail**: left rail
  316pt = fleet list, right = session detail (tabs become the `TabStrip`
  underline bar); tab bar disappears into the rail footer. Chat content column
  max-width 840. Command palette (⌘K) is the primary navigation. Hover states
  active; focus rings visible; text selection enabled in transcript/code/diff.
- **Dense chrome, generous prose**: every chrome element (rows, meta lines,
  section labels, badges) is dense and mono-technical per §6; chat message
  bodies are the one place type stays generous (`body` 13/21, not compressed
  further) — the "2a Air" scale compresses chrome, not conversation.
- One implementation: a `useBreakpoint()` hook + a `MasterDetail` layout
  component in ds/; route files stay identical (expo-router renders the same
  screens into either layout).

Accessibility floor: WCAG AA contrast for all ink-on-bg pairs listed in §1 ·
dynamic type up to 120% without breakage · every interactive element has
`accessibilityRole`/`Label` · reduce-motion per §5 · keyboard reachability on
web/desktop for every action.
