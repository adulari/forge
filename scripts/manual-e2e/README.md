# Forge manual E2E scenarios

These are the durable versions of useful real-world TUI tests developed while hardening Forge.
They contain no credentials, provider responses, Forge database, or session history. Each runnable
scenario has its original broken fixture, exact prompt, acceptance checks, and a saved successful
reference result.

Run a scenario automatically through the real TUI and unpinned mesh:

```bash
scripts/manual-e2e/run.sh rust-transaction-ledger
```

Open a normal interactive TUI with the fixture prepared and the prompt printed:

```bash
scripts/manual-e2e/run.sh aetherfront --manual
```

Locate a saved result without running a provider:

```bash
scripts/manual-e2e/run.sh aetherfront --reference
```

Runs are retained outside disposable worktrees under
`${XDG_DATA_HOME:-$HOME/.local/share}/forge/manual-e2e-runs/`. Each automatic run keeps the exact
fixture result, TUI transcript and progress timeline, harness summary with its resumable Forge
session ID, and a credential-free persisted tool-envelope integrity report. Set `FORGE_BIN` to test
a specific binary, `FORGE_E2E_TIMEOUT` to change the 1,500-second turn timeout, or
`FORGE_MANUAL_E2E_OUT` to store runs elsewhere. By default the runner prefers this checkout's
`target/debug/forge`, so it exercises the current development build.

## Saved scenarios

| Scenario | What it stresses | Reference result |
| --- | --- | --- |
| `aetherfront` | Large coherent tool writes, browser UI/game behavior, long-turn progress | Playable single-file Canvas RTS plus screenshot |
| `multifile-reservations` | Python async races, rollback, validation, idempotency | Corrected package with 8 passing tests |
| `go-ordered-pipeline` | Ordering, panic attribution, cancellation, backpressure, race detector | Corrected bounded concurrent pipeline |
| `typescript-config-recovery` | Broken build config, package exports, secure deep merge, public API | Strictly typed package with passing Node tests |
| `rust-transaction-ledger` | Transactional rollback, overflow, idempotency conflicts, stable ordering | Corrected standard-library Rust crate |
| `interrupt-resume-large-write` | Forced mid-turn interruption, large tool arguments, same-session recovery | Strictly verified 321-line artifact retained with the run |
| `long-session-reservations` | Six FIFO reprompts in one mesh-auto session: concurrency, rollback, cancellation, skeptical review, final verification | Live-only; visible suite plus exact hidden 100-way contention, duplicate, cancellation, deterministic reserve/cancel interleaving, and rollback checks |

The reference directory is evidence and something to inspect or run manually; the fixture is the
replayable pre-fix starting point. Aetherfront intentionally starts from an empty workspace because
the assignment is generative. Its `verify.js` launches real headless Chrome, exercises the title,
tutorial, start, pause/resume, simulation, entity counts, and runtime-error path, then saves a
screenshot. Each Aetherfront run also keeps a machine-readable `screenshot.verification.json`
beside the PNG.

Provider-backed runs consume real quota. The default is mesh-unpinned; set `FORGE_BIN`, not a model,
when comparing development binaries so routing and failover remain part of the test.

## Matched native long-session runner

`native_long_session.py` drives Codex CLI or Claude Code one turn at a time so an external quota
refresh can be recorded before and after every provider call. `prepare` creates one synthetic base
commit, removes remotes and generated caches, records the exact Git tree hash, and writes a
resumable `run-state.json`. `turn` fails closed when quota evidence is missing, stale, out of order,
decreasing within the same weekly window, or at the cap; it also denies a native Claude call while
the local CLI is logged out. A provider-started failure is retained and blocks duplicate calls.
Codex CLI's thread-cumulative usage snapshots are retained verbatim and differenced before per-turn
or aggregate token reporting.

```bash
python3 scripts/manual-e2e/native_long_session.py prepare \
  --provider claude --model 'opus[1m]' --effort high \
  --out-root /absolute/artifact/root --quota-baseline 27 --quota-cap 5

python3 scripts/manual-e2e/native_long_session.py record-quota \
  --run-dir /absolute/run/dir --weekly 28 \
  --external-observed-at 2026-07-28T04:11:08Z

python3 scripts/manual-e2e/native_long_session.py turn \
  --run-dir /absolute/run/dir --timeout 1500
```

Repeat the external refresh, `record-quota`, and `turn` sequence for all six prompts, then refresh
and record quota once more before `finalize`. A zero-token expired-OAuth failure is never retried
automatically. After the operator restores Claude login, `recover-auth` unlocks only that narrowly
proven failure and retains its time in the final attempt ledger. Zero-event transport failures do
not qualify as free parser failures.

## Provider cache probe

`cache_session_probe.js` runs two real headless turns against one explicitly selected model. The
first turn sends a long, meaningful stable prefix; the second resumes the same Forge session and
checks exact responses, session identity, usage telemetry, and provider-reported cached input. Its
JSON report is retained beside the run under Forge's persistent `manual-e2e-runs/` directory.

```bash
FORGE_BIN=/home/floris/.local/bin/forge \
  scripts/manual-e2e/cache_session_probe.js qwencloud::qwen3.8-max-preview --require-cache-hit

scripts/manual-e2e/cache_session_probe.js \
  nvidia::nvidia/nemotron-3-super-120b-a12b

scripts/manual-e2e/cache_session_probe.js gemini::gemini-3.5-flash --require-cache-hit
```

Use `--require-cache-hit` only where the provider exposes cache-read accounting. Without that flag,
the probe still requires two successful resumed turns and complete usage telemetry, while recording
zero if an otherwise compatible provider does not report cache hits.

## Interrupt/resume large-write probe

This scenario intentionally interrupts Forge while a model is producing a large one-call
`write_file` payload, resumes the same session, and rejects partial, malformed, or misnumbered
output. It needs `jq` in addition to the normal Python/TUI prerequisites.

```bash
FORGE_BIN=/home/floris/.local/bin/forge \
FORGE_MODEL=qwencloud::qwen3.8-max-preview \
  scripts/manual-e2e/run.sh interrupt-resume-large-write
```

Set `FORGE_E2E_INTERRUPT_AFTER` to adjust the default 25-second fault-injection point. Both TUI
timelines, the interrupted session ID, resumed transcript, workspace, verified artifact, and a
credential-free persisted tool-envelope integrity report remain under Forge's persistent
`manual-e2e-runs/` directory.
