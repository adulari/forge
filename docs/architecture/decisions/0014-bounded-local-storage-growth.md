# ADR-0014: Bound local build-cache and sync-staging growth

- **Status:** Accepted
- **Date:** 2026-07-30
- **Deciders:** Forge maintainers

## Context

Forge's self-hosted CI runners deliberately keep Cargo output between jobs, and write-capable
subagents deliberately share the parent repository's Cargo target directory. Cargo does not
garbage-collect obsolete build hashes. A workstation incident left hundreds of gigabytes in
runner `target/` directories before manual cleanup. The existing weekly timer was insufficient:
its installed version searched only four levels deep, did not include `node_modules`, and an
actively used target directory never becomes old enough for age-only cleanup. The same runner
also accumulated unbounded npm content and `npx` package caches under `~/.npm`.

The same audit found a separate unbounded database path. A live 1.50 GiB `forge.db` contained:

- 40,480 fully acknowledged `sync_journal` rows with 485.1 MiB of payload;
- 40,480 downloaded `anywhere_sync_remote` rows with the same 485.1 MiB of payload;
- terminal apply outcomes for every downloaded row (`superseded`, no unresolved conflict);
- 7,929 mutable snapshots for 977 sessions, including 2,337 revisions for one session.

The sync worker removed temporary ciphertext after acknowledgement but never removed acknowledged
plaintext journal revisions or terminal download staging.

## Decision

Forge uses independent bounds for the two storage owners:

1. Every cache-producing persistent self-hosted workflow runs
   `scripts/ci/trim-runner-cache.sh` in an `always()` step. It removes only allow-listed cache
   roots after the job finishes and only when an aggregate exceeds its hard budget: 16 GiB for
   workspace Cargo targets, 24 GiB for the four named release-build Docker volumes, 4 GiB for
   mobile `node_modules`, 4 GiB for npm's content-addressed cache, and 1 GiB for `npx` package
   installs. Before trimming `npx`, the script preserves entries referenced by a live process
   command line, environment, working directory, or executable; an active entry may therefore
   keep that cache temporarily above its target. The script rejects `/`, rejects a symlinked npm
   cache root, ignores symlinked cache directories, refuses ambiguous boolean controls, and has an
   executable destructive-behavior test. Release Docker volumes are trimmed in one downstream job
   after both parallel platform builds finish, so an in-use shared cache is never force-removed.
2. Each successful Anywhere sync pass deletes at most 1,000 acknowledged local revisions older
   than the newest `(record_kind, stable_id)` revision and at most 1,000 remote staging rows whose
   outcome is `applied` or `superseded`.
3. The newest local revision remains the revision/logical-clock and idempotency anchor. Pending
   uploads remain untouched. Remote conflicts remain staged for explicit inspection. The durable
   download cursor and materialized/domain rows are never deleted by this maintenance path.
4. SQLite file compaction remains explicit. Row pruning releases database pages, while `VACUUM`
   rewrites the full file and can stall a long-lived daemon; an operator should run it only after
   stopping other Forge processes.

## Evidence

- `terminal_sync_pruning_keeps_latest_revision_and_unresolved_conflicts` covers acknowledged
  revision compaction, terminal remote deletion, conflict retention, duplicate retry, and next
  revision creation.
- `scripts/ci/test-trim-runner-cache.sh` covers oversized Cargo, `node_modules`, npm content, and
  inactive `npx` cache deletion and live-process preservation; unrelated-directory and
  unrelated-Docker-volume preservation; dry-run behavior; rejection of filesystem and symlinked
  npm cache roots; exact Docker allow-listing; and workflow wiring.
- The mobile production dependency audit reports zero vulnerabilities after the lockfile update;
  the required `mobile checks` aggregate now includes this audit.

## Alternatives considered

- **Age-only weekly cleanup.** Rejected because a frequently used Cargo target can grow without
  ever becoming stale.
- **Delete every acknowledged local sync row.** Rejected because revision creation and idempotent
  retries need a retained local anchor.
- **Delete every remote row.** Rejected because unresolved conflicts are product state, not cache.
- **Automatic SQLite `VACUUM` in the daemon.** Rejected because it needs temporary disk roughly
  proportional to the database and holds a write lock for an unpredictable interval.

## Consequences

- A busy three-runner host now has a deterministic approximately 48 GiB ceiling for primary
  workspace Cargo caches, plus up to 5 GiB of npm caches per runner account and a separate 24 GiB
  ceiling for release Docker caches, instead of age-dependent unbounded growth.
- Sync staging converges toward newest local anchors plus unresolved conflicts instead of retaining
  duplicate payload history forever.
- Crossing a build-cache cap makes the next job cold; this is an intentional speed-for-disk safety
  tradeoff.
- Existing databases need one explicit `VACUUM` after the first pruning pass to return freed pages
  to the filesystem.
