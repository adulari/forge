#!/usr/bin/env bash
# Stamp a release version into the checkout, so a release needs no hand-written "prepare vX.Y.Z" PR.
#
#   scripts/ci/stamp-release-version.sh X.Y.Z [YYYY-MM-DD]
#
# Rewrites, in place:
#   - Cargo.toml          [workspace.package] version
#   - Cargo.lock          every workspace member's own entry (offline, no resolver run)
#   - mobile/src-tauri/tauri.conf.json   version
#   - CHANGELOG.md and crates/forge-cli/CHANGELOG.md   `## [Unreleased]` becomes `## [X.Y.Z] - DATE`
#     under a fresh empty `## [Unreleased]`, with the compare links repointed
#
# Idempotent and dual-mode: when the checkout already carries X.Y.Z (the old prep-PR flow) it only
# verifies and exits 0, so a release prepared by hand keeps working. It refuses to go backwards.
# release.yml runs it before building and the post-release PR runs the same script, so the shipped
# binary (which embeds CHANGELOG.md) and main end up byte-identical in these files.
set -euo pipefail

version="${1:?usage: stamp-release-version.sh X.Y.Z [YYYY-MM-DD]}"
date_utc="${2:-$(date -u +%F)}"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "version must look like 2.25.0: $version" >&2; exit 2; }
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
cd "$root"

py=$(command -v python3 || command -v python)
"$py" - "$version" "$date_utc" <<'PY'
import json, re, sys
from pathlib import Path

version, date = sys.argv[1], sys.argv[2]
key = lambda v: tuple(int(p) for p in v.split("."))

manifest = Path("Cargo.toml")
text = manifest.read_text()
m = re.search(r"(?ms)^\[workspace\.package\]\s*$(.*?)(?=^\[|\Z)", text)
if not m:
    sys.exit("stamp: no [workspace.package] in Cargo.toml")
v = re.search(r'(?m)^(\s*version\s*=\s*)"([^"]+)"', m.group(1))
if not v:
    sys.exit("stamp: no version in [workspace.package]")
old = v.group(2)
if key(version) < key(old):
    sys.exit(f"stamp: refusing to stamp {version} over newer {old}")

if old != version:
    start = m.start(1) + v.start(2)
    text = text[:start] + version + text[start + len(old):]
    manifest.write_text(text)

    lock = Path("Cargo.lock")
    blocks = lock.read_text().split("\n[[package]]\n")
    members = set()
    for p in Path("crates").glob("*/Cargo.toml"):
        n = re.search(r'(?m)^name\s*=\s*"([^"]+)"', p.read_text())
        if n:
            members.add(n.group(1))
    hits = 0
    for i, b in enumerate(blocks):
        name = re.search(r'(?m)^name = "([^"]+)"', b)
        ver = re.search(r'(?m)^version = "([^"]+)"', b)
        if name and ver and name.group(1) in members and ver.group(1) == old and "\nsource = " not in b:
            blocks[i] = b.replace(f'version = "{old}"', f'version = "{version}"', 1)
            hits += 1
    if hits == 0:
        sys.exit(f"stamp: no workspace member at {old} in Cargo.lock")
    lock.write_text("\n[[package]]\n".join(blocks))

    conf = Path("mobile/src-tauri/tauri.conf.json")
    raw = conf.read_text()
    raw, n = re.subn(r'(?m)^(\s*"version"\s*:\s*)"[^"]*"', rf'\g<1>"{version}"', raw, count=1)
    if n != 1:
        sys.exit("stamp: no version in tauri.conf.json")
    json.loads(raw)
    conf.write_text(raw)
    print(f"stamped {old} -> {version} ({hits} lock entries)")
else:
    print(f"already at {version}")

cl = Path("CHANGELOG.md")
t = cl.read_text()
if re.search(rf"(?m)^## \[{re.escape(version)}\]", t):
    print("changelog already has the section")
else:
    if "## [Unreleased]" not in t:
        sys.exit("stamp: CHANGELOG.md has no [Unreleased] section")
    t = t.replace("## [Unreleased]", f"## [Unreleased]\n\n## [{version}] - {date}", 1)
    link = re.search(r"(?m)^\[Unreleased\]: (\S+)/compare/(v\S+?)\.\.\.HEAD$", t)
    if link:
        base, prev = link.group(1), link.group(2)
        new = (f"[Unreleased]: {base}/compare/v{version}...HEAD\n"
               f"[{version}]: {base}/compare/{prev}...v{version}")
        t = t[:link.start()] + new + t[link.end():]
    cl.write_text(t)
Path("crates/forge-cli/CHANGELOG.md").write_text(cl.read_text())
PY
