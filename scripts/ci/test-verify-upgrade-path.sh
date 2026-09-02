#!/usr/bin/env bash
set -euo pipefail

# Offline unit tests for the pure helpers in scripts/ci/verify-upgrade-path.sh (asset selection,
# previous-release lookup, checksum parsing, version banner). The network-backed run is a manual
# release step documented in RELEASING.md §7.

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT

# shellcheck source=scripts/ci/verify-upgrade-path.sh
VERIFY_UPGRADE_PATH_LIB_ONLY=1 source "$script_dir/verify-upgrade-path.sh"

fail() { echo "test-verify-upgrade-path: $1" >&2; exit 1; }

[[ "$(asset_target Linux x86_64)" == x86_64-unknown-linux-gnu ]] || fail "linux x86_64 target"
[[ "$(asset_target Linux aarch64)" == aarch64-unknown-linux-gnu ]] || fail "linux aarch64 target"
[[ "$(asset_target Darwin arm64)" == aarch64-apple-darwin ]] || fail "darwin arm64 target"
[[ "$(asset_target Darwin x86_64)" == x86_64-apple-darwin ]] || fail "darwin x86_64 target"
! asset_target Linux riscv64 >/dev/null || fail "unsupported arch must fail"
! asset_target FreeBSD x86_64 >/dev/null || fail "unsupported os must fail"

tags=$'v2.13.7\nv2.13.6\nv2.13.5'
[[ "$(previous_release_tag v2.13.7 <<<"$tags")" == v2.13.6 ]] || fail "previous of latest"
[[ "$(previous_release_tag v2.13.6 <<<"$tags")" == v2.13.5 ]] || fail "previous of middle"
[[ -z "$(previous_release_tag v2.13.5 <<<"$tags")" ]] || fail "oldest has no previous"
[[ -z "$(previous_release_tag v9.9.9 <<<"$tags")" ]] || fail "unknown tag has no previous"
# Prereleases/drafts are filtered upstream; a non-semver line between releases is skipped.
[[ "$(previous_release_tag v2.13.7 <<<$'v2.13.7\nnightly\nv2.13.6')" == v2.13.6 ]] || fail "skip non-semver"

good=$(printf '%064d' 7 | tr 0 a)
cat > "$work/checksums.txt" <<EOF
$good  forge-x86_64-unknown-linux-gnu.tar.gz
deadbeef  forge-aarch64-unknown-linux-gnu.tar.gz
EOF
[[ "$(sha_for_asset "$work/checksums.txt" forge-x86_64-unknown-linux-gnu.tar.gz)" == "$good" ]] \
  || fail "sha lookup"
! sha_for_asset "$work/checksums.txt" forge-aarch64-unknown-linux-gnu.tar.gz >/dev/null \
  || fail "short sha must be rejected"
! sha_for_asset "$work/checksums.txt" forge-x86_64-apple-darwin.tar.gz >/dev/null \
  || fail "missing asset must fail"
printf '%s *forge-x86_64-apple-darwin.tar.gz\n' "$good" >> "$work/checksums.txt"
[[ "$(sha_for_asset "$work/checksums.txt" forge-x86_64-apple-darwin.tar.gz)" == "$good" ]] \
  || fail "binary-mode marker"

[[ "$(printf 'forge 2.13.7\nextra\n' | version_from_banner)" == 2.13.7 ]] || fail "version banner"

echo "test-verify-upgrade-path: ok"
