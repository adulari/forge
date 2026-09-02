#!/usr/bin/env bash
# Release verification for the install/upgrade path (RELEASING.md §7), run against published
# GitHub release assets in a throwaway HOME. Nothing touches the operator's ~/.local/bin, config,
# store, or daemon.
#
#   scripts/ci/verify-upgrade-path.sh [target-tag]
#   scripts/ci/verify-upgrade-path.sh v2.13.7
#
# Proves, for the target release (default: the latest published release):
#   1. the previous published release's `forge update --check` sees the target and `forge update`
#      self-replaces to it; the replaced binary is byte-identical to the one inside the target
#      archive whose sha256 is listed in checksums.txt, and no half-replaced file is left behind;
#   2. the documented one-liner (install.sh) installs the target into an isolated prefix and the
#      result reports the target version and passes `forge doctor` without panicking;
#   3. `gh attestation verify` accepts the CLI archive and its subject digest matches checksums.txt.
#
# Overrides:
#   VERIFY_UPGRADE_INSTALL_SH  path to a local install.sh to exercise instead of the raw main URL
#   KEEP_ARTIFACTS=1           keep the scratch directory on exit
set -euo pipefail

REPO="Adulari/forge"

# Pure helpers (unit-tested by test-verify-upgrade-path.sh).

# Map uname -s / uname -m to the release target triple.
asset_target() {
  case "$1" in
    Linux)
      case "$2" in
        x86_64 | amd64) echo x86_64-unknown-linux-gnu ;;
        aarch64 | arm64) echo aarch64-unknown-linux-gnu ;;
        *) return 1 ;;
      esac
      ;;
    Darwin)
      case "$2" in
        arm64 | aarch64) echo aarch64-apple-darwin ;;
        x86_64) echo x86_64-apple-darwin ;;
        *) return 1 ;;
      esac
      ;;
    *) return 1 ;;
  esac
}

# The published tag immediately older than $1 in a newest-first tag list read from stdin.
previous_release_tag() {
  awk -v target="$1" '
    found && /^v[0-9]+\.[0-9]+\.[0-9]+$/ { print; exit }
    $0 == target { found = 1 }
  '
}

# The 64-hex sha256 for asset $2 in checksums file $1; fails when absent or malformed.
sha_for_asset() {
  local sum
  sum=$(awk -v asset="$2" '$2 == asset || $2 == "*" asset { print $1; exit }' "$1")
  [[ "$sum" =~ ^[0-9a-f]{64}$ ]] || return 1
  echo "$sum"
}

# Version reported by `forge --version` output ("forge 2.13.7" -> "2.13.7").
version_from_banner() {
  awk 'NR == 1 { print $NF }'
}

if [[ "${VERIFY_UPGRADE_PATH_LIB_ONLY:-0}" == 1 ]]; then
  return 0 2>/dev/null || exit 0
fi

for tool in gh curl tar sha256sum python3; do
  command -v "$tool" >/dev/null 2>&1 || { echo "verify-upgrade-path: missing $tool" >&2; exit 2; }
done

TARGET=$(asset_target "$(uname -s)" "$(uname -m)") || {
  echo "verify-upgrade-path: no prebuilt CLI archive for $(uname -s)/$(uname -m)" >&2
  exit 2
}
ASSET="forge-$TARGET.tar.gz"

TAGS=$(gh release list --repo "$REPO" --limit 50 --json tagName,isDraft,isPrerelease \
  --jq '.[] | select(.isDraft == false and .isPrerelease == false) | .tagName')
TARGET_TAG="${1:-$(head -1 <<<"$TAGS")}"
[[ "$TARGET_TAG" == v* ]] || TARGET_TAG="v$TARGET_TAG"
PREVIOUS_TAG=$(previous_release_tag "$TARGET_TAG" <<<"$TAGS")
[[ -n "$PREVIOUS_TAG" ]] || { echo "verify-upgrade-path: no published release before $TARGET_TAG" >&2; exit 1; }

ROOT=$(mktemp -d "${TMPDIR:-/tmp}/forge-upgrade-path.XXXXXX")
cleanup() {
  if [[ "${KEEP_ARTIFACTS:-0}" == 1 ]]; then
    echo "verify-upgrade-path: kept scratch at $ROOT" >&2
  else
    rm -rf -- "$ROOT"
  fi
}
trap cleanup EXIT
trap 'echo "verify-upgrade-path: FAILED (scratch: $ROOT)" >&2' ERR

HOME_DIR="$ROOT/home"
mkdir -p "$HOME_DIR"
forge_env() {
  env HOME="$HOME_DIR" XDG_CONFIG_HOME="$HOME_DIR/.config" XDG_DATA_HOME="$HOME_DIR/.local/share" \
    FORGE_DB="$HOME_DIR/forge.db" FORGE_NO_UPDATE_CHECK=1 "$@"
}

fetch_release() {
  local tag=$1 dir=$2
  mkdir -p "$dir"
  gh release download "$tag" --repo "$REPO" --pattern "$ASSET" --pattern checksums.txt -D "$dir"
  local want
  want=$(sha_for_asset "$dir/checksums.txt" "$ASSET") \
    || { echo "verify-upgrade-path: $tag checksums.txt has no sha256 for $ASSET" >&2; return 1; }
  printf '%s  %s\n' "$want" "$dir/$ASSET" | sha256sum --check --status
  tar -xzf "$dir/$ASSET" -C "$dir"
  chmod 0755 "$dir/forge-$TARGET/forge"
}

echo "verify-upgrade-path: $PREVIOUS_TAG -> $TARGET_TAG ($ASSET)"

# 1. Self-update from the previous published release.
fetch_release "$PREVIOUS_TAG" "$ROOT/previous"
fetch_release "$TARGET_TAG" "$ROOT/target"
OLD="$ROOT/previous/forge-$TARGET/forge"
NEW_REF="$ROOT/target/forge-$TARGET/forge"
[[ "$(forge_env "$OLD" --version | version_from_banner)" == "${PREVIOUS_TAG#v}" ]]

CHECK_OUT=$(forge_env "$OLD" update --check)
echo "$CHECK_OUT"
grep -q "${TARGET_TAG#v}" <<<"$CHECK_OUT"

BEFORE=$(ls -A "$ROOT/previous/forge-$TARGET")
forge_env "$OLD" update
[[ "$(forge_env "$OLD" --version | version_from_banner)" == "${TARGET_TAG#v}" ]]
cmp -s "$OLD" "$NEW_REF" || { echo "verify-upgrade-path: updated binary differs from the $TARGET_TAG archive" >&2; exit 1; }
AFTER=$(ls -A "$ROOT/previous/forge-$TARGET")
[[ "$BEFORE" == "$AFTER" ]] || { echo "verify-upgrade-path: updater left files behind:"$'\n'"$AFTER" >&2; exit 1; }
echo "verify-upgrade-path: self-update ok ($(sha256sum "$OLD" | awk '{print $1}'))"

# 2. Documented one-liner into an isolated prefix, then a clean-HOME doctor.
INSTALL_DIR="$HOME_DIR/.local/bin"
if [[ -n "${VERIFY_UPGRADE_INSTALL_SH:-}" ]]; then
  INSTALLER=$(cat "$VERIFY_UPGRADE_INSTALL_SH")
else
  INSTALLER=$(curl -fsSL "https://raw.githubusercontent.com/$REPO/main/install.sh")
fi
forge_env FORGE_INSTALL_DIR="$INSTALL_DIR" FORGE_VERSION="$TARGET_TAG" FORGE_DESKTOP=0 \
  sh -c "$INSTALLER" </dev/null
INSTALLED="$INSTALL_DIR/forge"
[[ "$(forge_env "$INSTALLED" --version | version_from_banner)" == "${TARGET_TAG#v}" ]]
cmp -s "$INSTALLED" "$NEW_REF"
# Doctor inspects the cwd's repository (worktree disk usage etc.), so run it from an empty scratch
# project: a clean host's verdict must not depend on the operator's checkout.
mkdir -p "$ROOT/project"
DOCTOR_RC=0
(cd "$ROOT/project" && forge_env "$INSTALLED" doctor </dev/null) >"$ROOT/doctor.log" 2>&1 || DOCTOR_RC=$?
if [[ "$DOCTOR_RC" -ne 0 ]] || grep -q "panicked" "$ROOT/doctor.log"; then
  cat "$ROOT/doctor.log" >&2
  echo "verify-upgrade-path: doctor failed (rc=$DOCTOR_RC)" >&2
  exit 1
fi
echo "verify-upgrade-path: install one-liner ok ($(grep -c '✓' "$ROOT/doctor.log") doctor checks passed)"

# 3. Build provenance for the CLI archive.
WANT=$(sha_for_asset "$ROOT/target/checksums.txt" "$ASSET")
gh attestation verify "$ROOT/target/$ASSET" --repo "$REPO" --format json >"$ROOT/attestation.json"
python3 - "$ROOT/attestation.json" "$ASSET" "$WANT" <<'PY'
import json, sys
results = json.load(open(sys.argv[1]))
assert results, "no attestation returned"
subjects = results[0]["verificationResult"]["statement"]["subject"]
digests = {s["name"]: s["digest"]["sha256"] for s in subjects}
assert digests.get(sys.argv[2]) == sys.argv[3], (digests.get(sys.argv[2]), sys.argv[3])
PY
echo "verify-upgrade-path: attestation ok"

trap - ERR
echo "verify-upgrade-path: PASS $PREVIOUS_TAG -> $TARGET_TAG"
