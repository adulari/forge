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
| `phone-apns` | an iOS device registered for native push | Public relay: the text, sealed end-to-end to the device and opened by the app's notification extension, so the relay and Apple only carry ciphertext (ADR-0012 addendum); a device without a key, or with the app lock on, shows a static generic alert. Own APNs key or private relay: the text. |
| `phone-anywhere` | host enrolled and `[anywhere] enabled` | Generic `attention_required` alert only (no ciphertext; the Anywhere service is not the APNs relay). |
| `web-push` | a browser/PWA subscribed to `forge serve` | The title and message (self-hosted, RFC 8291 encrypted to the subscriber). |
| `desktop` | the daemon host has a notifier (`notify-send`, `osascript`) and a session | The title and message; `high` is urgent/critical. |

Hosted services never receive the message **in readable form** (docs/features/forge-anywhere.md,
"Generic push"). The real text is also kept in the daemon's in-memory inbox (last 50), read with
`GET /api/notices?after=<seq>`; over Anywhere that route (`list_notices`) travels inside the
end-to-end encrypted bridge.

### What each lock screen shows

| State | iPhone lock screen |
| --- | --- |
| App build with the notification extension, key registered, public relay | The real title and message, decrypted on the phone |
| Same, app killed or suspended | The same: the extension runs for every push with `mutable-content`, no app process needed |
| App lock on in Forge settings | Generic "Forge: Open Forge to view an update." (no key is registered), real text once unlocked |
| Older app build, or an unsigned sideload without the keychain group | Generic alert; the app replaces it with a local notification the next time it runs |
| Blob damaged, wrong key, extension out of time | Generic alert (the extension never blocks or drops a push) |
| Own APNs key or private relay | The text, in the alert itself |

### Sealed alerts

The app generates a P-256 key and a 16-byte auth secret, keeps them in the keychain group shared
with its Notification Service Extension, and registers the public half in
`POST /api/push/subscribe` as `keys: {p256dh, auth}` next to the APNs token. The daemon seals
`{title, message, session_id, seq, kind}` (at most 1500 bytes of JSON; a long message is shortened
on a character boundary) with RFC 8291 and sends it as the opaque `enc` field of the APNs payload,
beside the generic alert and `mutable-content: 1`. The public relay validates `enc` as a bounded
base64url string (3072 characters at most) and forwards it; it cannot read it. The Rust sealer
(`crates/forge-cli/src/apns_e2e.rs`), the TypeScript mirror (`mobile/src/lib/push/pushE2e.ts`) and
the Swift extension (`mobile/targets/notification-service`) are checked against one vector,
`protocol/fixtures/push-notice-vector.json`. Existing installs re-register their key the next time
the app opens; no action is needed.

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
background. On iOS, a push whose key is registered shows the real text on the lock screen through the
extension (above). Otherwise a local notification carries it only if the app process is running, and
the generic alert stays until the app next runs. On Android there is no remote push in this app, so a
fetched notice raises a local notification (channel `forge-sessions`, asking for the Android 13
notification permission once) when the app process is alive but not in front. With the app lock on,
OS surfaces stay generic.
