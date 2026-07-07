# Feature: Quota pace tracking (history + projection + statusline meter)

Status: **built**. Extends L3 (quota-aware routing) in
[`provider-cost-routing.md`](./provider-cost-routing.md#l3--quota-aware-wont-build-now--designed) —
that doc's L3 already ships a *latest-snapshot* view of subscription usage
(`subscription_usage`, one row per provider+window, upserted on every `QuotaHint`). This feature
adds the missing time dimension: a **history** of those snapshots, a **pure projection** derived
from it, and a **statusline meter** that surfaces the projection live.

## JTBD

> As a user on a Claude/Codex subscription plan, I want to see whether my current rate of usage
> will blow through a rolling window *before* it resets, not just how full the window is right
> now — so I can slow down (or switch to a metered model) ahead of hitting the wall, instead of
> being surprised by a hard stop mid-task.

`subscription_usage` answers "how full is the tank right now?". It cannot answer "at this rate,
will I run out before the window resets?" — that requires a series of observations over time, not
one snapshot. This feature adds that series and the arithmetic on top of it.

## Scope

| | |
|---|---|
| **Must** | Append-only history table (`quota_history`) recording every `QuotaHint` observation alongside the existing upsert. A pure `compute_quota_pace` function deriving rate/hour, rate/day, a projected fraction at the window's reset, and an exhaustion warning. A statusline widget (`QuotaPace`) surfacing the projection for whichever window is closest to exhausting. |
| **Should** | Guard against near-zero-elapsed-time samples (right after a window resets) spiking into a false rate/warning. |
| **Won't (this iteration)** | Feeding the projection back into mesh routing decisions (L3 already demotes on the *current* `QuotaStatus`; wiring the *projected* status into routing is a follow-on, not required here). Historical data retention/pruning policy (the table is small — one row per turn's quota hint — so this is deferred until it's actually a problem). |

## Design

### Data flow

```
CLI bridge (Claude Code / Codex)
   │  rate_limit_event / rollout token_count
   ▼
QuotaHint { provider, window, status, resets_at, fraction_used }
   │
   ▼
Store::record_quota(hint)                    (forge-store/src/lib.rs)
   ├─ UPSERT subscription_usage               (unchanged — latest snapshot, mesh routing input)
   └─ INSERT quota_history                    (NEW — append-only, one row per observation)
   │
   ▼ (same turn, forge-core's run loop)
Store::quota_history_since(provider, window, since)
   │
   ▼
forge_types::compute_quota_pace(history, resets_at, now)   (NEW — pure, no I/O)
   │
   ▼
PresenterEvent::QuotaPace { .. }              (NEW)
   │
   ▼
App.quota_pace: Option<QuotaPaceInfo>          (forge-tui, keeps the window closest to exhaustion)
   │
   ▼
StatuslineWidget::QuotaPace render             ("⏱ claude 5h → 118%")
```

### Why `record_quota` does the history insert, not a separate call site

`record_quota` is called from three places (`forge-core::seed_subscription_quota`, the turn-loop
quota-hint handler, `forge-cli::models::seed_store_quota`). Rather than duplicating "also insert a
history row" at all three (and risking one being missed on the next edit), the history append
lives *inside* `Store::record_quota` itself — it is still an additive INSERT alongside the
existing `subscription_usage` UPSERT, not a change to that table's schema or upsert semantics. The
mesh router's `current_quota`/`quota_at` queries are untouched.

### The pure pace function

`forge_types::compute_quota_pace(history: &[QuotaHistoryPoint], resets_at: Option<i64>, now: i64)
-> Option<QuotaPace>` lives in `forge-types` (the workspace's dependency-free leaf crate — see its
module doc, "provider, mesh, tools, store, core and tui all depend on it, it depends on none of
them"). Putting the calculation there, rather than in `forge-mesh` or `forge-store`, keeps it
reachable from any crate without adding an edge to the dependency graph, and keeps it trivially
unit-testable: no clock, no I/O, `now` and the history are both inputs.

Algorithm:
1. Require at least two history points, spanning at least `QUOTA_PACE_MIN_ELAPSED_SECS` (300s) —
   otherwise return `None` ("not enough data yet"). This is the guard against a near-zero
   denominator right after a window resets (two samples seconds apart would otherwise imply an
   absurd rate).
2. `rate_per_sec = (latest.fraction_used - earliest.fraction_used).max(0.0) / elapsed_secs` — a
   window rollover mid-range would show a fraction decrease; clamped to 0 rather than a negative
   rate, since there's nothing sensible to project from a reset in-range.
3. `rate_per_hour`/`rate_per_day` are `rate_per_sec` scaled.
4. `projected_fraction_at_reset = resets_at.map(|r| latest.fraction_used + rate_per_sec * (r -
   now).max(0))` — `None` when the reset time isn't known.
5. `time_to_exhaustion_secs` — seconds until 100% at the current rate (`None` if the rate isn't
   positive, `Some(0.0)` if already at/over 100%).
6. `exhaustion_warning = time_to_exhaustion_secs < time_remaining_in_window` — fires ahead of an
   overrun, not just once the window is already at `QuotaStatus::Warning`.

### Store: `quota_history`

New table (migration #9 in `forge-store`, `SCHEMA_VERSION` 8 → 9), following the exact same
`CREATE TABLE IF NOT EXISTS` + append-to-`MIGRATIONS` idiom as every prior migration in that file:

```sql
CREATE TABLE IF NOT EXISTS quota_history (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    provider      TEXT NOT NULL,
    window_kind   TEXT NOT NULL,
    fraction_used REAL NOT NULL,
    resets_at     INTEGER,
    observed_at   INTEGER NOT NULL DEFAULT (strftime('%s','now'))
);
CREATE INDEX idx_quota_history_lookup ON quota_history(provider, window_kind, observed_at);
```

Deliberately NOT touching `subscription_usage`'s schema, PK, or upsert behavior — the mesh router
depends on that table staying "one row per provider/window, latest wins".

CRUD: `record_quota_history`/`record_quota_history_at` (insert; the `_at` variant takes an
explicit `observed_at` for tests, mirroring `quota_at`'s testable-clock pattern for
`current_quota`), and `quota_history_since(provider, window, since) -> Vec<QuotaHistoryPoint>`
(oldest-first, filtered by cutoff).

### Statusline meter

`crates/forge-tui` already renders a config-driven statusline (`StatuslineWidget` enum in
`forge-config`, dispatched in `forge-tui::app::render_statusline_widget`) with existing
subscription-usage widgets `QuotaClaude`/`QuotaCodex` ("claude N%" / "codex N%", colored by
`ERRRED`/`WARNYEL`/`DIM` thresholds). The new `QuotaPace` widget mirrors that exact look and
color convention: `"⏱ claude 5h → 118%"` (red once `exhaustion_warning`, yellow once the
projection crosses 70%, dim otherwise), or a rate-only fallback `"⏱ claude 5h +12.0%/hr"` when no
reset time is known yet. Like `QuotaClaude`/`QuotaCodex`, it is opt-in (not in the default
statusline layout) — add it via `/statusline toggle quota_pace` (aliased `pace`) or directly in
config. It renders nothing (not a placeholder, not a panic) when `app.quota_pace` is `None` —
i.e. before enough history has accumulated for any subscription, or when there is none at all.

Data path: `forge-core`'s turn loop (where `resp.quotas` are already recorded and pushed as
`PresenterEvent::QuotaUpdate` for the `/usage` overlay) now also calls a new
`Session::emit_quota_pace(hint)` right after `record_quota`, which reads back up to 8 days of
history for that provider+window, calls `compute_quota_pace`, and — if `Some` — emits
`PresenterEvent::QuotaPace`. `forge-tui::App` keeps only the single window "closest to
exhaustion" across however many `QuotaPace` events arrive (a warning always outranks a
non-warning reading; otherwise the higher projected fraction wins).

## Acceptance criteria

```
AC-1  Given a fresh DB (schema_version 8)
      When Store::open() runs
      Then it upgrades to schema_version 9 and creates `quota_history`
      And `subscription_usage`'s schema/PK is unchanged.

AC-2  Given a QuotaHint with fraction_used = Some(f)
      When Store::record_quota(hint) is called
      Then subscription_usage is upserted (unchanged behavior)
      And exactly one new row is appended to quota_history.

AC-3  Given a QuotaHint with fraction_used = None
      When Store::record_quota(hint) is called
      Then no quota_history row is appended (nothing to record a rate from).

AC-4  Given two history points 5 hours apart, 10% -> 20% used, and a reset 2 hours away
      When compute_quota_pace runs
      Then rate_per_hour ≈ 0.02, projected_fraction_at_reset ≈ 0.24, exhaustion_warning = false.

AC-5  Given two history points 1 hour apart, 20% -> 80% used, and a reset 2 hours away
      When compute_quota_pace runs
      Then projected_fraction_at_reset ≈ 2.0 (200%), time_to_exhaustion_secs ≈ 1200s (20 min)
      And exhaustion_warning = true.

AC-6  Given two history points 2 seconds apart (just after a window reset), both near 0%
      When compute_quota_pace runs
      Then it returns None ("not enough data yet") — no spiked rate, no false warning.

AC-7  Given no quota history exists yet for any provider
      When the statusline renders the QuotaPace widget
      Then it renders nothing (None) — no panic, no placeholder text.
```

## Impact

| Layer | File | Change |
|---|---|---|
| Types | `crates/forge-types/src/lib.rs` | `QuotaHistoryPoint`, `QuotaPace`, `QUOTA_PACE_MIN_ELAPSED_SECS`, pure `compute_quota_pace()` + unit tests. |
| Store | `crates/forge-store/src/lib.rs` | `migration_0009` (`quota_history` table), `SCHEMA_VERSION` 8→9, `record_quota` now also appends history, `record_quota_history`/`record_quota_history_at`/`quota_history_since` + tests. |
| Core | `crates/forge-core/src/lib.rs` | `Session::emit_quota_pace(hint)` — reads history, calls `compute_quota_pace`, emits `PresenterEvent::QuotaPace`; called from the turn loop's existing quota-hint handling. |
| Presenter | `crates/forge-tui/src/lib.rs` | `PresenterEvent::QuotaPace` variant; `HeadlessPresenter` ignores it (mirrors `QuotaUpdate`). |
| TUI state | `crates/forge-tui/src/app.rs` | `QuotaPaceInfo` struct, `App.quota_pace: Option<QuotaPaceInfo>`, event handler (keeps the window closest to exhaustion), `W::QuotaPace` render arm. |
| Config | `crates/forge-config/src/lib.rs` | `StatuslineWidget::QuotaPace` variant. |
| CLI | `crates/forge-cli/src/cli/commands/run/dispatch.rs` | `/statusline toggle quota_pace`/`pace` + label mapping. |

## Definition of done

- [x] `quota_history` table + migration, `subscription_usage` schema/upsert untouched.
- [x] `record_quota` additionally appends history (AC-2/AC-3); CRUD + tests in `forge-store`.
- [x] Pure `compute_quota_pace` in `forge-types` with all required scenarios covered: normal pace
      (AC-4), over-pace warning with sane projection/TTL values (AC-5), just-reset near-zero-elapsed
      guard (AC-6).
- [x] `QuotaPace` statusline widget, degrading to nothing without history (AC-7); mirrors the
      existing `QuotaClaude`/`QuotaCodex` styling.
- [x] `cargo build --workspace --all-targets`, `cargo fmt --check --all`,
      `RUSTFLAGS="-D warnings" cargo clippy --workspace --all-targets`, `cargo test --workspace`
      all clean.
- [ ] Feeding the projection into mesh routing (demote ahead of a projected overrun, not just a
      current one) — explicitly out of scope for this iteration; a natural follow-on once this
      history exists.
