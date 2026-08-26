#!/usr/bin/env bash
set -euo pipefail

check=scripts/ci/check-docs-freshness.sh

bash "$check" \
  docs/features/lattice-token-savings.md crates/forge-index/src/lib.rs \
  docs/features/remote-control.md crates/forge-cli/src/serve_terminal.rs \
  mobile/EAS_UPDATE.md .github/workflows/eas-update.yml \
  .github/workflows/flake-hunt.yml scripts/flake-hunt.sh

if bash "$check" crates/forge-index/src/lib.rs >/dev/null 2>&1; then
  echo "a source change without its standing doc must fail" >&2
  exit 1
fi

if bash "$check" crates/forge-cli/src/lib.rs >/dev/null 2>&1; then
  echo "unrelated source path passed freshness check"
else
  # The unrelated path is not paired; a failure here would mean the matcher widened accidentally.
  echo "unrelated source path was incorrectly treated as stale" >&2
  exit 1
fi

echo "docs freshness pairing checks passed"
