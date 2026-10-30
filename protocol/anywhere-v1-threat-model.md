# Forge Anywhere v1 threat model

## Protected assets and trust boundaries

Forge Anywhere protects session content, prompts, responses, tool data, workspace capsules, synced
settings, and replay shares from the managed service, R2, network observers, and database/backup
readers. Device private keys, recovery entropy, the Account Data Key, and the local daemon token
remain client-side. Provider credentials stay client-side too, with one deliberate exception below.

### Account sync of credentials (explicit per-device opt-in)

A device that turns on `[anywhere.account_sync] keys` uploads its provider API keys, search keys, the
long-lived Claude token and static MCP secrets inside the same signed, AEAD-sealed sync envelope as
every other record. Each key value is sealed a second time under `HKDF(data key, "forge-vault-v1")`
for its epoch, bound to the record id, so a client that only renders sync history cannot display it.
Record ids are `HMAC-SHA256(HKDF(data key, "forge-account-sync-id-v1"), "<category>/<item>")`, so the
service sees neither a slot name nor whether a record is a key or a theme.

- The toggle is per device, off by default, and can only be changed on the machine itself or over a
  direct connection. The Anywhere bridge refuses it, as it refuses writing the same config key.
- Rotating OAuth logins (Codex, xAI, MCP OAuth) never sync: refresh tokens are single use, so a copy
  would sign both machines out. Only a presence marker travels and the receiving device is prompted
  to sign in.
- Environment-variable keys are never uploaded, and phones and controllers never receive secrets.
- Revocation rotates the Account Data Key but cannot recall ciphertext a revoked device already
  downloaded. Keys synced before the revocation must be rotated at their providers; `forge anywhere
  devices --revoke` re-seals this device's keys under the new epoch and lists their labels.
- The Account Data Key epochs and device private keys now live in the secret store (OS keyring, then
  the encrypted fallback file) rather than the plaintext `state.json`, so synced secrets are not
  protected more weakly than local ones.

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

## Operational verification

The public
[`known-plaintext canary harness`](../scripts/ci/check-anywhere-plaintext-canary.sh) scans captured
service logs, SQLite files, and object-store/backup artifacts for synthetic plaintext supplied in a
separate marker file. Its self-test runs in public CI; deployment operators must run the real scan
against live and restored captures. The harness intentionally prints artifact paths but never the
marker value. See the [recovery drill](../docs/anywhere/recovery-and-handoff-drills.md) and
[privacy inventory](../docs/anywhere/privacy-data-inventory.md) for capture and evidence rules.
