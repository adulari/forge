# Feature: remote control — drive `forge chat` from a phone/desktop browser

> **Status (shipped):** `/remote` (alias `/rc`) slash command in the interactive TUI; an
> in-process `axum` HTTP + WebSocket server (`crates/forge-cli/src/remote.rs`) with three
> exposures — `--lan` (default, `0.0.0.0`, **self-signed HTTPS**), `--local` (`127.0.0.1`,
> plain HTTP), and `--anywhere` (a public tunnel via cloudflared/ngrok — bore is excluded,
> see §3 — no router port-forwarding); a token-gated, **installable** single-page control surface
> (self-contained HTML/CSS/JS, no framework) with **Chat / Tasks / Agents** tabs, tappable
> permission + question-option buttons, quick-command chips, a session/cwd/exposure header,
> live notifications, and a **PWA** (token-scoped manifest + service worker + icon) so it
> adds to a phone home screen and runs standalone; a `◉ remote` statusline segment; a QR
> code printed into the TUI scrollback. The wire is a versioned `Snapshot`/`RemoteInput`
> protocol (`PROTOCOL_VERSION`, currently **8**); the page shows a "refresh to update" banner
> on a mismatch. Auto-start is configurable (`[remote] auto`). The server reuses the running
> session's presenter channel — no second process, no IPC, no keys to configure.
>
> **v4 — full command + picker parity:** every slash command, picker, palette, and overlay is
> now usable from the browser through ONE generic mechanism (§2a): `Snapshot.overlay` projects
> whichever modal surface owns the TUI keyboard (the command palette, every `PickerKind` —
> sessions/checkpoints/tempers/assay/models/model-pin/resume-mode/copy-blocks/**duel winner** —
> the `@path` picker, the `/config` wizard, and the `/usage`/`/mesh`/workflow views as text
> bodies), and `RemoteInput` gained a keystroke channel (`key`) plus overlay verbs
> (`overlay_select`/`overlay_nav`/`overlay_filter`/`overlay_cancel`) that inject through the
> **same key path local keystrokes take**. `/copy` now ships its payload in
> `Snapshot.copy_text` so the phone can copy it to its own clipboard. The page assets are
> split into separate token-scoped files (`remote_assets/`), which let the CSP drop
> `'unsafe-inline'` entirely. `/keys` is host-only by design (a blocking fullscreen
> configurator on the host terminal) — the remote gets an explanatory note.
>
> **v5 — bulletproof reconnect + full scrollback + rich transcript:** every broadcast frame
> lands in a bounded per-server event log, and the WS handshake takes `?rev=<last seen
> revision>` — a reconnecting page replays exactly the frames it missed (no gap, no flicker),
> falling back to one full snapshot flagged `resync` when the gap is unfillable (§2b). A
> token-scoped `GET /<token>/api/history?before=<seq>&limit=<n>` pages the session's persisted
> transcript from the store, and the page fetches older pages on scroll-up — unlimited
> scrollback (the live snapshot transcript stays a short tail). The page renders markdown with
> a self-contained, CSP-safe syntax highlighter and tap-to-copy fenced blocks.
>
> **v7 — fleet dashboard + review cards + upload + voice (Phase 6, the endgame):** the
> daemon's session list is a real **fleet dashboard** (`GET /api/sessions` grew
> `waiting`/`context_tokens`/`context_limit`; sessions **waiting on a decision** sort first
> and pulse red); `Snapshot.diff` is a **structured diff card** (per-file `@@` hunks with
> old/new line spans, capped with "+N more" markers) shown as "what will this touch" on a
> write permission prompt and as the landed latest-turn diff after; `Snapshot.plan` projects
> the `present_plan` proposal as a **plan-approval card** whose Approve/Revise/Cancel buttons
> answer the same seq-checked question a local choice does; `POST /<t>/api/upload` accepts
> **multipart file/image uploads** (10 MB cap, names sanitized, stored under the session's
> `.forge/uploads/`) that ride the next prompt — images as vision input, text files as
> `@path` mentions — with an attach button + paste-an-image support in the input bar; and a
> **voice input** mic button (Web Speech API, transcribe-never-send, hidden where
> unsupported). See §2e.
>
> **What remains:** an end-to-end-encrypted tunnel channel (today a tunnel provider terminates TLS
> and could observe traffic; E2E would blind it), cross-host session handoff/teleport, and an Apple
> Watch client. The `forge attach <id>` thin TUI client and native iOS/Android companion apps,
> including iOS Live Activities, have shipped.

> A new control surface layered onto the existing `run_chat_tui` loop. It adds *how a user
> can drive a session* (a browser anywhere on the LAN, or loopback for a single machine) and
> *how the live state is observed remotely* (a JSON `Snapshot` broadcast over a WebSocket),
> without changing the agent loop's contract or the `Presenter`/`UiMsg` seam the TUI already
> uses. The remote input path injects prompts / permission answers / interrupts exactly as
> local keystrokes do, so the permission gate, hooks, and temper all apply unchanged.

## 1. Problem (JTBD)

> When I start a long Forge run and step away from my desk, I want to keep an eye on it and
> steer it (approve a write, interrupt a runaway turn, send a follow-up) from my phone —
> without SSH, without a second app, without exposing my API keys to a web service — so the
> session isn't abandoned the moment I leave the keyboard.

Forge's interactive surface is a terminal TUI on the machine running the session. There is
no way to observe or drive it from another device. SSH + tmux works for power users but
needs a shell client on the phone and an exposed SSH port; a hosted web UI needs a relay
that sees the session. Neither is "easy and accessible for both desktop/mobile".

## 2. Design

**Transport:** a tiny `axum` server with two static, token-gated routes — `/<token>` (the
HTML control page) and `/<token>/ws` (a bidirectional WebSocket). The token is 16 hex chars
generated at start time and printed into the TUI scrollback (as a URL **and** a QR code).
A request that doesn't match either route hits a 404 fallback that doesn't reveal remote
control is running. `--lan` (default) binds `0.0.0.0` so a phone on the same network can
connect; `--local` binds loopback only. The **advertised** LAN host (connect URL, QR code,
cert SANs) is never the bind address: the outbound-interface IP is discovered via a
connected UDP socket (`connect("8.8.8.8:80")` + `local_addr()` — route resolution only, no
packet is sent), overridable with `[remote] host` for multi-homed/VPN machines.

**Live state → browser:** each dirty frame the render loop builds a `Snapshot` (protocol
version · session id · cwd · exposure · busy · temper · tier · model · cost · context fill ·
the streaming reply's tail · a bounded ring of recent scrollback lines · the live task list ·
running subagents · queued prompts · any pending permission prompt or question + its
tappable options · `prompt_seq` (the pending prompt's identity) · remote-facing `notes` · a
monotonic `revision`) and publishes it on a `tokio::sync::watch` channel — **only when it
differs from the last broadcast frame** (`Snapshot: PartialEq`), so a busy turn doesn't push
~60 identical JSON frames/s to every client. The WS task forwards each change to each
connected browser, so the page mirrors the TUI statusline, transcript edge, **Tasks** panel,
and **Agents** panel in real time. Because `watch` fans out to every subscriber and inputs
share one `mpsc`, several phones + a desktop can drive one session at once with no
per-client state — disconnect/reconnect is transparent (the page auto-retries).

**Browser → session:** the page sends `RemoteInput` JSON (`{kind:"prompt",text}`,
`{kind:"allow",yes,seq}`, `{kind:"answer",text,seq}`, `{kind:"interrupt"}`, plus the v4
keystroke channel `{kind:"key",key}` and the overlay verbs — see §2a) over the WS. A
prompt can be a plain task, a `/command`, or a `//`-escaped hook command — all routed through
the *same* `dispatch_command` + prompt-hook + `spawn_turn` paths a local keystroke takes, so
slash commands (`/plan`, `/compact`, `/model`, `/mode`, `/help`, …), the busy guard, the
permission gate, temper, and hooks all apply unchanged. Prompts sent while a turn is running
are **queued** to run after it, exactly like local typing. `allow` answers a pending
permission, `answer` resolves an `AskUserQuestion` (a tapped option button sends its 1-based
index), and `interrupt` aborts the turn task. `allow`/`answer` must echo the `prompt_seq`
their buttons were rendered from — a mismatch (the prompt changed under the tap) is ignored,
so a stale answer can never resolve a newer prompt. A remote prompt is indistinguishable from
a local one. Inbound frames over `MAX_INPUT_BYTES` (256 KiB) are dropped to bound a hostile
client.

## 2a. The generic overlay protocol (v4)

Nearly every previously-unreachable command failed the same way: it opened a picker/palette/
overlay that mutates TUI-only state (`app.picker`, `app.palette`, `app.config_editor`, …)
which was neither serialized into `Snapshot` nor driveable from the browser. v4 fixes the
CLASS, not the instances, with two halves:

- **Projection — `Snapshot.overlay` (`SnapOverlay`).** `App::remote_overlay()` (beside
  `remote_snapshot()`) projects whichever modal surface currently owns the keyboard, with the
  same precedence the key loop uses: workflow view → `/config` editor → palette → `/usage` →
  `/mesh` → `@path` picker → picker. The shape is
  `{kind, title, rows: [{id,label,detail,selected,group}], selected, filter, free_text, body}`:
  tappable `rows` for selectable surfaces, `filter` when the surface has a type-to-filter
  query, `free_text` while it's collecting a value (a `/config` field edit), and a
  pre-rendered text `body` for informational overlays (usage tables, the mesh routing
  verdict, workflow narration). Every `PickerKind` maps to a stable `kind` tag via one
  exhaustive `picker_kind_wire` match — **a future picker becomes remote-drivable by adding
  exactly one arm** (forgetting it is a compile error).
- **Drive — the keystroke channel.** `key` injects a named key (`Up | Down | Enter | Esc |
  Tab | BTab | PageUp | PageDown | Home | End | Backspace | Char:<c>`) into the head of the
  SAME input loop local keystrokes flow through, so a remotely-committed picker produces the
  identical `DispatchOutcome` handling. The overlay verbs are sugar over it:
  `overlay_select{id}` moves the server-side cursor onto that row then synthesizes Enter;
  `overlay_nav{delta}` becomes repeated ↑/↓ (bounded); `overlay_filter{text}` replaces the
  overlay's query (or the value being edited when `free_text`); `overlay_cancel` is Esc *only
  while something modal is open* — it can never interrupt a turn or quit the host. Two safety
  guards on raw keys: they are dropped (with a note) while a permission prompt/question is
  pending — those must go through the seq-checked `allow`/`answer` — and a bare Esc with
  nothing to close is ignored rather than quitting the host TUI.

Riding the mechanism with zero special cases: `/duel`'s winner pick (the Duel picker rows are
the candidates; a tap merges the winner exactly as a local Enter), `/model`/`/models` pin +
browse (provider drill-in included), `/mode` tempers, `/sessions`/`/resume`, `/checkpoints`/
`/undo`, `/assay`, `/copy`'s block picker, `/config`, `/help` (the palette itself), and
`@path` file mentions. The one special case is `/copy`'s payload: Enter still copies on the
host, AND the text ships in `Snapshot.copy_text` so the page can offer a "Copy here" button
for the phone's own clipboard.

## 2b. Reconnect/replay + full scrollback (v5)

Reliability is the #1 complaint about every remote-coding rival — a dropped connection that
loses state (or kills the session outright) makes the whole surface untrustworthy. v5 makes
disconnects a non-event:

- **Event log + `?rev=` replay.** `RemoteControl::broadcast` records every published frame in
  a bounded ring (`EventLog`, 512 entries) keyed by the snapshot's monotonic `revision`. The
  WS handshake takes `?rev=<last seen revision>`: when the ring can fill the gap, the client
  receives **exactly the frames it missed** (none when already current), then follows live —
  the page shows what happened while it was away with no gap and no flicker. When it can't
  (fresh connect, evicted, or a rev from a previous server), it gets ONE full snapshot flagged
  `resync: true`. The page persists its last seen revision in `sessionStorage` (keyed by the
  token base so it can never target a different server) and dedupes the replay/live overlap by
  revision, so a frame that raced the handshake is never rendered twice. The session itself
  never depended on a connected client — the server just kept broadcasting into the watch
  channel — so this closes the last gap: *the page* now survives the disconnect too.
- **Full scrollback via `GET /<token>/api/history?before=<seq>&limit=<n>`.** The live
  `Snapshot.transcript` stays a short tail (12 lines — a phone screen); real scrollback pages
  through the session's **persisted** messages. `Store::load_history_page` returns user +
  assistant turns plus `visibility='ui'` notes (user-facing, part of the conversation),
  newest first, excluding tool results / tool-call carriers / system prompts (harness
  plumbing) but including compacted-away rows (the user's history, not the model's context).
  Scroll to the top of the transcript and the page fetches the next-older page and prepends it,
  preserving the scroll position; `before` walks the window, `limit` is clamped server-side
  (max 200). The seam is a `HistoryProvider` closure built in `run.rs` over the session's
  store handle, so `remote.rs` never depends on `forge-store` — and the session id comes from
  the latest snapshot, so history follows `/new`/resume automatically. The service worker
  never caches `/api/` responses.
- **Rich transcript.** History messages and the live streaming edge render through a minimal
  markdown renderer written into the page (headings, lists, paragraphs, inline `code` /
  **bold** / *italic*, links as plain text — never live anchors) plus a self-contained
  syntax highlighter (strings / comments / numbers / keywords for rust, js/ts, python, go,
  bash, json; aliases like `py`/`rs`/`sh` fold in). Fenced blocks get a tap-to-copy button
  (device clipboard, like the `/copy` bar). Everything is built with
  `createElement`/`textContent` only — transcript content never reaches `innerHTML`, so it
  cannot inject markup even before the no-inline CSP is considered.

**PWA + notifications:** alongside `/<token>` and `/<token>/ws`, the server serves a
token-scoped `manifest.webmanifest`, `sw.js` (service worker), and `icon.svg`, so the page
installs to a home screen and launches standalone into *this* session's control surface.
The page (with permission) raises a local notification when a permission/question appears or
a turn completes while it's backgrounded — covering the "phone in pocket, PWA open" case
without a push relay that would see the session.

**Toggle:** `/remote` is a builtin command (`CommandAction::Remote { lan }`) that returns
`DispatchOutcome::ToggleRemote`. The loop's `toggle_remote` helper starts the server (on)
or drops the `RemoteControl` handle (off — its `Drop` sends a `closed` snapshot so
connected browsers stop reconnecting, then aborts the server task). It's in the non-mutating
guard list, so it toggles even mid-turn. The `◉ remote` statusline segment reflects the
state at a glance.

**Why in-process + WebSocket (not a second binary, not SSE):** the session, presenter
channel, and `App` state already live in the `forge chat` process — reusing them is zero
new IPC and zero key configuration. The control surface needs to *send* input (not just
receive state), so a server→client-only SSE isn't enough; a WebSocket carries both
directions over one connection. `axum` bundles `hyper` + `tokio-tungstenite`, and the
workspace already has `reqwest` (rustls) + a multi-thread tokio runtime, so the added
dependency surface is small.

## 3. Security posture

The threat model is **a peer on the same LAN** (coffee-shop / shared Wi-Fi), not a
determined adversary with a sniffer. Defenses:

- **Token-gated paths.** A 64-bit random token in the URL path; without it a peer gets a
  404 and can't even tell remote control is on. The token is only valid while `forge chat`
  is running.
- **`--local` escape hatch.** `forge chat` then `/remote --local` binds `127.0.0.1` —
  control from this machine only, never the LAN.
- **No secrets exposed.** The server serves only the static control page + the live
  `Snapshot` (model name, cost, transcript tail, prompts). API keys never leave the
  process.

- **TLS on the LAN — no cleartext fallback.** `--lan` generates a self-signed certificate at
  startup and serves HTTPS, so the token never travels in cleartext over the network; the
  cert's SHA-256 fingerprint is printed next to the connect URL for verification. If TLS
  setup fails, LAN remote control is declared **unavailable** (`start` errors on cert-gen
  failure; async setup failures set `tls_failed`, and the status reads "LAN (unavailable —
  TLS failed)") — it never silently downgrades to plain HTTP, which would both lie about the
  transport and be unreachable at the already-printed `https://` URL. Use `--local` or
  `--anywhere` instead. `--local` stays plain HTTP (loopback never leaves the machine);
  tunnels terminate TLS at the provider.
- **Prompt identity (`prompt_seq`).** Every pending permission/question carries a
  monotonically increasing sequence number; remote `allow`/`answer` inputs must echo it, and
  mismatches are ignored. A stale or raced tap (two phones, or a prompt replaced in the same
  frame) can never approve a different — possibly more dangerous — prompt than the one
  rendered. Legacy (v2) seq-less answers fail to parse and are dropped; the page's
  protocol-mismatch banner tells the operator to refresh.
- **`bore` is excluded from `--anywhere`.** bore forwards raw TCP with **no TLS**: the token,
  snapshots (source code, cwd, transcript), and permission approvals would cross the public
  internet in cleartext — a sniffed token means a stranger can approve shell commands (RCE).
  Its `http://` origin also breaks the PWA (no secure context → no service worker or
  notifications). Only cloudflared and ngrok (both HTTPS end-to-end) are probed.
- **Hardened page headers.** The control page ships `X-Frame-Options: DENY`, a
  same-origin `Content-Security-Policy` with **no `'unsafe-inline'`** (the v4 asset split
  moved all script/style into separate token-scoped files, and the page uses zero inline
  handlers — an injected `<script>`/`onclick` can never execute), and
  `Referrer-Policy: no-referrer` so the token-bearing URL never leaks via the Referer
  header.
- **Keystrokes can't bypass the prompt gate.** Remote `key` inputs are dropped while a
  permission prompt / question is pending (those must go through the seq-checked
  `allow`/`answer`), and a bare Esc with nothing modal open is ignored instead of quitting
  the host TUI.
- **Frame-size cap.** Inbound WebSocket frames are capped (`MAX_INPUT_BYTES`) so a hostile
  or buggy client can't exhaust memory with a giant payload.
- **`--anywhere` is loud.** Opening a public tunnel prints an explicit warning that anyone
  with the link can drive the session — the path token is then the only gate.

## 3a. Configuration

`[remote] auto` controls auto-start at `forge chat` launch: `off` (default; start with
`/remote`), `local`, `lan`, or `anywhere`. `[remote] host` overrides the auto-discovered
LAN IP in the connect URL / QR / cert SANs (multi-homed or VPN'd machines where discovery
picks the wrong interface). Example:

```toml
[remote]
auto = "lan"          # session is reachable from a phone the moment chat starts
host = "192.168.1.5"  # optional: advertise this interface instead of the discovered one
```

### Project browser roots

New daemon sessions use the directory where `forge serve` started when no project is selected. The
companion app remembers the last project separately for each paired server and offers recent
non-worktree projects. Its server-side folder browser is intentionally narrower than session
control: it can enumerate only the daemon working directory and configured `[remote]
project_roots`, with canonical-path containment checks that reject `..` and symlink escapes.
Hidden directories are omitted. This limits passive filesystem disclosure when someone is choosing
a project from a phone.

```toml
[remote]
project_roots = ["~/Projects", "/srv/work"]
```

The desktop app can use the operating system folder picker when connected to a loopback daemon.
Mobile and remote browsers browse the allowlisted server roots. A manually entered absolute path
remains available as an advanced fallback because the daemon token already grants full agent
control; `project_roots` limits browsing, not explicit session creation.

### Stable tunnel URL

By default `/remote --anywhere` or `forge serve --tunnel` opens an ephemeral **quick tunnel**: a new random
`trycloudflare.com`/`ngrok-free.app` URL every launch. Bookmarking or installing that URL to a
phone home screen is pointless — it dies the moment you restart. `[remote] tunnel_name` /
`tunnel_hostname` pin it to a stable hostname instead, so the same link keeps working across
every `forge chat --anywhere` / `forge serve --tunnel` launch. The older
`forge serve --anywhere` spelling remains a deprecated alias during migration.

**cloudflared (named tunnel)** — one-time setup, then a config line:

```sh
cloudflared tunnel login                              # authorize cloudflared against your account
cloudflared tunnel create forge                        # mints a named tunnel + credentials file
cloudflared tunnel route dns forge forge.example.com   # DNS-routes the hostname to the tunnel
```

```toml
[remote]
tunnel_name     = "forge"               # the named tunnel to run (`cloudflared tunnel run <name>`)
tunnel_hostname = "forge.example.com"   # the DNS-routed hostname to advertise as the connect URL
```

A named-tunnel run prints no public URL on its own (the DNS route is already configured), so
Forge waits for cloudflared's "registered tunnel connection" log lines instead of parsing a URL,
then advertises `https://forge.example.com` directly.

**ngrok (reserved domain)** — set `tunnel_hostname` alone (no `tunnel_name`, which is a
cloudflared-only concept):

```toml
[remote]
tunnel_hostname = "forge.ngrok.app"   # a domain reserved on your ngrok account/plan
```

This runs `ngrok http --domain=forge.ngrok.app <port>` instead of the default ephemeral
`ngrok http <port>`.

If both fields are unset, behavior is unchanged: an ephemeral quick tunnel, whichever of
cloudflared/ngrok is installed. If `tunnel_name` is set but only ngrok is on `PATH` (or
`tunnel_hostname` alone is set but only cloudflared is installed), Forge fails fast with a
message naming what's configured vs. what's actually installed, rather than silently falling
back to a quick tunnel.

## 3b. PWA lifetime

The in-chat `/remote` server still mints a fresh port + token per session, so an installed
home-screen app outlives that origin; its service worker answers dead navigations with an
explicit **"session ended — reopen `/remote` from the TUI"** page. The permanent install is
the daemon's (§2c): `forge serve`'s origin never changes, so its PWA never dies.

## 2c. The multi-session daemon — `forge serve` (v6)

The end-state of the overhaul: remote control stops being a per-chat bolt-on and becomes a
**headless daemon hosting N concurrent sessions**, all driveable from the same PWA. This is
the architecture that beats Claude Code remote control's one-session-per-process, ~10-minute
death timeout, and per-session URLs.

```
        forge serve  (headless daemon, stable port + stable token/origin)
  ┌──────────────────────────────────────────────────────────────────────┐
  │  SessionRegistry: id → Arc<SessionDriverHandle>                       │
  │     handle { snapshot_rx: watch<Snapshot>, events: EventLog,          │
  │              input_tx: mpsc<RemoteInput>,                             │
  │              meta { cwd, worktree, title, created, last_activity } }  │
  │   per session ── SessionDriver task (headless run_chat_tui):          │
  │       App + dispatch_command + spawn_turn* machinery, no terminal,    │
  │       consumes ChannelPresenter<UiMsg>, drains RemoteInput            │
  └───────────────┬───────────────────────────────────────────────────────┘
                  │ axum router (ONE stable base /<daemon-token>/)
   GET  /<t>/                     control page (session list + live UI)
   GET  /<t>/manifest|sw|icon|…   PWA assets (stable scope → install survives)
   WS   /<t>/ws?session=<id>&rev=<n>   per-session stream + replay-from-rev
   WS   /<t>/ws/fleet                   fleet revision invalidations
   GET  /<t>/api/sessions         list (id, title, cwd, busy, cost, activity)
   POST /<t>/api/sessions         create {cwd, worktree, title?, model?, resume?}
   POST /<t>/api/sessions/{id}/archive
   GET  /<t>/api/history?session=<id>&before&limit
```

- **The SessionDriver seam.** `run/driver.rs` runs one session as a plain tokio task using
  the SAME primitives `run_chat_tui` uses — `dispatch_command` (now taking `Option<&mut Tui>`;
  the daemon passes `None`, the TUI passes `Some`, behavior identical), `picker_accept`,
  `apply_overlay_input`, the `spawn_turn*`/`spawn_compact`/`spawn_duel` family, and one shared
  `build_snapshot_frame` producer — so a command dispatched from the phone produces the
  identical `DispatchOutcome` handling in both worlds. The driver mirrors the TUI's remote
  drain (prompt queueing while busy, seq-checked Allow/Answer, the named-key channel with the
  same idle-Esc guard) plus its done-signal pipeline (queued prompts, `/loop` continuation,
  the `/duel` winner picker, turn-end auto-compact). Host-terminal-only affordances (`/keys`,
  `$EDITOR` jumps, the host clipboard) degrade to explicit notes. `run_chat_tui` itself is
  unchanged in behavior — the pty e2e battery is the guard.
- **Stable origin = permanent PWA.** The port comes from `[remote] port` (default 7420) and
  the daemon token is minted once into `<config>/serve-token` (0600) and reused forever;
  `--rotate-token` revokes. The manifest's `scope`/`start_url` therefore never change: install
  once, use forever. Exposures mirror `/remote` — LAN + self-signed HTTPS by default,
  `--local` loopback, `--anywhere` via cloudflared/ngrok.
- **Sessions survive clients — by construction.** The driver broadcasts into a per-session
  `watch<Snapshot>` + `EventLog` whether or not anyone is connected; the WS route is just
  `pump_ws` (shared with the single-session server) pointed at a registry entry. Disconnect,
  come back an hour later, and `?rev=` replays what you missed (or resyncs).
- **Per-session working directories + worktrees.** An omitted `cwd` defaults to the daemon's
  canonical startup directory. `GET /api/projects` supplies that default plus recent durable
  projects and browse roots; `GET /api/projects/browse?path=...` provides the canonicalized,
  allowlisted directory browser used by the companion apps. With
  `worktree: true` the daemon creates `.forge/worktrees/<id>` branched from HEAD (the audited
  `WorktreeGuard` creation, minus its drop-side removal — the worktree must outlive the
  process) and the session runs inside it. `Session::set_work_root` roots every tool call's
  relative `path`/`cwd` there via the same rewrite subagent isolation uses, so concurrent
  sessions can't stomp each other's trees. Archive commits uncommitted worktree edits onto
  the session branch (never silently lose work) and leaves worktree + branch for manual merge.
- **Schema v8.** `session.worktree_path` + `session.archived` (archived sessions leave
  `forge sessions`, the resume picker, and the daemon list; history is kept), plus a
  pre-created `push_subscription` table so the upcoming actionable-web-push phase needs no
  migration.
- **Coexistence.** `forge chat`'s in-process `/remote` is untouched (own ephemeral server);
  the one control page serves both worlds by probing `/api/sessions`. Registering an already-live
  in-process chat session into a running daemon remains future work; `forge attach <id>` already
  drives daemon sessions from another terminal.

## 2d. Actionable Web Push + offline input queue (Phase 5)

Close the page, lock the phone — and still get told the moment a session **needs a decision**
(permission prompt / AskUserQuestion), finishes a turn, or fails. A permission notification
carries **Allow / Deny actions**: the service worker answers them itself over
`POST /<t>/api/answer`, so the agent is unblocked from the lock screen without ever opening
the app. Push is a `forge serve` feature (subscriptions bind to an origin; only the daemon's
is stable).

**Self-hosted, no relay — this applies to Web Push specifically.** (Native iOS push added
later, §2f, does default to an optional hosted relay — that claim doesn't extend to it; read
§2f before assuming "no relay" covers the whole push story.) Forge mints its own **VAPID
(RFC 8292) P-256 keypair** once, into
`<config>/vapid-key` (0600, next to `serve-token`). There is no Firebase project, no
third-party relay account, no vendor SDK: the daemon POSTs each message **directly to the
push endpoint the browser handed out** (that endpoint — run by the browser vendor — is how
Web Push delivers to a sleeping device; it is unavoidable and, crucially, blind). Every
payload is **end-to-end encrypted per RFC 8291** (`aes128gcm`: ECDH P-256 + HKDF-SHA256 +
AES-128-GCM, fresh ephemeral key + salt per message), so the push service relays ciphertext
it cannot read; the VAPID JWT (ES256, `aud` = endpoint origin, 12 h expiry) proves to the
service that the POST comes from the key the subscription was created with. The
implementation is in-tree (`forge-cli/src/push.rs`, RustCrypto `p256`/`hkdf`/`sha2`/
`aes-gcm`) and is verified byte-for-byte against the RFC 8291 §5 test vector.

**Routes.** `GET /<t>/api/push/key` (the `applicationServerKey`),
`POST /<t>/api/push/subscribe` (stores `{endpoint, keys:{p256dh, auth}}`, deduped by
endpoint, keys validated at the door), `POST /<t>/api/push/unsubscribe`. A push service
answering 404/410 gets its subscription pruned automatically.

**Triggers + debounce.** The session driver watches its own snapshot transitions
(pure `push::detect_trigger`): a **new** `prompt_seq` with a pending permission/question
pushes a decision card (TTL 5 min — a stale "Allow?" on a lock screen misleads); the `busy`
falling edge pushes "turn complete" with a final-line preview, or "turn failed" when the turn
surfaced a genuine error (TTL 1 h). A replaced prompt (seq bump) re-fires; the same pending
prompt never does. **No push is sent while any WebSocket client is connected** to that
session (`push::should_push`) — deliberately the simpler of the two debounce designs (the
alternative, page-visibility reporting over the WS, adds protocol surface and still lies for
unattended dashboards): a phone that locks or backgrounds the PWA drops its WS within
seconds, so "a WS is attached" reliably means "someone is watching". Trade-off, documented: a
desktop tab left open in the background suppresses pushes to the phone. Delivery is strictly
best-effort — dispatched fire-and-forget with a 10 s box, never blocking or delaying the turn.

**`/api/answer` = the WS `Allow`, over plain HTTP.** The notification action POSTs
`{session, seq, allow}`; the daemon 404s an unknown session, 409s when no prompt is pending
or the echoed `seq` is stale (the prompt changed after the notification rendered), and on
success feeds the same seq-checked `RemoteInput::Allow` the driver re-validates — the exact
stale-tap protection the WS path has.

**Offline input queue.** Prompts typed while the WS is down no longer vanish: they land in
`localStorage` (keyed by server + session, so a queue can never flush into a different
session), render as "📴 queued (offline)" above the actions area, survive reloads, and flush
**in order** on reconnect. Bounded at 20; past that, new input is dropped **loudly** (a
"queue full — N dropped" line), never silently.

**Enabling it.** The sessions panel (☰) grows a *push notifications* row showing the
permission state with an Enable/Disable button — subscribe runs
`Notification.requestPermission` → `PushManager.subscribe` (with the daemon's public key) →
`POST subscribe`. **iOS caveat:** Safari only exposes push to an **installed** PWA (Share →
Add to Home Screen) and only on an origin it trusts — the self-signed LAN cert must be
installed/trusted on the device, or use `--anywhere` (the tunnel provider's real HTTPS makes
this the low-friction path on iPhone). What remains manual to verify (not covered by the
automated suite, which proves encryption/VAPID/triggers/debounce against a local mock push
endpoint): real vendor-endpoint delivery on a physical device (FCM/Mozilla/APNs behavior,
OS-level notification display, action buttons on each platform's lock screen).

## 2e. Fleet dashboard + review cards + upload + voice (Phase 6, v7)

The final phase makes the phone a *decision surface*, not just a viewer.

**Fleet dashboard.** `GET /api/sessions` rows grew `waiting` (a permission prompt or
question is blocking the turn — read straight off each driver's live snapshot),
`context_tokens`, and `context_limit`. The server sorts **waiting sessions first** (then
newest-created, a stable tiebreak that doesn't reshuffle while sessions stream), so the
dashboard's top row is always the session that needs a human NOW — red pulsing dot + a
"needs decision" badge, alongside the busy/idle dot, title, cwd tail, ⎇ worktree badge,
cost, token gauge, and last-activity age. Tap to attach. The page's existing 5-second list
poll carries it; no extra protocol.

**Structured diff card (`Snapshot.diff`).** Reuses exactly what the TUI already computes:
the write-tool `preview()` `FileDiff` emitted as `PresenterEvent::Diff` *before* the
permission gate, hunked by the same `similar` grouped-ops(3) pass `diff_to_lines` renders
(`render::diff_file_snapshot` — no second diff implementation). The `App` keeps the preview
as `pending_diff` until the tool's `ToolResult` resolves it: ok → it *landed* and joins the
turn's `turn_diffs` (latest edit per path, capped at 10 files), failed/denied → dropped (the
file was never touched). Projection: while a permission prompt is armed and a preview is
pending, the card is that ONE proposed change flagged `pending` ("what will this touch" —
rendered above the Allow/Deny buttons); otherwise it's the landed latest-turn diff. Payload
caps: ~40 hunk lines/file + 10 files, with "+N more lines/files" markers; `adds`/`dels`
always count the whole change. Cleared when the next user prompt starts a new turn.

**Plan-approval card (`Snapshot.plan`).** `present_plan`'s `PlanProposal` (title, steps
with details, notes) is retained on the `App` and projected every frame — the TUI's
scrollback card is unreachable from a phone. Approval stays *exactly* the local path:
core's turn-end `resolve_plan_approval` asks a question with options `Build it` / `Cancel`
(+ free text = revise), and the page's **Approve & build** button answers that question by
option number over the seq-checked `Answer` input; **Revise** opens a prefilled free-text
box whose submission is the same free-text revision answer; **Cancel** answers the Cancel
option. No new approval mechanism, no drift from `/execute` — one code path, verified e2e
(mock `/plan` → card → remote `Answer("1")` → "plan approved — building in Auto-edit").

**File/image upload (`POST /<t>/api/upload`).** Multipart (axum's `multipart` feature),
token-scoped, session-addressed under the daemon (`?session=<id>`) and available on the
in-chat server too. Files are size-capped (10 MB), filenames flattened to one sanitized,
timestamp-prefixed component (traversal-proof — unit + route tested with hostile names),
and stored under the session's own scratch area `<cwd>/.forge/uploads/<session>/`. Non-image
non-UTF-8 bodies are refused (422): only images and text have an injection path. Delivery is
a new `RemoteInput::Attach {path, image}` the drains handle: **images** → `Session::
attach_images` (vision input on the next turn, the `/image` path's final leg), **text
files** → an `@path` mention prepended to the next prompt (expanded by the same
`expand_at_files` a typed mention uses — the remote prompt path now runs it too, closing a
v4 parity gap). The drain *confines* `Attach` paths to the canonical uploads dir, so a WS
client can't use it to read arbitrary host files. The page grows a 📎 attach button,
paste-an-image support in the prompt box, and upload chips showing each file's state.

**Voice input.** A 🎤 button in the input bar (Web Speech API): tap, speak, and the
transcript lands **in the prompt box** — never auto-sent. Hidden where `SpeechRecognition`
is unavailable. Zero dependencies, zero wire surface, CSP-safe (recognition runs in the
browser engine, not against our origin).

## 2f. Native iOS Push (APNs) + the hosted relay

Web Push (§2d) only reaches an **installed** PWA on iOS, and Safari's implementation is
comparatively unreliable for lock-screen delivery. The mobile app (Expo/React Native)
additionally registers for real native push via Apple's APNs — see ADR-0012 for the full
design rationale.

**Two ways this gets your Apple credential to Apple**, and the daemon (`forge serve`) picks
between them automatically:

- **Bring your own Apple key** — set `FORGE_APNS_TEAM_ID`/`FORGE_APNS_KEY_ID`/
  `FORGE_APNS_KEY_PATH` (an Apple Developer APNs Auth Key `.p8` you generate yourself in Apple
  Developer → Certificates, Identifiers & Profiles). Fully local, exactly the same "no relay"
  posture as Web Push above: the
  daemon signs its own ES256 JWT and POSTs straight to `api(.sandbox).push.apple.com`. Always
  wins if configured, regardless of the option below.
- **The hosted relay (default, zero setup)** — if no local key is configured, the daemon
  forwards through a small relay (`crates/forge-relay`, source in-tree, deployed by the
  project operator) that holds the operator's own Apple key centrally. This is what makes
  native push work out of the box for a typical self-hoster, without requiring everyone to
  personally enroll in the Apple Developer Program just to receive notifications.

**What crosses the relay, precisely:** an opaque device token, an environment string
(`sandbox`/`production`), and the notification payload itself. Updated stock clients send only a
static alert ("Forge — Open Forge to view an update") with fixed routing placeholders. The public
server independently replaces every alert again before APNs. An older daemon can still transmit a
rich alert to the relay during the upgrade window, but the public service does not log or forward
that text. Live Activity payloads still contain the deliberately small
`busy`/`waiting`/`cost_usd`/`context_tokens`/`context_limit` status object, but not the session ID
used for the local lookup. Forge also does not send the daemon auth token, transcript, or source
files. An explicitly configured private relay retains rich alert text for operators who control
that relay; a local Apple key bypasses every relay. The public service accepts only Forge's app and
Live Activity topics,
validates the narrow Forge notification schemas and Apple's 4 KiB payload ceiling, applies
per-client/per-device and global daily caps, and keeps raw client IPs/device tokens out of its
in-memory limiter state and origin logs. It cannot reach devices or apps outside Forge's bundle IDs.

**Opting out.** `FORGE_APNS_DISABLE_RELAY=1` turns native push off entirely rather than
silently falling back to some other behavior — an explicit choice, not a silent downgrade.
Since the relay's source lives in this repo (`crates/forge-relay`), anyone uncomfortable
trusting the project operator's instance can run their own and point `FORGE_APNS_RELAY_URL`
at it — a strictly better answer for a team/multi-machine setup than "bring your own key
only," since it centralizes one key without requiring the operator's trust.

A private relay may set `FORGE_RELAY_TOKEN` server-side and the same value as
`FORGE_APNS_RELAY_TOKEN` on each trusted `forge serve` daemon. The public project relay does not
embed a shared token—an open-source client secret would be extractable—so it instead enforces the
app-topic/payload allowlist, per-client and per-device limits, a hard daily cap, and edge/origin
network controls described in ADR-0012.

**Token lifecycle.** Registration accepts only Apple's 64-character lowercase-hex device and Live
Activity token shape. Before delivery the daemon removes malformed legacy rows. It also removes the
specific subscription after Apple's `410 Unregistered` response or a reason-qualified
`400 BadDeviceToken`/`DeviceTokenNotForTopic`, while preserving it for unrelated 400 responses.

## 2g. Run as a background service — `forge service`

`forge serve` is a foreground process by default: close the terminal (or log out), and the
daemon dies with it. `forge service` installs it as an opt-in, always-on user-level OS service
so it survives terminal closes, crashes (auto-restart), and — on Linux/macOS — login itself.
No root/sudo anywhere; this is a per-user service, not a system one.

```
forge service install [--anywhere|--lan|--local] [--port <p>]   # install + start now
forge service status                                            # installed? running? port up?
forge service start | stop | restart                            # control without reinstalling
forge service uninstall                                         # stop + remove
```

Exposure defaults to `--lan` (same default as `forge serve` itself) and, together with the
resolved port, is baked directly into the installed unit — the unit is the single source of
truth; `status` never parses flags back out of it, it only asks the OS service manager whether
the unit exists/is running and independently TCP-probes the port.

| OS | Backend | Install | Control |
|---|---|---|---|
| Linux | systemd `--user` unit at `~/.config/systemd/user/forge-serve.service` | `systemctl --user daemon-reload` + `enable --now forge-serve` | `systemctl --user start\|stop\|restart forge-serve`, `is-active` for status |
| macOS | launchd agent at `~/Library/LaunchAgents/dev.forge.serve.plist` (`RunAtLoad`, `KeepAlive.SuccessfulExit=false` — restart on crash only) | `launchctl bootstrap gui/$UID <plist>`, falling back to `launchctl load -w` on older macOS | `launchctl kickstart`/`kill SIGTERM`/`kickstart -k` against `gui/$UID/dev.forge.serve` |
| Windows | Task Scheduler logon task `ForgeServe` (`/SC ONLOGON`) — not a real Windows Service, since `forge serve` doesn't speak the SCM protocol and wrapping it with an external shim like NSSM isn't worth the added dependency | `schtasks /Create ... /F` + `/Run` to start immediately | `schtasks /Run\|/End /TN ForgeServe` |

Surviving a reboot **before** you log in (Linux) needs `loginctl enable-linger $USER` — `install`
prints this as a note but never runs it itself (it can require auth). Every backend call
surfaces the failing command's stderr with an actionable hint (e.g. "is a systemd user manager
available?") rather than a bare exit code.

## 4. Surfaces touched

| Layer | Change |
|---|---|
| `forge-cli/src/remote.rs` | Server, `Snapshot`/`RemoteInput` types + `PROTOCOL_VERSION` (8), `SnapOverlay`/`SnapRow` + `named_key`, v5 `EventLog` + `?rev=` replay + `Snapshot.resync`, `GET /api/history` (`HistoryRow`/`HistoryProvider` seam), v7 `SnapDiff`/`SnapPlan` + `RemoteInput::Attach` + `POST /api/upload` (`store_upload`/`sanitize_upload_name`, 10 MB cap), PWA manifest + service worker + icon, TLS, tunnels, QR renderer, `MAX_INPUT_BYTES` cap, `Exposure: From<RemoteAuto>` |
| `forge-cli/src/remote_assets/` | The control page split into `page.html` / `app.js` / `styles.css` / `sw.js` (served via `include_str!` as token-scoped routes; enables the no-`unsafe-inline` CSP); the page's generic overlay renderer + copy-here button; v5: `?rev=` reconnect + sessionStorage revision + replay dedup, scroll-up history pagination (`#hist` above the live `#tail`), markdown renderer + syntax highlighter + fenced-block copy buttons; v7: fleet dashboard (waiting-first list, needs-decision badge, token gauge), plan + diff cards, 📎 upload button + paste-an-image + chips, 🎤 voice input |
| `forge-config/src/lib.rs` | `[remote]` block: exposure/tunnel settings plus `project_roots` for the remote folder-browser allowlist |
| `forge-store/src/lib.rs` | v5: `Store::load_history_page` + `HistoryRow` (user-facing transcript pages, newest first, `before`/`limit` windowed); Phase 5: `PushSubscription` + `upsert/delete/list_push_subscriptions` (endpoint-deduped) |
| `forge-cli/src/push.rs` | Phase 5: VAPID keypair (persist 0600, ES256 JWTs), RFC 8291 `aes128gcm` encryption (verified against the §5 test vector), `PushNotifier` (fire-and-forget delivery, 404/410 pruning), pure `detect_trigger`/`should_push` decision fns |
| `forge-cli/src/serve.rs` | Multi-session daemon routes including project catalog/browser, push subscriptions and answers, per-session WS client counting, waiting-first fleet ordering, and session-addressed uploads |
| `forge-tui/src/app.rs` | `App.remote_active`, `question_prompt`, `recent_transcript` ring, `drain_flush_remote`, `remote_snapshot` (tasks/subagents/queued/question options), `remote_overlay()` + `OverlaySnapshot`/`OverlayRowSnapshot` + `picker_kind_wire`, `print_lines`, statusline `◉ remote` segment; v7: `pending_diff`/`turn_diffs` lifecycle (Diff → ToolResult), retained `plan`, `DiffSnapshot` types + `render::diff_file_snapshot` |
| `forge-tui/src/commands.rs` | `CommandAction::Remote { mode }`, `/remote` (alias `/rc`) parse + registry entry |
| `forge-cli/src/cli/commands/run.rs` | `DispatchOutcome::ToggleRemote`, `toggle_remote`, `[remote] auto` startup, remote input draining + full-state snapshot broadcast in `run_chat_tui`; v4: `next_input_event` (remote keys join the local key loop), `apply_overlay_input` + `RemoteOverlayOp`, `/keys` host-only note, `remote_copy_text`; v5: `RemoteControl::broadcast` (frame → event log + watch), the `HistoryProvider` closure over the session's store |
| `forge-cli/src/cli/commands/service.rs` | `forge service install\|uninstall\|status\|start\|stop\|restart` — user-level background daemon for `forge serve` (systemd `--user` / launchd / Task Scheduler backends, no root) |
| `Cargo.toml` | `axum` (ws), `axum-server` (rustls), `rcgen`, `tokio-tungstenite`, `qrcode`; `tokio` `net` feature |
| tests | snapshot wire-shape (v5 incl. overlay/copy_text/resync + `HistoryRow`), named-key table, overlay-verb units, per-`PickerKind` projection units, an e2e-style remote drive of the `/model` pin picker asserting the pin changed, `EventLog` replay/eviction/bounded units, history-page store units (windowing/ordering/ui-rows/session-scoping), manifest/base/SW/exposure-mapping units + two `--ignored` real-socket round-trips (page + WS + PWA assets; connect → drop → `?rev=` reconnect asserting exact gap-free replay + history pagination + token-gated 404) |

The stdin-prompt fix (`ef8a365`, feed CLI-bridge prompts via stdin to avoid `ARG_MAX`) is
included on this branch — it's the prior commit this feature builds on.
