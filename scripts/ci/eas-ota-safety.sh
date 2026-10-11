#!/usr/bin/env bash
set -euo pipefail

safe=true
ota_changed=false
unsafe=()
dependency_files=()
fingerprint_checked=false
scratch_dirs=()

cleanup() {
  local dir
  for dir in "${scratch_dirs[@]}"; do
    git worktree remove --force "$dir/base" >/dev/null 2>&1 || true
    rm -rf -- "$dir"
  done
}
trap cleanup EXIT

# Prints the iOS runtime fingerprint of the mobile app checked out under $1. OTA_FINGERPRINT_CMD
# replaces the real computation in tests.
fingerprint_of() {
  local root=$1
  if [[ -n ${OTA_FINGERPRINT_CMD:-} ]]; then
    "$OTA_FINGERPRINT_CMD" "$root"
    return
  fi
  (
    cd "$root/mobile"
    [[ -d node_modules ]] || npm ci --ignore-scripts --no-audit --no-fund >&2
    npx --no-install @expo/fingerprint fingerprint:generate --platform ios |
      node -e 'let s="";process.stdin.on("data",d=>s+=d).on("end",()=>console.log(JSON.parse(s).hash))'
  )
}

classify_path() {
  local path=$1

  case "$path" in
    mobile/src/*|mobile/assets/*)
      ota_changed=true
      ;;
    mobile/src-tauri/*|mobile/redesign/*|mobile/*.md)
      # Neutral mobile helpers and documentation do not enter the iOS runtime.
      ;;
    mobile/.gitignore|mobile/parity/*|mobile/public/*)
      # Named exceptions to the fail-closed default below, each verified not to reach iOS:
      #   .gitignore   — VCS metadata; never bundled, never changes the runtime fingerprint.
      #   parity/*     — surface-parity tracking data, read by humans and scripts, not by the app.
      #   public/*     — Expo's web-only static directory, copied into web exports; `eas update`
      #                  publishes `--platform ios`, so nothing here can reach the device.
      # Without these, a favicon edit reports as a native change and hides the real reason an OTA
      # was refused. Anything else under mobile/ still falls through to unsafe.
      ;;
    mobile/package.json|mobile/package-lock.json)
      # Judged by the runtime fingerprint after the whole range is known (see below): a
      # transitive lockfile bump or a non-native package.json edit leaves it unchanged and
      # cannot make an installed binary incompatible with the bundle.
      dependency_files+=("$path")
      ;;
    mobile/*)
      unsafe+=("$path")
      ;;
    *)
      # Neutral: does not enter the iOS OTA bundle or native runtime.
      ;;
  esac
}

if (($#)); then
  for path in "$@"; do
    classify_path "$path"
  done
else
  base_sha=${BASE_SHA:-}
  head_sha=${HEAD_SHA:-}

  if [[ -z $base_sha || $base_sha == 0000000000000000000000000000000000000000 ]]; then
    safe=false
    printf 'Missing trustworthy push base; refusing OTA publication.\n'
  else
    if [[ -z $head_sha ]]; then
      echo "HEAD_SHA is required for OTA range classification" >&2
      exit 1
    fi

    if ! base_commit=$(git rev-parse --verify --end-of-options "${base_sha}^{commit}"); then
      echo "Unable to resolve OTA baseline as a commit" >&2
      exit 1
    fi
    if ! head_commit=$(git rev-parse --verify --end-of-options "${head_sha}^{commit}"); then
      echo "Unable to resolve OTA head as a commit" >&2
      exit 1
    fi
    if ! git merge-base --is-ancestor "$base_commit" "$head_commit"; then
      echo "OTA baseline is not an ancestor of the requested head" >&2
      exit 1
    fi

    changed_paths=$(mktemp)
    scratch_dirs+=("$changed_paths")
    if git diff --no-renames --name-only -z "$base_commit" "$head_commit" > "$changed_paths"; then
      while IFS= read -r -d '' path; do
        classify_path "$path"
      done < "$changed_paths"
    else
      status=$?
      echo "Unable to inspect OTA push range" >&2
      exit "$status"
    fi
  fi
fi

if [[ $safe == true ]] && ((${#dependency_files[@]})); then
  # Only a range diff has a baseline to compare against; positional paths cannot be verified.
  if [[ -z ${base_commit:-} ]] || [[ $(git rev-parse HEAD) != "${head_commit:-}" ]]; then
    unsafe+=("${dependency_files[@]}")
  else
    work=$(mktemp -d)
    scratch_dirs+=("$work")
    git worktree add -q --detach "$work/base" "$base_commit"
    base_fp=$(fingerprint_of "$work/base" 2>/dev/null || true)
    head_fp=$(fingerprint_of "$(git rev-parse --show-toplevel)" 2>/dev/null || true)
    if [[ -n $base_fp && $base_fp == "$head_fp" ]]; then
      printf 'Dependency files changed but the iOS runtime fingerprint is unchanged (%s).\n' "$base_fp"
    else
      printf 'Dependency files changed the iOS runtime fingerprint (base=%s head=%s).\n' "${base_fp:-unavailable}" "${head_fp:-unavailable}"
      unsafe+=("${dependency_files[@]}")
    fi
  fi
fi

if ((${#unsafe[@]})); then
  safe=false
  for path in "${unsafe[@]}"; do
    printf 'Unsafe OTA path: %s\n' "$path"
  done
fi

output=${GITHUB_OUTPUT:-/dev/stdout}
printf 'safe=%s\nota_changed=%s\n' "$safe" "$ota_changed" >> "$output"
