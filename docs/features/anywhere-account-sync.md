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
| `mcp` | `mcp.toml` servers. URL servers apply directly; stdio servers apply only where the command is on `PATH`, otherwise they show as skipped ("needs `npx`, not installed here") | servers that embed a credential inline: a connection string or token-shaped value in an env value, a secret-looking flag or argument (`--api-key`, `--token=`, `Authorization:`), a token in the URL path or query (`?api_key=`); held back and shown as skipped |
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

## In the terminal: `/anywhere`

`/anywhere` (also in the command palette) opens the Forge Anywhere menu instead of printing a line:

- **Account and connection**: the account, connector state, this device, how many hosts are online,
  and whether `anywhere.sync` is on. When you are not signed in the first row is "Sign in to Forge
  Anywhere", which runs `forge anywhere setup` with the terminal handed over.
- **Sync across machines**: the five categories with an on/off switch, item count, pending changes,
  last sync, conflicts and skipped items with their reasons. Space or Enter toggles, `s` runs Sync now
  (it asks the running `forge serve` for a cycle), and a "Sign in again" list shows the rotating logins
  other devices use (`forge auth codex-oauth`, `forge mcp login <server>`); Enter runs the command.
- **Devices**: every enrolled device with last seen and enrolled date, this one marked. It is
  read-only: revoking needs the recovery phrase, so the menu shows `forge anywhere devices --revoke <id>`.
- **Open Recovery Center and approvals** opens the web page; it is a labelled row, not the default.

Keys: arrows or `j`/`k` move, Tab and Shift+Tab jump between sections, `r` refreshes, Esc closes.
Status refreshes every 3 seconds while the menu is open (the same builder as `forge anywhere sync`)
and the device list every 30 seconds.

Safety is the same as everywhere else. Turning `keys` on shows a confirmation that explains the second
vault key and that OAuth tokens never sync; it opens on Cancel, so Enter alone never turns it on, and
the desktop app shows the same confirmation. The switch can only be flipped at the machine itself: the bridge refuses it, and a remote or fleet channel cannot open `/anywhere` in the host terminal or type into it. Turning a category off says that
local data stays in place. No screen ever renders a secret value.

The statusline `anywhere_sync` badge now appears on its own while any category is on. `/statusline
toggle sync` hides it (`statusline.hide_sync_badge = true`) or brings it back; listing the widget
yourself works as before.

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
  secret store, migrated once on first sync and still read from the old layout until then. They are
  stored in the keyring and in the encrypted file, so a shell that cannot reach the keyring (SSH)
  reads the same keys. The stored copy names the device and key epoch it belongs to and is read past
  the process cache; a copy that does not match `state.json` stops the load with an error instead of
  being mixed in. If the secret store is unavailable they stay in the file with a warning.
- The vault seal on key values is a guardrail, not a boundary against enrolled devices: phones and
  controllers hold the Account Data Key the vault key derives from and download the key records
  (the app discards them unseen). See `protocol/anywhere-v1-threat-model.md`.
