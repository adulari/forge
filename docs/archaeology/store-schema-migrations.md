# Code Archaeology: Store schema migrations

## Summary

The store root owned two distinct persistence lifecycles: runtime data access and database-shape evolution. Versioned migrations, legacy compatibility repairs, singleton seeding, and schema-version refusal form one cohesive startup boundary. They are now isolated without changing migration order or the `Store::build` transaction sequence shared by file-backed and in-memory stores.

## Timeline and invariants

- `078d4acb` introduced the pre-schema `subscription_usage` primary-key repair. It must run before `schema::SCHEMA`, whose `CREATE TABLE IF NOT EXISTS` cannot reshape an existing table.
- `cf866b8d` replaced ad-hoc migration writes with ordered `PRAGMA user_version` steps, duplicate-sequence repair, and explicit failure propagation.
- `693e4ca0` added the tool-path backfill; malformed or truncated historical arguments remain non-fatal.
- `e3708644` through `70ecdb87` added remote/session state while preserving machine-local fields during restore and handoff.
- `380155eb` made opening an already-current database write-free and repaired the ambiguous pre-release Anywhere version window without silently downgrading a genuinely newer public database.
- `a316c532` separated runtime persistence domains but retained migrations in the root because their tests and startup wiring still depended on private helpers.

## Boundary

`migrations.rs` owns:

- compatibility repair needed before base-schema creation;
- the immutable ordered migration table;
- idempotent migration implementations and schema-version advancement;
- ambiguous pre-release Anywhere version detection and repair;
- singleton-row seeding that avoids writes on an initialized database.

`Store::build` remains the lifecycle coordinator used by both `Store::open` and `Store::open_in_memory`. Its ordering is unchanged: configure connection, perform the pre-schema compatibility repair, apply the base schema, seed singleton rows, run versioned migrations, then perform the existing Anywhere additive-column repair.

## Interface as test surface

The existing store regression suite calls the migration boundary directly and characterizes:

- `rejects_db_from_a_newer_build` and `a_newer_public_database_inside_the_window_is_refused_not_downgraded`;
- `prerelease_anywhere_versions_are_repaired_and_renumbered`;
- `run_migrations_writes_nothing_when_the_database_is_already_current`;
- `migration_0008_applies_to_a_v7_db_and_is_idempotent` and `migration_0018_preserves_v17_usage_and_adds_cached_input_tokens`;
- `a_database_written_by_this_build_survives_an_older_binarys_rewind_to_17`;
- `singleton_rows_are_seeded_once_and_never_duplicated` and `push_subscription_endpoint_has_a_unique_index`.

Only those test-facing migration symbols are exposed to the parent module, and the module remains crate-private.

## Safe changes

- Append a new idempotent migration and bump `SCHEMA_VERSION` in lockstep.
- Add compatibility detection when history proves a released or pre-release schema collision.
- Add direct migration regression tests using a database shaped like the historical version.

## Leave alone

- Never reorder or rewrite a shipped migration.
- Never trust an ambiguous version number without a structural marker.
- Never turn opening an initialized current database into a write.
- Never swallow migration failures other than a specifically recognized idempotent condition.
- Preserve the pre-schema `subscription_usage` repair and `Store::build` ordering.
