# Forge App — completed redesign build order

> **Historical implementation record (completed).** This file preserves the worker batches used
> to deliver the current app; references to stubs, handoffs, and future batches describe
> intermediate states, not current product gaps. Use [mobile/README.md](../README.md) for current
> development/release commands and [FEATURES.md](FEATURES.md) for delivered capability status.

Rules of engagement (every worker, every task):

- Read ARCHITECTURE.md + DESIGN_SYSTEM.md + FEATURES.md sections named in your task FIRST.
- File scopes are DISJOINT within a batch — do not touch files outside your scope; if you need a
  change in another scope, leave a `// HANDOFF(<task-id>):` comment and flag it in the PR.
- Real daemon data only. To hand-test: `cargo run -p forge-agent -- serve --local --mock` from the
  repo root (or a real daemon), pair with the printed connect URL.
- **Gate per batch** (all must pass before the next batch starts):
  `npx tsc --noEmit` clean · `npx expo lint` clean · `npx expo start --web` boots and the batch's
  "done" checks pass by hand · no raw hex outside `src/theme/tokens.ts` (`grep -rn "#[0-9a-fA-F]\{6\}" src --include="*.tsx" -l` returns nothing).
- Old code policy: the legacy visual layer (`src/components/*`, `src/app/*`, `src/lib/theme.ts`,
  `src/lib/tokens.json`, `src/lib/motion.ts`, `tailwind.config.js`) is REPLACED over B0–B3.
  Data-layer files listed in ARCHITECTURE §4.1 are PORTED (edited in place), never rewritten.

---

## Batch 0 — Foundation (3 tasks; T0.1 first, then T0.2 ∥ T0.3)

### T0.1 Scaffold & config (serial, blocks everything)
Files: `package.json`, `app.config.ts`, `tsconfig.json`, `babel.config.js`, `metro.config.js`,
`eas.json`, `assets/*` (add JetBrainsMono-Regular/Bold.ttf), delete `tailwind.config.js`,
`global.css` if present.
Do: upgrade to Expo SDK 57 (`npx expo install expo@^57 --fix`); remove `nativewind`,
`tailwindcss`; add `lucide-react-native`, `react-native-svg` (expo install); set
`web.output: "static"`; keep bundle id / EAS projectId / privacy manifest / usage strings /
scheme `forge` exactly as they are in the current `app.config.ts`; add
`userInterfaceStyle: "automatic"` (light+dark now supported).
Done: `npx expo start --web` boots a blank screen on SDK 57; `tsc --noEmit` clean.

### T0.2 Theme + motion foundation
Files (new): `src/theme/tokens.ts`, `src/theme/ThemeProvider.tsx`, `src/theme/typography.ts`,
`src/theme/motion.ts`, `src/theme/useBreakpoint.ts`. Delete `src/lib/theme.ts`,
`src/lib/tokens.json`, `src/lib/motion.ts` (port `usePulse`/`useCountUp`/`usePressScale` logic
into `src/theme/motion.ts` under the new names: Emberdot, Gaugeflow, Strike).
Spec: DESIGN_SYSTEM §1 (both theme tables verbatim), §2, §3, §5 (all motion tokens + the
pattern hooks: `useStrike`, `useForgeline(index)`, `useEmberdot(kind)`, `useGaugeflow`,
`useTemper`, spring/duration/easing constants). ThemeProvider: light/dark/system with
`useColorScheme`, persisted override in AsyncStorage, `useTokens()` hook.
Done: a scratch screen (deleted after) renders both themes; reduce-motion verified via the hooks'
static path; tsc clean.

### T0.3 Data-layer port
Files (edit in place): `src/lib/api.ts`, `src/lib/ws.ts`, `src/lib/queries.ts`,
`src/lib/auth.tsx`, `src/lib/sessionContext.tsx`, `src/lib/secureStore.ts`, `.web.ts`;
(new) `src/lib/transport/index.ts`, `src/lib/platform.ts`, `src/lib/haptics.ts`.
Do: ARCHITECTURE §4.1 changes only — multi-server `auth.tsx` (list + activeServerId, same
`useAuth().baseUrl` contract, plus `servers`, `addServer`, `removeServer`, `setActive`);
route fetch/WebSocket through `lib/transport` (default = globals; this foundation task initially
left the Tauri branch behind `isTauri` on globals, then T5.2 replaced it with native plugins);
export a `useTurnCompleted(snapshot)` helper (busy
true→false edge) for the history-invalidation rule. Do NOT change wire types.
Done: tsc clean; `parseConnectUrl` unit behavior unchanged (paste a real connect URL against a
running mock daemon and probe succeeds).

## Batch 1 — Design-system primitives (4 tasks, parallel; deps: B0)

All tasks implement DESIGN_SYSTEM §6 components with EVERY listed state, both themes, Strike
press feedback, reduce-motion paths, `accessibilityRole`/`Label`.

- **T1.1 Controls** — `src/components/ds/Button.tsx`, `IconButton.tsx`, `Input.tsx`,
  `SearchField.tsx`, `Chip.tsx`, `Segmented.tsx`, `Switch.tsx`, `Checkbox.tsx`.
- **T1.2 Status & data** — `ds/StatusDot.tsx`, `Badge.tsx`, `ContextGauge.tsx`,
  `CostMetric.tsx`, `KeyValueRow.tsx`, `RelativeTime.tsx`, `SectionHeader.tsx`.
- **T1.3 Containers** — `ds/Screen.tsx`, `Card.tsx`, `ListRow.tsx`, `BoundedList.tsx`,
  `Sheet.tsx` (Anvil: gesture native / CSS-transition web), `Toast.tsx` + `ToastHost.tsx`,
  `Banner.tsx`, `EmptyState.tsx`, `Skeleton.tsx` (Temper), `ConfirmDialog.tsx` (incl.
  press-and-hold destructive variant), `MasterDetail.tsx` (+`useBreakpoint` wiring).
- **T1.4 Content renderers** — `src/components/chat/Markdown.tsx`, `CodeBlock.tsx` (port the
  keyword highlighter from `crates/forge-cli/src/remote_assets/app.js` `highlight()`/`HL_KW` —
  same language sets, token colors from DESIGN_SYSTEM §6 CodeBlock), `StreamingText.tsx`
  (Kindle: rAF-coalesced updates + ember caret).
Done (each): a `__gallery__` dev route (T1.3 owns `src/app/gallery.tsx`, others add sections via
a registry file each task owns: `ds/gallery/<task>.tsx`) shows every component in every state in
both themes at 3 breakpoints.

## Batch 2 — App shell + entry screens (4 tasks, parallel; deps: B1)

- **T2.1 Root shell** — `src/app/_layout.tsx` (ThemeProvider, AuthProvider,
  PersistQueryClientProvider, ToastHost, palette host slot, redirect-to-connect when unpaired),
  `src/app/(tabs)/_layout.tsx` (bottom tabs compact/medium with lucide icons + Inbox badge from
  `useSessions` waiting count; MasterDetail on expanded), `src/components/AppLock.tsx` (port the
  existing Face ID gate onto ds primitives).
- **T2.2 Connect + Settings** — `src/app/connect.tsx` (QR scan native via
  `src/components/pairing/QRScan.native.tsx` / `.web.tsx` paste-only; URL field mono; test
  states idle/testing/ok/bad-token/unreachable/server-error with the TLS guidance copy from
  FEATURES §3; deep-link `?url=`), `src/app/(tabs)/settings.tsx` (servers list add/remove/switch,
  appearance light/dark/system, app-lock toggle, about/diagnostics: app version + protocol 8 +
  active server host with masked token `…{last4}`).
- **T2.3 Fleet + New Session** — `src/app/(tabs)/index.tsx` (fleet header Σ cost/waiting/busy;
  `src/components/fleet/SessionCard.tsx` per DESIGN_SYSTEM; swipe + action sheet
  archive/merge/discard incl. 409 `dirty_files`/`conflicts` result sheets per FEATURES §1.1;
  Forgeline entrance; skeletons; empty state), `src/app/new-session.tsx` (modal, Rise; cwd/title/
  model/worktree; inline `{error}`; navigate on success).
- **T2.4 Inbox + History** — `src/app/(tabs)/inbox.tsx` (waiting sessions, Emberdot beacons,
  empty "nothing needs you"), `src/app/(tabs)/history.tsx` (infinite past sessions, client search,
  resume confirm → `POST {resume}` → navigate, archived badges, pull-to-refresh).
Done: pair → create → see fleet → archive, all against a live mock daemon, on iOS sim AND web.

## Batch 3 — Session experience (4 tasks; T3.1 first, then T3.2–T3.4 parallel)

- **T3.1 Session shell** — `src/app/session/[id]/_layout.tsx`: SessionProvider (one socket),
  header (title/cwd/worktree/exposure badges), status strip (dot, tier·model, temper,
  CostMetric, ContextGauge), Segmented with counts, banners (`closed`, protocol mismatch,
  public exposure, reconnecting strip), `copy_text`→clipboard+toast, `notes`→toasts,
  history invalidation on `useTurnCompleted`.
- **T3.2 Chat** — `src/app/session/[id]/index.tsx` + `src/components/chat/Composer.tsx`,
  `MessageRow.tsx`, `attach.ts`: timeline per ARCHITECTURE §4.1.4 (history rows + streaming edge
  + warm-start tail with dedupe), inverted BoundedList, jump-to-latest pill, offline queue
  (port the existing AsyncStorage queue), command Chips, attach flow (image/document pickers,
  upload chips, web paste-image), Kindle streaming, haptics per map.
- **T3.3 Decision & review cards** — `src/components/cards/PermissionCard.tsx`,
  `QuestionCard.tsx`, `src/components/review/PlanCard.tsx`, `DiffCard.tsx`,
  `src/app/session/[id]/review.tsx`, plus mounting cards above the composer (Chat exposes a
  slot: `src/components/chat/CardSlot.tsx` owned by THIS task). prompt_seq discipline
  (disable-after-tap, no retry), plan Build-it/Cancel option-number mechanic, pending-diff
  embed, Approve/Deny commit animation.
- **T3.4 Tasks + Agents** — `src/app/session/[id]/tasks.tsx`, `agents.tsx`,
  `src/components/fleet/TaskRow.tsx`, `AgentCard.tsx`.
Done: full loop on a real session — prompt → stream → permission Allow (watch the diff) →
question answer → plan approve → tasks/agents update — on iOS sim and web.

## Batch 4 — Power surfaces (3 tasks, parallel; deps: B3)

- **T4.1 Overlay mirror** — `src/components/overlay/OverlayPanel.tsx` + auto-present wiring in a
  new `src/components/overlay/OverlayHost.tsx` (mounted by session shell via a HANDOFF-agreed
  one-line slot): Sheet on compact / centered 560 on expanded; filter (debounced 150ms
  `overlay_filter`), grouped rows → `overlay_select`, `body` mono view, free-text → filter +
  `key Enter`, close → `overlay_cancel`.
- **T4.2 Command palette** — `src/components/overlay/CommandPalette.tsx`, `src/lib/shortcuts/`
  (`useHotkeys.web.ts` + native no-op): sources = live sessions (navigate), actions
  (new session, archive current, theme toggle, `/`-commands sent to the current session),
  navigation (tabs). ⌘K/Ctrl+K web+desktop; a palette IconButton in headers on native.
  Keyboard nav + selection ticks; DecisionPeek rows for waiting sessions.
- **T4.3 DecisionPeek + web push** — `src/components/cards/DecisionPeek.tsx` (short-lived
  `useSessionSocket` attach inside a Sheet, renders Permission/Question card, detaches on close),
  `src/lib/push/push.web.ts` + `push.ts` no-op, `public/sw.js` + `public/manifest.webmanifest`
  (push handler with Allow/Deny actions → `POST {base}/api/answer`; notification click →
  `/session/{id}` route), Settings → Notifications row (web only).
Done: every slash command drivable from the app; ⌘K works on web; a web push notification's
Allow button resolves a real prompt with the page closed.

## Batch 5 — Platform targets (2 tasks, parallel; deps: B4)

- **T5.1 Web/desktop layout + input polish** — `src/components/ds/MasterDetail.tsx` final
  (rail = fleet + inbox pills), hover/focus-visible audit across ds/ (web branches inside the
  existing files — this task MAY touch `src/components/ds/*` as B1 owners are done), text
  selection enabled in transcript/code/diff, external links, ⌘1..4/⌘N/⌘Enter, overlay keyboard
  passthrough (named `key` inputs while OverlayPanel open).
- **T5.2 Tauri shell** — `src-tauri/` (Cargo.toml, tauri.conf.json per ARCHITECTURE §6.1,
  icons, `src/main.rs` with notification/opener plugins registered), `package.json` scripts
  (`tauri`, `tauri:dev`, `tauri:build`), `src/lib/transport/index.ts` Tauri branch
  (plugin-http fetch for `http:` daemons + plugin-websocket adapter per §6.3), notification
  feature-detect in the existing toast/notify seam.
Done: `npm run tauri:dev` opens the app on Linux, pairs with a `--local` daemon over plain http,
streams a session; `npx expo export -p web` output loads standalone with the PWA manifest.

## Batch 6 — Polish + distribution (3 tasks, parallel; deps: B5)

- **T6.1 Animation & feel pass** — sweep every screen against DESIGN_SYSTEM §5: Forgeline on
  first mounts only, Temper skeletons everywhere data loads, Bellows refresh, Approve/Deny
  commit animation, haptic map audit, reduce-motion full audit. Scope: may touch any
  `src/app`/`src/components` file (single worker — no parallel edits this batch task).
- **T6.2 State & copy pass** — every screen × {0 items, 1 item, 200 items, 300-char title,
  dead server, wrong token}: designed empty/error/loading per DESIGN_SYSTEM §4 microcopy;
  verify the five-way check from old UI_RULES #23. Scope: same files as T6.1 — run AFTER T6.1
  (serial within batch), or restrict to a `copy.ts` strings module + screen-local fixes
  coordinated via HANDOFF comments.
- **T6.3 Distribution** — `.github/workflows/app-web.yml` (export + artifact),
  `app-desktop.yml` (five-target Tauri release matrix), keep/refresh `mobile-sidestore.yml`
  (SideStore) for SDK 57,
  `eas.json` profiles check, `mobile/README.md` (pairing, dev loops, build matrix, SideStore
  source URL). No store submission automation yet.
Final gate: the FEATURES §1 table walked end-to-end against a real daemon on: iPhone (sim or
SideStore device), Android emulator, Chrome, and one Tauri desktop build. Every P0/P1 row checks.

---

## Dependency graph

```
T0.1 → (T0.2 ∥ T0.3) → [B1: T1.1–T1.4 ∥] → [B2: T2.1–T2.4 ∥] → T3.1 → (T3.2 ∥ T3.3 ∥ T3.4)
   → [B4: T4.1–T4.3 ∥] → (T5.1 ∥ T5.2) → T6.1 → T6.2 ∥ T6.3
```

23 tasks, 7 batches. Suggested worker model: Sonnet for all tasks (they are bounded and fully
specified); escalate a task to Opus only if it fails its gate twice.
