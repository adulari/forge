# Forge Anywhere recovery and failed-handoff drills

Run these drills with synthetic canary content and throwaway repositories only. Do not paste real
recovery words into tickets, terminals with recording enabled, shell arguments, logs, or chat. Store
the drill record as metadata: date, versions, opaque test-account ID, step outcomes, timings, and
incident links.

## Account recovery drill

### Prepare

- Create a dedicated test GitHub identity and bootstrap Anywhere on device A.
- Record the recovery phrase offline and complete the sampled-word check.
- Connect host A, sync synthetic history containing a unique known-plaintext canary, and verify that
  the client can read it.
- Capture service logs, SQLite, R2 objects, and a restored backup for the canary scan. The canary must
  appear inside client plaintext but nowhere in captured service artifacts.

### Recover

1. Revoke/delete the local Anywhere credentials on a clean device B; do not copy device A's files.
2. Start account recovery through the GitHub device flow.
3. Enter the recovery phrase only into the trusted local client.
4. Verify account identity and enroll fresh signing/exchange keys for device B.
5. Download wrapped key epochs and encrypted history. Verify locally that the synthetic history
   decrypts and that corrupt/tampered objects are rejected.
6. Revoke device A. Confirm its tokens fail, a new data-key epoch is created, and device A cannot
   download or decrypt objects from the new epoch.
7. Upload a new synthetic record from B and verify retry/idempotency behavior.
8. Export the encrypted account and confirm it contains no recovery phrase or private device key.

### Pass criteria

- Recovery needs both GitHub account authorization and local recovery material.
- The service never receives the phrase, plaintext, private key, or local daemon token.
- Old history is readable by B; new-epoch data is unavailable to revoked A.
- The known-plaintext canary scan passes for live and restored service artifacts.
- Direct/local Forge remained usable throughout.

## Recovery failure cases

- Use one incorrect word: recovery must fail without revealing which key material was close.
- Corrupt a wrapped epoch key: recovery must fail closed and identify the affected opaque epoch.
- Expire/reuse the device-flow and pairing tickets: both must be rejected.
- Lose all devices and phrase: confirm support can export ciphertext/delete/bill but cannot decrypt.
- Suspend the subscription: recovery/export/delete remain reachable according to the lifecycle;
  new uploads remain blocked.

## Failed-handoff drill

Create a synthetic repository with committed, staged, unstaged, renamed, deleted, binary, and safe
untracked files. Keep the source clone. Verify successful handoff first, then inject one failure at
a time. The source session must be idle before each attempt.

### Safety rejections

- Add a symlink, traversal path, special file, ignored build output, suspected secret, file over
  25 MB, and capsule over 100 MB in separate runs.
- Confirm Forge reports the visible rejection list, creates no transferable capsule, and silently
  drops no non-secret user file.

### Failure injection matrix

| Injection point | Required outcome |
| --- | --- |
| Before capsule upload completes | Reservation can expire/retry idempotently; source lease unchanged |
| After upload, before destination claim | Capsule remains encrypted and expires within policy; source lease unchanged |
| After claim, before download completes | Claim can safely resume; no destination session/worktree becomes authoritative |
| Missing base commit | Destination rejects before applying; isolated worktree removed; source lease unchanged |
| `git apply --3way --binary` conflict | Actionable conflict returned; new worktree removed; source lease unchanged |
| Unsafe untracked extraction | Extraction fails closed without following links; new worktree removed |
| Session-ID collision/import failure | Import remaps or fails transactionally; source lease unchanged |
| Destination crash after import, before acknowledgement | Service lease remains source; retry detects prior claim/import rather than duplicating blindly |
| Lost acknowledgement response | Status reveals one authoritative lease; repeated acknowledgement is idempotent |
| Source disconnect after acknowledged transfer | Destination lease remains authoritative; source cannot resume without an explicit later transfer |

### Indeterminate outcome procedure

1. Stop automation on both hosts. Do not retry, delete a workspace, or resume the session on both.
2. Query capsule and lease status using authenticated clients. Treat the service's single
   authoritative lease as the coordination result, not either client's timeout message.
3. If the destination is authoritative, verify its detached worktree and imported session before
   resuming. Preserve the source workspace until this succeeds.
4. If the source remains authoritative, remove only the destination worktree created for the failed
   attempt. Do not alter the source repository/session, then resume or retry with the same
   idempotency key when supported.
5. If status cannot be obtained, preserve both local workspaces but resume neither. Restore service
   access or escalate using opaque capsule/session/host IDs only.
6. Capture metadata-only evidence and verify capsule deletion after acknowledgement or expiry.

### Pass criteria

- At every injected failure there is exactly one authoritative lease.
- No partial destination is exposed as a normal working session.
- Rollback removes only the isolated worktree it created.
- Source data is never deleted automatically because of a timeout.
- A successful capsule is consumed/deleted; abandoned data expires within 24 hours.
- Direct/LAN/user-managed-tunnel access remains independent.
