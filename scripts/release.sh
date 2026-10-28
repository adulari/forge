#!/usr/bin/env bash
# One-command release: tag the tip of main, push the tag, dispatch release.yml. See docs/ci/releasing.md.
#
#   scripts/release.sh vX.Y.Z        # real release
#   scripts/release.sh --dry-run     # build + upgrade gate on main's tip, publishes nothing, no tag
#
# The version comes from the tag: release.yml stamps Cargo.toml, Cargo.lock, tauri.conf.json and the
# CHANGELOG `[Unreleased]` section inside the build, then opens one PR that brings main up to it.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

if [[ "${1:-}" == "--dry-run" ]]; then
  gh workflow run release.yml --ref "$(git branch --show-current)" -f release_tag=v99.0.0 -f dry_run=true
  echo "dry run dispatched: gh run list -w release.yml -L 1"
  exit 0
fi

tag="${1:?usage: scripts/release.sh vX.Y.Z | --dry-run}"
[[ "$tag" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "tag must look like v2.25.0" >&2; exit 2; }

git fetch -q origin main --tags
[[ "$(git rev-parse HEAD)" == "$(git rev-parse origin/main)" ]] || {
  echo "check out the tip of origin/main first (release.yml requires tag == dispatch SHA)" >&2; exit 1; }
git rev-parse -q --verify "refs/tags/$tag" >/dev/null && { echo "$tag already exists" >&2; exit 1; }
scripts/ci/require-release-version.sh "$tag"
grep -q '^### ' < <(awk '/^## \[Unreleased\]/{f=1;next} /^## \[/{f=0} f' CHANGELOG.md) || {
  echo "CHANGELOG.md [Unreleased] is empty; a release needs real entries" >&2; exit 1; }

git tag "$tag"
git push origin "refs/tags/$tag"
gh workflow run release.yml --ref main -f "release_tag=$tag"
echo "dispatched $tag: gh run list -w release.yml -L 1"
