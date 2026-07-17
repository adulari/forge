# Forge Anywhere

Forge Anywhere is an optional encrypted companion for individual developers: leave your desk
without leaving your Forge session. Forge itself, local/LAN control, direct pairing, and your own
`forge serve --anywhere` tunnel remain free.

## Availability and price

The managed service costs €10/month or €79/year through Paddle. A 14-day no-card trial starts when
the first host connects and is available once per GitHub account. V1 includes three active hosts,
unlimited personal controller devices, and 5 GB of encrypted cloud storage. There is no permanent
hosted free tier. Annual checkout is the default, but both prices are always shown together.

Managed value includes a stable host identity, encrypted relay, multi-host fleet, encrypted
sync/history, remote jobs, generic push, workspace handoff, and encrypted replay links. If the
service is unavailable or the entitlement expires, local Forge and every direct access path keep
working normally.

## Security and recovery

Forge generates an Account Data Key and device signing/exchange keys locally. The managed service
sees routing metadata and ciphertext, never session or workspace plaintext. Provider credentials,
keyring values, the local daemon token, Lattice indexes/embeddings, caches, build artifacts, and
queue internals are never synced.

The host writes eligible records to a transactional local outbox, encrypts each immutable snapshot
before upload, and retains the exact ciphertext until the service verifies completion. Cloud quota
is reserved before R2 upload and charged only after the service independently verifies ciphertext
size and SHA-256. A service outage leaves journal rows pending without affecting local Forge.

The current client verifies and decrypts downloaded changes into a durable local staging table.
Memory records are then applied in bounded transactions using deterministic
`(logical_clock, device_id)` ordering; tombstones are honored, losing versions remain recorded as
superseded, untracked pre-Anywhere memories are never overwritten, and device-local embeddings are
preserved. For sessions already present on the host, portable title/archive/view metadata and
append-only messages, checkpoints, tool calls, routing decisions, and usage are applied in dependency
order. Remote workspace paths, worktrees, and permission modes never replace host-local values;
missing parents remain staged, while identity or sequence collisions become durable conflicts.
Compaction summaries and tombstones use the same deterministic ordering and only toggle rows marked
by compaction, preserving unrelated `/undo` state. Entirely remote sessions, settings, and files are
still staged rather than applied. This boundary prevents incomplete workspace and file-conflict
policies from mutating local history, so this build must not yet be described as full two-way sync.

Initial setup shows a 24-word recovery phrase exactly once and asks for sampled words before any
upload. Store it offline. Adding a device uses a short-lived QR challenge. Revoking a device also
rotates the Account Data Key epoch, so the removed device cannot decrypt future data. Losing every
device and the recovery phrase is unrecoverable by Forge support.

The wire contract is documented in [`protocol/anywhere-v1.md`](../../protocol/anywhere-v1.md) and
the trust assumptions in the adjacent threat model.

## Handoff safety

Handoff starts only at an idle checkpoint. The source packages session records, repository base,
a binary full-index diff, and safe untracked files. Forge reports and aborts on secrets, links,
special files, unsafe paths, ignored caches/build output, files above 25 MB, or capsules above
100 MB instead of silently dropping user files. The destination uses an isolated detached worktree
and does not transfer the session lease until import and patch application are acknowledged.

## Retention and troubleshooting

Storage over 5 GB blocks new writes but still permits download and deletion. Consumed capsules and
relay blobs expire within 24 hours; replay shares expire after 24 hours, 7 days, or 30 days. Cloud
data is retained for 90 days after service expiry, with warnings 30 and 7 days before deletion.

When Anywhere cannot connect, first run `forge anywhere status`; then verify the local daemon still
works directly. Re-login for an expired local token, re-enroll a revoked host, or use the billing
portal for a suspended entitlement. Disabling or logging out never deletes local Forge data.
