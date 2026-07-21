#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
release="$root/.github/workflows/release.yml"
desktop="$root/.github/workflows/app-desktop.yml"

if grep -Eq '^[[:space:]]*make_latest:[[:space:]]*true' "$release"; then
  echo "release.yml must not expose an incomplete CLI-only release as Latest" >&2
  exit 1
fi

grep -Fq 'make_latest: false' "$desktop" || {
  echo "desktop publication must upload without moving Latest" >&2
  exit 1
}
grep -Fq 'desktop-checksums.txt' "$desktop"
grep -Fq 'sha256sum --check' "$desktop" || {
  echo "desktop publication must verify the publicly downloaded checksum manifest" >&2
  exit 1
}

verify_line=$(grep -n 'sha256sum --check' "$desktop" | tail -1 | cut -d: -f1)
latest_line=$(grep -n 'gh release edit .*--latest' "$desktop" | tail -1 | cut -d: -f1)
if [[ -z "$verify_line" || -z "$latest_line" || "$latest_line" -le "$verify_line" ]]; then
  echo "Latest must move only after public checksum verification" >&2
  exit 1
fi

echo "Desktop release publication contract passed"
