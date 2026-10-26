#!/usr/bin/env bash
# Refresh the benchmark snapshot compiled into the binary (crates/forge-cli/data/benchmarks-snapshot.json)
# from the published mirror document. It is what ranks models for a user who is offline on first
# run, so a release should ship a recent one (RELEASING.md step 2).
#
#   scripts/refresh-benchmarks-snapshot.sh                 # download the published document
#   scripts/refresh-benchmarks-snapshot.sh path/to/benchmarks.json   # or use a local copy
#
# The document is validated with the same rules as the client (schema 1, 0-100 indices, at least
# half as many models as the snapshot it replaces) before anything is written.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DEST="$ROOT/crates/forge-cli/data/benchmarks-snapshot.json"
SRC="${1:-https://github.com/adulari/forge/releases/download/benchmarks-data/benchmarks.json}"

TMP="$(mktemp)"
trap 'rm -f "$TMP"' EXIT
if [[ "$SRC" =~ ^https?:// ]]; then
  curl --fail --silent --show-error --location --max-time 60 "$SRC" --output "$TMP"
else
  cp "$SRC" "$TMP"
fi

python3 -I - "$TMP" "$DEST" <<'PY'
import json
import math
import pathlib
import sys

src = pathlib.Path(sys.argv[1])
dest = pathlib.Path(sys.argv[2])

doc = json.loads(src.read_text())
models = doc.get("models")
if doc.get("schema") != 1 or not isinstance(models, list) or not models:
    sys.exit("refresh-benchmarks-snapshot: not a schema 1 mirror document")
if not isinstance(doc.get("source_fetched_at"), int):
    sys.exit("refresh-benchmarks-snapshot: missing source_fetched_at")
for m in models:
    for field in ("intelligence", "coding"):
        x = m.get(field)
        if not isinstance(x, (int, float)) or not math.isfinite(x) or not 0 <= x <= 100:
            sys.exit(f"refresh-benchmarks-snapshot: {m.get('name')!r} has a bad {field}")
    if not isinstance(m.get("name"), str) or not m["name"]:
        sys.exit("refresh-benchmarks-snapshot: a model has no name")

if dest.exists():
    previous = len(json.loads(dest.read_text()).get("models", []))
    if previous and len(models) * 2 < previous:
        sys.exit(f"refresh-benchmarks-snapshot: {len(models)} models vs {previous} in the current snapshot")

dest.write_text(json.dumps(doc, indent=2) + "\n")
print(f"snapshot refreshed: {len(models)} models")
PY
