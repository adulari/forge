# Code archaeology: model routing persistence

## Summary

The store root mixed transcript persistence with two routing-feedback domains: durable model availability/capability metadata and repository-scoped duel learning. These are now separate owners. The split preserves failover ordering, provider-wide auth benches, bounded capability exclusions, fetched context/pricing overrides, and the exact duel boost shown to users and applied by the router.

## History and invariants

- `85096746` introduced durable model health and failover so transiently unavailable models are skipped and automatically reconsidered after cooldown.
- `b057ddb0` distinguished capability exclusions from transient benches with an `excluded:` reason prefix and a bounded re-probe window.
- `1af35e92` added ordered transient fallbacks for the all-benched case while leaving credential filtering to the caller.
- `06cfd855` added provider-wide authentication benches keyed by the canonical provider bench key.
- `e9012010` persisted fetched context windows to bound turn transcripts.
- `0f4877be` added fetched pricing as accounting overrides.
- `347fac3f` made stored context windows a routing input through `all_model_contexts`.
- `36039137` added repository-scoped duel outcomes and the bounded wins-minus-losses routing boost.

## Boundaries

`model_health_store.rs` owns model/provider benches, capability exclusions, context-window metadata, and pricing metadata. `duel_store.rs` owns the repository-specific candidate outcome ledger, boost projection, and scoreboard projection.

Active in-process model reservations remain in the store root because they are connection/store-identity concurrency guards rather than durable routing evidence.

## Interface as test surface

Existing regressions characterize:

- `bench_is_upsert_and_clear_removes_it`, `bench_persists_across_reopen`, and `benched_model_is_in_snapshot_until_cooldown_elapses`;
- `transient_benches_are_ordered_by_recovery_time` and `exclude_model_benches_long_and_soonest_skips_exclusions`;
- `provider_auth_exclusion_benches_all_of_its_model_aliases`;
- `model_context_round_trips_and_upserts`, `all_model_contexts_returns_every_discovered_window`, and `model_pricing_round_trips_and_upserts`;
- `duel_outcome_roundtrips_and_boost_math_is_correct`, `duel_boost_clamps_at_the_bound_for_a_long_streak`, and `scoreboard_mirrors_duel_boost_math_and_sorts_by_boost`.

Deleting either module removes public `Store` routing APIs and breaks the corresponding characterizations.

## Leave alone

- Capability failures remain bounded exclusions, not permanent deletion.
- Provider authentication failures bench the provider key, not only one model alias.
- Existing transient rows remain eligible to the least-dead fallback even after their cooldown timestamp; callers use this as a last-resort ordering rather than a current-bench snapshot.
- Context windows and prices are fetched overrides and remain independently upsertable.
- Duel boosts stay repository-scoped and clamped to `[-2.0, 2.0]`.
- Scoreboard ordering and values must use exactly the same boost formula as routing.
