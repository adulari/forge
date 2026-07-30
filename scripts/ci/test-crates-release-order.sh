#!/usr/bin/env bash
set -euo pipefail

runbook="docs/RELEASING-crates.md"
fork_manifest="vendor/genai-0.6.5/Cargo.toml"
metadata="$(cargo metadata --locked --no-deps --format-version 1)"
mapfile -t documented_order < <(
  sed -n \
    '/Then publish the versioned Forge graph:/,/^(`forge-relay`/s/^[0-9][0-9]*\. `\([^`]*\)`.*/\1/p' \
    "$runbook"
)
mapfile -t publishable < <(
  jq -r '.packages[] | select(.publish != []) | .name' <<<"$metadata" | sort
)

readarray -t fork_identity < <(
  python3 - "$fork_manifest" <<'PY'
import sys
import tomllib

with open(sys.argv[1], "rb") as handle:
    package = tomllib.load(handle)["package"]
print(package["name"])
print(package["version"])
PY
)
[[ "${fork_identity[0]}" == "forge-agent-genai" ]] || {
  echo "vendored provider fork must publish as forge-agent-genai" >&2
  exit 1
}
[[ "${fork_identity[1]}" == "0.6.5-forge.1" ]] || {
  echo "unexpected forge-agent-genai version: ${fork_identity[1]}" >&2
  exit 1
}
grep -Fq \
  '0. `forge-agent-genai` (`0.6.5-forge.1`; published once, not once per Forge release)' \
  "$runbook" || {
  echo "release runbook does not match the vendored provider fork identity" >&2
  exit 1
}
jq -e '
  .packages[]
  | select(.name == "forge-agent-provider")
  | any(
      .dependencies[];
      .name == "forge-agent-genai"
      and .rename == "genai"
      and .req == "=0.6.5-forge.1"
      and .path != null
    )
' <<<"$metadata" >/dev/null || {
  echo "forge-agent-provider must publish against the exact vendored forge-agent-genai fork" >&2
  exit 1
}

declare -A position=()
for index in "${!documented_order[@]}"; do
  package="${documented_order[$index]}"
  if [[ -n "${position[$package]+present}" ]]; then
    echo "crate publication order lists $package more than once" >&2
    exit 1
  fi
  position["$package"]="$index"
done

for package in "${publishable[@]}"; do
  [[ -n "${position[$package]+present}" ]] || {
    echo "publishable workspace package is missing from the release order: $package" >&2
    exit 1
  }
done
for package in "${documented_order[@]}"; do
  printf '%s\n' "${publishable[@]}" | grep -Fxq "$package" || {
    echo "release order names a non-publishable or unknown package: $package" >&2
    exit 1
  }
done

# A package may be published only after every publishable path dependency it references.
while IFS=$'\t' read -r package dependency; do
  [[ "$package" == "$dependency" ]] && continue
  [[ -n "${position[$package]+present}" ]] || continue
  [[ -n "${position[$dependency]+present}" ]] || continue
  if (( position[$dependency] >= position[$package] )); then
    echo "crate publication order places $dependency after its dependent $package" >&2
    exit 1
  fi
done < <(
  jq -r '
    .packages[] as $package
    | $package.dependencies[]
    | select(.path != null)
    | [$package.name, .name]
    | @tsv
  ' <<<"$metadata"
)

# The executable loop is the copy-paste release path; it must be the same ordered set. Exact token
# comparison matters because every package name is a prefix of the final `forge-agent` package.
mapfile -t loop_order < <(
  sed -n '/^for crate in /,/; do$/p' "$runbook" \
    | sed -e '1s/^for crate in //' -e 's/; do$//' -e 's/\\$//' \
    | tr -s '[:space:]' '\n' \
    | sed '/^$/d'
)
if (( ${#loop_order[@]} != ${#documented_order[@]} )); then
  echo "documented publish loop does not contain the full ordered crate list" >&2
  exit 1
fi
for index in "${!documented_order[@]}"; do
  if [[ "${loop_order[$index]}" != "${documented_order[$index]}" ]]; then
    echo "documented publish loop differs at position $((index + 1)): expected ${documented_order[$index]}, found ${loop_order[$index]}" >&2
    exit 1
  fi
done

echo "crates.io publication order covers the full publishable workspace graph"
