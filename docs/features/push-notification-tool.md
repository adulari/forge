# `push_notification` — let the agent tell you

A model-callable tool, available on the direct-API path and the claude-cli/codex bridge
(`forge mcp-serve`), for "the build finished" or "I need your decision". Arguments: `message`
(required, one short sentence, trimmed to 240 characters), `title` (optional, 60), `priority`
(`normal` | `high`; `urgency` is accepted as an alias).

## Delivery

Every channel that is available is tried; the tool result lists which ones delivered. Nothing
configured returns a "no channel available, configure X" result and never fails the turn.

| Channel | When | What it shows |
| --- | --- | --- |
| `phone-apns` | an iOS device registered for native push | Public relay: a static generic alert (ADR-0012). Own APNs key or private relay: the text. |
| `phone-anywhere` | host enrolled and `[anywhere] enabled` | Generic `attention_required` alert only. |
| `web-push` | a browser/PWA subscribed to `forge serve` | The title and message (self-hosted, RFC 8291 encrypted to the subscriber). |
| `desktop` | the daemon host has a notifier (`notify-send`, `osascript`) and a session | The title and message; `high` is urgent/critical. |

Hosted services never receive the message (docs/features/forge-anywhere.md, "Generic push"). The
real text is also kept in the daemon's in-memory inbox (last 50), read with
`GET /api/notices?after=<seq>`; over Anywhere that route (`list_notices`) travels inside the
end-to-end encrypted bridge. So a locked iPhone shows "Forge needs you" and the app shows the
text once opened and authenticated. **The mobile app does not read the inbox yet**; that is the
remaining follow-up (fetch on open and on a push tap, render a banner).

## Routing

`forge serve` delivers in-process. Any other process (`forge run`, the TUI, the bridge child) posts
to the running daemon's `POST /api/notify` and falls back to a desktop notification when none
answers. The Tauri desktop app has no event channel for this yet; it relies on web push and the
inbox.

## Limits and permission

Per session: 4 per minute, 20 per hour, identical message suppressed for 10 minutes. The caller
enforces it, and the daemon enforces it again keyed by session so a bridge child (a fresh process
per turn) cannot loop around it. Refused calls are not counted, so a loop does not extend its own
penalty.

Classified as a network side effect to the user's own devices: refused in Plan mode, allowed in
Auto, AcceptEdits and Bypass without a prompt. Guarded (`default`) would ask for a network call, but
a virtual tool has no prompt to show, so it proceeds there under the limiter.

On a lean bridge it is deferred behind `tool_search`, so it costs no advertised schema.

## In the app

The app (`mobile/src/lib/notices/`) reads `GET /api/notices?after=<seq>` (Anywhere: bridge route
`list_notices`) for every paired host: at launch, on foreground, when a hosted push is received or
tapped, and every 30 s for the active host while the app is in front. It keeps the held list in
memory and persists only two marks per host (last alerted, last seen) in AsyncStorage, so a
relaunch neither replays banners nor forgets what was read. Marks compare `seq` and `at`, and a
`latest` below the cursor means the daemon restarted (the log is in memory) so the log is re-read
from the start. Notices older than 15 minutes when first fetched are listed but not announced.

New notices raise a banner (inside the app lock) and fill **Inbox > Notices**; tapping opens the
notice's session. On web and desktop an OS notification carries the text when the window is in the
background. On iOS a local notification carries it only if the app process is running; a generic
push that arrives while the app is suspended or killed is drawn by iOS from the APNs payload alone,
so the lock screen keeps the generic text until the app next runs. Replacing it earlier needs a
`remote-notification` background mode or a Notification Service Extension, both native. With the
app lock on, OS surfaces stay generic.
