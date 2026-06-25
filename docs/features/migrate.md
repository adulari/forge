# `forge migrate` — move a full install to another machine

Copy a complete Forge install — config, skills, commands, MCP servers, hooks, and
machine-agnostic model metadata — to another PC or server. Session history and API keys are
opt-in. The bundle is a plain **directory** (not an archive): transport-agnostic and fully
inspectable, which matters because `--include-keys` writes secrets in plaintext.

## Commands

```bash
forge migrate export <dir> [--include-keys] [--include-sessions]
forge migrate import <dir> [--force]
forge migrate push  <user@host> [--include-keys] [--include-sessions]
```

- **export** — write the bundle directory `<dir>` (created if missing).
- **import** — restore a bundle into *this* machine's config + data dirs.
- **push** — `export` to a temp dir → `scp -r` to the host → run `forge migrate import` over
  SSH. Requires SSH access and `forge` on the remote's `PATH`.

## What's in the bundle

| File / dir | Contents | When |
| --- | --- | --- |
| `config/` | the entire user config dir: `config.toml`, `skills/`, `commands/`, MCP config, hooks | always |
| `model-metadata.json` | `model_health`, `model_context`, `model_pricing` rows (no history) | always |
| `forge.db` | the full session/usage database | `--include-sessions` |
| `secrets.json` | `{provider: api_key}` in **plaintext** | `--include-keys` |
| `manifest.json` | schema version, source host, timestamp, what's included | always |

Paths are resolved with the `directories` crate: config dir = e.g. `~/.config/forge`
(Linux/XDG), data dir = e.g. `~/.local/share/forge`.

## Safety model

- **API keys are opt-in and plaintext.** `--include-keys` writes `secrets.json` and prints a
  loud warning. On import they're restored into the new machine's OS keyring (encrypted-file
  fallback if no keyring), and you're reminded to delete the bundle. Prefer omitting the flag
  and running `forge auth <provider>` on the new machine; only use `--include-keys` over a
  trusted channel (USB, `scp` between your own boxes).
- **History is opt-in.** Without `--include-sessions` the bundle contains no transcripts,
  usage, or routing data — only model metadata.
- **No-leak allow-list.** The metadata export names exactly three tables
  (`model_health`, `model_context`, `model_pricing`). Both export and import are allow-listed,
  so a session-free bundle can never carry transcripts and a tampered bundle can't write
  arbitrary tables.
- **Import never destroys history.** If the target already has a `forge.db`, an incoming one
  is saved alongside as `forge.imported.db` unless you pass `--force`. Config files *are*
  overwritten on name collision (it's a migration), so import into a fresh machine for a clean
  result.

## Examples

Move everything except history to a new laptop:

```bash
# old machine
forge migrate export ~/forge-bundle --include-keys
scp -r ~/forge-bundle newlaptop:~/

# new machine
forge migrate import ~/forge-bundle
rm -rf ~/forge-bundle   # it held keys in plaintext
```

One-shot to a server you control:

```bash
forge migrate push me@myserver --include-keys --include-sessions
```

## Notes

- The bundle is a directory; to ship a single file, `tar czf bundle.tgz <dir>` yourself.
- `push` uses the system `scp`/`ssh`; configure your SSH keys/agent first.
- Re-running `import` is idempotent for config/metadata (overwrite / upsert).
