# Host durability and power

What a long-running `forge serve` does so a laptop, a container or a bad upgrade cannot cost you
your sessions. Everything here is on by default except the three `[host]` limits.

## Staying awake: `[power] keep_awake`

A sleeping laptop freezes the daemon mid-turn and the phone driving it loses the session. While the
mode wants it, the daemon holds an OS sleep inhibitor.

| Value | Holds the inhibitor |
|---|---|
| `off` | never |
| `while_working` (default) | while any session has a turn running, and 30 seconds after the last one ends |
| `while_remote` | while a turn runs or any app is connected to the daemon |
| `always` | for as long as `forge serve` runs |

The inhibitor is `systemd-inhibit --what=idle:sleep` on Linux, `caffeinate -i` on macOS and
`SetThreadExecutionState` on Windows. It stops the computer sleeping, not the screen locking, and a
closed laptop lid may still suspend a machine that is not set to ignore it. The Settings screen of
the apps has the same switch ("Keep this computer awake"), and the Diagnostics screen shows whether
the inhibitor is held, or why it could not be.

The Linux inhibitor runs a small guard command that ends when the daemon does, so a killed daemon
cannot leave the machine unable to sleep.

## The session store

* **A copy before every migration.** When opening the store would run a schema migration, Forge
  first copies it to `forge.db.pre-v<N>.bak` (N is the schema being migrated to; the two newest are
  kept) with SQLite's online backup. A build that carries the wrong schema, or a migration that is
  interrupted, can no longer take the only copy with it.
* **An integrity check at daemon start.** `forge serve` runs `PRAGMA quick_check`. A store that
  fails it is renamed `forge.db.corrupt-<timestamp>`, the newest backup that still passes the check
  is copied into place, and the Diagnostics screen reports what happened for a week. With no usable
  backup the daemon starts empty and the damaged file stays for manual salvage. A check that merely
  could not run (a locked file) is never treated as damage.
* **A recovery page instead of a dead daemon.** If the store still will not open (written by a
  newer Forge, or a migration that fails), the daemon stays up on its port in recovery mode. Every
  normal route answers `503` with `code: "store_recovery"`; the apps then show a recovery screen
  that offers **Restore** for each backup, **Run migration again** and, in the desktop app,
  **Open data folder**. The store a restore replaces is kept as `forge.db.replaced-<timestamp>`.
  When the store opens, the daemon carries on starting.

`forge backup` makes a consistent copy any time (default `backups/` beside the store; `--out`
chooses the file), `forge backup list` shows the copies, and `forge backup restore <file>` puts one
back while the daemon is stopped.

## One daemon per config directory

`forge serve` takes an advisory lock on `serve.lock` in the config directory. A second daemon stops
at startup and names the first: `another forge serve is already running for <dir> (pid 4242)`. The
lock belongs to the process, so a crashed daemon releases it.

## Restarts are announced

When the daemon is told to stop, it lets running turns finish (up to 30 seconds), then closes every
open socket (sessions, fleet, terminals) with WebSocket code **1001, "host restarting"**. The apps
read that as a restart rather than an outage: they keep retrying every second, say "host
restarting" instead of "unreachable", and toast it once.

## Limits: `[host]`

```toml
[host]
max_active_sessions = 8   # live sessions; a ninth is refused
max_terminals = 12        # open terminals across all sessions
max_helpers = 4           # subagent helpers running at once, host-wide
```

`0` (the default) means no limit. A refused session is `429` with `code: "host_cap"`; a refused
terminal closes its socket with application code `4429`. The apps show either as a toast.
Helpers are different: a helper over the limit waits for a free slot rather than failing the turn
that spawned it.

## The systemd unit

`forge service install` writes a user unit with `NoNewPrivileges`, `ProtectSystem=full`,
`LockPersonality` and `RestrictSUIDSGID`: nothing an agent runs can gain privileges through setuid,
and `/usr`, `/boot` and `/etc` are read-only to it. Your home directory and the project trees stay
writable. If a workflow needs `sudo` or system writes, loosen it with a drop-in
(`systemctl --user edit forge-serve.service`). `forge service logs [-n 100] [-f]` shows the
daemon's output (journald on Linux, `serve.log` in the data directory on macOS).

## Rolling an update back

`forge update` keeps the build it replaced beside the binary as `forge.prev` (one extra binary of
disk space). `forge update --rollback` swaps the two; running it again undoes the rollback.
Package-manager installs (Homebrew, cargo, AUR, Scoop) are rolled back with that tool.

## Windows

Shell children are placed in a job object created with `KILL_ON_JOB_CLOSE`, so a daemon that exits
or crashes takes its dev servers and build tools with it instead of leaving them running.

## Running it in a container

```sh
docker build -f packaging/docker/Dockerfile -t forge-serve .
docker run -d --name forge --stop-timeout 60 -p 127.0.0.1:7420:7420 -v forge-home:/home/forge forge-serve
docker logs forge          # prints the connect link on first start only when stdout is a terminal
docker exec forge cat /home/forge/.config/forge/serve-token
```

The image installs the released binary with `install.sh` (checksum verified); pin a release with
`--build-arg FORGE_VERSION=v2.24.1`. It runs `forge serve --lan --port 7420`, which listens on every
interface with a self-signed certificate; publish the port on loopback as above or put a
TLS-terminating proxy in front of it. Keep `/home/forge` on a volume: it holds the config, the
pairing token and the session store.
