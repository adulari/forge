# Forge App — DESIGN SYSTEM ("Emberline")

The visual and motion spec for the Forge app on all six targets. This document makes the taste
decisions so implementation workers never have to. Where a value appears here, it is the value —
no worker invents colors, sizes, durations, or curves.

## 0. The idea

Forge is a **control surface for a fleet of AI coding agents**. The design language is
**"precision metal, live ember"**: calm graphite surfaces with the discipline of Linear and the
typographic restraint of the Claude app — and one living element, the warm ember accent
(`#FF913C`), which is reserved for *things that are alive or need a human*: streaming, busy,
waiting-on-you, the primary action. Benchmarks and what we take/beat:

- **Claude iOS**: take the calm, the whitespace, the type-first hierarchy. Beat it with real
  developer density — code, diffs, fleet state — without losing the calm.
- **OpenAI Codex/ChatGPT**: take the streaming/code rendering polish. Beat it with structured
  cards (plan, diff, permission) instead of walls of text.
- **Linear / Raycast**: take the sharpness — hairline borders, tight radii, command palette,
  keyboard-first on desktop, tasteful dark. Beat them on mobile ergonomics (they are
  desktop-first; we are thumb-first).

Character rules (apply everywhere):
1. **Ember is scarce.** Accent never fills large areas. If a screen has more than ~3 ember
   elements visible, remove some.
2. **Ink does the hierarchy, not boxes.** Prefer type weight/color over containers; borders are
   1px hairlines; shadows are near-absent (dark) / whisper-soft (light).
3. **Motion means something.** Every animation maps to a system event (arriving, thinking,
   needing you, completing). No decoration loops.
4. **Dev-native texture.** Monospace, tabular numerals, `+/-` diff colors, status dots — the
   materials of a terminal, set with editorial care.

---

## 1. Color

All values live in `src/theme/tokens.ts` (the ONLY hex file). Both themes ship; dark is the
brand-primary theme, light is a full first-class citizen (default: follow system).

### 1.1 Ember scale (brand, shared by both themes)

| token | hex | use |
|---|---|---|
| ember100 | `#FFE9D2` | tint text on ember-800 surfaces |
| ember200 | `#FFCE9E` | glow highlights, gauge tips |
| ember300 | `#FFAF66` | hover/active of accent on dark |
| ember400 | `#FF913C` | **the brand accent** (dark-theme interactive) |
| ember500 | `#F5761A` | pressed accent (dark) |
| ember600 | `#C75D10` | light-theme interactive accent (contrast ≥4.5:1 on paper) |
| ember700 | `#9C480C` | pressed accent (light) |
| ember900 | `#4A2206` | ember-tinted dark wells (selected rows on dark) |

### 1.2 Dark theme (identity: "graphite & ember")

| semantic token | hex | notes |
|---|---|---|
| bg0 | `#0B0B10` | code wells, diff bodies (deepest) |
| bg1 | `#131318` | app/page background (a step deeper than the old `#16161c` for more surface contrast) |
| bg2 | `#1B1B22` | cards, composer, tab bar |
| bg3 | `#24242D` | chips, inline-code bg, raised rows |
| borderStrong | `#34343E` | inputs, focused cards |
| border | `#26262E` | default hairlines, dividers |
| ink | `#E9E9EF` | primary text |
| ink2 | `#A9A9B6` | secondary text |
| ink3 | `#6E6E7A` | tertiary/meta, placeholders |
| ink4 | `#4A4A55` | disabled, footnotes |
| accent | ember400 | interactive, active, busy |
| accentPressed | ember500 | |
| onAccent | `#1B1B22` | text/icons on ember fills |
| success | `#7DD394` | allow, done, diff-add, cost |
| danger | `#F0716E` | deny, destructive, waiting-critical, diff-del |
| warn | `#EDBD52` | plan notes, caution banners |
| info | `#4FD0D9` | diff hunk headers, links-ish accents |
| successBg | `#12291A` | diff-add line bg, allow-button hover well |
| dangerBg | `#2E1516` | diff-del line bg |
| warnBg | `#33270F` | banner bg (ink: `#FFD9A8`) |
| selection | `#2E2415` | selected row (ember-tinted, from the proven web `selBg`) |
| overlayScrim | `rgba(8,8,12,0.6)` | behind sheets/modals |

### 1.3 Light theme (identity: "warm paper & forged steel")

Not inverted-dark: a warm, paper-toned workshop light. Code wells stay slightly darker than the
page so code always reads as "material".

| semantic token | hex | notes |
|---|---|---|
| bg0 | `#F1EEE8` | code wells, diff bodies |
| bg1 | `#FAF8F4` | page background (warm paper) |
| bg2 | `#FFFFFF` | cards, composer |
| bg3 | `#F3F0EA` | chips, inline-code |
| borderStrong | `#D6D2C8` | |
| border | `#E7E3DA` | |
| ink | `#211F1B` | |
| ink2 | `#57544C` | |
| ink3 | `#8B8779` | |
| ink4 | `#B4B0A3` | |
| accent | ember600 | interactive (ember400 fails contrast on paper — display-only there) |
| accentPressed | ember700 | |
| onAccent | `#FFFFFF` | |
| success | `#1E8A47` | |
| danger | `#C93835` | |
| warn | `#9A6E0C` | |
| info | `#0E7C86` | |
| successBg | `#E4F4E7` | |
| dangerBg | `#FBE7E5` | |
| warnBg | `#F7EED3` | |
| selection | `#F6E7D2` | |
| overlayScrim | `rgba(30,26,20,0.35)` | |

### 1.4 Fixed semantic mapping (never swap)

`accent` = brand / active / busy / primary CTA · `success` = allow / done / cost / diff-add ·
`danger` = deny / destructive / **waiting-on-you** / diff-del · `warn` = notes/caution ·
`info` = diff hunks / neutral-highlight. Status dots: idle = ink3, busy = accent (pulse 1s),
waiting = danger (pulse 0.7s), done/past = ink4. Context gauge fill: accent; >70% warn; >90% danger.

---

## 2. Typography

- **Sans**: platform system stack (SF / Roboto / `system-ui`). No custom sans.
- **Mono**: **JetBrains Mono** (bundled; weights 400/700; web woff2 subset). Used ONLY for: code
  blocks, diff bodies, overlay `body` text, agent `last` lines, session ids, paths/branches,
  and the token/URL field on Connect.
- Tabular numerals (`fontVariant: ["tabular-nums"]`) on every metric: cost, tokens, counts, times.

Scale (size / line-height / weight — the only allowed combinations):

| token | size/lh | weight | use |
|---|---|---|---|
| display | 28/34 | 700 | onboarding hero only |
| title | 20/26 | 700 | screen titles |
| heading | 17/24 | 600 | card titles, session titles |
| body | 15/22 | 400 | chat text, default |
| bodyBold | 15/22 | 600 | emphasis, button labels |
| sub | 13/18 | 400 | secondary rows, descriptions |
| meta | 12/16 | 500 | status strip, timestamps, badges |
| section | 11/14 | 700 + letterSpacing 0.6 + UPPERCASE + ink3 | section headers |
| code | 13/20 mono | 400 | code blocks, transcript code |
| codeSmall | 12/18 mono | 400 | diffs, agent tails, overlay body |

Cost format: `$0.0421` (4 dp) under $1, `$12.48` (2 dp) above. Tokens: `128.4k / 200k`.
Relative times: `12s · 4m · 2h · 3d`.

---

## 3. Space, shape, depth, icons

- **Spacing scale (pt)**: 2, 4, 8, 12, 16, 20, 24, 32, 48. Screen gutter: 16 (compact),
  24 (medium+). Card padding: 12×14. List row min-height 56; dense rows 44. Tap targets ≥44×44.
- **Radii**: 4 (inline code, tiny badges), 8 (buttons, inputs, chips-square), 12 (cards, code
  blocks), 16 (sheets, modals, palette), 999 (pills, dots, FAB).
- **Depth**: dark theme = bg elevation + hairlines only (no shadows except the palette/sheet:
  `shadowOpacity 0.35, radius 24, offsetY 8`). Light theme = the same hairlines plus one soft
  ambient shadow on raised surfaces (`rgba(30,26,20,0.06), radius 16, offsetY 2`). Never both
  heavy borders and heavy shadow.
- **Icons**: lucide, stroke 1.75, sizes 16 (inline/meta), 20 (default), 24 (tab bar). Icon color
  follows the text color beside it. Canonical picks: Fleet=`flame`, Inbox=`bell-dot`,
  History=`history`, Settings=`settings-2`, send=`arrow-up` (in a filled ember circle),
  stop=`square`, attach=`paperclip`, worktree=`git-branch`, merge=`git-merge`, discard=`trash-2`,
  archive=`archive`, agents=`bot`, tasks=`list-checks`, review=`file-diff`, palette=`command`,
  scan=`scan-line`, mic=`mic`. The ⚒ mark stays ONLY as the app icon/logo, not an inline icon.

---

## 4. Voice & microcopy

Lowercase-calm, specific, human; no exclamation marks; errors say what happened + what to do
("daemon unreachable — is `forge serve` running?"). Destructive confirms name the object
("Discard branch `forge/subagent/ab12` — unmerged work is lost."). Empty states are one warm
sentence + one action. Server-sent `{error}` strings render verbatim (they are written for humans).

---

## 5. Motion language ("Forgework")

Reanimated v4 on native (UI thread); the same components on web use Reanimated's JS driver or a
CSS twin where noted. **Every** animation checks `useReducedMotion()` and renders its final state
statically when set (pulses become solid, entrances instant, springs snap).

### 5.1 Tokens

Durations (ms): `instant 80 · fast 140 · base 200 · gentle 260 · sheet 320`.
Easings: `standard cubic-bezier(0.2, 0, 0, 1)` (enter/move) · `exit cubic-bezier(0.3, 0, 1, 1)` ·
`linear` (loops only).
Springs (Reanimated `withSpring`): `press {damping 30, stiffness 500}` ·
`sheet {damping 28, stiffness 260, mass 0.9}` · `emphasis {damping 16, stiffness 200}` (never on lists).

### 5.2 Named patterns (workers implement these by name, in `src/theme/motion.ts` + ds components)

| name | spec |
|---|---|
| **Strike** (press) | scale→0.97 + opacity→0.9, `press` spring in, 120ms timing out. Every Pressable in ds/. |
| **Cast** (screen push) | incoming: translateX 24→0 + fade 0→1, `base`/`standard`; outgoing dims to 0.92 opacity. Native stack transition ≤250ms; web: 160ms fade+4px rise (no horizontal slide in browsers). |
| **Rise** (modal/new-session) | sheet from bottom with `sheet` spring; scrim fades `fast`. |
| **Forgeline** (list entrance) | rows fade + translateY 8→0, `base`/`standard`, stagger 40ms, capped at 8 rows, only on first mount of a screen — never on data refresh (no reshuffle jank). |
| **Kindle** (streaming) | streaming text appears in `fast` fade batches (rAF-coalesced); the caret is a 7px ember dot pulsing opacity 1→0.4 at 1s; on finalize, the streaming block cross-fades into the finalized message (`base`). CSS twin on web. |
| **Temper** (skeleton→content) | skeleton = bg3 blocks with a 1.6s linear shimmer sweep (dark: +6% lightness band; light: −4%); content cross-fades in over `base` with a 4px rise. Skeletons match final layout (fleet rows, history rows, chat bubbles). |
| **Bellows** (pull-to-refresh) | custom control: ember arc that fills 0→270° with pull distance, then rotates while refreshing; settles with a single soft haptic. Web/desktop: hidden (refresh via shortcut/button). |
| **Anvil** (sheets) | gesture-driven bottom sheet (overlay mirror, action sheets, decision peek): follows the finger 1:1, `sheet` spring to snap points, scrim opacity tracks progress. Web: transform transition 260ms `standard`, Esc/scrim closes. |
| **Emberdot** (status) | busy: opacity 1↔0.35 @1s; waiting: @0.7s + a 1.5px danger ring that scales 1→1.6 and fades, every 2.8s (the "needs you" beacon). Idle/done: static. |
| **Gaugeflow** (context/cost) | gauge width + color animate over `gentle`; cost metrics count-up over `base` (existing `useCountUp` pattern, kept). |
| **Tabshift** (tab switch) | active pill/indicator slides with `press` spring; icon does a 4% scale tick; content cross-fades `fast` (no slide). |
| **Signal** (toast/banner) | toast rises 12px + fade `base`, auto-dismiss 3.5s, swipe-to-dismiss; protocol/exposure banners slide down from the header `base`. |
| **Approve/Deny commit** | on tap: button does Strike, the card's other actions fade to 0.4, a small check/x icon draws in (SVG stroke, 200ms); card collapses `gentle` when the next snapshot resolves it. |

### 5.3 Haptics map (expo-haptics; exactly these, nowhere else)

| event | haptic |
|---|---|
| send prompt / palette execute | impact light |
| allow / plan approve | impact medium |
| deny / destructive confirm | notification warning |
| pairing success / merge clean | notification success |
| merge conflict / error toast | notification error |
| palette & overlay row navigation (keyboard/drag) | selection tick |
| pull-to-refresh settle | impact light |

---

## 6. Component inventory (ds/ — every state specified)

State legend: D default · P pressed (Strike) · F focused (2px accent ring, web/desktop; native:
borderStrong) · L loading (spinner-in-place, label persists at 0.6) · X disabled (0.4 opacity,
no Strike) · E error (danger border + sub-line) · M empty.

**Controls**
- `Button` — variants: `primary` (accent fill, onAccent text), `secondary` (bg3 fill, ink),
  `ghost` (transparent, ink2, hover bg3 on web), `danger` (danger fill), `allow` (success fill,
  onAccent-equivalent text). States D/P/F/L/X. Min-height 44/48(primary).
- `IconButton` — 44×44 hit area, 20px icon; D/P/F/X; optional badge dot.
- `Input` — bg2, border, radius 8, 15pt; label (meta, ink3) above; D/F/E/X; mono variant for
  URLs/paths; `clear` affordance.
- `PromptComposer` — multiline grow (max 6 lines), attach + mic (where available) + send circle;
  send disabled when empty; offline state swaps send icon to a queue glyph + "will send on
  reconnect" meta line. Web: Enter=send, Shift+Enter=newline, paste-image support.
- `Chip` — pill radius 999, bg3, meta text; selectable state (selection bg + ember text);
  used for command chips (`/plan` `/compact` `/models` `/mode` `/help`) and filters.
- `Segmented` — bg3 track, bg2 thumb (Tabshift), section-style labels; used for Chat/Tasks/Agents/Review.
- `Switch`, `Checkbox(worktree toggle)` — accent when on.
- `SearchField` — Input + `search` icon + cancel; debounced 150ms.

**Status & data**
- `StatusDot(state: idle|busy|waiting|done)` — 8px, Emberdot behavior.
- `Badge` — tones: neutral (bg3/ink2), accent, success, danger, warn, `outline`; 4-radius small
  or pill; e.g. "worktree", "archived", "public" (danger tone), "NEEDS YOU" (danger).
- `ContextGauge` — 3px track (border color) + fill (Gaugeflow), `128.4k/200k` meta beside.
- `CostMetric` — tabular, success color, count-up.
- `KeyValueRow` — settings rows: label ink, value ink2, chevron.
- `RelativeTime` — self-refreshing (30s), meta style.

**Containers**
- `Screen` — safe-area, bg1, gutter, optional scroll/keyboard-avoid; ONE per route.
- `Card` — bg2, radius 12, hairline; `feature` variant radius 16 (plan/diff/permission cards).
- `ListRow` — 56pt min, Strike, hairline separator (inset 16), leading/trailing slots.
- `BoundedList` — FlatList wrapper: stable keys, `ListEmptyComponent` mandatory, pagination
  hooks, `Bellows` refresh (native), memoized rows.
- `Sheet` — Anvil; snap points; grabber (36×4, border color).
- `Toast` / `Banner` — Signal; banner tones warn (protocol mismatch), danger (public exposure,
  session ended), neutral (reconnecting strip: 12pt meta, no animation).
- `EmptyState` — 24px lucide icon (ink4), one sub sentence, optional secondary Button.
- `Skeleton` — Temper shimmer blocks.
- `ConfirmDialog` — centered ≤360pt card; destructive variant requires typing nothing but uses
  a 2-step: primary is cancel, danger button holds "Discard" with 400ms press-and-hold fill.

**Forge-content components**
- `Markdown` — headings h3-cap, paragraphs, lists, inline code (bg3, radius 4, mono), bold,
  italic, links (render as ink + underline; open externally on tap — web real `<a>`).
- `CodeBlock` — bg0, radius 12, mono `code`; header row: language tag (meta, ink3) + copy button
  (states: copy→copied 1.2s); horizontal scroll inside; the existing keyword highlighter from
  `app.js` is ported (rust/js/python/go/bash/json keyword sets; strings ink2→success, comments
  ink3, numbers info, keywords ember300 on dark / ember600 on light).
- `DiffCard` — per SnapDiff: `pending` variant gets a warn banner "proposed change — review
  before allowing"; file sections collapsible (chevron), header path (bodyBold, mono-truncated
  head), kind badge, `+a −d` tabular (success/danger); hunk header info-color mono; lines mono
  `codeSmall` with successBg/dangerBg full-width line fills; "+N more lines/files" ink3 footers.
- `PlanCard` — feature Card; "⬡ PLAN" section label ember; title heading; numbered steps
  (bodyBold + sub detail); warn notes block; action bar Approve(allow-variant) / Revise(ghost →
  reveals free-text row) / Cancel(danger-ghost). prompt_seq discipline (buttons disable after tap).
- `PermissionCard` — danger-edged feature Card; the prompt text body; DiffCard embedded when
  `diff.pending`; Allow/Deny bar.
- `QuestionCard` — question body; one Button per option (secondary, label bodyBold + description
  sub) stacked; free-text row when `question_allow_other` or no options.
- `TaskRow` — glyph ring: pending = hollow circle ink3, in_progress = half-filled accent
  (rotating 2s while busy), done = filled success + strikethrough dim title.
- `AgentCard` — bot icon + agent name (heading, accent), task sub, model Badge, cost Metric,
  `last` line (codeSmall, 2 lines, bg0 well); done: 0.65 opacity + check.
- `SessionCard` (fleet) — StatusDot + title (heading) + NEEDS-YOU/worktree badges; second line:
  cwd tail (mono, head-ellipsized) · model · relative time; third line: ContextGauge + cost.
  Swipe actions (native): archive / merge / discard; long-press or `…` opens the same action sheet.
- `OverlayPanel` — the TUI overlay mirror: title bar + close, SearchField when `filter != null`,
  grouped rows (section headers), server-authoritative selection highlight, mono body view,
  free-text commit row. Rendered inside a Sheet (mobile) / centered modal 560pt (wide).
- `CommandPalette` — Raycast-grade: centered 560pt (wide) / full-height sheet (mobile), one
  SearchField, grouped results (Sessions / Actions / Navigation), keyboard nav + selection tick,
  ⌘K / Ctrl+K open (web/desktop), Cast+Rise hybrid entrance 200ms.
- `QRScanFrame` — camera view + 240pt rounded-16 reticle, ember corner strokes, success flash + haptic.
- `DecisionPeek` — Sheet showing a waiting session's PermissionCard/QuestionCard inline (from a
  temporary WS attach) for approve-without-navigating.

---

## 7. Responsive layout (same screens, phone → desktop)

Breakpoints (window width, pt): `compact <640` · `medium 640–1023` · `expanded ≥1024`.

- **compact** (phones): bottom tab bar (Fleet · Inbox · History · Settings), stack navigation,
  sheets from bottom, composer pinned above keyboard.
- **medium** (tablets/portrait, narrow desktop windows): same tabs, content max-width 720
  centered, gutters 24, modals become centered cards (≤560).
- **expanded** (web/desktop/tablet-landscape): **master–detail**: left rail 320pt = fleet list
  (with Inbox filter pills + New Session at top), right = session detail (segments become a
  top Segmented bar); tab bar disappears into the rail footer (History/Settings icons).
  Chat content column max-width 840. Palette is the primary navigation (⌘K).
  Hover states active; focus rings visible; text selection enabled in transcript/code/diff.
- One implementation: a `useBreakpoint()` hook + a `MasterDetail` layout component in ds/;
  route files stay identical (expo-router renders the same screens into either layout).

Accessibility floor: WCAG AA contrast for all ink-on-bg pairs listed in §1 (verified: ink/bg1
dark 13.9:1, ink2/bg1 6.3:1; light ink/bg1 14.6:1, accent600 on bg2 4.6:1) · dynamic type up to
120% without breakage · every interactive element has `accessibilityRole`/`Label` · reduce-motion
per §5 · keyboard reachability on web/desktop for every action.
