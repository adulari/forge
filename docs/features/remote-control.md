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
> protocol (`PROTOCOL_VERSION`); the page shows a "refresh to update" banner on a mismatch.
> Auto-start is configurable (`[remote] auto`). The server reuses the running session's
> presenter channel — no second process, no IPC, no keys to configure.
>
> **Deferred:** image/file attachments from the phone, true push notifications while the app
> is fully closed (live notifications fire while the page/PWA is open in the background), and
> in-page session switching (one server drives the session that started it).

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
`{kind:"allow",yes,seq}`, `{kind:"answer",text,seq}`, `{kind:"interrupt"}`) over the WS. A
prompt can be a plain task, a `/command`, or a `//`-escaped hook command — all routed through
the *same* `dispatch_command` + prompt-hook + `spawn_turn` paths a local keystroke takes, so
slash commands (`/plan`, `/compact`, `/diff`, `/model`, …), the busy guard, the permission
gate, temper, and hooks all apply unchanged. Prompts sent while a turn is running are
**queued** to run after it, exactly like local typing. `allow` answers a pending permission,
`answer` resolves an `AskUserQuestion` (a tapped option button sends its 1-based index), and
`interrupt` aborts the turn task. `allow`/`answer` must echo the `prompt_seq` their buttons
were rendered from — a mismatch (the prompt changed under the tap) is ignored, so a stale
answer can never resolve a newer prompt. A remote prompt is indistinguishable from a local
one. Inbound frames over `MAX_INPUT_BYTES` (256 KiB) are dropped to bound a hostile client.

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
  same-origin `Content-Security-Policy` (inline script/style allowed until the asset split),
  and `Referrer-Policy: no-referrer` so the token-bearing URL never leaks via the Referer
  header.
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

## 3b. PWA lifetime (interim)

Each session gets a fresh port + token, so an installed home-screen app outlives its
session's origin. When the server is gone the service worker now answers navigations with an
explicit **"session ended — reopen `/remote` from the TUI"** page (and the live page stops
claiming "reconnecting…" after ~12s of failures) instead of a stale shell that spins
forever. A stable origin that makes installs permanent is the Phase-4 daemon's job.

## 4. Surfaces touched

| Layer | Change |
|---|---|
| `forge-cli/src/remote.rs` | Server, `Snapshot`/`RemoteInput` types + `PROTOCOL_VERSION`, the mobile control page (tabs/panels/option taps/chips), PWA manifest + service worker + icon, TLS, tunnels, QR renderer, `MAX_INPUT_BYTES` cap, `Exposure: From<RemoteAuto>` |
| `forge-config/src/lib.rs` | `[remote]` block: `RemoteConfig` (`auto`, `host`) + `RemoteAuto` + `startup_exposure()` |
| `forge-tui/src/app.rs` | `App.remote_active`, `question_prompt`, `recent_transcript` ring, `drain_flush_remote`, `remote_snapshot` (now incl. tasks/subagents/queued/question options), `print_lines`, statusline `◉ remote` segment |
| `forge-tui/src/commands.rs` | `CommandAction::Remote { mode }`, `/remote` (alias `/rc`) parse + registry entry |
| `forge-cli/src/cli/commands/run.rs` | `DispatchOutcome::ToggleRemote`, `toggle_remote`, `[remote] auto` startup, remote input draining + full-state snapshot broadcast in `run_chat_tui` |
| `Cargo.toml` | `axum` (ws), `axum-server` (rustls), `rcgen`, `tokio-tungstenite`, `qrcode`; `tokio` `net` feature |
| `forge-cli/src/remote.rs` (tests) | snapshot wire-shape, manifest/base/SW/exposure-mapping units + the `--ignored` real-socket page + WS + PWA round-trip |

The stdin-prompt fix (`ef8a365`, feed CLI-bridge prompts via stdin to avoid `ARG_MAX`) is
included on this branch — it's the prior commit this feature builds on.
