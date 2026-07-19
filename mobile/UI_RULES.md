# Original UI rules — superseded by the Emberline redesign

> **Historical implementation record.** These rules governed the first companion-app build and
> are retained to explain older decisions. They are not binding for current work: component paths,
> styling, theming, motion, haptics, and responsive behavior were superseded by
> [redesign/DESIGN_SYSTEM.md](redesign/DESIGN_SYSTEM.md) and
> [redesign/ARCHITECTURE.md](redesign/ARCHITECTURE.md). The shipped app supports mobile, web, and
> Tauri desktop with light, dark, and system themes.

## 1. Structure

1. **One `Screen` wrapper per page.** Every route file renders exactly one `<Screen>` (from
   `src/components/ui.tsx`) as its root. Screen owns: safe-area insets, bg `theme.bg`, horizontal
   gutter 12, and the scroll container. Never nest Screens; segments inside `session/[id]` render
   content only (the shell owns the Screen).
2. **ONE header pattern.** Headers come from the route layout (expo-router header or the session
   shell's custom header) — never hand-rolled `<View>` headers inside page bodies. Header =
   title (16/700, `ink`), optional subtitle (12, `dim`, numberOfLines={1}), optional right-side
   Badge/icon button. No screen invents its own header variant.
3. **Data via hooks only.** Screens call hooks from `src/lib/queries.ts` / `src/lib/ws.ts`.
   No raw `fetch` or `new WebSocket` in a component file, ever.
4. **Types are the wire.** Use the snake_case types from `src/lib/api.ts` verbatim
   (`cost_usd`, `context_tokens`, `prompt_seq`). No camelCase re-mapping layers.

## 2. Layout

5. **Rows are `flex-row` with `flex-1` + `numberOfLines`.** Any text sharing a row with other
   elements gets `className="flex-1"` and `numberOfLines={1}` (2 for previews). Paths/branches
   ellipsize head (`ellipsizeMode="head"` for cwd tails), titles ellipsize tail. No row may ever
   push its siblings off-screen.
6. **Touch targets ≥ 44×44pt.** Buttons min-height 44; icon buttons get `hitSlop` to reach 44.
   Chips (pill, radius 14) keep ≥44 total height via padding. Adjacent destructive/confirm
   buttons need ≥8 gap.
7. **Lists are `BoundedList`** (FlatList wrapper): keyed by stable id (`id`, `seq`), pagination
   via `onEndReached` wired to the infinite query, `refreshControl` for pull-to-refresh, and a
   mandatory `ListEmptyComponent`. Never `.map()` an unbounded array in a ScrollView; transcript
   uses inverted FlatList.
8. Spacing scale only: 2/4/6/8/10/12/16. Card padding 8×10 (10 radius for feature cards,
   8 default). Screen gutter 12. No magic numbers outside the scale.

## 3. Theme

9. **Historical rule: theme tokens only.** The original build took colors from `theme.ts` and was
   dark-only. The current source of truth is `src/theme/tokens.ts`, with light/dark/system theme
   selection through the redesign's ThemeProvider.
10. Semantic color use is fixed: `accent` = brand/active/busy/pending-attention; `ok` =
    success/allow/cost/done/diff-add; `no` = danger/deny/waiting/diff-del; `dim` = secondary;
    `ink` = primary text. Allow buttons are ALWAYS `ok` bg with `#1c1c22`-equivalent
    (`theme.panel`) text; Deny/destructive ALWAYS `no`. Never swap these.
11. Numbers (cost, tokens, counts, times) use tabular numerals (`font-variant-numeric` via
    fontVariant: ['tabular-nums']) and the Metric primitive. Cost format: `$` + 4 decimals
    under $1, 2 decimals above. Context gauge: `Xk/Yk` + thin bar, `accent` >70%, `no` >90%.
12. Monospace (`Menlo`/platform mono, 12/1.5) is used ONLY for: code blocks, diff bodies,
    overlay `body`, agent `last` lines — always on `codeBg` with radius 8 and horizontal scroll
    inside its own container.

## 4. States

13. **Loading/error/empty via primitives, on every data surface.** `Loading` (centered spinner,
    dim) while first fetch; `ErrorText` + retry button on failure (surface the server's `{error}`
    string verbatim — it is written for humans); `EmptyState` (icon + one dim sentence + optional
    action) when lists are empty. No blank screens, no bare `<Text>Error</Text>`.
14. Distinguish the three failure classes everywhere the connection can fail: wrong token (404 →
    "pairing invalid, re-scan"), unreachable (network error → "server unreachable" + hint), and
    server error (5xx → show `{error}`). Connect and Settings must show which one occurred.
15. **Destructive actions confirm.** Archive = one confirm. Discard = double confirm naming the
    branch ("deletes branch and worktree, unmerged work is lost"). Merge failure states render
    their payloads: 409 `dirty_files` and 409 `conflicts` as file lists, never a generic toast.
16. **prompt_seq discipline.** Allow/Answer UI always sends the `prompt_seq` of the snapshot it
    rendered from. On 409/ignored, do NOT retry — re-render from the next snapshot. Disable the
    card's buttons after first tap until a new snapshot arrives.

## 5. Motion & feel

17. **Reduce-motion guard on every animation.** All Reanimated animations (pulse dots, card
    entrances, tab transitions) check `useReducedMotion()` and render the final state statically
    when true. Pulse dots become solid.
18. Animations are subtle and short: ≤200ms entrances, no springs on lists, the ONLY looping
    animations are the busy (1s) and waiting (0.7s) dot pulses (opacity to 0.35, matching web).
19. Haptics (expo-haptics, light) only on: send prompt, allow/deny, destructive confirm. Nowhere
    else.

## 5b. QUALITY BAR (first-class, binding on every screen)

The app must feel SUPER fast, look genuinely beautiful, and be obviously intuitive. These are
checkable rules, not adjectives — a screen that fails any of them is not done.

**Performance budget (per screen):**
25. Warm start is INSTANT: on cold open, screens render from the persisted react-query cache
    (`PersistQueryClientProvider` + AsyncStorage) BEFORE any network — never a spinner over stale
    cache. First contentful paint from cache < 1 frame; network refresh happens underneath.
26. Every list is virtualized (`BoundedList`/FlatList, never ScrollView+map). Rows are `React.memo`
    with stable keys and a custom `areEqual` comparing only the fields the row renders. No inline
    arrow functions or object literals in `renderItem` props (hoist/`useCallback`/`useMemo`).
27. No layout thrash: fixed row heights where possible (`getItemLength`), `removeClippedSubviews`,
    sensible `windowSize`/`maxToRenderPerBatch`. Images (icons) are fixed-size. Zero synchronous
    layout in scroll handlers.
28. 60fps always: Reanimated animations run on the UI thread (worklets) — never animate via
    React state/`setState` in a loop. WS snapshots (up to many/sec) are throttled/coalesced before
    hitting React (dedupe on `revision`, batch to ≤ the frame rate) so a busy session can't cause
    render storms. Streaming text updates are debounced to ~1 frame.
29. Interaction latency: every tap gives feedback within 100ms (pressed state or haptic), even if
    the network result is pending. Navigation transitions are ≤250ms.

**Design polish:**
30. Consistent rhythm: identical card padding, row height, and gutter across all screens (the
    §2 spacing scale is the only source). Two screens side by side must look like one system.
31. Depth is subtle: 1px `border`/`borderSoft` separators and bg elevation (`panel` on `bg`,
    `panelDeep` wells) — no heavy shadows. Accent is used sparingly for emphasis, never as fill on
    large areas (matches the web control page's restraint).
32. Beautiful empty/loading/error states — never a bare spinner or blank. Empty = a soft icon +
    one warm sentence + a clear next action. Loading = skeletons that match final layout where the
    list has a known shape (fleet/history), spinner only for first-ever unknown content. Errors are
    calm, specific, and recoverable (retry button always present).
33. Typographic hierarchy is enforced: exactly the sizes/weights in BUILD_PLAN §4 — titles 16/700,
    body 14–15, meta 12–13 dim, section heads uppercase dim. No ad-hoc font sizes.

**Intuitive / UX:**
34. The most important action on a screen is the most prominent and thumb-reachable (bottom third):
    Send on Chat, FAB on Fleet, primary CTA on Connect. Destructive actions are never the primary
    button and never adjacent-without-gap to a confirm.
35. Reachability: primary actions and the tab bar sit in the bottom third; nothing critical lives
    only in a top corner. Long screens keep the input/CTA pinned above the keyboard.
36. Feedback on every mutation: optimistic-forbidden data (§21) still shows immediate pressed +
    haptic + a pending affordance (disabled button/spinner-in-button), then the confirmed snapshot.
37. Haptics (light) on exactly: send prompt, allow/deny, plan approve, destructive confirm, upload
    complete, successful pairing. Nowhere else (§19).
38. Zero dead ends: every error/empty state offers a way forward; every modal has an obvious
    dismiss; back navigation always works and never loses unsent input silently.

**Self-check (a screen ships only if all pass):** cache warm-start instant · list scrolls at 60fps
with 200 rows · every tap < 100ms feedback · empty/loading/error all designed · primary action
thumb-reachable · reduce-motion honored · no raw hex · types match the wire.

## 6. Behavior

20. WS lifecycle: connect on session-shell focus, close on blur/background, reconnect with
    stored `rev` on foreground. Show a thin "reconnecting…" strip (dim, 12px) — never a modal.
    `closed:true` → static "session ended" banner, no reconnect loop.
21. Optimistic UI is forbidden for anything server-authoritative (session lists, prompts,
    answers). The snapshot/query response is the truth; render only what the server confirmed.
    (Exception: the offline prompt queue, which is explicitly rendered as "queued".)
22. Keyboard: prompt inputs use `enterkeyhint="send"` equivalents (`returnKeyType="send"`),
    `autoCapitalize/autoCorrect` off for paths/commands/URLs, and screens with bottom inputs use
    KeyboardAvoidingView inside Screen (Screen provides it via a prop, not per-screen hacks).
23. Every screen renders correctly with: 0 items, 1 item, 200 items, a 300-char title, and a
    dead server. If you didn't try those five, the screen isn't done.
24. No new dependencies without flagging. The stack in BUILD_PLAN §3 is closed; adding a package
    requires a PR note explaining why a primitive can't do it.
