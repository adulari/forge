#!/usr/bin/env bash
set -euo pipefail

usage() {
  echo "usage: $0 MARKERS_FILE ARTIFACT..." >&2
  echo "MARKERS_FILE must contain one ASCII canary (at least 16 bytes) per non-empty line." >&2
}

if (( $# < 2 )); then
  usage
  exit 2
fi

markers_file=$1
shift

if [[ ! -f "$markers_file" || ! -r "$markers_file" ]]; then
  echo "canary scan: marker file is not a readable regular file" >&2
  exit 2
fi

if [[ ! -s "$markers_file" || $(wc -c < "$markers_file") -gt 4096 ]]; then
  echo "canary scan: marker file must contain 1-4096 bytes" >&2
  exit 2
fi

if ! LC_ALL=C awk '
  /[^ -~]/ { exit 1 }
  length($0) < 16 { exit 1 }
  END { if (NR == 0) exit 1 }
' "$markers_file"; then
  echo "canary scan: every marker must be a non-empty printable ASCII line of at least 16 bytes" >&2
  exit 2
fi

marker_path=$(realpath "$markers_file")
declare -a artifacts=()

for target in "$@"; do
  if [[ -f "$target" ]]; then
    artifacts+=("$target")
  elif [[ -d "$target" ]]; then
    while IFS= read -r -d '' artifact; do
      artifacts+=("$artifact")
    done < <(find "$target" -type f -print0)
  else
    echo "canary scan: artifact target does not exist or is not a regular file/directory: $target" >&2
    exit 2
  fi
done

if (( ${#artifacts[@]} == 0 )); then
  echo "canary scan: no artifact files found" >&2
  exit 2
fi

declare -a matches=()
scanned=0

for artifact in "${artifacts[@]}"; do
  artifact_path=$(realpath "$artifact")
  [[ "$artifact_path" == "$marker_path" ]] && continue
  if [[ ! -r "$artifact" ]]; then
    echo "canary scan: artifact is not readable: $artifact" >&2
    exit 2
  fi

  set +e
  LC_ALL=C grep -a -F -q -f "$markers_file" -- "$artifact"
  grep_status=$?
  set -e

  case $grep_status in
    0) matches+=("$artifact") ;;
    1) ;;
    *)
      echo "canary scan: failed to scan artifact: $artifact" >&2
      exit 2
      ;;
  esac
  ((scanned += 1))
done

if (( ${#matches[@]} > 0 )); then
  echo "canary scan FAILED: known plaintext found in ${#matches[@]} captured artifact(s):" >&2
  printf '  %s\n' "${matches[@]}" >&2
  echo "Quarantine the capture and investigate; the marker value is intentionally not printed." >&2
  exit 1
fi

echo "canary scan passed: $scanned captured artifact file(s), no known plaintext"
