#!/usr/bin/env bash
set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
scanner="$script_dir/check-anywhere-plaintext-canary.sh"
scratch=$(mktemp -d)
trap 'rm -rf -- "$scratch"' EXIT

marker='FORGE_ANYWHERE_CANARY_7f86c68d93c84517'
printf '%s\n' "$marker" > "$scratch/markers"
printf 'FANY\001\007encrypted-envelope-without-plaintext\000' > "$scratch/clean.fany"

bash "$scanner" "$scratch/markers" "$scratch/clean.fany" >/dev/null

printf 'metadata-before\n%s\nmetadata-after\n' "$marker" > "$scratch/leaked.sqlite"
set +e
failure_output=$(bash "$scanner" "$scratch/markers" "$scratch/clean.fany" "$scratch/leaked.sqlite" 2>&1)
failure_status=$?
set -e

if [[ $failure_status -ne 1 ]]; then
  echo "canary self-test: expected plaintext detection to exit 1, got $failure_status" >&2
  exit 1
fi
if [[ "$failure_output" == *"$marker"* ]]; then
  echo "canary self-test: scanner disclosed the marker value" >&2
  exit 1
fi
if [[ "$failure_output" != *"leaked.sqlite"* ]]; then
  echo "canary self-test: scanner did not identify the captured artifact" >&2
  exit 1
fi

printf 'short\n' > "$scratch/invalid-markers"
if bash "$scanner" "$scratch/invalid-markers" "$scratch/clean.fany" >/dev/null 2>&1; then
  echo "canary self-test: scanner accepted an unsafe short marker" >&2
  exit 1
fi

echo "Anywhere plaintext-canary harness passed"
