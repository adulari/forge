# Forge companion app — original build plan (completed)

> **Historical implementation record.** This was the phase-0 contract used to build the first
> companion app on 2026-07-06. The project has since shipped the redesign, protocol 8, native iOS
> push, light/dark/system themes, Tauri desktop, Xcode Cloud/TestFlight, EAS OTA, SideStore, and
> Android distribution. Do not treat its worker instructions or “future” flags as current status.
> Use [mobile/README.md](README.md), [redesign/FEATURES.md](redesign/FEATURES.md), and
> [the current release checklist](../docs/mobile/APP_STORE_CHECKLIST.md) for current operations.

The backend is `forge serve` (`crates/forge-cli/src/serve.rs` + `remote.rs`). The app calls the
real API; this record intentionally describes the initial implementation scope.

---

## 1. Backend contract (verified against source, 2026-07-06)

### 1.1 Base URL + auth

- The daemon (`forge serve`) listens on port **7420** by default and serves **everything under
  `/<daemon-token>/`**. The token is a **32-char lowercase hex** string (16–64 hex accepted),
  persisted 0600 at `<config-dir>/serve-token`, stable across restarts (`serve.rs::daemon_token`).
- **Auth = the token path segment. Nothing else.** No headers, no cookies, no query token.
  Wrong/missing token → plain `404 Not Found` (deliberately unrevealing).
- App connection config = **one URL**: `{scheme}://{host}:{port}/{token}` — exactly the "connect:"
  URL `forge serve` prints and renders as a QR code at startup. The app accepts it via paste OR
  QR scan and derives `baseUrl` from it. Store in `expo-secure-store` (it is a credential).
- Exposure modes (affects scheme, app must handle all):
  - `--local` → `http://127.0.0.1:7420/<token>` (same-machine clients only; a phone cannot reach
    the daemon's loopback address)
  - default LAN → `https://<lan-host>:7420/<token>` with a **self-signed cert** — RN `fetch`/WS
    will reject it. Supported paths: same-machine desktop/web via `--local`; another device via
    `--anywhere`; or a deliberately configured trusted HTTPS reverse proxy such as Tailscale
    Serve.
  - `--anywhere` → `https://<tunnel-host>/<token>` (cloudflared/ngrok, real TLS — works out of box)
- `forge api` (api_serve.rs, port 8787, `GET /v1/models`, `POST /v1/chat/completions`,
  `GET /health`, optional `Authorization: Bearer`) is a **separate OpenAI-compat server — OUT OF
  SCOPE** for this app.

### 1.2 HTTP endpoints (all paths relative to `{baseUrl}` = `…/{token}`)

All JSON responses are `Cache-Control: no-store`. All errors are `{"error": string}` with a 4xx/5xx
status unless noted.

| Method + path | Request | Response (real serde field names) |
|---|---|---|
| `GET /api/sessions` | — | `SessionRow[]`: `{id:string, title:string, cwd:string, worktree:string\|null, busy:bool, waiting:bool, cost_usd:number, context_tokens:number, context_limit:number\|null, model:string, created_at:number(unix s), last_activity:number}`. Server-sorted: `waiting` first, then newest `created_at`, then `id`. |
| `POST /api/sessions` | `{cwd?:string, worktree?:bool, title?:string, model?:string, resume?:string}` (resume = past session id; un-archives it) | `201-ish 200 {id,title,cwd,worktree}`; `400 {error}` bad cwd / not a git repo (worktree); `500 {error}` |
| `GET /api/sessions/past?limit=&before=` | `limit` clamp 1..=200 (default 50), `before` = unix-s cursor on `last_activity` | `PastSessionRow[]`: `{id, title, cwd, worktree:string\|null, archived:bool, message_count:number, cost_usd:number, last_activity:number, created_at:number, preview:string\|null(≤140 chars)}` newest-activity first. Excludes currently-running ids. |
| `POST /api/sessions/{id}/archive` | — | `{ok:true}`; `404`. Stops driver, snapshots worktree edits onto its branch, hides session. |
| `POST /api/sessions/{id}/merge` | — | `{ok:true, merged:true, branch}` on clean merge (changes STAGED in base repo, worktree removed); `409 {error, dirty_files:[string]}` base repo dirty (session left running); `409 {error, conflicts:[string], branch, worktree}` merge conflicts (session stopped, worktree kept); `400` no worktree; `404`; `500 {error}` |
| `POST /api/sessions/{id}/discard` | — | `{ok:true, discarded:true, branch, warnings:[string]}`; `400` no worktree; `404`. DESTRUCTIVE (force-deletes branch) — app MUST confirm first. |
| `GET /api/history?session=<id>&before=<seq>&limit=<n>` | `limit` clamp 1..=200 default 60 | `HistoryRow[]` newest first: `{seq:number, role:"user"\|"assistant"\|"system", content:string, model:string\|null, created_at:number, visibility:"llm"\|"ui"}`. `visibility:"ui"` rows are user-facing notes — render them (styled as notes). Empty `session` ⇒ `[]`. |
| `POST /api/upload?session=<id>` | multipart form-data; per-file cap **10 MB**, body cap 12 MB; images (`image/*` or png/jpg/jpeg/gif/webp ext) or UTF-8 text only | `{files:[{name, path, image:bool}]}`; `400` malformed/empty; `413` too large; `422 {error}` rejected type; `404` no such session; `409` shutting down. Uploaded file auto-rides the session's NEXT prompt (image → vision, text → `@path` mention). |
| `POST /api/answer` | `{session:string, seq:number, allow:bool}` | `{ok:true}`; `404`; `409` no prompt pending / stale seq / shutting down. HTTP twin of WS `allow` — usable from a notification action without a WS. |
| `GET /api/push/key` | — | `{key:string}` (VAPID b64url); `503` push unavailable. **Web Push (browser) only — useless for a native iOS app.** Do not build push settings around it; see §5 flag. |
| `POST /api/push/subscribe` / `unsubscribe` | browser `PushSubscription.toJSON()` shape | `{ok:true}` — N/A for native app. |
| `GET /` `/app.js` `/styles.css` `/manifest.webmanifest` `/sw.js` `/icon.svg` | — | PWA assets. Not used by the app (except `GET /api/sessions` is the app's connectivity/auth probe — a 200 JSON array proves URL+token are right; a 404 means wrong token). |

### 1.3 WebSocket `/ws?session=<id>&rev=<n>`

- Connect: `{wsScheme}://{host}:{port}/{token}/ws?session=<id>&rev=<lastSeenRevision>` (`rev=0` or
  omitted on first connect).
- **Server → client**: each frame is one JSON **`Snapshot`** (full state, not a delta). Fields
  (remote.rs::Snapshot, `PROTOCOL_VERSION = 8`):

```
protocol:number            // current value 8; warn on a mismatch
session_id, title, cwd:string; worktree:string|null
exposure:string            // "loopback" | "LAN" | "public (…)"
busy:bool; done:bool
temper:string; tier:string|null; model:string
cost_usd:number; context_tokens:number; context_limit:number|null
streaming:string           // in-flight reply tail, re-sent each frame
transcript:string[]        // recent finalized lines (short live tail only)
tasks:  {title, status:"pending"|"in_progress"|"done"}[]
subagents: {agent, task, model:string|null, last, done:bool, cost:number}[]
queued: string[]           // prompts queued while busy
permission_prompt:string|null
question:string|null
question_options: {label, description}[]
question_allow_other:bool
overlay: null | {kind:string, title, rows:{id,label,detail,selected,group:string|null}[],
                 selected:number, filter:string|null, free_text:bool, body:string|null}
        // kind: "palette" | "picker:<k>" | "config" | "overlay:usage" | "overlay:mesh" | "overlay:workflow"
diff: null | {pending:bool, skipped_files:number,
              files:{path, kind:"created"|"modified"|"deleted", binary:bool, adds, dels,
                     hunks:{header, lines:string[]}[], skipped_lines:number}[]}
        // hunk lines: first char is the gutter "+"|"-"|" " — style on it
plan: null | {title, steps:{title,detail}[], notes:string|null}
copy_text:string|null      // put on the DEVICE clipboard when set
prompt_seq:number          // MUST be echoed by allow/answer
notes:string[]             // remote-facing notices, render as toasts/inline notes
revision:number; resync:bool; closed:bool
```

  Reconnect protocol: track last `revision`; reconnect with `?rev=<it>`. Server replays missed
  frames or sends one full frame with `resync:true` (accept it even though revision jumps).
  `closed:true` ⇒ stop reconnecting. Dedupe on `revision`.

- **Client → server**: one JSON object per message, tagged `{"kind": …}` snake_case
  (remote.rs::RemoteInput). Max frame 256 KB.

```
{kind:"prompt", text}                     // prompt or /command, exactly as typed in the TUI
{kind:"allow", yes:bool, seq}             // permission prompt; seq = snapshot.prompt_seq
{kind:"answer", text, seq}                // question: text = "1"-based option NUMBER as string,
                                          //   or free text when question_allow_other
{kind:"interrupt"}                        // Esc-while-busy (Stop)
{kind:"key", key}                         // "Up|Down|Enter|Esc|Tab|BTab|PageUp|PageDown|Home|End|Backspace|Char:<c>"
{kind:"overlay_select", id}               // tap an overlay row (id from overlay.rows)
{kind:"overlay_nav", delta:int}
{kind:"overlay_filter", text}             // also sets the free_text value; commit with key Enter
{kind:"overlay_cancel"}
```

  Plan card semantics (mirrors app.js): while `plan != null` AND a `question` whose options include
  "Build it" is pending — Approve/Cancel = send `answer` with that option's 1-based number;
  Revise = send free-text `answer`. Stale-seq answers are silently ignored server-side (and 409 on
  `/api/answer`) — on 409/no-op just re-render from the next snapshot.

---

## 2. Web surface — honest assessment

Forge's web UI is a **single-page PWA control page** (remote_assets/page.html + app.js, one route),
not a multi-page app. Its surfaces on that one page: header/status bar, sessions drawer (live list,
new-session form, past-sessions browser, push toggle), Chat/Tasks/Agents tabs, overlay mirror, plan
card, diff card, permission/question action area, command chips, prompt bar with attach + voice.

"Mirror every page" therefore means: **promote each surface of that one page into a proper native
screen**. The API is rich enough to support all screens below; nothing here invents data the
backend doesn't serve.

---

## 3. Original tech-stack proposal (superseded where implementation differs)

The delivered redesign replaced NativeWind and the one-file primitive layer with semantic tokens,
JetBrains Mono assets, and components under `src/components/ds/`. The list below is retained to
explain the initial build, not to pin new work.

Expo SDK 57 · React Native 0.86 · expo-router (typed, file-based, `src/app/`) · NativeWind v4 ·
@tanstack/react-query v5 with `PersistQueryClientProvider` + AsyncStorage · react-native-reanimated
v4 · TypeScript strict · npm + `npx expo install` · expo-font (system font stack — the web UI uses
`-apple-system/system-ui`, so load NO custom font; use SF defaults + `ui-monospace`/Menlo for code)
· expo-secure-store (connect URL/token) · expo-camera or expo-barcode-scanner (QR pairing) ·
expo-clipboard (copy_text) · expo-document-picker + expo-image-picker (uploads).

Primitives (single file `src/components/ui.tsx`): `Screen, Card, Tile, ListRow, StatCard, Metric,
SectionTitle, Badge, Chip, Segmented, SearchInput, PrimaryButton, ConfirmButton, FAB, EmptyState,
Loading, ErrorText, BoundedList`.

**App config (`app.json` / `app.config.ts`) — pin these now:**
- `ios.bundleIdentifier = "dev.adulari.forge"`
- `expo.extra.eas.projectId = "e1d145b5-344e-4147-ba35-5f0b993b4c8c"`; `owner` = the EAS account
  that owns that project.
- `ios.appleTeamId = "95VXXPD28Y"` (Apple Developer enrollment is complete). Signed iOS builds use
  Xcode Cloud; EAS Build/EAS Submit are not the current iOS distribution path.
- `scheme = "forge"`, `ios.icon`/splash bg `#16161c`, `ios.infoPlist`: camera usage string (QR),
  photo-library + document usage strings (uploads).

Core libs (`src/lib/`):
- `theme.ts` — tokens from §4 (single source; tailwind.config.js reads it).
- `api.ts` — `baseUrl` from secure store; typed fetch wrapper (`{error}` → thrown `ApiError`);
  all endpoint functions from §1.2; TS types mirroring the serde structs VERBATIM (snake_case —
  do not camelCase the wire).
- `ws.ts` — `useSessionSocket(sessionId)`: connects `/ws?session&rev`, exposes latest `Snapshot`,
  `send(input: RemoteInput)`, connection state; auto-reconnect with backoff + `rev` replay;
  handles `resync`/`closed`; pauses on app background, reconnects on foreground.
- `queries.ts` — react-query hooks: `useSessions()` (poll 3s while focused), `usePastSessions()`
  (infinite, `before` cursor), `useHistory(sessionId)` (infinite, `before` seq cursor), mutations
  `useCreateSession/useArchive/useMerge/useDiscard/useAnswer/useUpload`.

## 4. Original dark-palette extraction

This table captured the original web UI. The shipped app supports light, dark, and system themes;
`src/theme/tokens.ts` is now the source of truth.

| Token | Value | Source/use |
|---|---|---|
| `bg` | `#16161c` | page background, theme-color |
| `panel` | `#1c1c22` | cards, status bar, inputs |
| `panelDeep` | `#101015` | tab panel / content wells |
| `codeBg` | `#0b0b10` | code blocks, diff bodies |
| `ink` | `#d8d8e0` | primary text |
| `dim` | `#6e6e78` | secondary text, comments token |
| `accent` | `#ff913c` | brand orange: h1, active tab, busy dot, borders, keyword token |
| `ok` | `#78d28c` | success/allow, cost, done, diff adds, string token |
| `no` | `#f06e6e` | danger/deny, waiting dot, diff dels |
| `border` | `#33333c` | default borders (inputs, chips, opts) |
| `borderSoft` | `#2a2a32` (also `#2a2a33`) | session rows, dividers |
| `chipBg` | `#23232b` | option/overlay-row/inline-code background |
| `selBg` | `#2a2313` | selected overlay row / archived badge bg |
| `bannerBg` / `bannerInk` | `#3a2a12` / `#ffd9a8` | warning banner, note messages |
| `pubBg` / `pubInk` | `#3a1c1c` / `#ffb0b0` | "public" exposure badge, rec mic |
| `notes` | `#eebc52` | plan notes |
| `hunk` | `#4bd4da` | diff `@@` headers |
| `tokNum` | `#7db8f0` | syntax: numbers |
| `stream` | `#c8c8d0` | streaming text (italic) |
| `histBorder` | `#1d1d25` | message separators |
| `footer` | `#4a4a54` | footnote text |

Typography: system font; base **15/1.5**; status/meta 12–13; transcript 14; mono
(`ui-monospace`/Menlo) **12/1.5** for code/diff; h1 16/700 accent; section heads 11–13/600–700
uppercase `letter-spacing:0.5px` dim. Tabular numerals (`font-variant-numeric: tabular-nums`) on
all metrics (cost, tokens, times).
Radii: **6** (small badges/copy btn), **8** (default: buttons, inputs, rows, cards), **10**
(feature cards: overlay/plan/diff), **14** (chips = pill). Spacing: base unit 2/4/6/8/10/12;
screen gutter 12; card padding 8×10; button padding 11×16 (≥44pt tap). Buttons: primary =
accent bg + `#1c1c22` text 700; allow = ok bg; deny = no bg. Dots: 8px circles — ok=idle,
accent+pulse=busy, no+fast-pulse=waiting, dim=idle-past. Pulse animation: opacity to .35, 1s
(0.7s for waiting) — guard with reduce-motion.

---

## 5. Auth conclusion and gaps resolved after the initial build

**Conclusion:** the app authenticates by embedding the daemon token as the URL's first path
segment. Pairing = paste or QR-scan the exact `connect:` URL `forge serve` prints. No backend
change is required for auth. Probe validity with `GET {base}/api/sessions` (200 array = good,
404 = wrong token, network error = unreachable). Token rotation (`forge serve --rotate-token`)
invalidates the stored URL → app shows "pairing invalid, re-scan" on persistent 404.

**Current resolution of the original gaps:**
1. **Native push shipped:** iOS registers its APNs device token with the user's daemon. The daemon
   uses a direct operator key when configured or the open-source hosted relay by default. The
   relay receives the opaque device/Live Activity token and notification title/body/status; those
   snippets can contain sensitive session text. It does not deliberately receive the daemon token,
   full transcript, source files, or API credentials. Users can self-host, bring their own APNs
   key, or set `FORGE_APNS_DISABLE_RELAY=1`.
2. **Self-signed LAN TLS:** default `--lan` mode's self-signed cert is rejected by RN fetch/WS.
   Use `--local` only for a client on the daemon machine. For another device, use `--anywhere` or
   put the daemon behind a trusted HTTPS reverse proxy. A tunnel provider terminates TLS and can
   technically observe the bearer token and session traffic.

---

## 6. Navigation architecture + page inventory

expo-router file tree (`mobile/src/app/`):

```
_layout.tsx                     // providers: query client (persisted), theme, safe area
connect.tsx                     // pairing screen (shown when no stored URL, or from Settings)
(tabs)/_layout.tsx              // bottom tabs
(tabs)/index.tsx                // Tab 1: Fleet
(tabs)/alerts.tsx               // Tab 2: Alerts (waiting sessions)
(tabs)/history.tsx              // Tab 3: History (past sessions)
(tabs)/more.tsx                 // Tab 4: More (search-first launcher + settings entry)
session/[id]/_layout.tsx        // session detail shell (header + status strip + Segmented)
session/[id]/index.tsx          // Chat (default segment)
session/[id]/tasks.tsx          // Tasks segment
session/[id]/agents.tsx         // Agents segment
session/[id]/review.tsx         // Review segment (plan card + diff card)
session/[id]/overlay.tsx        // modal route: overlay mirror (palette/pickers/config/usage/mesh/workflow)
new-session.tsx                 // modal route: create session form
settings.tsx                    // server settings (from More)
```

### Screen contracts

**Connect** (`/connect`) — Actions: scan QR (expo-camera), paste URL, test connection
(`GET /api/sessions`), save to secure store. States: idle/testing/ok/bad-token/unreachable
(distinct copy for 404 vs network error, mention `--local`+VPN / `--anywhere` for TLS failures).
Primitives: Screen, Card, SearchInput(as URL field), PrimaryButton, ErrorText, Loading.

**Fleet** (`/(tabs)/index`) — Data: `useSessions()` poll 3s. Trust server order (waiting first).
Row: status dot (busy/waiting/idle per §4), title (fallback id-prefix), cwd tail, model,
cost (`$x.xxxx` ok-color), context gauge `context_tokens/context_limit`, "NEEDS YOU" badge when
waiting, worktree Badge. Tap → `/session/[id]`. Long-press → action sheet: Archive (confirm),
Merge (worktree only), Discard (worktree only, double-confirm). FAB → `/new-session`.
Empty state: "No live sessions — start one" + button. Primitives: Screen, ListRow, Badge, Metric,
FAB, EmptyState, Loading, ErrorText, BoundedList.

**New session** (`/new-session`, modal) — Fields: cwd (placeholder "daemon cwd"), title (optional),
model (optional free text), toggle "isolated git worktree". Submit → `POST /api/sessions` → on
success invalidate sessions + navigate to `/session/[id]`. Surface `{error}` inline (bad cwd,
not a git repo). Primitives: Screen, Card, SearchInput, Chip(toggle), PrimaryButton, ErrorText.

**Alerts** (`/(tabs)/alerts`) — Data: same `useSessions()`, filtered `waiting`. Each row shows
title + "waiting on a decision" + tap-through to the session's Chat segment (which renders the
actual prompt/question card). Badge count on the tab icon = waiting count. Empty state: "Nothing
needs you." This tab exists because `waiting` is the fleet's killer signal (serve.rs sorts on it).

**History** (`/(tabs)/history`) — Data: `usePastSessions()` infinite (`before` = last row's
`last_activity`). Row: title/preview (numberOfLines=1), cwd tail, message_count, cost, relative
time, "ARCHIVED" Badge when `archived`. Actions: tap → confirm sheet "Resume this session?" →
`POST /api/sessions {resume:id}` → navigate to `/session/[newId]`. Pull-to-refresh. Search filter
client-side over title/preview/cwd. Primitives: Screen, SearchInput, ListRow, Badge, EmptyState,
BoundedList, Loading.

**Session shell** (`/session/[id]/_layout`) — Owns ONE `useSessionSocket(id)` instance (context to
segments). Header: title (fallback id), cwd tail, exposure Badge (danger-styled when public),
worktree Badge. Status strip (sticky): dot busy/waiting, tier+model, temper, cost, context gauge.
Segmented: Chat | Tasks (count) | Agents (count) | Review (dot when plan/diff present).
`closed:true` → banner "session ended" + stop reconnect. Also: when `copy_text` changes non-null →
set device clipboard + toast; render `notes` as transient toasts; `snapshot.queued` shown above
input bar.

**Chat** (`/session/[id]/index`) — Data: `useHistory(id)` infinite upward (`before` = oldest seq)
MERGED with live snapshot: history rows → styled messages (role label, `visibility:"ui"` = note
styling `bannerInk`); then `transcript` tail lines; then `streaming` in italic stream color.
Content rendered as markdown-lite (code blocks in `codeBg` with copy button — match web).
Action cards (rendered above input, from snapshot):
- Permission card: `permission_prompt` text + Allow (ok) / Deny (no) buttons → `{kind:"allow",
  yes, seq: prompt_seq}`.
- Question card: `question` + one button per `question_options[i]` (label bold accent,
  description dim) → `{kind:"answer", text:String(i+1), seq}`; free-text row when
  `question_allow_other` or no options.
Input bar: attach button (→ upload flow: document/image picker → `POST /api/upload?session=` →
chips of uploaded names), text field ("type a task or /command…"), Send → `{kind:"prompt",text}`.
Chips row: Stop (`interrupt`, danger) + `/plan` `/compact` `/models` `/mode` `/help` (send as
prompt). Offline: queue prompts locally, flush on reconnect (mirror web's offline queue), show
"queued (offline)" chip. Primitives: Screen(embedded), Chip, PrimaryButton, ConfirmButton,
Loading, EmptyState.

**Tasks** (`/session/[id]/tasks`) — `snapshot.tasks`. Row: status glyph (`○`dim / `◐`accent /
`●`ok), title (strikethrough+dim when done). Empty: "No task list yet." Read-only.

**Agents** (`/session/[id]/agents`) — `snapshot.subagents`. Card per agent: agent name (accent),
task, model Badge, cost Metric, `last` (2-line dim mono), opacity .7 when done. Read-only.

**Review** (`/session/[id]/review`) — Plan card when `plan`: "⬡ PLAN" tag, title, numbered steps
(title bold + detail dim), notes in `notes` color; when a question containing a "Build it" option
is pending → Approve & build (ok) / Revise (free text) / Cancel (no) via the numbered-answer
mechanic (§1.3). Diff card when `diff`: header "PROPOSED CHANGE (pending Allow)" accent when
`pending`, else "changes this turn"; per file: path bold, kind, `+adds −dels` tabular; hunks in
mono on codeBg — line color by first char (`+` ok, `-` no, ` ` dim, header hunk-cyan);
"… N more lines/files" footers from `skipped_lines`/`skipped_files`. Empty: "Nothing to review."

**Overlay mirror** (`/session/[id]/overlay`, modal) — Auto-present when `snapshot.overlay`
becomes non-null (and dismiss when null). Title bar + close (→ `overlay_cancel`). If `filter`
non-null: SearchInput → `overlay_filter` (debounced 150ms). Rows (group headers uppercase dim):
tap → `overlay_select {id}`; server-authoritative highlight on `selected`. If `body`: mono
scrollable text (usage//mesh/workflow views). If `free_text`: value field + OK →
`overlay_filter{text}` then `key Enter`. This ONE screen makes every slash command drivable.

**More** (`/(tabs)/more`) — Search-first launcher (SearchInput at top) over: Settings, Pair new
server, per-session quick actions (jump to any live session's Tasks/Agents/Review/palette —
opens `/session/[id]/overlay` after sending `{kind:"key",key:"Char:/"}`? NO — just deep-link to
segments; palette opens via chip `/help` etc.), About (protocol version 8, app version), and
docs links. Keep it thin and honest — Forge's surface is session-centric.

**Settings** (`/settings`) — Show current server host (token masked, `…{last4}`), exposure of
last snapshot, Test connection, Re-pair (→ `/connect`), Forget server (confirm; clears secure
store + query cache). Note about push limitations (§5).

---

## 7. Batched build order (Sonnet fleet)

Rule: a batch starts only when its dependency batch is merged. Screens within a batch are
independent files — parallelize freely. Every worker reads UI_RULES.md first.

**Batch 0 — Foundation (serial, ONE worker, everything else depends on it)**
`app.json`/scaffold, `tailwind.config.js` + `src/lib/theme.ts` (§4 tokens verbatim),
`src/components/ui.tsx` (all primitives), `src/lib/api.ts` (types + endpoints §1.2),
`src/lib/ws.ts` (§1.3 incl. reconnect/rev/resync), `src/lib/queries.ts`,
`src/app/_layout.tsx` + tabs shell with placeholder screens, `connect.tsx` storage plumbing
(screen UI itself is Batch 1). Definition of done: app boots, tabs render, `api.ts`+`ws.ts`
compile strict.

**Batch 1 — Entry surfaces (3 workers, parallel; deps: B0)**
W1: Connect screen + Settings screen. W2: Fleet screen + New-session modal + session action
sheets (archive/merge/discard incl. 409 dirty/conflict rendering). W3: History tab (infinite +
resume flow) + Alerts tab.

**Batch 2 — Session core (2 workers; deps: B0; can run parallel with B1)**
W4: Session shell `_layout` (socket context, header, status strip, Segmented, toasts/clipboard/
queued). W5: Chat segment (history merge + streaming + markdown-lite/code blocks + input bar +
chips + offline queue).

**Batch 3 — Session panels (3 workers; deps: B2 shell merged)**
W6: Permission + Question action cards (seq discipline) — lands inside Chat. W7: Tasks + Agents
segments. W8: Review segment (plan + diff cards).

**Batch 4 — Power surfaces + polish (3 workers; deps: B3)**
W9: Overlay mirror modal (auto present/dismiss). W10: Upload/attach flow + upload chips.
W11: More launcher, empty/error state sweep, reduce-motion audit, tab badge counts, app icon/
splash (bg #16161c, accent ⚒ mark).

**Gate:** human confirms tab structure (§6) before Batch 1 screen code.

---

## 9. Current testing and distribution paths

The original dual-track proposal evolved into four independently verified paths:

1. **Web and development:** `npm run web` is the fast browser loop; `npm run tauri:dev` exercises
   the native desktop transport. Pair same-machine clients with `--local` and other devices with
   `--anywhere` or a trusted HTTPS reverse proxy.
2. **Signed iOS:** Xcode Cloud regenerates the Expo native project, signs it, and uploads it to
   TestFlight. `scripts/trigger-ios-build.mjs` can trigger the configured workflow and assign the
   processed build to a beta group. EAS Build/EAS Submit are no longer used for native iOS.
3. **iOS OTA:** `.github/workflows/eas-update.yml` publishes JavaScript/assets to the production
   channel only when the full diff from the installed-runtime baseline is native-compatible. The
   exact installed archive fingerprint is supplied as the runtime version.
4. **Alternative/native artifacts:** `.github/workflows/mobile-sidestore.yml` publishes an
   unsigned SideStore IPA/source from protected `main` for validated existing `mobile-v*` tags,
   while `mobile-android.yml` builds Android
   artifacts. Both still require installation and device smoke tests; artifact creation alone is
   not end-to-end proof.

See [mobile/README.md](README.md), [RELEASING.md](../RELEASING.md), and the
[App Store/mobile checklist](../docs/mobile/APP_STORE_CHECKLIST.md) for the live commands and
manual release gates.

## 10. Capability list (what the app can do, exhaustively)

Pair via QR/URL · list live sessions with waiting/busy/cost/context · create sessions (worktree
opt) · resume past sessions · archive/merge/discard (with conflict reporting) · full transcript
with infinite scrollback + live streaming · send prompts and slash commands · approve/deny
permissions (seq-safe) · answer questions (options + free text) · approve/revise/cancel plans ·
read structured diffs · watch tasks + subagents live · drive every TUI overlay (palette, pickers,
config wizard, usage, mesh, workflow) · upload images/text files into a session · stop a turn ·
receive host clipboard payloads · offline prompt queue. Native iOS push, widgets/Live Activities,
light/dark/system themes, and the Tauri shell shipped after this original list. `forge api` remains
a separate OpenAI-compatible server rather than a companion-app surface.
