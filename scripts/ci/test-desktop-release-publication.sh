#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
release="$root/.github/workflows/release.yml"
desktop="$root/.github/workflows/app-desktop.yml"
installer_sh="$root/install-desktop.sh"
installer_ps1="$root/install-desktop.ps1"

if grep -Eq '^[[:space:]]*make_latest:[[:space:]]*true' "$release"; then
  echo "release.yml must not expose an incomplete CLI-only release as Latest" >&2
  exit 1
fi

# The CLI half must be staged as a draft. A published-but-not-Latest release is still resolvable
# at releases/download/<tag>/..., which is exactly the window that broke the installer (#837).
grep -Eq '^[[:space:]]*draft:[[:space:]]*true' "$release" || {
  echo "release.yml must stage the CLI release as a draft for app-desktop.yml to publish" >&2
  exit 1
}
grep -Eq '^[[:space:]]*draft:[[:space:]]*false' "$desktop" || {
  echo "app-desktop.yml must be the step that publishes the staged draft release" >&2
  exit 1
}

# Every filename an installer asks for by name must be a required entry of the desktop manifest.
for asset in \
  Forge-desktop-linux-x86_64.AppImage \
  Forge-desktop-linux-aarch64.AppImage \
  Forge-desktop-macos-aarch64.dmg \
  Forge-desktop-macos-x86_64.dmg \
  Forge-desktop-windows-x86_64.nsis.exe; do
  grep -Fq "$asset" "$desktop" || {
    echo "app-desktop.yml must require $asset before writing desktop-checksums.txt" >&2
    exit 1
  }
done
for asset in $(grep -Eo 'Forge-desktop-[a-z0-9_-]+\.(AppImage|dmg|nsis\.exe)' "$installer_sh" "$installer_ps1" | cut -d: -f2- | sort -u); do
  grep -Fq "$asset" "$desktop" || {
    echo "installer requests $asset but app-desktop.yml never publishes it" >&2
    exit 1
  }
done

grep -Fq 'make_latest: false' "$desktop" || {
  echo "desktop publication must upload without moving Latest" >&2
  exit 1
}
grep -Fq 'desktop-checksums.txt' "$desktop"
grep -Fq 'sha256sum --check' "$desktop" || {
  echo "desktop publication must verify the publicly downloaded checksum manifest" >&2
  exit 1
}

publish_line=$(grep -n '^[[:space:]]*draft:[[:space:]]*false' "$desktop" | tail -1 | cut -d: -f1)
verify_line=$(grep -n 'sha256sum --check' "$desktop" | tail -1 | cut -d: -f1)
latest_line=$(grep -n 'gh release edit .*--latest' "$desktop" | tail -1 | cut -d: -f1)
if [[ -z "$verify_line" || -z "$latest_line" || "$latest_line" -le "$verify_line" ]]; then
  echo "Latest must move only after public checksum verification" >&2
  exit 1
fi
if [[ -z "$publish_line" || "$verify_line" -le "$publish_line" ]]; then
  echo "public verification must run after the draft is published, not before" >&2
  exit 1
fi

# Artifact production must not be reachable-only-if package-manager bookkeeping succeeds. The
# desktop/web dispatch is the ONLY thing that starts app-desktop.yml (it has no tag trigger), so a
# manifest flake ahead of it in the same job leaves the tag with no bundles, no latest.json, and no
# desktop-checksums.txt at all — run 29831376887 (v2.8.4) did exactly that.
RELEASE_WORKFLOW="$release" python3 - <<'PY'
import os
import re
from pathlib import Path

# Comment lines quote these very commands when explaining why they moved; scan the YAML only.
text = "\n".join(
    line
    for line in Path(os.environ["RELEASE_WORKFLOW"]).read_text().splitlines()
    if not line.lstrip().startswith("#")
)


def job_block(name: str) -> str:
    match = re.search(
        rf"(?ms)^  {re.escape(name)}:\n(.*?)(?=^  [A-Za-z0-9_-]+:\n|\Z)", text
    )
    if not match:
        raise SystemExit(f"release.yml is missing the `{name}` job")
    return match.group(1)


release_job = job_block("release")
if "gh workflow run app-desktop.yml" not in release_job:
    raise SystemExit("the release job must dispatch app-desktop.yml")

# Anything in this list can fail on its own; none of it may run before the dispatch, and none of it
# belongs in the job that owns the dispatch at all.
bookkeeping = ("update-package-manifests.sh", "gh pr create", "gh pr merge", "makepkg")
for command in bookkeeping:
    if command in release_job:
        raise SystemExit(
            f"`{command}` must not run in the release job — a flake there starves the "
            "desktop/web dispatch; keep it in its own job"
        )

manifest_jobs = [
    name
    for name in re.findall(r"(?m)^  ([A-Za-z0-9_-]+):$", text)
    if "update-package-manifests.sh" in job_block(name)
]
if len(manifest_jobs) != 1:
    raise SystemExit("exactly one job may own the package-manager manifest update")
if not re.search(r"(?m)^    needs:.*\brelease\b", job_block(manifest_jobs[0])):
    raise SystemExit(f"job `{manifest_jobs[0]}` must declare `needs: release`")
PY

echo "Desktop release publication contract passed"
