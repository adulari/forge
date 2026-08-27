#!/usr/bin/env bash
set -euo pipefail

# The desktop bundle's version must match the workspace version.
#
# `mobile/src-tauri/tauri.conf.json` carries its own `version`, and nothing kept it in step with
# the Rust workspace. It drifted to 2.6.6 while the workspace shipped 2.12.2, so bundles were
# published as `Forge_2.6.6_*` and the updater offered a version the running app could not identify
# itself as. Found by hand, twice; this makes it a check.
#
# Deliberately compared against the workspace `[workspace.package] version` rather than a release
# tag, so it holds on every PR instead of only at release time.

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
manifest=${1:-"$root/Cargo.toml"}
tauri_conf=${2:-"$root/mobile/src-tauri/tauri.conf.json"}

python3 - "$manifest" "$tauri_conf" <<'PY'
import json
import re
import sys
from pathlib import Path

manifest_path, conf_path = Path(sys.argv[1]), Path(sys.argv[2])

for path in (manifest_path, conf_path):
    if not path.is_file():
        sys.exit(f"desktop version guard: missing {path}")

# Only the [workspace.package] version counts; a per-crate `version` elsewhere in the file would
# otherwise match first and silently compare the wrong thing.
manifest = manifest_path.read_text()
section = re.search(r"(?ms)^\[workspace\.package\]\s*$(.*?)(?=^\[|\Z)", manifest)
if not section:
    sys.exit("desktop version guard: no [workspace.package] section in Cargo.toml")
found = re.search(r'(?m)^\s*version\s*=\s*"([^"]+)"', section.group(1))
if not found:
    sys.exit("desktop version guard: no version in [workspace.package]")
workspace_version = found.group(1)

try:
    conf = json.loads(conf_path.read_text())
except json.JSONDecodeError as error:
    sys.exit(f"desktop version guard: {conf_path} is not valid JSON: {error}")

bundle_version = conf.get("version")
if bundle_version is None:
    sys.exit(f"desktop version guard: {conf_path} has no top-level \"version\"")

if bundle_version != workspace_version:
    sys.exit(
        f"desktop bundle version {bundle_version} does not match workspace {workspace_version}.\n"
        f"Update \"version\" in {conf_path.name} — a mismatch ships bundles named for the wrong\n"
        "release and makes the updater offer a version the app cannot identify itself as."
    )

print(f"desktop bundle version matches the workspace ({workspace_version})")
PY
