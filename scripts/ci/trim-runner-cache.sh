#!/usr/bin/env bash
set -euo pipefail

# Persistent self-hosted runners intentionally reuse build caches, but Cargo never garbage-collects
# superseded test/build hashes. Keep that speed benefit until a cache crosses a hard disk budget,
# then remove only the known cache root after the current job has finished using it.
workspace="${1:-${GITHUB_WORKSPACE:-$PWD}}"
workspace="$(realpath -m -- "$workspace")"
if [[ -z "$workspace" || "$workspace" == "/" ]]; then
  echo "refusing unsafe runner-cache workspace: ${workspace:-<empty>}" >&2
  exit 2
fi

max_target_kib="${FORGE_MAX_TARGET_CACHE_KIB:-25165824}" # 24 GiB
max_node_modules_kib="${FORGE_MAX_NODE_MODULES_CACHE_KIB:-4194304}" # 4 GiB
max_release_docker_kib="${FORGE_MAX_RELEASE_DOCKER_CACHE_KIB:-25165824}" # 24 GiB
trim_release_docker="${FORGE_TRIM_RELEASE_DOCKER_VOLUMES:-0}"
dry_run="${FORGE_CACHE_TRIM_DRY_RUN:-0}"

case "$max_target_kib:$max_node_modules_kib:$max_release_docker_kib" in
  *[!0-9:]*)
    echo "runner-cache limits must be non-negative integers" >&2
    exit 2
    ;;
esac
if [[ "$dry_run" != 0 && "$dry_run" != 1 ]]; then
  echo "FORGE_CACHE_TRIM_DRY_RUN must be 0 or 1" >&2
  exit 2
fi
if [[ "$trim_release_docker" != 0 && "$trim_release_docker" != 1 ]]; then
  echo "FORGE_TRIM_RELEASE_DOCKER_VOLUMES must be 0 or 1" >&2
  exit 2
fi

trim_if_oversized() {
  local artifact="$1"
  local limit_kib="$2"
  local label="$3"
  [[ -d "$artifact" && ! -L "$artifact" ]] || return 0

  local size_kib
  size_kib="$(du -skx -- "$artifact" | awk '{print $1}')"
  if (( size_kib <= limit_kib )); then
    echo "$label cache is within budget: ${size_kib} KiB <= ${limit_kib} KiB"
    return 0
  fi

  if (( dry_run == 1 )); then
    echo "would remove oversized $label cache: $artifact (${size_kib} KiB > ${limit_kib} KiB)"
    return 0
  fi
  echo "removing oversized $label cache: $artifact (${size_kib} KiB > ${limit_kib} KiB)"
  rm -rf -- "$artifact"
}

cargo_roots=(
  "$workspace/target"
  "$workspace/vendor/genai-0.6.5/target"
  "$workspace/mobile/src-tauri/target"
)
cargo_total_kib=0
for cargo_root in "${cargo_roots[@]}"; do
  if [[ -d "$cargo_root" && ! -L "$cargo_root" ]]; then
    cargo_total_kib=$((cargo_total_kib + $(du -skx -- "$cargo_root" | awk '{print $1}')))
  fi
done
if (( cargo_total_kib <= max_target_kib )); then
  echo "aggregate Cargo cache is within budget: ${cargo_total_kib} KiB <= ${max_target_kib} KiB"
elif (( dry_run == 1 )); then
  echo "would remove oversized aggregate Cargo cache (${cargo_total_kib} KiB > ${max_target_kib} KiB)"
else
  echo "removing oversized aggregate Cargo cache (${cargo_total_kib} KiB > ${max_target_kib} KiB)"
  for cargo_root in "${cargo_roots[@]}"; do
    [[ -d "$cargo_root" && ! -L "$cargo_root" ]] && rm -rf -- "$cargo_root"
  done
fi

trim_if_oversized \
  "$workspace/mobile/node_modules" "$max_node_modules_kib" "mobile node_modules"

trim_release_docker_volumes() {
  (( trim_release_docker == 1 )) || return 0
  for tool in docker jq numfmt; do
    command -v "$tool" >/dev/null || {
      echo "release cache trimming requires $tool" >&2
      return 2
    }
  done

  # These volumes are created only by release.yml's pinned Bullseye builds. Keep the allow-list
  # exact: a broad `forge-*` match could destroy an unrelated developer database or service.
  local release_volumes=(
    "forge-release-bullseye-target-aarch64-unknown-linux-gnu"
    "forge-release-bullseye-target-x86-64-unknown-linux-gnu"
    "forge-release-cargo-git"
    "forge-release-cargo-registry"
  )
  local snapshot
  snapshot="$(docker system df -v --format '{{json .}}')"
  local aggregate_kib=0
  local present_volumes=()
  local volume size size_bytes size_kib
  for volume in "${release_volumes[@]}"; do
    size="$(jq -r --arg name "$volume" \
      '.Volumes[]? | select(.Name == $name) | .Size' <<<"$snapshot" | head -n 1)"
    [[ -n "$size" ]] || continue
    # Docker reports SI strings such as `739.6MB`; numfmt accepts the same suffix without the
    # trailing `B` (`739.6M`).
    size_bytes="$(numfmt --from=si "${size%B}")" || {
      echo "could not parse Docker volume size for $volume: $size" >&2
      return 2
    }
    size_kib=$(((size_bytes + 1023) / 1024))
    aggregate_kib=$((aggregate_kib + size_kib))
    present_volumes+=("$volume")
  done

  if (( aggregate_kib <= max_release_docker_kib )); then
    echo "aggregate release Docker cache is within budget: ${aggregate_kib} KiB <= ${max_release_docker_kib} KiB"
    return 0
  fi
  if (( dry_run == 1 )); then
    echo "would remove oversized aggregate release Docker cache (${aggregate_kib} KiB > ${max_release_docker_kib} KiB)"
    return 0
  fi

  echo "removing oversized aggregate release Docker cache (${aggregate_kib} KiB > ${max_release_docker_kib} KiB)"
  for volume in "${present_volumes[@]}"; do
    # Docker refuses an in-use volume. Propagate that failure instead of forcing removal or
    # pretending the disk bound was restored.
    docker volume rm "$volume"
  done
}

trim_release_docker_volumes
