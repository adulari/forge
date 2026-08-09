#!/usr/bin/env bash
set -euo pipefail

# Refuse a branch that claims a schema migration number already taken on the mainline.
#
# On 2026-08-06 five PRs branched from main at schema 24 and each added its own "migration 25":
# #973 harness, #975 heartbeats, #976 fleet messages, #977 detached children, #980 presence
# columns. The first to merge took 25; the other four went DIRTY and were rebased and renumbered by
# hand, one at a time. Nothing detected the collision until git conflicted — and it only conflicted
# because they happened to touch adjacent lines. Two migrations added in different files, or in a
# different order, could have merged cleanly and left `MIGRATIONS` inconsistent with
# `SCHEMA_VERSION`, which is only caught later by a debug assertion inside `run_migrations`.
#
# THE COMPARISON MUST BE AGAINST CURRENT MAINLINE, NOT THE MERGE BASE. In the case above every
# branch's merge base was 24, so a merge-base comparison sees "24 → 25, fine" for all five and
# reports nothing. Only the mainline's present value shows that 25 is already spoken for.
#
# Usage:
#   scripts/ci/migration-version-guard.sh            # compares HEAD against origin/main
#   MAIN_REF=<ref> HEAD_REF=<ref> scripts/ci/...     # explicit refs (used by the tests)

main_ref=${MAIN_REF:-origin/main}
head_ref=${HEAD_REF:-HEAD}

LIB=crates/forge-store/src/lib.rs
MIGRATIONS=crates/forge-store/src/migrations.rs

# `SCHEMA_VERSION` as declared at a ref. Empty when the file or constant is absent, which the
# caller treats as "nothing to compare" rather than as zero.
schema_version_at() {
  git show "$1:$LIB" 2>/dev/null |
    sed -n 's/^const SCHEMA_VERSION: i64 = \([0-9]*\);/\1/p' |
    head -1
}

# How many steps the ordered MIGRATIONS array actually lists at a ref.
migration_count_at() {
  git show "$1:$MIGRATIONS" 2>/dev/null |
    sed -n '/^pub(super) const MIGRATIONS/,/^];/p' |
    grep -c '^    migration_' || true
}

main_version=$(schema_version_at "$main_ref")
head_version=$(schema_version_at "$head_ref")
head_count=$(migration_count_at "$head_ref")
main_count=$(migration_count_at "$main_ref")

# "Did THIS branch add migrations?" is answered against the merge base, not the mainline: in the
# collision both sides carry the same number of steps (each added one to a shared base of 24), so
# comparing counts with the mainline sees no difference at all. The mainline is still what decides
# whether the resulting NUMBER is free — the two refs answer two different questions.
base_ref=$(git merge-base "$main_ref" "$head_ref" 2>/dev/null || echo "")
base_count=${main_count}
if [ -n "$base_ref" ]; then
  base_count=$(migration_count_at "$base_ref")
fi

if [ -z "$head_version" ] || [ -z "$main_version" ]; then
  echo "migration guard: no SCHEMA_VERSION at $head_ref or $main_ref — nothing to compare"
  exit 0
fi

fail() {
  echo "migration version guard failed:" >&2
  echo "  - $1" >&2
  exit 1
}

# 1. Internal consistency. `run_migrations` debug-asserts this, but a release-profile run would
#    not, and a clear message here beats an assertion failure inside a test binary.
if [ "$head_version" -ne "$head_count" ]; then
  fail "SCHEMA_VERSION is $head_version but MIGRATIONS lists $head_count steps — every step must be registered and counted"
fi

# 2. The collision this exists for: the branch adds migrations without advancing past what the
#    mainline already uses.
if [ "$head_count" -gt "$base_count" ] && [ "$head_version" -le "$main_version" ]; then
  fail "this branch adds migrations but SCHEMA_VERSION is $head_version and $main_ref is already at $main_version — renumber the new migration(s) to $((main_version + 1)) and up, and bump SCHEMA_VERSION to match"
fi

# 3. A branch that lowers the number is either stale or renumbering backwards over a shipped step.
#    Shipped migrations are immutable by contract, so this is always wrong.
if [ "$head_version" -lt "$main_version" ]; then
  fail "SCHEMA_VERSION is $head_version but $main_ref is at $main_version — rebase; a shipped migration number is never reused or walked back"
fi

echo "migration version guard: SCHEMA_VERSION $head_version ($head_count steps) is consistent with $main_ref at $main_version"
