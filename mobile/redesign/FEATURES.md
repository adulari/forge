# Forge App — FEATURES (capability inventory, parity gaps, net-new, IA)

Ground truth: `crates/forge-cli/src/serve.rs` + `remote.rs` (protocol v7) and the daemon web PWA
(`remote_assets/page.html` + `app.js`), both read on 2026-07-06. Every row below is backed by a
real endpoint or Snapshot field — nothing here invents data the daemon doesn't serve.

Priorities: **P0** = core parity (app is unusable without) · **P1** = full parity with the web
PWA · **P2** = enhancement beyond both existing UIs.

---

## 1. Capability → endpoint/WS → screen map (complete)

### 1.1 REST

| Capability | Endpoint | Screen / flow | Pri |
|---|---|---|---|
| Pair with a daemon (probe token) | `GET /api/sessions` (200 array = ok, 404 = bad token, network err = unreachable) | Connect (QR scan / paste / deep link `…/connect?url=`) | P0 |
| Live fleet list (waiting-first, busy, cost, context, model) | `GET /api/sessions` (3s focused poll) | Fleet tab (+ rail on wide) | P0 |
| Create session (cwd, title, model pin, worktree toggle) | `POST /api/sessions` | New Session modal; inline `{error}` for bad cwd / not-a-git-repo | P0 |
| Resume a past session (un-archives) | `POST /api/sessions {resume:id}` | History row tap → confirm → navigate to live session | P0 |
| Browse past sessions (cursor paging, preview, archived badge) | `GET /api/sessions/past?limit&before` | History tab (infinite, pull-to-refresh, client-side search) | P0 |
| Archive session (stop + hide, history kept) | `POST /api/sessions/{id}/archive` | Fleet swipe/action-sheet + session header menu; single confirm | P0 |
| Merge worktree back (409 dirty_files / 409 conflicts payloads) | `POST /api/sessions/{id}/merge` | Fleet/session action sheet → result sheet rendering file lists (never a generic toast) | P1 |
| Discard worktree + branch (destructive) | `POST /api/sessions/{id}/discard` | Action sheet → press-and-hold ConfirmDialog naming the branch; render `warnings` | P1 |
| Transcript scrollback (persisted, paginated) | `GET /api/history?session&before&limit` | Chat timeline (infinite upward) | P0 |
| Upload files/images into a session (rides next prompt) | `POST /api/upload?session=` multipart | Composer attach (photo library / files / web paste-image); chips with per-file state | P1 |
| Answer a permission prompt over HTTP (no WS needed) | `POST /api/answer {session,seq,allow}` | Web notification actions (sw.js); DecisionPeek fallback path | P1 |
| Web push subscribe/unsubscribe (VAPID) | `GET /api/push/key`, `POST /api/push/subscribe|unsubscribe` | Settings → Notifications (web target only; hidden natively) | P1(web) |

### 1.2 WebSocket Snapshot fields → UI

| Snapshot field(s) | UI surface | Pri |
|---|---|---|
| `transcript`, `streaming` | Chat timeline live tail + Kindle streaming edge (dedupe rule: ARCHITECTURE §4.1.4) | P0 |
| `busy`, `done` | StatusDot states; composer hint; turn-complete signal (invalidate history) | P0 |
| `permission_prompt` + `prompt_seq` | PermissionCard (Allow/Deny, seq-echoed) | P0 |
| `question`, `question_options`, `question_allow_other` + `prompt_seq` | QuestionCard (option buttons answer "1"-based number as string; free text when allowed) | P0 |
| `plan` (+ pending "Build it" question) | PlanCard in Review segment + inline banner in Chat; Approve/Revise/Cancel via the numbered-answer mechanic | P1 |
| `diff` (`pending` vs landed) | DiffCard in Review segment; pending diff embedded in PermissionCard | P1 |
| `tasks` | Tasks segment (read-only rows) + count in Segmented | P1 |
| `subagents` | Agents segment (AgentCard grid) + count | P1 |
| `overlay` (palette/pickers/config/usage/mesh/workflow) | OverlayPanel sheet — auto-present on non-null, auto-dismiss on null; rows→`overlay_select`, filter→`overlay_filter`, free-text→filter+`key Enter`, close→`overlay_cancel` | P1 |
| `queued` | "queued" rows above composer | P1 |
| `notes` | Toasts (Signal) | P1 |
| `copy_text` | On change non-null: set device clipboard + toast "response copied" | P1 |
| `cost_usd`, `context_tokens`, `context_limit`, `model`, `tier`, `temper` | Session status strip (CostMetric, ContextGauge, model/tier, temper chip) | P0 |
| `title`, `cwd`, `worktree`, `session_id` | Session header | P0 |
| `exposure` | Header badge; danger Banner when `public (…)` | P1 |
| `protocol` | Persistent warn Banner on mismatch (≠7), with "update Forge / update app" copy | P1 |
| `revision`, `resync`, `closed` | ws.ts reconnect machinery (kept); `closed` → "session ended" banner, stop reconnecting, offer History | P0 |

### 1.3 RemoteInput sends

| Input | Trigger | Pri |
|---|---|---|
| `prompt {text}` | Composer send; command Chips (`/plan` `/compact` `/models` `/mode` `/help`); palette "send as command" | P0 |
| `allow {yes,seq}` | PermissionCard buttons (also HTTP twin from notifications) | P0 |
| `answer {text,seq}` | QuestionCard options/free text; PlanCard Approve/Revise/Cancel | P0 |
| `interrupt` | Stop button (composer, danger) | P0 |
| `key {key}` | Overlay free-text commit (`Enter`); web/desktop keyboard passthrough while overlay open (Up/Down/Enter/Esc/Tab/BTab/paging) | P1 |
| `overlay_select {id}` / `overlay_nav {delta}` / `overlay_filter {text}` / `overlay_cancel` | OverlayPanel interactions | P1 |

---

## 2. Parity gap list (what the CURRENT Expo app is missing / gets wrong)

Vs the daemon web PWA and the daemon itself — these must all be closed by the redesign:

1. **Web Push** — the PWA subscribes and receives Allow/Deny-actionable notifications with the
   page closed; the Expo app has nothing. New app: full Web Push on the web target
   (`public/sw.js` + `lib/push/push.web.ts`); native remains a flagged backend gap (§3).
2. **Voice input** — PWA has Web Speech transcription into the prompt box; Expo app has none.
   New app: web target parity via `voice.web.ts`; native mic deferred (flagged, §3).
3. **Paste-image upload** — PWA uploads images pasted into the prompt; missing natively-shaped
   composer. New app: web composer paste handler.
4. **Protocol-mismatch banner** — PWA warns when `snapshot.protocol != 7`; Expo app ignores it.
5. **Desktop keyboard parity for overlays** — PWA forwards arrows/Enter/Esc/Tab as `key` inputs
   while an overlay is open; Expo app has no keyboard story. New app: web/desktop hotkeys.
6. **Chat duplication bug** — the Expo chat renders snapshot `transcript` tail rows AND the
   newest history page, which can show the same content twice (`session/[id]/index.tsx`
   `combined`). Fixed by the timeline rule in ARCHITECTURE §4.1.4.
7. **`done` field unused**, and turn-completion doesn't refresh history — finalized turns only
   appear via the tail ring. Fixed by busy→idle history invalidation.
8. **Installability** — the PWA is installable (manifest/sw); the Expo web export currently
   ships no manifest/service worker. New app: PWA-complete web export.
9. **Session quick actions from the list** — PWA has archive/merge/discard on every session row;
   Expo app hides some behind navigation. New app: swipe + action sheet on SessionCard.
10. **Public-exposure warning** — PWA badges `public (…)` exposure; carry through as a danger
    banner (it means anyone with the link can drive the session).

Things the current Expo app does that the PWA lacks (keep them): QR pairing, Face ID app lock,
offline prompt queue with persistence, native attach pickers, per-session haptics.

---

## 3. Flagged backend gaps (do NOT build app-side; keep flagged)

- **Native push (APNs/FCM)**: `/api/push/*` is Web-Push-only. Smallest backend addition later:
  accept `{kind:"apns"|"fcm", device_token}` on subscribe + a sender. Until then native ships
  foreground-only alerts (Inbox tab + badge counts from the 3s fleet poll).
- **Self-signed LAN TLS**: unfixable app-side; pairing errors must show the `--anywhere` /
  `--local`+VPN guidance verbatim.
- **Fleet prompt text**: `SessionRow` doesn't carry the pending prompt/question text — the Inbox
  list can say *that* a session needs you, not *what* it asks. DecisionPeek (temporary WS attach)
  is the app-side answer; a `waiting_reason` field on SessionRow is the flagged backend nicety.

---

## 4. Information architecture (final)

```
src/app/
  _layout.tsx                 # providers: Theme, Auth(servers), QueryClient(persist), palette host
  connect.tsx                 # pairing (QR native / paste; deep-link ?url=; TLS guidance)
  (tabs)/_layout.tsx          # compact/medium: bottom tabs · expanded: MasterDetail rail
  (tabs)/index.tsx            # FLEET — live sessions, aggregate header (Σ cost, waiting count)
  (tabs)/inbox.tsx            # INBOX — waiting sessions; DecisionPeek sheets; tab badge = count
  (tabs)/history.tsx          # HISTORY — past sessions, search, resume
  (tabs)/settings.tsx         # SETTINGS — servers (multi-daemon), appearance, app lock,
                              #   notifications (web), about/diagnostics (protocol v7, version)
  new-session.tsx             # modal (Rise)
  session/[id]/_layout.tsx    # session shell: header, status strip, Segmented, banners,
                              #   copy_text→clipboard, notes→toasts, socket provider
  session/[id]/index.tsx      # CHAT (timeline + composer + cards)
  session/[id]/tasks.tsx      # TASKS
  session/[id]/agents.tsx     # AGENTS
  session/[id]/review.tsx     # REVIEW (plan + diff)
  # non-route surfaces: OverlayPanel (sheet, auto-presented), CommandPalette (global),
  #   DecisionPeek (sheet), AppLock (gate view)
```

15 surfaces total: Connect, Fleet, Inbox, History, Settings, New Session, Session shell,
Chat, Tasks, Agents, Review, OverlayPanel, CommandPalette, DecisionPeek, AppLock.

Navigation rules: `waiting` is the killer signal — Inbox badge everywhere, waiting sessions
pinned first (server already sorts), Emberdot beacon on their rows. On expanded layouts the
rail replaces tabs and ⌘K is the primary mover.

---

## 5. Net-new features (beyond Claude/Codex apps — all real-data-grounded)

| Feature | Grounding | Kind | Pri |
|---|---|---|---|
| **Command palette** (⌘K / swipe): jump to any session, run any slash command in the attached session (sends `prompt`), fleet actions (new/archive), local nav | local nav + `prompt` + existing overlay mirroring for server pickers | Enhancement | P1 |
| **Live fleet dashboard header**: Σ cost across live sessions, waiting count, busy count, per-session context gauges at a glance | `GET /api/sessions` fields | Enhancement | P1 |
| **DecisionPeek**: approve/deny/answer from Inbox without opening the session (short-lived WS attach renders the real card) | WS snapshot + `allow`/`answer` | Enhancement | P2 |
| **Multi-daemon support**: pair N servers, switch instantly; per-server query caches already namespaced | token-as-path auth is per-URL; no backend change | Enhancement | P1 |
| **Notification actions on web**: Allow/Deny straight from a push notification | `POST /api/answer` + sw.js (daemon built this route for exactly this) | Parity(web) | P1 |
| **Rich diff review**: collapsible files, full-width add/del line fills, hunk headers, pending-change review embedded in the permission card | `Snapshot.diff` | Parity+ | P1 |
| **Plan approval UX**: PlanCard with Approve/Revise/Cancel + revision free-text | `Snapshot.plan` + question mechanic | Parity | P1 |
| **Agent tree/board**: live subagent cards with cost roll-up | `Snapshot.subagents` | Parity+ | P1 |
| **Keyboard-first desktop**: ⌘K palette, ⌘1..4 tabs, ⌘N new session, Esc/arrow overlay passthrough, ⌘Enter send | `key` input + local nav | Enhancement | P2 |
| **Cost analytics view** (Settings → Usage): total/per-project spend aggregated from live + past sessions | `cost_usd` on SessionRow/PastSessionRow | Enhancement | P2 |
| **Voice input**: web Speech API now; native mic flagged | PWA parity | Parity(web) | P2 |
| **Desktop later** (flagged, not v1): tray fleet glance, local-daemon auto-pair bridge | ARCHITECTURE §6.4 | Enhancement | later |

Non-goals (explicitly out): `forge api` OpenAI-compat server (separate product), any Rust/daemon
changes, mock data of any kind, native APNs until the backend gap closes.
