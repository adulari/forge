#!/usr/bin/env bash
set -euo pipefail

# Exercises migration-version-guard.sh against a synthetic repository, so every branch of the guard
# is proven — including the failure paths, which is the point. A guard that has only ever passed is
# not known to work.

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
guard="$script_dir/migration-version-guard.sh"
scratch=$(mktemp -d)
trap 'rm -rf -- "$scratch"' EXIT

failures=0

# Write lib.rs + migrations.rs describing a schema at `version` with `count` registered steps.
write_schema() {
  local version=$1 count=$2
  mkdir -p crates/forge-store/src
  printf 'const SCHEMA_VERSION: i64 = %s;\n' "$version" > crates/forge-store/src/lib.rs
  {
    printf 'pub(super) const MIGRATIONS: &[fn()] = &[\n'
    for ((i = 1; i <= count; i++)); do printf '    migration_%04d,\n' "$i"; done
    printf '];\n'
  } > crates/forge-store/src/migrations.rs
}

expect() {
  local want=$1 name=$2 main_ref=$3 head_ref=$4
  local got=0
  MAIN_REF="$main_ref" HEAD_REF="$head_ref" bash "$guard" >/dev/null 2>&1 || got=$?
  if [ "$got" -ne "$want" ]; then
    echo "FAIL: $name — expected exit $want, got $got" >&2
    failures=$((failures + 1))
  else
    echo "ok: $name"
  fi
}

cd "$scratch"
git init -q .
git config user.email t@example.com
git config user.name t

# Mainline at schema 24.
write_schema 24 24
git add -A && git commit -qm "main at 24"
git branch mainline-24

# Mainline advances to 25 — someone else's migration landed first.
write_schema 25 25
git add -A && git commit -qm "main at 25"
git branch mainline-25

# A branch cut from 24 that adds its own migration 25: the exact 2026-08-06 collision.
git checkout -q mainline-24
git checkout -qb collision
write_schema 25 25
git add -A && git commit -qm "branch adds its own 25"

expect 1 "collision: branch claims 25 while mainline is already at 25" mainline-25 collision
# The merge-base comparison that would MISS it, kept as a demonstration that the ref choice is
# load-bearing: against the 24 base this same branch looks perfectly correct.
expect 0 "same branch looks fine against its merge base (why the guard uses mainline)" mainline-24 collision

# Correctly renumbered on top of the mainline's 25.
git checkout -q mainline-25
git checkout -qb renumbered
write_schema 26 26
git add -A && git commit -qm "renumbered to 26"
expect 0 "renumbered to 26 over mainline 25" mainline-25 renumbered

# A branch that changes nothing about the schema must not be flagged.
git checkout -q mainline-25
git checkout -qb unrelated
echo "// unrelated change" >> crates/forge-store/src/lib.rs
git add -A && git commit -qm "no schema change"
expect 0 "branch with no migration change" mainline-25 unrelated

# SCHEMA_VERSION and the registered step count disagreeing.
git checkout -q mainline-25
git checkout -qb inconsistent
write_schema 27 26
git add -A && git commit -qm "version ahead of the array"
expect 1 "SCHEMA_VERSION ahead of the MIGRATIONS array" mainline-25 inconsistent

# Walking the number backwards over a shipped step.
git checkout -q mainline-25
git checkout -qb backwards
write_schema 24 24
git add -A && git commit -qm "walks back to 24"
expect 1 "SCHEMA_VERSION behind the mainline" mainline-25 backwards

if [ "$failures" -ne 0 ]; then
  echo "$failures migration-version-guard case(s) failed" >&2
  exit 1
fi
echo "migration version guard behaves correctly on every case"
