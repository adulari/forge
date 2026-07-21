#!/usr/bin/env bash
set -euo pipefail

safe=true
ota_changed=false
unsafe=()

classify_path() {
  local path=$1

  case "$path" in
    mobile/src/*|mobile/assets/*)
      ota_changed=true
      ;;
    mobile/src-tauri/*|mobile/redesign/*|mobile/*.md)
      # Neutral mobile helpers and documentation do not enter the iOS runtime.
      ;;
    mobile/*)
      unsafe+=("$path")
      ;;
    *)
      # Neutral: does not enter the iOS OTA bundle or native runtime.
      ;;
  esac
}

if [[ ${EVENT_NAME:-} == workflow_dispatch ]]; then
  safe=true
  ota_changed=true
elif (($#)); then
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

    changed_paths=$(mktemp)
    trap 'rm -f -- "$changed_paths"' EXIT
    if git diff --no-renames --name-only -z "$base_sha" "$head_sha" > "$changed_paths"; then
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

if ((${#unsafe[@]})); then
  safe=false
  for path in "${unsafe[@]}"; do
    printf 'Unsafe OTA path: %s\n' "$path"
  done
fi

output=${GITHUB_OUTPUT:-/dev/stdout}
printf 'safe=%s\nota_changed=%s\n' "$safe" "$ota_changed" >> "$output"
