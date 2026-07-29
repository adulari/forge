# Code archaeology: subscription quota persistence

## Summary

Subscription quota state is not ordinary usage accounting. It is a routing input with snapshot freshness, append-only pace history, and alias semantics for transports sharing one account. The complete quota persistence/read model is now isolated so CLI, TUI, API, and Mesh consumers continue to derive one canonical view.

## History and invariants

- `ddac4631` introduced quota snapshots and routing pressure: current server windows are authoritative and expired windows must not influence selection.
- `dab76aa6` added quota pace from bounded history.
- `31d9f527` established that `codex-cli` and `codex-oauth` share one ChatGPT allowance. Their latest observation wins per window and is duplicated to both surfaces, never summed.
- `5ab839ce` and `078d4acb` hardened source timestamps, stale-arrival rejection, rollout-history deduplication, and direct OAuth freshness.
- `8dab3baa` added bounded privacy-preserving Mesh outcome calibration alongside quota routing evidence.

## Boundary

`quota_store.rs` owns:

- current subscription snapshots and append-only fraction history;
- stale-observation rejection and duplicate-history suppression in one retried transaction;
- atomic shared-account alias, live plan, and direct OAuth freshness-marker updates;
- shared-account alias expansion and per-window latest-wins merging;
- pace computation from bounded history;
- direct Codex OAuth freshness gating;
- bridge-fraction projection used by UI and routing;
- privacy-preserving Mesh outcome recording and bounded calibration.

Model health/failover and duel learning now have separate routing-persistence modules. The parent store retains schema/open lifecycle, sessions, messages, spending, and other persistence domains.

## Interface as test surface

Existing regressions directly characterize the boundary, including:

- `live_codex_account_updates_aliases_plan_and_freshness_together`;
- `record_quota_at_older_timestamp_is_a_noop_newer_overwrites`;
- `codex_alias_group_latest_updated_at_wins_never_sums`;
- `codex_alias_group_merges_per_window_across_providers`;
- `codex_alias_group_exhausted_threshold_shared_across_both_surfaces`;
- `bridge_fractions_share_the_latest_codex_window_with_both_surfaces`;
- `stale_codex_quota_does_not_pressure_either_shared_surface`;
- `only_a_live_oauth_codex_snapshot_advances_the_canonical_freshness_gate`;
- quota-history cutoff/order and pace-projection tests.

Deleting or bypassing the module breaks the public `Store` quota API and these routing-state characterizations.

## Leave alone

- Shared Codex surfaces represent one allowance; update both aliases atomically, merge latest per window, and never add fractions.
- Source-derived quota and plan rows use source observation time; live header observations use receipt time.
- A late stale observation is a complete no-op for snapshot and history.
- Expired or stale Codex windows must not apply routing pressure.
- Direct OAuth quota, plan, aliases, and freshness marker commit together.
- Temporary outcome-recording failure remains best-effort and cannot fail an otherwise successful turn.
