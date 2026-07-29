# Code archaeology: model discovery and Codex quota source

## Boundaries

Two more owners split out of the models command (which already delegated its rendering to
`models/presentation.rs`):

- `models/discovery.rs` — which models are actually usable, and remembering the answer: the
  per-provider discovery calls under their time budget, affordability filtering, and the on-disk
  catalog cache with its expiry and explicit invalidation.
- `models/codex_quota.rs` — where a Codex quota reading comes from: the source preference, the
  staleness rules, the CLI rollout seed, and the refresh entry point.

`models.rs` keeps composition (provider/router construction, classifier selection, outcome
calibration) and the user-facing commands.

## The rules each owner exists to hold

Discovery is *time-bounded and cached on purpose*: a slow or unreachable provider must not stall a
launch, and paying full discovery on every command would be absurd — so the cache expires, and
anything that changes which models exist invalidates it explicitly rather than waiting for the
clock.

Codex quota has a *preference order*, not just a value: a direct OAuth header observation
describes the account Forge is about to use, while the local CLI's rollout log can be fresh yet
belong to a different account or carry an obsolete plan. The CLI is therefore a no-cost fallback
and never a reason to skip a due OAuth probe.

## Interface

`models.rs` re-exports `discover_catalog`, `load_cached_catalog`, `save_catalog`,
`invalidate_catalog_cache`, and `refresh_codex_quota`, so every caller across the CLI, serve, and
mesh paths is unchanged. `drop_unaffordable_models` narrowed to discovery-internal, its only use.

## Characterization

The quota source truth table and the rollout-seed timestamp-preservation test moved with their
code; discovery and mesh-smoke tests stay with the command. Both `models` and `codex` test
selections pass.
