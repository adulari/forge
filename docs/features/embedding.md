# Feature: embedding Forge — drive a session programmatically over `forge serve`

> **Status (shipped):** the `forge serve` daemon (`crates/forge-cli/src/serve.rs`) is a stable,
> documented embed surface. An external consumer — an IDE extension, an editor plugin, another
> app — can create and drive Forge sessions over plain HTTP + one WebSocket, with no in-process
> coupling to Forge's TUI. This document specifies that surface as a **stable embed API**.

The daemon is the same control plane the installable web PWA uses, so everything the browser page
does, an embedder can do. There is **no separate SDK**: the contract is the HTTP/WS wire below.

## 1. Start the daemon

```
forge serve            # LAN: binds 0.0.0.0, self-signed HTTPS
forge serve --local    # loopback, plain HTTP (easiest to embed against)
forge serve --anywhere # loopback + a public cloudflared/ngrok tunnel
```

On start it prints the connect URL, which contains the daemon token (see §2):

```
⚒ forge serve — multi-session daemon
  listening on 127.0.0.1:7420 (stable port; sessions survive disconnects)
  connect: http://127.0.0.1:7420/<token>
```

The port is stable (`[remote] port`, default **7420**) and sessions keep running with **zero
clients attached**, so an embedder can disconnect and reattach freely.

## 2. Authentication — the daemon token

Auth is a single **bearer capability**: a long-lived hex token that is a **path prefix** on every
route, not a header. A request to the wrong prefix is a `404` (the token is the only gate — treat
it like a password, especially under `--anywhere`).

- The token is persisted (minted once, reused forever so the PWA origin is stable) at
  `<config_dir>/serve-token`, created **`0600`** (owner-only). `<config_dir>` is the platform
  Forge config dir (e.g. `~/.config/forge` on Linux).
- Present it by prefixing every path with `/<token>`. All routes below are written relative to
  that base — e.g. `GET /<token>/api/sessions`.
- Rotate/revoke with `forge serve --rotate-token` (invalidates every installed PWA/embedder link).

An embedder that runs on the same machine can read the token file directly (respecting the `0600`
permission); a remote embedder must be handed the token out of band.

## 3. Create a session — `POST /api/sessions`

```jsonc
// POST /<token>/api/sessions   (content-type: application/json)
{
  "cwd":      "/abs/path/to/project", // optional; defaults to the daemon's cwd
  "worktree": false,                  // true = run in an isolated git worktree branched from HEAD
  "title":    "my embed session",     // optional display title
  "model":    "anthropic/claude-...", // optional model pin
  "resume":   "<session-id>"          // optional: reattach a persisted session instead of new
}
```

Response:

```json
{ "id": "<session-id>", "title": "…", "cwd": "…", "worktree": "…|null" }
```

Errors are JSON `{ "error": "…" }` with an appropriate status (e.g. `400` for a non-directory
`cwd`). Use the returned `id` on the WebSocket and history routes.

To **resume** a session from a previous daemon run, first list resumable sessions with
`GET /api/sessions/past` (§7), then `POST /api/sessions {"resume": id}`; the session's stored cwd
is used automatically.

## 4. The live stream — `WS /ws?session=<id>&rev=<n>`

Open one WebSocket per attached session:

```
ws(s)://<host>/<token>/ws?session=<id>&rev=<n>
```

- `session` — the session id from §3.
- `rev` — the last snapshot `revision` you rendered; the server **replays exactly the frames you
  missed** from that revision (gap-free reconnect). Use `rev=0` for a full resync on a cold start.

**Server → client:** each message is one JSON `Snapshot` — the entire renderable session state,
re-sent whenever it changes. Deduplicate on `revision` (monotonic; a frame with `revision <=` your
last is a replay overlap — drop it unless `resync` or `closed` is set). Key fields
(`crates/forge-cli/src/remote.rs`, `PROTOCOL_VERSION = 7`):

| field | meaning |
|---|---|
| `protocol` | wire version; warn/refresh on a mismatch with your embedded constant |
| `session_id`, `title`, `cwd`, `worktree` | identity / context |
| `busy` | a turn is in flight |
| `streaming` | trailing edge of the in-flight reply (plain text, re-sent each frame) |
| `transcript` | recent finalized scrollback lines (short tail; use §6 for full history) |
| `tasks`, `subagents` | live task list + running spawned agents |
| `model`, `tier`, `cost_usd`, `context_tokens`, `context_limit` | routing + spend + context gauge |
| `permission_prompt` | non-null ⇒ the turn is blocked on a y/n permission decision (§5) |
| `question`, `question_options`, `question_allow_other` | a blocking AskUserQuestion |
| `prompt_seq` | identity of the pending prompt/question — echo it back when answering (§5) |
| `plan`, `diff`, `overlay`, `copy_text`, `notes` | plan card, diff card, open modal, `/copy`, notices |
| `revision`, `resync`, `closed` | stream cursor; `closed:true` ⇒ session ended, stop reconnecting |

**Client → server:** send JSON `RemoteInput` messages (tagged by `kind`, snake_case). The ones an
embedder needs:

```jsonc
{ "kind": "prompt", "text": "add a test for foo" }   // submit a prompt or a /command
{ "kind": "interrupt" }                               // Esc-while-busy: stop the current turn
{ "kind": "allow",  "yes": true,  "seq": <prompt_seq> } // answer a permission prompt (§5)
{ "kind": "answer", "text": "1",   "seq": <prompt_seq> } // answer a question (a number picks an option)
```

(There are more `kind`s — `key`, `overlay_*` — for driving TUI pickers/overlays; embedders rarely
need them.)

## 5. Permission prompts — over the WebSocket or over plain HTTP

When a turn needs approval, the snapshot carries `permission_prompt` (non-null) and a `prompt_seq`.
Answer it either:

- **On the WS:** `{ "kind":"allow", "yes":true|false, "seq":<prompt_seq> }`, or
- **Without the WS:** `POST /api/answer` — for a background/service-worker style responder:

```jsonc
// POST /<token>/api/answer
{ "session": "<id>", "seq": <prompt_seq>, "allow": true }
```

`seq` **must** echo the snapshot's current `prompt_seq`. A stale/unknown seq is a `409` no-op and
the driver re-validates on receipt, so a tap that races a prompt swap can never approve the newer
(possibly more dangerous) prompt. Unknown session ⇒ `404`.

## 6. Scrollback — `GET /api/history`

The live `transcript` is only a short tail. For full scrollback paginate:

```
GET /<token>/api/history?session=<id>&before=<seq>&limit=<n>
```

Returns rows **newest-first**; page backwards by passing the oldest `seq` you have as `before`.
Each row: `{ seq, role, content, model, created_at, visibility }` where `visibility` is `"llm"`
for normal turns and `"ui"` for user-facing notes (still part of the visible conversation). `limit`
is clamped server-side.

## 7. Managing sessions

- `GET /api/sessions` — sessions the daemon is **currently running** (fleet dashboard data: id,
  title, cwd, worktree, `busy`, `waiting`, `cost_usd`, `context_tokens`, `model`, activity). Rows
  needing a human decision (`waiting`) sort first.
- `GET /api/sessions/past?limit=&before=` — persisted top-level sessions that are **not** currently
  running, most-recently-used first: the set an embedder can offer to **resume** (§3). `before` is a
  `last_activity` unix-seconds cursor for pagination. Each row: `{ id, title, cwd, worktree,
  archived, message_count, cost_usd, last_activity, created_at, preview }`.
- `POST /api/sessions/{id}/archive` — stop the driver and hide the session (history + worktree kept).
- `POST /api/sessions/{id}/merge` — for a worktree session: stop it, snapshot its uncommitted edits
  onto its branch, and 3-way-merge that branch back into the base repo. On success the changes are
  **staged** (not committed) in the base tree; the worktree + branch are removed. Refuses (`409`)
  if the base tree has uncommitted tracked changes, and on a merge conflict returns `409` with a
  `conflicts` file list, leaving the worktree + branch intact for manual resolution — it never
  auto-resolves.
- `POST /api/sessions/{id}/discard` — for a worktree session: stop it and drop the worktree +
  branch **without** merging. Destructive (force-deletes the branch), so an embedder MUST confirm
  with the user before calling it.

## 8. File upload — `POST /api/upload?session=<id>`

Multipart file/image upload; stored in the session's `.forge/uploads/<id>/` scratch area and
attached to the next turn (images as vision input, text files as an `@path` mention).

## 9. Notes for embedders

- **One WS per session.** Sessions are independent; drive several concurrently over separate sockets.
- **Reconnect with `rev`.** Persist the last `revision` you rendered; reconnecting with it yields a
  gap-free replay. Use `rev=0` only on a cold start with no rendered state.
- **All responses are JSON** with `Cache-Control: no-store`; errors are `{ "error": "…" }`.
- **The protocol is versioned** (`PROTOCOL_VERSION`). Pin the version you built against and handle a
  `protocol` mismatch by prompting the user to update, exactly as the PWA does.
- **No embed-only endpoint was added** for this contract — it documents the daemon's existing
  surface. The only new routes are the session-management ones in §7 (`past`, `merge`, `discard`),
  which are equally useful to the PWA.
