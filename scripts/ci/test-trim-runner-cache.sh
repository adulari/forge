#!/usr/bin/env bash
set -euo pipefail

scratch="$(mktemp -d)"
trap 'rm -rf -- "$scratch"' EXIT
mkdir -p "$scratch/target" "$scratch/mobile/node_modules" "$scratch/keep"
dd if=/dev/zero of="$scratch/target/oversized" bs=1024 count=8 status=none
dd if=/dev/zero of="$scratch/mobile/node_modules/oversized" bs=1024 count=8 status=none

FORGE_MAX_TARGET_CACHE_KIB=1 \
FORGE_MAX_NODE_MODULES_CACHE_KIB=1 \
  bash scripts/ci/trim-runner-cache.sh "$scratch"

test ! -e "$scratch/target"
test ! -e "$scratch/mobile/node_modules"
test -d "$scratch/keep"

mkdir -p "$scratch/target"
dd if=/dev/zero of="$scratch/target/preserved" bs=1024 count=8 status=none
FORGE_MAX_TARGET_CACHE_KIB=1 \
FORGE_MAX_NODE_MODULES_CACHE_KIB=1 \
FORGE_CACHE_TRIM_DRY_RUN=1 \
  bash scripts/ci/trim-runner-cache.sh "$scratch"
test -e "$scratch/target/preserved"

if bash scripts/ci/trim-runner-cache.sh / >/dev/null 2>&1; then
  echo "cache trimmer accepted filesystem root" >&2
  exit 1
fi

if FORGE_CACHE_TRIM_DRY_RUN=2 \
  bash scripts/ci/trim-runner-cache.sh "$scratch" >/dev/null 2>&1; then
  echo "cache trimmer accepted an ambiguous dry-run value" >&2
  exit 1
fi

fake_bin="$scratch/bin"
docker_log="$scratch/docker-removals"
mkdir -p "$fake_bin"
cat >"$fake_bin/docker" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
if [[ "$1 $2" == "system df" ]]; then
  cat <<'JSON'
{"Volumes":[{"Name":"forge-release-bullseye-target-aarch64-unknown-linux-gnu","Size":"2kB"},{"Name":"forge-release-cargo-registry","Size":"2kB"},{"Name":"unrelated-database","Size":"900GB"}]}
JSON
elif [[ "$1 $2" == "volume rm" ]]; then
  printf '%s\n' "$3" >>"$FORGE_TEST_DOCKER_LOG"
else
  echo "unexpected fake docker invocation: $*" >&2
  exit 2
fi
SH
chmod +x "$fake_bin/docker"

PATH="$fake_bin:$PATH" \
FORGE_TEST_DOCKER_LOG="$docker_log" \
FORGE_TRIM_RELEASE_DOCKER_VOLUMES=1 \
FORGE_MAX_RELEASE_DOCKER_CACHE_KIB=1 \
  bash scripts/ci/trim-runner-cache.sh "$scratch"
test "$(wc -l <"$docker_log")" -eq 2
grep -Fxq "forge-release-bullseye-target-aarch64-unknown-linux-gnu" "$docker_log"
grep -Fxq "forge-release-cargo-registry" "$docker_log"
! grep -Fq "unrelated-database" "$docker_log"

: >"$docker_log"
PATH="$fake_bin:$PATH" \
FORGE_TEST_DOCKER_LOG="$docker_log" \
FORGE_TRIM_RELEASE_DOCKER_VOLUMES=1 \
FORGE_MAX_RELEASE_DOCKER_CACHE_KIB=1 \
FORGE_CACHE_TRIM_DRY_RUN=1 \
  bash scripts/ci/trim-runner-cache.sh "$scratch"
test ! -s "$docker_log"

# Keep the policy wired to every workflow that intentionally leaves a large workspace or Docker
# build cache on a persistent runner.
for workflow in \
  .github/workflows/app-desktop.yml \
  .github/workflows/app-web.yml \
  .github/workflows/ci.yml \
  .github/workflows/e2e.yml \
  .github/workflows/eas-update.yml \
  .github/workflows/mobile-android.yml \
  .github/workflows/mobile-typecheck.yml \
  .github/workflows/release.yml; do
  grep -Fq "scripts/ci/trim-runner-cache.sh" "$workflow" || {
    echo "persistent cache workflow lacks a trim step: $workflow" >&2
    exit 1
  }
done
grep -Fq "FORGE_TRIM_RELEASE_DOCKER_VOLUMES: \"1\"" .github/workflows/release.yml

echo "runner cache trim tests passed"
