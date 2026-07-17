# Forge Anywhere v1 threat model

## Protected assets and trust boundaries

Forge Anywhere protects session content, prompts, responses, tool data, workspace capsules, synced
settings, and replay shares from the managed service, R2, network observers, and database/backup
readers. Device private keys, recovery entropy, the Account Data Key, provider credentials, and the
local daemon token remain client-side.

The service is trusted for availability, entitlement decisions, routing, quota accounting, expiry,
and metadata integrity. It is not trusted with plaintext. GitHub and Paddle establish identity and
billing, not encryption authority. A compromised authorized endpoint can read epochs it possesses;
revocation limits future epochs but cannot retract plaintext or old keys already copied by that
endpoint.

## Required controls

- TLS plus signed, AEAD-authenticated envelopes; neither control alone substitutes for the other.
- Server-enforced unique `(sender_device_id, key_epoch, sequence)` tuples and single-use tickets.
- Explicit daemon route IDs; no arbitrary proxy target and no uploaded daemon token.
- Atomic token revocation and epoch rotation when a device is removed.
- Strict account ownership in every service query and object key.
- Generic push text only. No repository, session, prompt, command, or filename content in push.
- Metadata-only structured logs and canaries that search DB, logs, and R2 for known plaintext.
- Capsule extraction rejects links, special files, traversal, absolute paths, secrets, ignored
  caches/build output, individual files over 25 MB, and compressed capsules over 100 MB.

## Explicit non-goals

The protocol does not hide account/device/recipient IDs, object kind, time, ciphertext size,
connection timing, IP address, subscription state, or aggregate quota use from the service. It does
not protect an unlocked compromised device, malicious local Forge process, screen capture, traffic
analysis, denial of service, or a user who loses both all enrolled devices and the recovery phrase.

