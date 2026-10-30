# Anywhere account sync

Set up one machine, sign in to Forge Anywhere on the next, and your keys, skills, settings, MCP
servers and instructions follow you. It rides the existing end-to-end-encrypted Anywhere sync: no new
record kind and no service change.

Every category is a **per-device** switch under `[anywhere.account_sync]`, all off by default.

| Category | Syncs | Never syncs |
|---|---|---|
| `keys` | provider API keys, `brave`, `artificialanalysis`, the long-lived Claude token, static MCP secrets | env-var keys, `provider-oauth:*`, `mcp-oauth*`; only a "this login is in use" marker |
| `skills` | `skills/`, `commands/`, `agents/`, marketplaces and installed packs (re-installed from source) | skill-usage counts |
| `settings` | theme and palette, statusline widgets, notifications, default permission mode, permission rules, voice and telemetry prefs, `keybindings.json`, and which CLIs you use | anything with a path, command, host or endpoint: hooks, `statusline.command`, `lsp`, `terminal.shell`, `local.endpoint`, `tools.extra_roots` |
| `mcp` | `mcp.toml` servers. URL servers apply directly; stdio servers apply only where the command is on `PATH`, otherwise they show as skipped ("needs `npx`, not installed here") | servers that embed a credential inline |
| `instructions` | the user-level `AGENTS.md` | `~/.claude/CLAUDE.md` |

## Using it

```
forge anywhere sync                 # status: items, pending, conflicts, skipped, sign-ins to redo
forge anywhere sync enable skills
forge anywhere sync enable keys     # asks first; --yes in scripts
forge anywhere sync disable mcp
forge anywhere sync now             # ask the running daemon for a cycle
```

The same switches are in `/config` (Anywhere sync group), the onboarding Anywhere step, the daemon
(`GET`/`PUT /api/anywhere/sync`, `POST /api/anywhere/sync/now`) and the apps. The statusline widget
`anywhere_sync` shows `sync ✓`, `sync 2↑` (changes waiting to upload) or `sync !` (a conflict or an
error), and hides itself when account sync is off.

## How it works

- A reconcile pass runs in the 10-second sync loop after download and apply, only for categories
  enabled on this device. It compares each item's keyed content hash with the one recorded in
  `anywhere_account_sync_item`: a local change uploads, a local removal becomes a tombstone, an
  incoming newer record applies and its hash is recorded so it is never echoed back.
- Both sides changed: the newer edit wins (file mtime against the sender's `updated_at`; keys have no
  mtime, so the device holding them wins). The loser is recorded as a conflict, and a replaced file
  keeps a `<name>.conflict` copy.
- Items over 4 MB are skipped and reported. Values never appear in status output or logs.
- Record ids are `HMAC-SHA256(HKDF(data key, "forge-account-sync-id-v1"), "<category>/<item>")` using
  the oldest key epoch the device holds, so rotation does not rename records. Key values are sealed
  again under `HKDF(data key, "forge-vault-v1")`.

## Safety

- Turning `keys` on needs a local action (terminal, `/config`, or a direct connection). The Anywhere
  bridge refuses both `PUT /api/anywhere/sync` for `keys` and `PUT /api/config` for
  `anywhere.account_sync.keys` with `403 direct_only`.
- Rotating logins never sync, because refresh tokens are single use. Other devices list a "Sign in to
  Codex" prompt (`forge auth codex-oauth`) instead.
- Revoking a device cannot recall ciphertext it already downloaded. Rotate synced keys at the
  provider; `forge anywhere devices --revoke` re-seals this device's keys under the new epoch and
  prints the labels to rotate.
- The Account Data Key epochs and device private keys moved from plaintext `state.json` into the
  secret store (keyring first, encrypted file second), migrated once on first sync and still read from
  the old layout until then. If the secret store is unavailable they stay in the file with a warning.
