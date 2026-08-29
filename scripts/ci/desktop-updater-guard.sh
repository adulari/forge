#!/usr/bin/env bash
set -euo pipefail

# The desktop bundle must emit signed updater artifacts.
#
# `bundle.createUpdaterArtifacts` is the only switch that makes tauri-cli sign anything: with it
# `false` the bundler builds `updater_settings = None` and `sign_updaters` returns immediately, so
# no `.sig` is produced on any platform even when `TAURI_SIGNING_PRIVATE_KEY` is present. It was
# flipped `true` -> `false` inside an unrelated squash (#954) and nothing noticed for four weeks,
# until v2.13.0 failed on every desktop leg at `cp *.sig`.
#
# `"v1Compatible"` is rejected as well: it produces the deprecated v1 artifact layout, while
# app-desktop.yml collects the v2 self-contained names (`*.AppImage.sig`, `*.exe.sig`,
# `*.app.tar.gz.sig`).

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
tauri_conf=${1:-"$root/mobile/src-tauri/tauri.conf.json"}

python3 - "$tauri_conf" <<'PY'
import json
import sys
from pathlib import Path

conf_path = Path(sys.argv[1])
if not conf_path.is_file():
    sys.exit(f"desktop updater guard: missing {conf_path}")

try:
    conf = json.loads(conf_path.read_text())
except json.JSONDecodeError as error:
    sys.exit(f"desktop updater guard: {conf_path} is not valid JSON: {error}")

artifacts = conf.get("bundle", {}).get("createUpdaterArtifacts")
if artifacts is not True:
    sys.exit(
        f"desktop updater guard: bundle.createUpdaterArtifacts is {json.dumps(artifacts)}, "
        "must be true.\n"
        "Anything else stops tauri-cli signing the bundles, so the release workflow finds no\n"
        '.sig files and the whole desktop release fails at asset collection. "v1Compatible"\n'
        "emits the deprecated v1 layout the workflow does not collect."
    )

pubkey = conf.get("plugins", {}).get("updater", {}).get("pubkey")
if not pubkey:
    sys.exit(
        "desktop updater guard: plugins.updater.pubkey is missing or empty.\n"
        "tauri-cli reads it to build updater settings and fails the bundle without it."
    )

print("desktop updater artifacts are enabled and signable")
PY
