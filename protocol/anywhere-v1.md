# Forge Anywhere protocol v1

Status: draft implementation contract. All multi-byte integers are unsigned big-endian. IDs are
opaque 16-byte values. Implementations must treat unrecognized versions, kinds, recipient kinds,
lengths, and trailing bytes as errors.

## Binary envelope

Every encrypted relay frame and R2 object uses one canonical envelope:

| Offset | Size | Field | v1 meaning |
| ---: | ---: | --- | --- |
| 0 | 4 | `magic` | ASCII `FANY` |
| 4 | 1 | `version` | `1` |
| 5 | 1 | `kind` | Object kind below |
| 6 | 2 | `flags` | Bit field; currently zero unless a route defines it |
| 8 | 16 | `account_id` | Routing account |
| 24 | 16 | `sender_device_id` | Signing device and replay namespace |
| 40 | 1 | `recipient_kind` | `1=device`, `2=host`, `3=account`, `4=share` |
| 41 | 16 | `recipient_id` | Opaque target identifier |
| 57 | 4 | `key_epoch` | Account Data Key epoch |
| 61 | 8 | `sequence` | Strictly increasing sender/epoch sequence |
| 69 | 8 | `created_at_ms` | Unix epoch milliseconds |
| 77 | 24 | `nonce` | XChaCha20 nonce |
| 101 | 4 | `ciphertext_len` | Ciphertext plus 16-byte Poly1305 tag |
| 105 | variable | `ciphertext` | XChaCha20-Poly1305 output |
| variable | 64 | `signature` | Ed25519 signature |

The 105-byte fixed header, including `ciphertext_len`, is XChaCha20-Poly1305 AAD. The Ed25519
signature input is exactly `header || ciphertext`. Receivers parse strict lengths, resolve the
sender's active signing key, verify the signature, and only then decrypt. The hosted service may
verify signatures and inspect header routing metadata but never receives payload keys.

Kinds are `1=bridge_request`, `2=bridge_response`, `3=websocket_frame`, `4=sync_record`,
`5=key_wrap`, `6=capsule`, `7=share`, `8=blob`, `9=command`, and `10=acknowledgement`.

Bridge plaintext is canonical JSON matching the public Rust/TypeScript types. A bridge request
contains a route ID, redundant HTTP method, route-relative parameters, allowlisted headers, and
body bytes. The host rejects a method that does not match the route ID and never accepts a URL.
A `web_socket` bridge request opens one `/ws` stream using `request_id` as its stream ID and
parameters `[session_id, revision]`; subsequent `websocket_frame` envelopes carry opaque data or a
close marker.

`bridge_request` and `bridge_response` JSON have an optional `body_blob`; `websocket_frame` has an
optional `bytes_blob`. A relay blob reference is
`{"blob_id":"<32 lowercase hex>","ciphertext_bytes":u64,"ciphertext_sha256":"<43 unpadded base64url>"}`.
The inline byte field
defaults to an empty array, an absent reference defaults to `null`, and serializers omit absent
references. A message must not set both an externalized byte field and its reference. Inline
messages at or below 256 KiB retain the original representation.

`(sender_device_id, key_epoch, sequence)` is globally unique within an account. Clients persist the
next sequence before transmission. The service atomically rejects a repeated tuple. Clients also
generate a fresh uniformly random 24-byte nonce for every encryption; a sequence does not replace
nonce uniqueness. Relay payloads above 256 KiB are put in a temporary encrypted blob and the frame
contains only its encrypted object reference.

## Managed relay binding

An authenticated host requests a 60-second, single-use ticket with
`POST /v1/relay/tickets {"host_id":"…"}` and connects to
`GET /v1/relay?ticket=…` over WebSocket. Data messages are binary envelopes only. The ticket is an
opaque bearer value and must not be logged. Before accepting controller traffic, the host loads the
account's active device IDs and Ed25519 public keys from `GET /v1/devices`; an unknown or revoked
sender is rejected even when the envelope is otherwise well formed. The service routes by envelope
header and never rewrites envelope bytes.

### Temporary relay blobs

Temporary relay blobs use the following authenticated service contract. IDs are lowercase hex,
hashes are unpadded base64url, and `recipient_kind` is `device` or `host`.

- `POST /v1/relay/blobs` with `Idempotency-Key` and JSON
  `{recipient_kind,recipient_id,ciphertext_bytes,ciphertext_sha256}` reserves an upload and returns
  `{blob_id,upload_url,required_headers}`. The client uploads the exact encrypted object to
  `upload_url`, including every returned required header.
- `POST /v1/relay/blobs/{blob_id}/complete` with `Idempotency-Key` verifies the uploaded object and
  makes it claimable. Retrying a completed reservation returns `already_complete=true` without a
  new upload URL; deleted or expired reservations can never be reopened.
- `GET /v1/relay/blobs/{blob_id}` claims the object and returns
  `{download_url,ciphertext_bytes,ciphertext_sha256,required_headers}` to its authenticated
  recipient. The client includes every returned required header.
- `DELETE /v1/relay/blobs/{blob_id}` with `Idempotency-Key` consumes a claimed object.

The stored object is one complete signed `kind=8` FANY envelope. Its plaintext is exactly the
externalized HTTP body or WebSocket frame bytes, without JSON wrapping, credentials, or daemon
tokens. Its envelope recipient must equal the destination of the referencing control envelope. A
sender seals and reserves the blob before sealing its referencing control envelope, so the blob
sequence immediately precedes (but need not be adjacent to) the reference sequence.

Receivers cap the complete encrypted object at 32 MiB. Before local dispatch they compare claim
metadata with the typed reference, stream-download with the declared-length cap, verify exact
length and SHA-256, decode `kind=8`, bind account/sender/recipient/key epoch to the referencing
message, verify the active sender signature, authenticate and decrypt, and atomically advance replay
state across the blob and reference sequences. They attempt to consume the blob only after all
those checks and local delivery succeed. Cleanup failure must not suppress an already accepted
delivery; unconsumed ciphertext expires automatically. A failed or unverified object is never
dispatched or consumed.

### Durable encrypted commands

Durable commands use this authenticated private-service API. Every binary request and response has
`Content-Type: application/octet-stream`; JSON list responses contain metadata only. A
`command_id` is a service-assigned opaque 16-byte value encoded as 32 lowercase hexadecimal
characters in JSON and URL paths.

- `POST /v1/hosts/{host_id}/commands` with `Idempotency-Key` sends one complete signed `kind=9`
  envelope and returns `{version,command_id,expires_at_ms,already_queued}`. The envelope plaintext
  is exactly the canonical JSON encoding of `BridgeRequest`; there is no second command plaintext
  type.
- `GET /v1/hosts/{host_id}/commands` returns `QueuedCommandList`, whose `commands` contain only `command_id`,
  `sender_device_id`, `created_at_ms`, `expires_at_ms`, and `ciphertext_bytes`.
- `GET /v1/hosts/{host_id}/commands/{command_id}` returns the exact stored binary `kind=9` envelope
  to its target host.
- `POST /v1/hosts/{host_id}/commands/{command_id}/ack` with `Idempotency-Key` sends one complete signed
  `kind=10` envelope. Its plaintext is canonical `CommandAcknowledgement` JSON, either
  `{"command_id":"ID","result":{"status":"success"}}` or
  `{"command_id":"ID","result":{"status":"error","code":CODE,"retryable":BOOL}}`, where
  `ID` must equal the path's `command_id`. `CODE` is one of `invalid_command`,
  `permission_denied`, `host_unavailable`, or `execution_failed`. Free-form messages and additional
  fields are invalid.
- `GET /v1/hosts/{host_id}/commands/{command_id}/ack` returns the exact stored binary `kind=10`
  envelope to the device that submitted the command, or `404` while no acknowledgement exists.

The complete encoded command and acknowledgement envelopes are each capped at 256 KiB; commands
cannot use temporary relay blobs. `expires_at_ms` is exactly `created_at_ms + 86,400,000`, and the
command and any acknowledgement expire no later than 24 hours after command creation. Expired
objects return `404` and cannot be recreated under the same `command_id`.

The authenticated posting device must be the active `sender_device_id` in the command envelope,
and the envelope must use `recipient_kind=host` for a host owned by the same account. Only that
authenticated host may list or fetch the command and post its acknowledgement. The acknowledgement
must name the fetched command in its decrypted plaintext, use `recipient_kind=device` with the
original sender device as recipient, and be signed by an active device identity owned by the target
host's account. Only the original sending device may fetch it. Cross-account IDs and ownership
mismatches return `404` rather than revealing existence.

Both POST routes are idempotent: retrying the same account, authenticated principal, route, and
`Idempotency-Key` with byte-identical envelope data returns the original result; reusing that key
with different bytes is a conflict. A command has at most one immutable acknowledgement, so a later
different acknowledgement is also a conflict. The service atomically applies the envelope replay
rule `(account_id, sender_device_id, key_epoch, sequence)` before storing either kind. Clients still
persist sequence state and treat a fetched command or acknowledgement they have already accepted as
a replay rather than dispatching it again.

Creating new commands requires a `trialing`, `active`, or `grace` entitlement. Listing and fetching
already queued commands, posting their acknowledgements, and fetching existing acknowledgements
remain available in read-only entitlement states until expiry so accepted work can complete.

The service stores and returns exact signed envelope bytes. It may inspect authenticated identity,
envelope headers, ciphertext size, timestamps, expiry, and service-assigned IDs for routing and
policy enforcement, but it never receives payload keys and cannot read `BridgeRequest`,
`CommandAcknowledgement`, daemon credentials, tokens, filenames, prompt content, or error detail.

## Cryptography and key epochs

- Device exchange: X25519.
- Sender authentication: Ed25519.
- Symmetric encryption: XChaCha20-Poly1305 with a 256-bit key.
- Derivation: HKDF-SHA256.
- Account recovery: a random 256-bit secret encoded as 24 English BIP39 words.

The device-wrap key is HKDF with X25519 shared secret as IKM, `account_id` as salt, and
`"forge-anywhere/v1/device-wrap" || key_epoch:u32` as info. The recovery-wrap key substitutes the
256-bit recovery entropy as IKM and uses
`"forge-anywhere/v1/recovery-wrap" || key_epoch:u32` as info.

Each Account Data Key epoch is independently wrapped to every authorized device and to recovery.
Revocation atomically invalidates the device's tokens, advances the epoch, and publishes wraps only
for remaining devices and recovery. Old epochs remain available for decrypting retained history.
Recovery words are displayed once; uploads remain disabled until sampled-word confirmation passes.

## Encrypted payloads

Bridge control payloads are UTF-8 JSON matching the public Rust/TypeScript types. A `route` is an
enum, never a caller-controlled URL. Route parameters are individual decoded values and the host
connector maps them to a reviewed local-daemon allowlist. The daemon bearer token never leaves the
host. Existing `remote-v8` WebSocket messages are opaque bytes inside `websocket_frame` envelopes.

Sync payloads use stable record IDs. Append-only records are idempotent by stable ID. Mutable
metadata compares `(logical_clock, device_id)` lexicographically. File records carry base and
content hashes; divergent bases create conflict copies. Deletion is a tombstone, never an implicit
absence.

### Sync object transfer

Clients seal one complete `sync_record` envelope before reserving an upload. The exact ciphertext
and its SHA-256 are retained locally until completion so retries never reseal the same revision
under a different nonce. `POST /v1/sync/uploads` requires an idempotency key and the record kind,
stable ID, revision, logical clock, operation, content/base hashes, ciphertext size, and ciphertext
SHA-256. The service transactionally reserves quota before returning a 15-minute SigV4 R2 `PUT`.

After upload, `POST /v1/sync/uploads/{id}/complete` streams and hashes the encrypted object, checks
its exact length and SHA-256, then atomically creates the change cursor and charges quota. A
completed `(account, kind, stable_id, revision)` is immutable; a retry with different metadata or
ciphertext is a conflict. `GET /v1/sync/changes?cursor=N` returns account-scoped metadata and
15-minute download URLs. Downloads and deletion remain available in read-only entitlement states;
new upload reservations require `trialing`, `active`, or `grace`.

Normative deterministic vectors live in [`fixtures/anywhere-v1`](fixtures/anywhere-v1/). Randomized
production values must never reuse fixture keys or nonces.
