#!/usr/bin/env bash
# Synchronize every checked-in package-manager manifest from one published release's checksums.
# The release workflow runs this only AFTER assets exist, preventing placeholder hashes and
# version/hash races. It is also the manual recovery path:
#
#   scripts/update-package-manifests.sh 2.6.5
#   scripts/update-package-manifests.sh 2.6.5 path/to/checksums.txt
set -euo pipefail

VERSION="${1:?usage: update-package-manifests.sh <version-without-v> [checksums.txt]}"
VERSION="${VERSION#v}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CHECKSUMS="${2:-}"
TEMP_CHECKSUMS=""
if [[ -z "$CHECKSUMS" ]]; then
  TEMP_CHECKSUMS="$(mktemp)"
  CHECKSUMS="$TEMP_CHECKSUMS"
  gh release download "v${VERSION}" --pattern checksums.txt --output "$CHECKSUMS" --clobber
fi
trap '[[ -z "$TEMP_CHECKSUMS" ]] || rm -f "$TEMP_CHECKSUMS"' EXIT

python3 - "$ROOT" "$VERSION" "$CHECKSUMS" <<'PY'
import json
import pathlib
import re
import sys

root = pathlib.Path(sys.argv[1])
version = sys.argv[2]
checksums_path = pathlib.Path(sys.argv[3])

checksums = {}
for line in checksums_path.read_text().splitlines():
    parts = line.split()
    if len(parts) == 2 and re.fullmatch(r"[0-9a-fA-F]{64}", parts[0]):
        checksums[pathlib.Path(parts[1].lstrip("*")).name] = parts[0].lower()

required = {
    "forge-aarch64-apple-darwin.tar.gz",
    "forge-x86_64-apple-darwin.tar.gz",
    "forge-aarch64-unknown-linux-gnu.tar.gz",
    "forge-x86_64-unknown-linux-gnu.tar.gz",
    "forge-x86_64-pc-windows-msvc.zip",
}
missing = sorted(required - checksums.keys())
if missing:
    raise SystemExit("checksums.txt is missing required release assets:\n- " + "\n- ".join(missing))

# Homebrew: associate each sha with the release asset URL immediately above it.
formula = root / "homebrew/forge.rb"
out = []
last_asset = None
for line in formula.read_text().splitlines():
    asset = re.search(r"forge-[\w.-]+\.(?:tar\.gz|zip)", line)
    if asset:
        last_asset = asset.group(0)
    if line.strip().startswith("version "):
        line = re.sub(r'"[^"]+"', f'"{version}"', line, count=1)
    elif line.strip().startswith("sha256 ") and last_asset in checksums:
        line = re.sub(r'"[0-9a-fA-F]+"', f'"{checksums[last_asset]}"', line, count=1)
    out.append(line)
formula.write_text("\n".join(out) + "\n")

# AUR forge-bin: published version, license, and both native Linux archive hashes.
aur = root / "packaging/aur/PKGBUILD"
text = aur.read_text()
text = re.sub(r"(?m)^pkgver=.*$", f"pkgver={version}", text)
text = re.sub(r"(?m)^pkgrel=.*$", "pkgrel=1", text)
text = re.sub(r"(?m)^license=.*$", "license=('AGPL-3.0-only')", text)
text = re.sub(
    r"(?m)^sha256sums_x86_64=.*$",
    f"sha256sums_x86_64=('{checksums['forge-x86_64-unknown-linux-gnu.tar.gz']}')",
    text,
)
text = re.sub(
    r"(?m)^sha256sums_aarch64=.*$",
    f"sha256sums_aarch64=('{checksums['forge-aarch64-unknown-linux-gnu.tar.gz']}')",
    text,
)
aur.write_text(text)

# Scoop: the checked-in manifest is immediately usable; autoupdate handles later releases too.
scoop = root / "packaging/scoop/forge.json"
manifest = json.loads(scoop.read_text())
manifest["version"] = version
manifest["license"] = "AGPL-3.0-only"
win = manifest["architecture"]["64bit"]
win["url"] = (
    f"https://github.com/Adulari/forge/releases/download/v{version}/"
    "forge-x86_64-pc-windows-msvc.zip"
)
win["hash"] = checksums["forge-x86_64-pc-windows-msvc.zip"]
scoop.write_text(json.dumps(manifest, indent=4) + "\n")

print(f"updated Homebrew, AUR, and Scoop manifests -> v{version}")
PY
