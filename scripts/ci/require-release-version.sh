#!/usr/bin/env bash
# Gate for a release tag against the checked-out Cargo.toml.
#
#   scripts/ci/require-release-version.sh vX.Y.Z
#
# Passes when the tag's version equals the workspace version (a release prepared by hand), or is
# greater and this checkout carries scripts/ci/stamp-release-version.sh, which the release jobs run
# before building. Anything else (older tag, or a newer tag on a source that cannot stamp itself)
# is refused, so a mismatch still fails in seconds instead of shipping a mislabelled binary.
set -euo pipefail
tag="${1:?usage: require-release-version.sh vX.Y.Z}"
[[ "$tag" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "release tag must look like v2.6.6: $tag" >&2; exit 1; }
python3 - "${tag#v}" "$(dirname "${BASH_SOURCE[0]}")" <<'PY'
import sys, tomllib
from pathlib import Path
tag, here = sys.argv[1], Path(sys.argv[2])
cargo = tomllib.load(open("Cargo.toml", "rb"))["workspace"]["package"]["version"]
key = lambda v: tuple(int(p) for p in v.split("."))
if tag == cargo:
    sys.exit(0)
if key(tag) < key(cargo):
    sys.exit(f"release tag v{tag} is older than workspace version {cargo}")
if not (here / "stamp-release-version.sh").is_file():
    sys.exit(f"tag v{tag} does not match workspace version {cargo} and this source cannot stamp itself")
print(f"v{tag} will be stamped over workspace version {cargo} in every release job")
PY
