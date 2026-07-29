# Code archaeology: CLI run ownership correction

## Decision

Commit `9400423f` extracted two generic `*_support` modules from `run.rs`. Independent review and history showed that they did not own cohesive domains: their functions served unrelated onboarding, quota, turn-lifecycle, palette, dispatch, and configuration concerns. The correction deliberately restores those helpers to their existing owners rather than retaining line-count-only modules.

## Owner map

- First-run onboarding remains with the run entrypoint and its existing setup/local-command dependencies.
- Subscription quota ingestion, staleness checks, probe orchestration, and overlay projection stay co-located in the run flow until a complete quota capability owns the full snapshot-to-session-to-view lifecycle.
- Generation-tagged `DoneGuard` stays beside the run-loop completion consumer.
- Palette synchronization and quit cleanup remain run-loop state transitions.
- Custom shell and scrollback routing remain adjacent to their only command/dispatch callers.
- Provider-prefix validation calls `forge_config::is_known_provider` directly; no CLI pass-through wrapper exists.
- `/config` row mapping stays with its controller until the editor lifecycle is extracted as a whole.

## Invariants

- `--mock` must not initiate onboarding or real provider probes.
- Claude quota probes are best-effort and staleness-gated; session quota remains normalized before view projection.
- A completion generation only releases its matching turn.
- Quitting clears prompt senders before awaiting session state.

## Verification

The corrective change runs focused CLI tests, Clippy, the architecture-size guard, and diff validation. It intentionally does not claim a deep module extraction: deleting the reverted generic modules restores behavior without moving an artificial boundary.
