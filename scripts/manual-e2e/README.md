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

Runs are retained under `scripts/.manual-e2e-out/` and ignored by Git. Set `FORGE_BIN` to test a
specific binary, `FORGE_E2E_TIMEOUT` to change the 1,500-second turn timeout, or
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

The reference directory is evidence and something to inspect or run manually; the fixture is the
replayable pre-fix starting point. Aetherfront intentionally starts from an empty workspace because
the assignment is generative. Its `verify.js` launches real headless Chrome, exercises the title,
tutorial, start, pause/resume, simulation, entity counts, and runtime-error path, then saves a
screenshot.

Provider-backed runs consume real quota. The default is mesh-unpinned; set `FORGE_BIN`, not a model,
when comparing development binaries so routing and failover remain part of the test.
