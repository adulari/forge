# RFC: Anywhere wire v2 and device revoke by any device

Status: sections 1 and 2 implemented and deployed (service 3b20168). Section 3 ("Make owner")
implemented (forge `feat/anywhere-make-owner`, forge-anywhere-service `feat/make-owner`); its
service deploy is pending owner approval.

Two changes (plus a follow-up, section 3) to Forge Anywhere that ship together because they touch the same three components
(service, daemon/connector, app):

1. **Wire v2**: byte fields inside sealed relay payloads travel as base64url instead of JSON number
   arrays, negotiated per peer so nothing older breaks.
2. **Device revoke**: any active device of an account may revoke another; only the owner device
   may revoke the owner device.

## 1. Wire v2

### Measured

Real payloads from an isolated `forge serve --mock` daemon (2.29.0), wrapped exactly as the
connector wraps them, sealed size = plaintext + 185 bytes of envelope (105 header, 16 tag, 64
signature). The measuring script is in the forge PR description.

| shape | raw body | v1 sealed | v2 sealed | v1/v2 | deflate(raw) |
|---|---:|---:|---:|---:|---:|
| `GET /api/sessions` | 706 | 2,826 | 1,250 | 2.26x | 422 |
| `GET /api/history` (session) | 589 | 2,420 | 1,094 | 2.21x | 263 |
| `GET /api/models` | 9,975 | 36,039 | 13,606 | 2.65x | 1,022 |
| `GET /api/config` | 67,786 | 241,805 | 90,690 | 2.67x | 11,984 |
| WS snapshot frame | 4,779 | 17,582 | 6,667 | 2.64x | 1,516 |
| WS keepalive frame | 18 | 396 | 319 | 1.24x | 26 |
| sync record (message row) | 188 | 1,178 | 675 | 1.75x | 142 |
| 64 KiB random (voice/upload) | 65,536 | 234,495 | 87,690 | 2.67x | 65,562 |

v1 costs 3.13x to 3.57x the raw body; v2 brings any body over a few KiB to ~1.34x. The 67 KiB
config response sealed to 236 KiB in v1, 79% of the relay's 300 KiB envelope cap; in v2 it is 89 KiB.

### Why not compression

Deflate would shrink JSON a further 4-8x (last column), and `flate2` is already in `Cargo.lock`. The
app is the peer that matters, though: Hermes has no `CompressionStream`, `mobile/package.json` has
no inflate implementation (no `pako`, `fflate`), and this change may not add npm dependencies.
Compression only between daemons would add a second negotiated axis for traffic that is a small
share of relay bytes. Base64url only; compression is a candidate wire v3 if the app gains an
inflater. Compressing before encryption also needs a length-leak review (CRIME-style) that v2
does not.

### What changes and what does not

- The `FANY` envelope is unchanged: same version byte (1), header, AAD, XChaCha20-Poly1305, Ed25519
  signature, sequence and replay rules. Only the JSON plaintext inside gets smaller. A test seals
  the same response in both versions and checks the envelope parses and opens identically.
- v2 encodes `request_id`, `stream_id`, `body` and `bytes` as **unpadded base64url** (the encoding
  the rest of the Anywhere API already uses for hashes and keys). Everything else in the JSON is
  byte-for-byte what v1 writes.
- **Every reader accepts both forms** for every byte field (`forge_anywhere_protocol::wire::{bytes,
  fixed, optional_fixed}`, `mobile/src/lib/transport/anywhereWire.ts`). Malformed values fail
  closed; non-canonical base64 (non-zero trailing bits) is rejected on both sides alike.
- The service never decrypts a payload, so it needs no change and no advertisement for wire v2.
  Relay limits (300 KiB envelope, 256 KiB blob minimum, 32 MiB blob maximum) are unchanged.

### Negotiation

The existing handshake for a bridge exchange is the request/response pair, so negotiation rides
on it:

- Every bridge request and response carries `"wire": N`, the highest version its sender reads
  (this build: 2). The bridge types never denied unknown fields, so released daemons and the
  App Store app ignore it.
- A **host** answers a request in v2 only when that request advertised `wire >= 2`. A stream
  opened by such a request sends its frames in v2 too (stored on the stream handle).
- A **controller** (app, or a daemon calling another host) sends v1 until an authenticated
  response from that host advertised v2, then v2. The app forgets what it learned when its relay
  socket closes, so a host downgraded meanwhile is relearned as v1.
- With v2 negotiated, a sender keeps bodies up to 192 KiB inline (256 KiB encoded) instead of
  64 KiB, so 64-192 KiB bodies (config, models, history pages, short voice clips) no longer take
  the padded-blob round trip (reserve, PUT, complete, claim, GET, DELETE).

Durable commands and sync records are stored by the service and read later by peers nobody
negotiated with, so their **writers stay on v1**. Their readers accept v2 already, which lets a
later release switch the writers once old builds are gone. Durable commands are additionally
checked to be canonical v1 JSON on the host.

### Compatibility (all covered by tests)

| controller | host | result |
|---|---|---|
| App Store app (no `wire`) | new daemon | host answers in v1 arrays, plus an ignored `wire` field (`an_app_store_build_that_says_nothing_gets_the_v1_form_it_parses`) |
| new app | released daemon (no `wire`) | app never upgrades, sends v1 forever (`never sends v2 to a host that does not advertise it`) |
| new app | new daemon | first request v1 + advert, then v2 both ways (`upgrades to v2 only after the host advertises it`) |
| new daemon controller | any host | requests v1 + advert, reads either answer (`controller.rs` test) |
| any | prod service (Jul 25 build) | unaffected: it only sees ciphertext |

## 2. Device revoke

### What was there

`POST /v1/devices/{id}/revoke` already rotated the account key atomically, and `forge anywhere
devices --revoke`, the TUI and the app already called it. The service checked only that the
caller was an active, enrolled device of the account: **any device could revoke any device,
including the account's first device**, and nothing marked an owner. ("Owner-only by design" was
not enforced anywhere in the service or the clients.) Revocation trusts the caller's new recovery
wrap, which the service cannot open, so a single compromised secondary device could revoke every
other device and replace the recovery wrap with garbage.

### Rule

- **Owner device**: the account's oldest active, key-enrolled device (`ORDER BY created_at, rowid`;
  normally the device that ran `forge anywhere setup`), unless the owner chose another device
  (section 3).
- Any active, enrolled device of the account may revoke any other device of that account.
- Only the owner device may revoke the owner device. A non-owner gets `403
  owner_device_protected`.
- The owner may revoke itself; ownership passes to the next oldest active device.
- A revoked device can do nothing (its access and refresh tokens die in the same transaction, and
  every request joins `device.revoked_at IS NULL`), so it cannot revoke anyone. Nothing un-revokes:
  there is no route, and revoking an already-revoked device is `404 device_not_found` with
  `revoked_at` untouched. A revoked device must pair again as a new device.
- Devices of another account are invisible (`404`).
- Every decision is audited in `security_audit_event` as `device_revoked` with outcome `success`
  (`target_device_id`, `target_was_owner`, `self_revoke`, `epoch`) or `denied`
  (`reason: owner_device_protected`), inside the same transaction as the decision.
- `GET /v1/devices` gains `owner: bool` per row; `GET /v1/capabilities` gains
  `features.owner_device_protection`.

A lost **owner** device cannot be revoked by the others; that is the point of the rule. Its
access still ends on its own (below), and the account stays recoverable through the existing
seven-day clean reset, which any active device can schedule. Known limit: any active device can
also cancel a scheduled reset, so someone actively using a stolen owner device can hold off a
reset; the owner rule trades that case for protection against a compromised secondary device.

### Token expiry

Already in place, so no change: access tokens live 15 minutes; refresh tokens live 30 days and
rotate on every use (each refresh issues a new token with a fresh 30-day window, a reused one
revokes the whole login family). A phone that is lost and not used loses access 30 days after its
last refresh. New tests pin both the sliding window and the lapse.

### Surfaces

- **CLI**: `forge anywhere devices` tags `[this device, owner]`; `--revoke` refuses the owner
  device from a non-owner before asking for the Recovery Kit.
- **TUI** (`/anywhere`): device rows show `owner`; Enter or `x` opens a confirmation (Cancel
  focused), then hands the terminal to `forge anywhere devices --revoke <id>` for the Recovery
  Kit prompt. This device and a protected owner explain why instead of offering it.
- **Phone, desktop and web** (one Expo codebase): the Devices page and the Anywhere hub show an
  OWNER badge; the revoke button is hidden where the service would refuse, with a one-line
  reason. Against a service without `owner`, every client behaves as before and the service
  decides.

## 3. Choosing the owner ("Make owner")

The implicit rule made the browser app owner on accounts where it was paired first. The owner
device can now hand ownership to another active device.

### Service

- `account_owner (account_id PK, device_id, updated_at)` stores the explicit choice. The effective
  owner is that device while it is active and key-enrolled; otherwise (no row, or a row whose
  device was revoked) the oldest-active rule applies. Revoke protection uses the effective owner.
- `POST /v1/devices/{id}/make-owner` (bearer auth, no body): only the current owner device may
  call it (`403 owner_required` otherwise); the target must be an active, key-enrolled device of
  the same account (`404 device_not_found`). Returns the updated device list (same shape as
  `GET /v1/devices`). Making yourself the owner again is a no-op. Idempotent by nature, so no
  idempotency key.
- Every decision except not-found is audited in `security_audit_event` as
  `device_owner_transferred`: `success` (`previous_owner_device_id`, `target_device_id`) or
  `denied` (`reason: owner_required`).
- An owner that revokes itself clears the row, so ownership still passes to the oldest remaining
  device.
- `GET /v1/capabilities` gains `features.owner_transfer: true`.

### Migration and rollback

The service refuses to start on a database whose `PRAGMA user_version` is newer than it knows, so
a normal versioned migration would make the previous binary unable to roll back. The table is
therefore created by an **additive step outside the `user_version` ladder**: on open, inside an
immediate transaction, if `account_owner` does not exist it is created and backfilled with each
account's current implicit owner (so nothing changes for anyone). `user_version` stays 12. The
previous binary starts on the migrated database and ignores the table; while rolled back it
applies the oldest-active rule, and a row it leaves stale (its device revoked) is ignored by the
new binary. Verified by running the previous release binary against a migrated copy.

### Clients

All gated on `features.owner_transfer` and on this device being the owner; against an older
service nothing is offered and nothing is called.

- **CLI**: `forge anywhere devices --make-owner <id>` asks `[y/N]` (`--yes` skips it), checks the
  capability and the rule, posts, and confirms the new owner from the response.
- **TUI** (`/anywhere`): `m` on another device opens a confirmation (Cancel focused); the footer
  shows `m make owner` only where it applies, and a non-owner or an older service gets a one-line
  reason instead.
- **Phone, desktop and web**: a crown action on qualifying rows of the Devices page opens an inline
  confirmation; TypeScript only, so it ships OTA.

## Rollout

Each step is safe on its own; nothing requires a later step to keep working.

1. **Service** (`forge-anywhere-service` PR, needs owner approval to deploy to `forge`):
   deploy; the owner rule and audit take effect, `owner` appears in `GET /v1/devices`. Wire v2
   needs nothing from the service. Check after deploy: `GET /v1/capabilities` shows
   `owner_device_protection: true`; `GET /v1/devices` shows exactly one `owner: true`.
2. **Daemon**: the next Forge release carries the connector, controller and CLI/TUI changes.
   Hosts answer v2 to controllers that ask; they keep answering v1 to the App Store app.
3. **App**: every change is TypeScript (no native module, no `package.json`, no `app.config.ts`),
   so iOS and Android take it as an **OTA update**; desktop takes it with its usual OTA/release.
   No native build is required.
4. **Later**: once telemetry shows no pre-v2 readers, switch the sync-record and durable-command
   writers to v2 (their readers already accept it).

Make owner follows the same order: deploy the service (check `owner_transfer: true` and exactly
one `owner: true`), then clients offer the action as they update. Rolling the service back hides
the action again (the capability disappears) and restores the oldest-active rule.

Rollback: reverting the service restores the old revoke rule; clients keep working (the `owner`
field is optional everywhere). Reverting a daemon or app falls back to v1 by negotiation.
