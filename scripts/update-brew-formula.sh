#!/usr/bin/env bash
# Backward-compatible entry point. Package publication is one transaction now, so this also keeps
# AUR and Scoop synchronized instead of repairing Homebrew alone.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
exec "$ROOT/scripts/update-package-manifests.sh" "$@"
