# Known issues & deferred work

Tracked limitations and intentionally-deferred features. Each entry: symptom, what
we know, and the planned fix.

## What is open right now

Headings ending in `(fixed)` are kept as history. Currently open:

- **Store poisoning by dev builds — the outage is LIVE, the code fix is not** — anything built from
  a working tree opened the real store and migrated it to that branch's schema; the installed
  release binary then refuses it and `forge serve` cannot start. The store sits at **schema 26**
  against an installed binary supporting **23**, with the daemon at **34,353 restarts** and
  Anywhere dark (measured 2026-08-11 14:15). Every fix has now **merged** — per-route #985/#994,
  the class fix #995, test isolation #965, permanent-failure exit #996 — but **none is released**,
  so the machine still runs a binary that has none of them. Ending this needs a release, not more
  code.
- **No deadman alarm for the daemon** — those 34,353 restarts produced nothing but repeated journal
  lines: no push, no notification, no statusline signal. The obvious sender reads its subscriber
  list *from the store*, so it is silent in exactly the case it exists for.
- **Version-notice labelling** — `OperationalNotice` renders the updater's *available* version
  while `UpdateNotice` headings a bare `Forge <version>` for the *installed* one. #978 removed the
  drift that made them diverge wildly (2.6.6 vs 2.12.1), so the numbers now agree, but the
  installed-build heading still carries no qualifier saying which it is.

Everything else below is either resolved or a recorded measurement finding.

## Store poisoning by dev builds

**Symptom (four occurrences):** `forge serve` refuses to start with `database schema version <n> is
newer than this build supports (<m>)`, Anywhere goes down, and local Forge may keep answering on a
connection opened before the migration — so "the daemon is running" looks healthy while cloud sync
is dead. Recovery has meant a hand-written `PRAGMA user_version` write each time.

**Cause:** a binary built from a working tree opens `~/.local/share/forge/forge.db` and runs
whatever migrations that branch carries. Routes found so far: the quota probe handing `FORGE_DB`
and the project cwd to a `claude --print` child, which loads `.mcp.json` and spawns a `forge`
grandchild (#985); and this repo's own `.mcp.json` / `.forge/mcp.toml` pointing the self-MCP agent
directly at `target/debug/forge` (#994).

**Occurrences:** 2026-07-17 (v17→v21), 2026-08-06 twice (24→25), 2026-08-07 (23→25). Each left
`forge-serve.service` restarting roughly every ten seconds, because the `network-resilience.conf`
drop-in sets `StartLimitIntervalSec=0` so transient network failures retry forever, and nothing
distinguished a permanent failure from a transient one.

**This is not past tense.** Measured 2026-08-11 14:15: the store is at schema **26** while the
installed binary (2.12.2) supports **23**, and `forge-serve` is at **NRestarts=34353** — days of
continuous failure with Anywhere dark throughout. The 2026-08-07 recovery (a hand-written
`PRAGMA user_version = 23`) did not hold. Prefer re-measuring over trusting the number above; it
climbs by roughly 6 per minute:

```
sqlite3 ~/.local/share/forge/forge.db 'PRAGMA user_version;'
systemctl --user show forge-serve.service -p NRestarts -p SubState
```

**The store moved after the previous analysis, which that analysis said it would not.** An earlier
version of this section read 25 and concluded "this is one stranded store, not an active leak,"
reasoning that a build from the tree carried 26 and would have taken the store to 26. The store now
reads **26** — so something at schema 26 did open it, and the inference was wrong. Worth stating
plainly, because the same reasoning would understate a live leak again.

What the current numbers do and do not tell you: the store and `forge-dev.db` both read 26 while
`main` is at **27**, so whatever last migrated the real store predates the 27 bump (#973). That is
consistent with the class now being closed — #995 sends `debug_assertions` builds to `forge-dev.db`,
and that file existing at 26 is the mechanism working — but it is not proof, and it will only be
proof once an installed release carries the fix. Do not read recency off the file's mtime: the crash
loop opens it every ten seconds, so it is always freshly touched.

**Fixes — all merged to `main`, none released.** #985 and #994 close the two known routes. #995
closes the class from the build side — a `debug_assertions` build resolves to `forge-dev.db`, so
only installed releases touch the real store (explicit `FORGE_DB` still overrides) — and #965
closes it from the test side, giving unit tests a per-test store instead of falling through to the
default path. #996 makes a permanent failure exit 78 (`EX_CONFIG`) with
`RestartPreventExitStatus=78` in the generated unit, so the service stops and shows as `failed`
rather than looping — note that applies on the next `forge service install`.

**Still open:** nothing reports the outage to the user (see the deadman-alarm entry); recovering a
poisoned store still needs a manual PRAGMA rather than a supported command; and **none of the fixes
above have been released** — the newest release is v2.12.1 (2026-07-30) and `main` is 50 commits
ahead, so an installed Forge has none of them. Note the installed binary *reports* 2.12.2: the
`[2.12.2]` changelog section was written but never tagged, so the version string cannot be used to
tell what is installed (see the version-notice entry). #1014 at least makes the failure self-explanatory:
the error now names the remedy (install a build at least as new as the store) instead of suggesting
`forge doctor` and `RUST_LOG=debug`, neither of which leads anywhere.

**The single action that ends this is a release.** Every code fix is on `main`; the machine is
broken because it runs a binary from before them. No further code change shortens that path.

**The upgrade does not carry the whole of #996.** Its systemd half (`RestartPreventExitStatus=78`)
lives in the unit that `forge service install` renders, and installing a new version never rewrites
an existing unit. So on any install predating it, an upgraded binary exits 78 and systemd restarts
it anyway. #1010 makes `forge doctor` report a unit that lacks the directive, and one whose
`ExecStart` points at a different binary than the one running; re-rendering still means running
`forge service install` deliberately.

## Linux Wayland/WebKitGTK launch crash (fixed by default renderer workaround)

**Finding (verified):** the release desktop binary exited before mapping a window on a Hyprland/
wlroots Wayland session with `Gdk-Message: Error 71 (Protocol error) dispatching to Wayland
display`. The same binary mapped normally when launched with
`WEBKIT_DISABLE_DMABUF_RENDERER=1`.

**Fix:** Linux release launches now set `WEBKIT_DISABLE_DMABUF_RENDERER=1` in `main()` before
Tauri/GTK/WebKitGTK initialization, but only when the variable is not already set. This preserves
an explicit operator override and applies equally to the AppImage's packaged binary. The default
renderer path should be re-evaluated when the relevant WebKitGTK/wlroots defect is resolved.

**Status:** fixed in source; release/AppImage before/after launch evidence remains to be captured
on the affected Wayland session.

**Cost status:** the workaround's possible compositing, frame, and startup cost is **unquantified**;
this host crashes on the default renderer, so no A/B measurement is available. Do not treat the
Wayland workaround as performance-neutral without an unaffected comparison host.

## Changelog gate and unattended launch (fixed)

The update/changelog notice used a persisted seen-build gate. On an unattended launch with no
paired state, the gate could cover the initial surface and leave the user at the changelog/connect
flow until acknowledged. The capture-only `FORGE_PERF_SEED_UPDATE_SEEN=1` path clears that gate at
runtime for measurement; it does not weaken the user-facing feature.

## Perf fixture build-time constant-fold regression

A release export made with `EXPO_PUBLIC_PERF_FIXTURE=1` folded the root redirect to `/perf-fixture`
while another export could fold the fixture's own gate off, producing an artifact stranded on
"Performance fixture unavailable". Both route selection and fixture activation are now runtime
Tauri commands, and clean export verification removes stale `dist`/Metro state and checks emitted
gates.


**Finding (verified):** the workspace release source is `Cargo.toml` `[workspace.package] version =
"2.12.2"`, while the committed Tauri manifest had `mobile/src-tauri/tauri.conf.json` version
`"2.6.6"`. A desktop build therefore truthfully identified itself as `Forge 2.6.6`, while the
updater truthfully offered the newer release from `latest.json` (observed as `2.12.1`). The two
numbers came from different sources, but the UI did not make that boundary clear.

**Mechanism:** the Tauri bundle embeds `tauri.conf.json`'s hand-maintained version; the updater
compares that embedded value with the release manifest generated from the release tag. When the
embedded value remains old, an installed build can keep reporting the old version and repeatedly
show the newer updater offer, so users cannot tell which build they are running and the banner can
nag indefinitely.

**Fix:** release tooling now reads the package version from the workspace `Cargo.toml` via
`cargo metadata`, requires the release tag to match it, and stamps `tauri.conf.json` only as a
build-time generated input. The desktop version is no longer an independent release value. A
release fails before bundling if the tag and workspace version diverge.

**Status:** release-tooling fix present; existing published `2.6.6` desktop artifacts remain
historically affected and must be replaced by a correctly stamped release.

## Lucide bundle import boundary (fixed)

**Outcome (#981):** a local Babel plugin rewrites named imports to per-icon files, mapping icon
name → dist file by parsing the package's own CJS barrel (so legacy aliases like
`AlertTriangle`→`triangle-alert.js` stay correct). Entry bundle 5,586,699 → 3,814,586 bytes
(−1,772,113, −31.7%), measured with the #968 machinery on fresh exports both sides; the budget
baseline was ratcheted down to hold it. The export-map obstacle below was sidestepped: relative
per-icon paths bypass the exports map entirely, so no resolver override was needed.


The clean export attributed approximately **585,934 generated bytes** to the Lucide barrel inside
the **5,589,476-byte** entry bundle; only 83 distinct icons are used while the package ships 3,490
icon modules. A one-attempt Babel/Metro private-specifier rewrite was tested, but the installed
package exports only `.` and `./icons`; Metro failed on `dist/esm/icons/settings2.mjs` (with
package-export warnings before failure). The transform was removed without changing source
imports. Bundle reduction and startup impact are therefore **unmeasured**; a future fix requires
an upstream export-map change or a maintained local wrapper/package.


A normal release route, with the performance fixture not mounted, was captured on eDP-1 (scale
2.0, 60.012 Hz, logical 1890x1114 / estimated physical 3780x2228), native Wayland. It still showed
a first-paint-region spike: startup-to-interactive 405 ms, max frame 134 ms at ~315 ms, 16 dropped
frames, and zero long tasks. The fixture's module-scope 10,000-row construction therefore is not
the sole cause of the stall. The earlier fixture figures remain fixture-route measurements, not
normal-app figures; the normal route confirms a real startup/first-raster cost, while the exact
remaining contributor (raster, font/asset load, or bundle evaluation) is not yet isolated.

## Release fixture frame-stall finding

The isolated release measurements on eDP-1 (scale 2.0, 60.012 Hz, logical 1890x1114 /
estimated physical 3780x2228, native Wayland) show the largest observed frame spikes occur
during the initial fixture phase rather than in a JavaScript long task. Idle measured 125 ms at
~312 ms after monitor start; programmatic scroll measured 181 ms at ~342 ms; streaming measured
180 ms at ~349 ms. All three recorded zero long tasks. This is evidence of a compositor/paint/
raster or startup-resource stall, not a median-frame problem.

## AppImage XWayland packaging defect (fixed)

**Finding (verified):** the locally-built release binary mapped as native Wayland (`xwayland=false`),
while the packaged AppImage child mapped through XWayland (`xwayland=true`) on the same session and
output. These are different rendering and input stacks; their performance figures must not be
combined or compared. Native-binary figures describe GTK/WebKitGTK Wayland; AppImage figures
currently describe XWayland.

**Investigation:** the AppImage's packaged AppRun/AppDir path does not preserve the native Wayland
launch environment. The AppImage child is `Forge-desktop`/XWayland while the direct binary is
`forge-desktop`/Wayland. The packaging path needs explicit Wayland backend handling and dependency
review (AppRun environment, `GDK_BACKEND`, and bundled GTK/WebKit libraries) before shipping.

**Status:** release-blocking packaging defect; no AppImage number is comparable to the native
binary until the packaged child is verified `xwayland=false`.

actions the user expects to be auto-approved.

**What we know (verified in code):** `permission::decide_mode` for `AcceptEdits` auto-allows
`Write` side effects and **gates `Shell` with a prompt by design**; read-only never prompts. The
`ask_user` virtual tool always prompts regardless of temper (it's a question to the user, not a
side effect). So a turn that runs shell commands or calls `ask_user` will still prompt in
auto-edit — that part is expected.

**Verified (file edits do NOT prompt):** the end-to-end test
`auto_edit_allows_file_writes_without_prompting` (forge-core) drives a live `AcceptEdits` session
whose model calls `write_file` with a presenter that *denies* any prompt; the file is still
written, proving the write was auto-allowed without a confirm. `--mode` sticks
(`build_session_with`: `config.permission_mode = m.into()` → `Session.mode`), and with no matching
allow/ask rule `decide` falls back to `decide_mode(AcceptEdits, Write) = Allow`.

**Residual (by design):** a live SHIFT+TAB temper switch applies on the **next** turn, not the
in-flight one. A configured `ask`/`deny` rule for `write_file` also still prompts (rules outrank
the mode by design).

**Status:** common case verified + regression-tested; only the by-design residual remains.

## No way to remove / disable a provider key or model

**Symptom:** Once a provider key is set (env or keyring) there is no command to remove
it or to disable a specific provider/model. Workaround used in practice: set the key
to a junk value so auth fails and the mesh benches/avoids it.

**Shipped:**
- `forge auth --remove <provider>` deletes the keyring entry (idempotent — reports if nothing
  was stored).
- `[mesh] disabled = ["openai", "gemini::antigravity-preview-05-2026"]` excludes a provider
  or exact model id from discovery and routing.
- `forge models --clear` wipes stale model benches.

**Status:** shipped + tested.

## Shell tool: Windows execution (fixed) — denylist OS-awareness (fixed)

**Was:** the `shell` tool ran `sh -c <command>`, which doesn't exist on Windows, so shell
commands wouldn't run there at all.

**Fixed:** `shell` now selects the OS shell — `sh -c` on Unix, `cmd /C` on Windows
(`shell_invocation()` in `forge-tools/src/shell.rs`). The rest of the path (null stdin, capture,
timeout-kill) was already cross-platform. Windows exec tests (`mod exec_windows`) run on the
`windows-latest` CI runner: echo+exit, non-zero exit, timeout-kill (`ping -n`), bad-cwd spawn
failure.

**Also fixed:** the catastrophic denylist now includes Windows-specific dangerous commands:
`del /s`, `del /f /s`, `rd /s`, `rmdir /s`, `format ?:*` — added to `builtin_deny_rules()` in
`forge-config/src/lib.rs`. The `inner_script` unwrapper in `permission.rs` also handles
`cmd /C "<command>"` so patterns are checked recursively inside cmd-wrapped calls.

**Also fixed:** the hooks system now uses the same OS-appropriate shell as the shell tool
(`hook_shell()` in `forge-core/src/hooks.rs`: `sh -c` on Unix, `cmd /C` on Windows).

**Status:** all three items shipped + tested.

## Racy startup hang with a real provider in a minimal container (fixed)

**Was:** in a fresh/minimal container (Docker, no desktop), `forge run` with a REAL provider
occasionally printed only `● session <id>` then hung until killed. Did NOT reproduce with `--mock`
(completes, rc=0), did NOT reproduce on a full host or a fresh-HOME host, and **vanished under
`strace`** (the run then exited 0) — the classic signature of a CPU-scheduling-sensitive race.

**Root cause:** the background lattice auto-index at `forge-cli/src/cli/commands/run.rs` ran the
**synchronous, CPU-bound** `Lattice::update()` (walks the repo, tree-sitter-parses every file,
writes SQLite) inside a plain `tokio::spawn`. That occupies a tokio *worker* thread for the whole
walk. On a machine with few cores the multi-thread runtime is sized to `num_cpus`, so the indexer
starved the executor and the first turn's `route_hinted` never got scheduled → the hang right after
`● session`. `strace` perturbed scheduling enough to let the tasks interleave, hence the "vanishes
under strace" tell. Amplified by `forge-store`'s then-single blocking `Mutex<Connection>` (since replaced by an
`r2d2` pool, #308; see [backlog](#deferred-store-connection-pool)).

**Fixed:** the indexer now runs on the blocking pool via `tokio::task::spawn_blocking`, so worker
threads stay free for the agent turn regardless of core count. `scripts/e2e-docker.sh` keeps the
`E2E_REAL=1` probe to guard against regressions.

## Panic when the system has no CA certificates (fixed)

**Was:** on a stripped system/container with no `ca-certificates` installed, the genai/reqwest
HTTPS client build panicked: `Failed to build reqwest client: … No CA certificates were loaded from
the system`. A user on such a system saw a raw panic, not a clear error.

**Fixed:** `build_reqwest_client()` in `forge-provider/src/genai_provider.rs` now builds a
`reqwest::Client` with `tls_certs_only()` seeded from the bundled `webpki-root-certs` crate
(Mozilla root CAs compiled into the binary) and passes it to genai via `Client::builder()
.with_reqwest(…)`. The platform verifier (`rustls-platform-verifier`) is bypassed entirely, so
HTTPS no longer depends on the OS certificate store. Both `build_client()` (the main provider
client) and `list_models()` (auto-discovery) use this path.

Hardened further: (1) `GenAiProvider`'s derived `Default` was a latent landmine — it built
genai's *own* default client (which calls `rustls-platform-verifier` and panics on a CA-less host);
`Default` now routes through `GenAiProvider::new()` so every Forge-constructed genai client uses the
bundled-roots path. (2) A reusable `forge_provider::bundled_http_client()` was exported and the
remaining `reqwest::Client::new()` HTTPS sites in the CLI (update-check, balance, context-windows,
benchmarks, MCP, remote, local) now use it, so secondary commands no longer panic on a bare system
either.

**Update — gap closed:** `forge-index/src/embed.rs` now has its own `bundled_ca_client()`
(`webpki-root-certs`), and `forge-mcp/src/transport.rs` has `bundled_client_builder()`, used by both
the streamable-HTTP transport and the OAuth flow (`forge-mcp/src/oauth.rs`). No remaining
`reqwest::Client::new()` sites in either crate.

<a id="deferred-store-connection-pool"></a>
**Related — store connection contention (RESOLVED, #308, v0.4.67):** `forge-store` used to wrap a
single SQLite connection in one blocking `std::sync::Mutex`, shared by the agent turn, the background
indexer, and the file watcher — serializing those actors and amplifying the startup hang above. It is
now an **`r2d2` connection pool**: WAL-backed file DBs serve concurrent reads from separate pooled
connections (writes still serialize on SQLite's one-writer rule, waiting on `busy_timeout`); the
in-memory store is pinned to one connection. Covered by an 8-thread concurrency test.

**Status:** fixed + full workspace builds clean; clippy clean; 286 forge-core/forge-provider tests
pass.
</content>

## Normal-route measurement correction

The previously reported **405 ms startup, 134 ms maximum frame at ~315 ms, and 16 dropped
frames** did not measure the user's connected application surface. They measured an **unconnected
empty shell with the changelog sheet covering it**: no server, no session list, no transcript, and
no chat. Those numbers are retained as an explicitly labeled empty-shell capture and must not be
called an app baseline. The real connected application's first paint remains **unmeasured**.

The capture harness now supports `FORGE_PERF_SEED_UPDATE_SEEN=1`, which seeds the existing
`forge.lastSeenBuild.v1` AsyncStorage marker before the update notice is evaluated. This is a
gated capture aid; it does not remove or weaken the user-facing update feature.

## Unattended-launch and version-notice defects (gate fixed; labelling open)

The empty-shell capture exposed a real automation/UX defect: an unattended desktop launch can land
behind a changelog/update gate rather than in a usable connected state. The capture cannot proceed
until the gate is seeded and a real server/session is connected.

The screenshot also showed two distinct version sources at once: the updater banner reported
**2.12.1 ready to install**, while the installed-build notice reported **2.6.6**. This is not one
number being formatted two ways: `OperationalNotice` renders the updater plugin's available
release version, while `UpdateNotice` renders the installed app version from `useAppVersion`.
The notices are therefore internally consistent with different sources, but the unqualified
presentation is confusing and is recorded as a version-notice UX defect requiring explicit labels.

## Parity guard live catches

The parity inventory guard has now caught undeclared platform drift twice on live pull requests:
first for the performance fixture/performance library, and second for `UpdateNotice.tsx` and
`auth.tsx` after the runtime update-seen and pairing-seed instrumentation was added. Both catches
were genuine inventory omissions, not synthetic negative tests or checker loopholes; the files are
being declared with their desktop capture-instrumentation boundary rather than weakening the guard.
