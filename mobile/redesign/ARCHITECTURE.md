# Forge App — ARCHITECTURE (implemented redesign record)

One TypeScript/React codebase, six targets: **iOS, Android (native RN), Web (react-native-web),
Windows/macOS/Linux (Tauri v2 wrapping the web build)**. Mobile is the priority target; web is
first-class; desktop is a thin shell. This was the binding redesign plan; the current code and
[`../README.md`](../README.md) are authoritative for shipped status. Companion docs:
DESIGN_SYSTEM.md (visual/motion spec), FEATURES.md (capability → screen map), BUILD_ORDER.md
(completed task batches).

Verified against the daemon source and refreshed on 2026-07-17: `crates/forge-cli/src/serve.rs`
(routes) and `crates/forge-cli/src/remote.rs` (`PROTOCOL_VERSION = 8`, `Snapshot`, `RemoteInput`). The app
consumes ONLY this real API. No mocks, ever.

---

## 1. Stack (pinned decisions + one-line justifications)

| Choice | Decision | Why |
|---|---|---|
| SDK | **Expo SDK 57** (React Native 0.86, React 19.2) | Current stable (mid-2026); RN 0.86 New Architecture; the earlier 57→54 downgrade existed only for an old Expo Go binary — this build uses dev clients/EAS, so Expo Go compatibility is irrelevant. |
| Router | **expo-router** (SDK 57's version via `npx expo install`) | File-based typed routes that are identical on native and web; static web export; deep-link parity for free. |
| Styling | **`StyleSheet.create` + typed tokens + `ThemeProvider`** — **NO NativeWind, no Tailwind** | NativeWind v5 (the Reanimated-4-aligned one) is still pre-release; v4 pins Tailwind 3 and adds a build-tool layer between the design system and the pixels. DESIGN_SYSTEM.md is the spec — typed tokens + StyleSheet give exact control, runtime light/dark switching, zero web-export/Tauri surprises, and faster renders (no className parsing). Delete `nativewind`, `tailwindcss`, `tailwind.config.js` from the project. |
| Animation | **react-native-reanimated v4** + `react-native-worklets` (expo-pinned) | UI-thread worklets on native; on web Reanimated falls back to its JS driver — acceptable for our short, transform/opacity-only motion. Web-only CSS niceties (shimmer, caret pulse) may additionally use plain CSS in `.web.tsx` files. |
| Gestures | react-native-gesture-handler (expo-pinned) | Sheets/swipe actions; works on web. |
| Data | **@tanstack/react-query v5** + `@tanstack/react-query-persist-client` + AsyncStorage persister | Already proven in the current app; persisted cache = instant warm start on all platforms. |
| Icons | **lucide-react-native** (+ `react-native-svg` via expo install) | One consistent 1.5px-stroke dev-native icon set that renders identically on RN and web; tree-shakeable. This replaces all emoji-as-icon usage. |
| Mono font | **JetBrains Mono** bundled via expo-font (subset woff2 on web) | Code/diff is Forge's core content; platform monos (Menlo/Roboto Mono/Consolas) diverge across 6 targets — one bundled mono keeps diffs pixel-identical everywhere. Sans stays the platform system font (SF/Roboto/system-ui). |
| Desktop | **Tauri v2** (CLI ~2.11) wrapping the static web export | Tiny binary, native WebView, one `src-tauri/` dir; forward path to a Rust daemon bridge. |
| Language | TypeScript `strict: true`; ESLint `eslint-config-expo` | Non-negotiable; every batch gate runs `tsc --noEmit`. |

Version rule for workers: **all Expo-ecosystem packages are installed with `npx expo install`**
(it pins the SDK-57-compatible version — do not hand-pin RN/reanimated/router versions).
Non-Expo packages (`@tanstack/*`, `lucide-react-native`) use caret ranges.

Dependencies to REMOVE from `mobile/package.json`: `nativewind`, `tailwindcss`,
`@react-navigation/native` (direct dep — expo-router brings its own; the one usage,
`useIsFocused` in `queries.ts`, moves to `import { useIsFocused } from "expo-router"` if
available in SDK 57's router, else keep the transitive import — worker verifies at B0).

---

## 2. Repository layout

Everything stays in `mobile/` — one package, one `node_modules`, one app. No monorepo: the
sharing mechanism across all six targets is react-native-web + Metro platform extensions, not
workspace packages.

```
mobile/
  app.config.ts            # universal config (see §7)
  package.json
  tsconfig.json
  eas.json
  assets/                  # icons, splash, JetBrainsMono-*.ttf
  public/                  # WEB ONLY: sw.js, manifest.webmanifest, favicons (copied into export)
  src/
    app/                   # expo-router routes (see FEATURES.md §4 for the tree)
    components/
      ds/                  # design-system primitives (DESIGN_SYSTEM.md §6) — no feature logic
      chat/                # Markdown, CodeBlock, composer, message rows
      review/              # DiffCard, PlanCard
      cards/               # PermissionCard, QuestionCard, DecisionPeek
      overlay/             # overlay mirror + command palette
      fleet/               # SessionCard, gauges, fleet header
    lib/                   # data layer — KEPT from the current app (see §5)
      api.ts  ws.ts  queries.ts  auth.tsx  sessionContext.tsx
      secureStore.ts  secureStore.web.ts
      transport/           # fetch/WS seam (Tauri escape hatch, §6.3)
      push/                # web push (web-only) + native no-op
      haptics.ts           # expo-haptics wrapper, no-op on web
      shortcuts/           # keyboard shortcuts (web/desktop), no-op native
    theme/
      tokens.ts            # ALL raw values (both themes) — the only hex-literal file
      ThemeProvider.tsx    # light/dark/system, useTheme(), useTokens()
      typography.ts  motion.ts
  src-tauri/               # Tauri v2 shell (Rust) — §6.4
  dist/                    # `expo export -p web` output (gitignored)
```

### Platform escape hatches — the complete list

Low duplication is a hard goal. These are the ONLY places platform-specific code is allowed;
anything else platform-forked in a PR is a defect. Mechanisms: Metro extensions
(`.native.tsx` / `.web.tsx` / `.ios.tsx`), `Platform.select`, and one runtime flag (`isTauri`).

| Seam | Files | Why unavoidable |
|---|---|---|
| Credential storage | `lib/secureStore.ts` (expo-secure-store) / `.web.ts` (localStorage) | Keychain vs browser storage. |
| Transport override | `lib/transport/index.ts` + runtime Tauri branch | Mixed-content: see §6.3. |
| Push | `lib/push/push.web.ts` for Web Push; `push.ios.ts` + daemon APNs routes for native iOS | Web Push is browser-only; native iOS uses APNs and Live Activities. |
| QR pairing | `components/pairing/QRScan.native.tsx` / `.web.tsx` (paste-only + "scan on your phone" hint) | expo-camera scanning is native; web pairing is paste/link. |
| Haptics | `lib/haptics.ts` (Platform-gated no-op on web) | No web API. |
| Voice input | `lib/voice/voice.ts` (native: expo-audio m4a/aac) / `.web.ts` (web + Tauri desktop: getUserMedia + WebAudio, client-side 16kHz WAV encode) — both POST to the daemon's local-whisper `/api/voice/transcribe` | Recording API differs per platform; server-side whisper (not Web Speech API) needs a real audio file upload from both. |
| Keyboard shortcuts / hover | `lib/shortcuts/useHotkeys.web.ts` / no-op native; hover styles inside ds/ primitives via `Platform.OS === "web"` | Pointer/keyboard idioms only exist on web/desktop. |
| Sheet gesture vs CSS | internal to `ds/Sheet.tsx` (one file, Platform branches) | Gesture-driven on native, transform-transition on web. |
| Blur/app-state reconnect | already inside `lib/ws.ts` (AppState) — web maps to visibilitychange in a small branch | Lifecycle APIs differ. |

Everything else — every screen, every component, the whole design system, the whole data layer —
is one shared implementation.

---

## 3. The daemon contract (what the app talks to)

Base URL: `{scheme}://{host}:{port}/{token}` — the token is a URL **path segment**, the sole
auth. Wrong token ⇒ bare 404. `GET {base}/api/sessions` is the connectivity/auth probe.
The daemon sends `CorsLayer::very_permissive()` (serve.rs), so **cross-origin browser clients
are explicitly supported** — the web app can be deployed anywhere and still reach any daemon.

REST (all verified in serve.rs):

- `GET  /api/sessions` → `SessionRow[]` (waiting-first server sort; fleet fields incl. `context_tokens`/`context_limit`)
- `POST /api/sessions` `{cwd?, worktree?, title?, model?, resume?}` → `{id,title,cwd,worktree}`
- `GET  /api/sessions/past?limit=&before=` → `PastSessionRow[]` (cursor = `last_activity`)
- `POST /api/sessions/{id}/archive` | `/merge` (409 `dirty_files` / 409 `conflicts`) | `/discard`
- `GET  /api/history?session=&before=&limit=` → `HistoryRow[]` newest-first (cursor = `seq`)
- `POST /api/upload?session=` multipart (≤10 MB/file) → `{files:[{name,path,image}]}`
- `POST /api/answer` `{session, seq, allow}` — HTTP twin of WS `allow` (notification actions)
- `GET  /api/push/key`, `POST /api/push/subscribe|unsubscribe` — Web Push (VAPID), web target only

WS `/ws?session=<id>&rev=<n>`: one full-state `Snapshot` JSON per frame, `PROTOCOL_VERSION = 8`.
The canonical cross-client fixture is `protocol/remote-v8.json`. WS `/ws/fleet` emits lightweight
revision invalidations so fleet screens refresh immediately without a hot REST poll.
Reconnect protocol: track `revision`, reconnect with `?rev=`, accept `resync:true` frames,
stop on `closed:true`, dedupe on `revision`. Client→server `RemoteInput` (snake_case `kind` tag):
`prompt`, `allow{yes,seq}`, `answer{text,seq}`, `interrupt`, `key{key}`, `overlay_select{id}`,
`overlay_nav{delta}`, `overlay_filter{text}`, `overlay_cancel`. Max inbound frame 256 KB.
(`attach` exists server-side only — delivered by the upload route, never sent by clients.)

The full field-by-field Snapshot shape is already encoded in `mobile/src/lib/ws.ts` and
`mobile/BUILD_PLAN.md` §1.3 — both verified accurate against remote.rs. Types mirror the serde
field names VERBATIM (snake_case). Never camelCase the wire.

**prompt_seq discipline** (unchanged, binding): Allow/Answer always echo the `prompt_seq` of the
snapshot they rendered from; stale answers are ignored server-side / 409 on HTTP; never retry —
re-render from the next snapshot; disable the card's buttons after first tap until a new
snapshot arrives.

TLS reality (document in-app on pairing errors): default `--lan` uses a self-signed cert that native
fetch/WS, browsers, and Tauri's WebView do not trust automatically. Use `--anywhere` for another
device. Same-machine desktop/web clients can use loopback-only plain HTTP via `--local`; a remote
VPN client needs an explicitly configured trusted HTTPS reverse proxy such as Tailscale Serve.

---

## 4. Data layer

### 4.1 Kept verbatim (port, do not rewrite)

The current app's data layer is correct and verified against the daemon — it is the KEEP pile:

- `src/lib/api.ts` — typed fetch client, wire-verbatim types, `ApiError` with 404→"pairing invalid" mapping.
- `src/lib/ws.ts` — `useSessionSocket`: rev-replay reconnect, backoff, resync/closed handling, AppState pause/resume.
- `src/lib/queries.ts` — `useSessions` (fleet-event driven with a 60s recovery poll), `usePastSessions`/`useHistory` (infinite cursors), all mutations, baseUrl-namespaced keys.
- `src/lib/auth.tsx` — `parseConnectUrl` (`connect:` scheme normalization, 16–64 hex token), pair/forget/testConnection.
- `src/lib/sessionContext.tsx` — one socket per session shell, context to segments.
- `src/lib/secureStore.ts` / `.web.ts`.

Required changes while porting (the only ones):

1. **Multi-server**: `auth.tsx` grows from one stored URL to a list
   (`forge.servers` = `[{id, name, baseUrl, token, host, addedAt}]` + `forge.activeServerId`).
   `useAuth().baseUrl` keeps its exact signature (returns the ACTIVE server's baseUrl) so
   api/queries/ws need zero changes. Query keys are already baseUrl-namespaced — switching
   servers is cache-safe by construction.
2. **Transport seam**: `api.ts`'s `fetch` and `ws.ts`'s `new WebSocket` go through
   `lib/transport` (§6.3). Default export IS the globals — zero behavior change outside Tauri.
3. **Protocol guard**: surface `snapshot.protocol !== 8` as a persistent banner on every app
   target, matching the daemon page.
4. **Chat timeline dedupe** (bug fix, FEATURES.md §2): the live `transcript` tail and the newest
   `history` page can render the same content twice. New rule: the timeline's source of truth is
   `useHistory` rows; the snapshot contributes ONLY (a) the `streaming` edge while busy and
   (b) `transcript` tail lines as instant warm-start filler that is dropped once the first
   history page for this session arrives; on turn completion (`busy` true→false) invalidate the
   history query so the finalized turn appears from the store.

### 4.2 Caching / offline model

- `PersistQueryClientProvider` + AsyncStorage (works on all targets; AsyncStorage maps to
  localStorage on web): fleet, past sessions, and history pages render from cache before any
  network. Never a spinner over stale cache.
- Offline prompt queue (kept from current chat screen): per server+session AsyncStorage queue,
  cap 20, flushed in order on WS reconnect, rendered as "queued (offline)" rows. Loud drop past cap.
- Reconnect: `ws.ts` behavior kept (backoff 500ms→15s, rev replay). A thin "reconnecting…"
  strip, never a modal.

### 4.3 Credential storage per platform

| Platform | Mechanism |
|---|---|
| iOS/Android | expo-secure-store (Keychain/Keystore) |
| Web | localStorage via `secureStore.web.ts` (documented risk; token is the user's own daemon credential) |
| Tauri | same web code path — WebView localStorage persists per-app. Optional later: `tauri-plugin-store` behind the same `secureStore` interface (deferred, do not build in v1). |

---

## 5. Web target (first-class)

- `app.config.ts`: `web: { bundler: "metro", output: "static", favicon: … }`.
  Build: `npx expo export -p web` → `dist/` static site. Deployable to any static host; it can
  ALSO later be served by the daemon itself (out of scope for the app repo).
- **Routing parity**: expo-router gives the same URL space on web as native deep links
  (`/session/abc123`, `/connect?url=…`). The native scheme is `forge://`; pairing links work as
  `https://<app-host>/connect?url=<connect-url>` on web and `forge://connect?url=…` on native.
- **PWA + Web Push parity** (the old daemon PWA had this; the new web app must too):
  `public/manifest.webmanifest` + `public/sw.js`. The service worker handles `push` events and
  notification `Allow`/`Deny` actions by POSTing `{base}/api/answer` (the daemon designed this
  route exactly for that). Subscribe flow uses `GET /api/push/key` → `PushManager.subscribe` →
  `POST /api/push/subscribe`. All of this lives in `lib/push/push.web.ts` + `public/sw.js` and
  is invisible to native builds.
- **Real DOM wins we must use**: native text selection in transcript/code/diff (do NOT disable
  selection), `user-select` enabled on message bodies, real `<a>` for external links, paste-image
  upload in the composer (web-only branch in the composer's paste handler).
- Mixed-content caveat: an `https://`-deployed web app cannot call an `http://127.0.0.1` daemon
  (browser blocks). Guidance shown on pairing failure; `http://localhost` dev serving and the
  Tauri shell (§6.3) are the workarounds. Not solvable app-side — documented, not fought.

---

## 6. Desktop target (Tauri v2, thin)

### 6.1 Shape

`mobile/src-tauri/` — standard Tauri v2 project. `tauri.conf.json`:
`build.frontendDist = "../dist"`, `build.devUrl = "http://localhost:8081"` (Metro dev server),
`beforeBuildCommand = "npx expo export -p web"`. Window: 1180×800 default, min 380×640,
`titleBarStyle: "Overlay"` on macOS, background matching `tokens.bg1` per theme. One window.

The shipped shell includes window/icons, native notifications, external-link opening, native folder
selection, signed updater support, HTTP/WebSocket transport plugins, and a basic macOS app/Edit
menu. Three narrow Rust commands detect a live local daemon, locate the Forge binary, and start
`forge serve --local`; the webview has no general filesystem or shell capability.

### 6.2 Runtime detection

`isTauri` = `"__TAURI_INTERNALS__" in window` (checked once in `lib/platform.ts`). The desktop
app is the WEB build — `Platform.OS === "web"` everywhere; Tauri-only behavior branches on
`isTauri` at runtime for transport, notifications, external-link opening, folder selection,
updating, and local-daemon discovery/startup.

### 6.3 Transport escape hatch (the one real problem)

On macOS/Linux the Tauri page origin is a custom scheme treated as a secure context, so calls to
a same-machine plain-`http://` daemon (`--local`) can be blocked as mixed
content by the WebView. Fix: `tauri-plugin-http` (requests execute in Rust, immune to WebView
mixed-content policy) behind the `lib/transport` seam:

```ts
// lib/transport/index.ts
export const tFetch: typeof fetch      // default: globalThis.fetch
export const TWebSocket: typeof WebSocket // default: globalThis.WebSocket
// When isTauri && baseUrl.startsWith("http:"): tFetch = fetch from @tauri-apps/plugin-http.
// WS: try native WebSocket first; if it errors on an http daemon, fall back to
// @tauri-apps/plugin-websocket wrapped in a WebSocket-compatible adapter (small, ~60 lines).
```

api.ts/ws.ts import from `lib/transport` and know nothing else. This is the entire Tauri-specific
data-path surface.

### 6.4 Local daemon bridge and forward path

The shell now discovers a live daemon through `<config-dir>/serve-state.json`, finds Forge in
Cargo/Homebrew/user-local locations, and can start `forge serve --local` for zero-setup desktop
pairing. A tray/fleet-glance surface remains a possible follow-up.

---

## 7. App Store review posture (iOS ships to the store)

Guideline 4.2 (minimum functionality / "app-like") defenses, all real:

- Native capabilities in the critical path: **camera QR pairing**, **Face ID app lock**
  (expo-local-authentication — keep the current `AppLock` concept), **haptics map**
  (DESIGN_SYSTEM §5), native share/clipboard, home-screen quick actions (later).
- Offline-graceful: persisted cache renders everything last-known; explicit offline queue for
  prompts; designed empty/error states everywhere (never a dead spinner).
- Onboarding that stands alone: the Connect screen explains what Forge is and links setup docs —
  the app must not look like a blank webview to a reviewer without a daemon. Include a
  "watch how it works" static walkthrough (bundled images, not network).
- Push: native iOS APNs alerts and Live Activities are shipped; Web Push remains the browser/PWA
  channel. Store privacy copy must disclose that the public relay handles opaque device tokens,
  generic alerts, and coarse Live Activity state, while custom private relays can retain rich text.
- Privacy manifest: keep the existing `PrivacyInfo.xcprivacy` / `ios.privacyManifests` wiring and
  usage strings from the current `app.config.ts` (camera, Face ID, photo library, documents).
- No remote code: the app is fully bundled; daemon data is user content. Standard permission
  usage strings already exist.

---

## 8. Build & distribution (per target, high level)

| Target | Build | Distribution |
|---|---|---|
| iOS | Xcode Cloud runs `ios/ci_scripts/ci_post_clone.sh` (`npm ci` + Expo prebuild) and archives/signs the generated project | TestFlight/App Store; `mobile-sidestore.yml` separately publishes an unsigned IPA + SideStore source |
| Android | `mobile-android` Actions workflow / `eas build -p android` | Internal APK artifact, tagged AAB release, optional Play internal-track submission |
| Web | `npx expo export -p web` → `dist/` | Any static host; version-stamped; served over HTTPS |
| Windows | `npm run tauri build` on windows runner → NSIS `.exe` | GitHub Releases |
| macOS | tauri build → `.dmg` (unsigned initially; notarization when the Apple account lands) | GitHub Releases |
| Linux | tauri build → `.AppImage` + `.deb` | GitHub Releases |

CI: `app-desktop.yml` transactionally publishes five desktop targets, `app-web.yml` exports the web
artifact, `mobile-android.yml` builds APK/AAB artifacts and can submit to Play internal testing,
`mobile-sidestore.yml` publishes unsigned iOS builds, and Xcode Cloud owns signed iOS/TestFlight
builds. Compatible iOS JavaScript/assets ship through the production EAS Update workflow.

Dev loops: `npx expo start` (native dev client / Expo Go where possible), `npx expo start --web`
(primary inner loop against a `--local`/`--anywhere` daemon), `npm run tauri dev` (desktop).

---

## 9. Performance budget (binding, inherited + extended from UI_RULES)

- Warm start from persisted cache: first contentful paint < 1 frame, network refresh underneath.
- Every list virtualized (FlatList/FlashList-equivalent via the ds `BoundedList`), memoized rows,
  stable keys, no inline closures in `renderItem`.
- WS snapshots coalesced to ≤1 React commit per frame; streaming text updates batched via
  `requestAnimationFrame`-throttled state (the ds `StreamingText` owns this).
- All animation transform/opacity only, UI-thread on native; ≤300ms except loops named in
  DESIGN_SYSTEM §5; every animation behind the reduce-motion guard.
- Interaction feedback <100ms (pressed state/haptic) regardless of network.
