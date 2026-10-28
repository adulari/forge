#!/usr/bin/env bash
# Bring main's committed version files up to a release that was stamped in CI.
#
#   scripts/ci/release-catchup.sh X.Y.Z [YYYY-MM-DD]   # in a clean checkout of main with tag vX.Y.Z fetched
#
# release.yml builds the tag with the version stamped in-job (stamp-release-version.sh), so main still
# says the previous version. This applies the same stamp to main.
#
# CHANGELOG.md cannot simply be re-stamped: main's `[Unreleased]` may already hold entries merged
# after the tag that belong to the NEXT release. Entries (one line each) that were in the tag's
# `[Unreleased]` move into the new `[X.Y.Z]` section; everything else stays under `[Unreleased]`.
#
# No-op when main already carries X.Y.Z (a release prepared by hand).
set -euo pipefail
version="${1:?usage: release-catchup.sh X.Y.Z [YYYY-MM-DD]}"
date_utc="${2:-$(date -u +%F)}"
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
cd "$root"
tag="v$version"

main_version=$(python3 -c 'import tomllib; print(tomllib.load(open("Cargo.toml","rb"))["workspace"]["package"]["version"])')
if [ "$main_version" = "$version" ]; then
  echo "main already carries $version"
  exit 0
fi

tag_changelog=$(mktemp)
trap 'rm -f "$tag_changelog"' EXIT
git show "$tag:CHANGELOG.md" > "$tag_changelog"

scripts/ci/stamp-release-version.sh "$version" "$date_utc" >/dev/null
# The stamp rolled main's whole [Unreleased]; undo that for the changelog and redo it precisely.
git checkout -- CHANGELOG.md

python3 - "$version" "$date_utc" "$tag_changelog" <<'PY'
import re, sys
from pathlib import Path

version, date, tag_path = sys.argv[1:4]
unrel = re.compile(r"(?ms)^## \[Unreleased\]\n(.*?)(?=^## \[|^\[Unreleased\]:)")
tag_txt = Path(tag_path).read_text()
main_txt = Path("CHANGELOG.md").read_text()
if re.search(rf"(?m)^## \[{re.escape(version)}\]", main_txt):
    sys.exit(0)

released = unrel.search(tag_txt).group(1)
shipped = {l for l in released.splitlines() if l.strip() and not l.startswith("#")}
m = unrel.search(main_txt)

kept, section = [], []
def flush():
    if any(l.strip() for l in section[1:]):
        kept.extend(section)
for line in m.group(1).splitlines():
    if line.startswith("### "):
        flush()
        section = [line]
    elif line in shipped:
        continue
    elif section:
        section.append(line)
    else:
        kept.append(line)
flush()

remaining = "\n".join(kept).strip("\n")
new = "## [Unreleased]\n\n" + (remaining + "\n\n" if remaining else "") + f"## [{version}] - {date}\n" + released
main_txt = main_txt[:m.start()] + new + main_txt[m.end():]
link = re.search(r"(?m)^\[Unreleased\]: (\S+)/compare/(v\S+?)\.\.\.HEAD$", main_txt)
if link:
    base, prev = link.group(1), link.group(2)
    main_txt = (main_txt[:link.start()]
                + f"[Unreleased]: {base}/compare/v{version}...HEAD\n[{version}]: {base}/compare/{prev}...v{version}"
                + main_txt[link.end():])
Path("CHANGELOG.md").write_text(main_txt)
PY
cp CHANGELOG.md crates/forge-cli/CHANGELOG.md
echo "main caught up to $version"
