# Forge Anywhere

> **Pre-release:** Forge Anywhere is being built in the public
> [Forge pull request](https://github.com/adulari/forge/pull/811) and the separately maintained
> [private service repository](https://github.com/adulari/forge-anywhere-service). This document is
> the launch contract. A pre-release client may implement only part of it; `forge anywhere status`
> is authoritative for the capabilities enabled on an account.

Forge Anywhere is the optional, end-to-end encrypted companion for individual developers. Its
promise is simple: **leave your desk without leaving your Forge session.** Forge itself remains
open source and free.

## What is free and what is paid

| Always free | Forge Anywhere subscription |
| --- | --- |
| The Forge CLI, TUI, daemon, and local session history | Managed encrypted relay with stable host identity |
| Loopback and LAN access | Up to three active hosts in a personal fleet |
| Direct device pairing | Unlimited personal controller devices |
| `forge serve --anywhere` through a tunnel you operate | 5 GB of encrypted cloud sync/history |
| Any self-hosted networking and storage you operate | Remote jobs and generic push notifications |
| Local workspace export/import | Safe encrypted workspace handoff between hosts |
| Direct HTTP/WebSocket protocol (`remote-v8`) | Expiring end-to-end encrypted replay links |

The managed service costs **€10/month or €79/year** through Paddle. Both prices are shown together
and annual billing is selected by default. A 14-day trial starts when the first host connects, needs
no card, and is available once per GitHub account. There is no permanent hosted free tier, lifetime
deal, advertising, model hosting, or AI-provider markup.

Disabling Anywhere, losing connectivity, reaching a quota, or losing an entitlement never disables
local Forge, LAN access, direct pairing, or a user-managed tunnel. The local daemon remains the
source of session behavior; Anywhere does not replace or reinterpret it.

V1 is personal-only. Team administration, shared organization accounts, and commercial
single-tenant hosting are future products rather than hidden V1 requirements.

## Set up a host

1. Sign in with the GitHub device flow:

   ```console
   forge anywhere login
   ```

2. On a new account, record the 24-word recovery phrase offline. Forge displays it once and asks
   for sampled words before encrypted upload is enabled. Forge support cannot recover it.
3. Register and connect the current machine:

   ```console
   forge anywhere enable --name workstation
   forge anywhere status
   ```

   Omitting `--name` uses a generated local name. The first successful host connection starts the
   trial. Registering a fourth active host requires revoking or disabling another one.
4. Sign in from the mobile or web app and approve the short-lived pairing QR challenge from an
   authorized device. Verify the displayed account and device fingerprints before approval.

`forge anywhere enable` starts or attaches the managed connector to local `forge serve`; it does
not expose the daemon token. An optional configuration block controls the connector:

```toml
[anywhere]
enabled = true
host_name = "workstation"
sync = true
```

`service_url` is development-only. Access/refresh tokens, device keys, recovery material, and the
local daemon token never belong in `config.toml`. CLI secrets live in Forge's platform data
directory with owner-only permissions; mobile uses SecureStore; web keys are wrapped by a
non-extractable WebCrypto key stored in IndexedDB.

Useful account commands are:

```console
forge anywhere status
forge anywhere devices
forge anywhere disable
forge anywhere logout
```

`disable` revokes only this host. `logout` revokes local tokens. Neither command deletes local
Forge data.

## Encryption and service visibility

On first setup, Forge generates a random Account Data Key and device-specific X25519 exchange and
Ed25519 signing keys. Payloads use XChaCha20-Poly1305 with a signed, authenticated routing header.
Every key epoch is wrapped separately to authorized devices and to the recovery secret. The
service enforces unique sender/epoch/sequence tuples to reject replay, but cannot decrypt payloads.

The service can observe account, sender, recipient and object identifiers, envelope kind, time,
ciphertext size, connection timing, IP address, subscription state, and aggregate quota use. It
cannot observe session, prompt, response, command, diff, workspace, or replay plaintext. GitHub and
Paddle establish identity and billing; neither is an encryption authority.

An authorized, unlocked, or compromised endpoint can read epochs it possesses. Revocation protects
future epochs; it cannot retract plaintext or old keys already copied by that endpoint. Forge
Anywhere also does not prevent screen capture, traffic analysis, denial of service, or compromise
of the local Forge process.

The normative envelope and API contract is
[`protocol/anywhere-v1.md`](../../protocol/anywhere-v1.md). See the
[`threat model`](../../protocol/anywhere-v1-threat-model.md) and
[`licensing boundary`](../../protocol/anywhere-licensing-boundary.md) for the complete trust and
source-code boundaries.

## Recovery, device loss, and revocation

- **One device lost, another remains:** list devices from an authorized endpoint and revoke the
  missing device immediately. Revocation atomically invalidates its tokens and creates a new data
  key epoch wrapped only to remaining devices and the recovery secret.
- **All devices lost, recovery phrase available:** install Forge on a trusted device, choose account
  recovery during `forge anywhere login`, enter the phrase locally, and re-enroll the device. Do not
  send the phrase to Forge support or enter it on the service website.
- **Recovery phrase exposed:** from an authorized device, recover/rotate the account keys, revoke
  unknown devices, and store the replacement phrase offline. Assume old ciphertext readable by
  anyone who obtained an old authorized key or phrase.
- **All devices and the phrase lost:** encrypted cloud data is unrecoverable. Support can verify
  identity for billing, export encrypted account data, or delete the account, but cannot decrypt it.
- **GitHub account lost:** a still-authorized device can preserve local/decrypted data. Contact
  support for identity and billing procedures; the recovery phrase alone does not grant service
  account access.

Practice recovery before relying on cloud-only history. The operator procedure is in
[`docs/anywhere/recovery-and-handoff-drills.md`](../anywhere/recovery-and-handoff-drills.md).

## Encrypted sync and conflicts

Eligible records include sessions, messages, checkpoints, tool calls, routing decisions, usage,
compactions, memories, user settings, commands, skills, agents, and workflows. The host journals an
eligible record only after the local store transaction succeeds and retains retry state until the
service verifies the encrypted upload.

Forge never syncs:

- provider credentials, keyring contents, device private keys, recovery material, or daemon tokens;
- Lattice indexes or embeddings;
- push secrets;
- host schedules, queue internals, or caches;
- build artifacts, checkpoint scratch files, or pending uploads;
- workspace files except inside an explicit, validated handoff capsule.

Append-only records use stable IDs. Mutable metadata uses deterministic
`(logical_clock, device_id)` last-write-wins ordering. File records carry base and content hashes;
divergent edits create a visible conflict copy rather than overwriting either side. Deletions create
tombstones. Mobile stores offline history with device-local encryption.

The current pre-release client safely materializes a narrower subset: memory ordering/tombstones
and portable metadata plus append-only records for sessions already present locally. Remote
workspace paths and permission modes never replace host-local values; missing dependencies remain
staged and identity/sequence collisions become durable conflicts. Entirely remote sessions,
settings, and files remain staged until their import and conflict policies ship. Do not describe a
build as full two-way sync until `forge anywhere status` reports it.

At 5 GB, new cloud writes are blocked, but download, restore, export, and deletion continue. Delete
unneeded shares/history or turn sync off; local writes continue normally.

## Workspace handoff

```console
forge anywhere handoff SESSION --to HOST
```

A handoff starts only at an idle checkpoint. Let an active tool call finish or explicitly interrupt
it first. The encrypted, compressed capsule contains a portable session export, repository/base
metadata, `git diff --binary --full-index BASE`, and safe untracked files.

Forge aborts with a visible list instead of silently dropping non-secret user files when it finds
`.git`, links, special files, absolute/traversal paths, suspected secrets, ignored caches or build
output, a file above 25 MB, or a compressed capsule above 100 MB.

The destination verifies the base commit, creates an isolated detached worktree, applies the patch
with Git's three-way binary support, safely extracts untracked files without following links, and
imports or remaps the session ID. The service transfers the authoritative session lease only after
the destination acknowledges success. A conflict or failed import removes the new worktree and
leaves the source lease unchanged.

If connectivity fails after upload or claim, do **not** immediately repeat the handoff. Check status
on both hosts: exactly one service lease is authoritative. Resume/acknowledge an already imported
capsule if offered; otherwise clean up only the destination's isolated failed worktree and retry
from the source. Never delete the source workspace based solely on a client timeout. See the
[`failed-handoff drill`](../anywhere/recovery-and-handoff-drills.md#failed-handoff-drill).

## Encrypted replay shares

```console
forge anywhere share SESSION --expires 24h
forge anywhere share SESSION --expires 7d
forge anywhere share SESSION --expires 30d
```

The client creates a random share key and encrypts the replay before upload. The URL fragment holds
both the decryption key and sender Ed25519 public key as `#key=...&signing=...`. Fragments are not
sent in HTTP requests or persisted by the service, so it stores ciphertext without either viewer
secret. The public viewer fails closed if either fragment value is missing or if the signature,
header, or ciphertext is tampered. Treat the complete URL like the replay itself. Anyone with it can
decrypt the snapshot until it is revoked or expires. Shares are read-only, contain a point-in-time
replay rather than a live session, and have a maximum lifetime of 30 days. Revocation prevents later
retrieval but cannot retract a copy already downloaded by a recipient.

## Generic push

Anywhere push means only that a host or job needs attention. Hosted notifications never contain a
prompt, response, command, repository, filename, session title/ID, diff, error text, or daemon
credential. Opening the app authenticates and retrieves encrypted details. Push tokens are secrets,
are never synced as history, and can be revoked per device. The existing APNs-only `forge-relay`
remains isolated; it is not the Anywhere backend.

## Billing and retention lifecycle

| State | Access |
| --- | --- |
| `trialing` | Full service for 14 days from the first host connection |
| `active` | Full service through the paid-through date; cancellation does not end it early |
| `grace` | Seven days after payment failure, with relay and full read access |
| `read_only` | 30 days after trial/period/grace expiry: download, restore, delete, export, and billing only |
| `suspended` | Billing, export, and deletion only until the 90-day retention deadline |

A successful Paddle event restores service immediately. New relay work, uploads, remote commands,
shares, and capsules are blocked in `read_only` and `suspended`. Expired subscription data is kept
for 90 days, with warnings 30 and 7 days before deletion. Cancellation remains active through the
paid-through date.

Consumed capsules and temporary relay blobs are deleted after successful consumption or within 24
hours. Superseded sync revisions are retained for seven days and tombstones for 30. Replay shares
use their selected expiry. Account deletion removes live database and object-store data within 24
hours; encrypted backups expire within 30 days.

## Account export and deletion

Account controls remain available when service is read-only or suspended:

- **Export** returns account metadata plus the encrypted objects needed for user-controlled
  retention. It does not decrypt them and should be stored with the recovery phrase separately.
- **Delete account** requires an idempotent authenticated confirmation, revokes tokens and hosts,
  schedules live database/R2 removal within 24 hours, and stops billing according to the displayed
  Paddle terms. Encrypted backup copies age out within 30 days.
- Deleting the hosted account does not delete local Forge data. Uninstalling or logging out of a
  client does not delete the hosted account.

## Troubleshooting

| Symptom | Check and recovery |
| --- | --- |
| Local Forge works, Anywhere does not | Run `forge anywhere status`; check entitlement and service reachability. Continue over loopback/LAN/direct pairing or your own tunnel. |
| Authentication expired | Run `forge anywhere login`; a revoked device must be re-enrolled rather than merely refreshed. |
| Host is offline | Confirm local `forge serve` works, then restart/enable the connector. The local daemon token must stay on the host. |
| Host limit reached | Disable or revoke an unused host, then enroll the new host. |
| Writes rejected | Check the 5 GB quota and entitlement state. Download/export/delete remain available. |
| Sync appears stuck | Keep local Forge running; journal entries remain pending across an outage. Check conflicts and missing dependencies before retrying. |
| Device disappeared or was stolen | Revoke it from another authorized device and confirm a new key epoch was created. |
| Pairing QR expired | Start a new pairing; tickets last ten minutes and are single-purpose. Never reuse screenshots. |
| Replay link cannot open | Confirm it is complete including the URL fragment, unexpired, and not revoked. The service cannot recover a missing fragment key. |
| Handoff timed out | Do not delete or resume both copies. Inspect the authoritative lease and follow the failed-handoff procedure above. |
| Subscription issue | Open the Paddle billing portal. Local/direct Forge access is unaffected. |

Security incidents should include metadata and envelope identifiers only. Do not send prompts,
commands, filenames, diffs, credentials, recovery words, or decrypted captures to support.

## Release operations

- [Rollout gates and launch checklists](../anywhere/rollout-checklists.md)
- [Recovery and failed-handoff drills](../anywhere/recovery-and-handoff-drills.md)
- [Privacy data inventory and analytics allowlist](../anywhere/privacy-data-inventory.md)
- [AGPL/private-service boundary checklist](../anywhere/agpl-service-boundary-checklist.md)
- [Known-plaintext canary harness](../../scripts/ci/check-anywhere-plaintext-canary.sh)
