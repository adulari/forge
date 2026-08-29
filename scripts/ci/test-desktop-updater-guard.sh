#!/usr/bin/env bash
set -euo pipefail

# Guards scripts/ci/desktop-updater-guard.sh. The regression it exists to catch is the exact
# config that took v2.13.0 down, so that case is tested first.

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
guard="$script_dir/desktop-updater-guard.sh"
work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT

write_conf() {
  python3 - "$1" "$2" <<'PY'
import json
import sys

artifacts = sys.argv[2]
value = {"true": True, "false": False, "null": None}.get(artifacts, artifacts)
conf = {
    "version": "2.13.0",
    "plugins": {"updater": {"pubkey": "dW50cnVzdGVk", "endpoints": ["https://example.invalid"]}},
    "bundle": {"active": True, "createUpdaterArtifacts": value},
}
if value is None:
    del conf["bundle"]["createUpdaterArtifacts"]
open(sys.argv[1], "w").write(json.dumps(conf))
PY
}

# 1. The shipping configuration passes.
write_conf "$work/tauri.conf.json" true
bash "$guard" "$work/tauri.conf.json" >/dev/null \
  || { echo 'createUpdaterArtifacts true must pass' >&2; exit 1; }

# 2. The real defect: signing silently disabled, so no platform emits a .sig.
write_conf "$work/tauri.conf.json" false
if output=$(bash "$guard" "$work/tauri.conf.json" 2>&1); then
  echo 'createUpdaterArtifacts false must fail' >&2
  exit 1
fi
grep -q 'createUpdaterArtifacts' <<<"$output" \
  || { echo 'the error must name the flag' >&2; exit 1; }

# 3. The deprecated v1 layout is not what app-desktop.yml collects, so it must not pass either.
write_conf "$work/tauri.conf.json" v1Compatible
bash "$guard" "$work/tauri.conf.json" >/dev/null 2>&1 \
  && { echo 'v1Compatible must fail' >&2; exit 1; }

# 4. An absent flag defaults to disabled in tauri-cli; it must not pass by omission.
write_conf "$work/tauri.conf.json" null
bash "$guard" "$work/tauri.conf.json" >/dev/null 2>&1 \
  && { echo 'a missing flag must fail' >&2; exit 1; }

# 5. Artifacts without a pubkey cannot be signed; tauri-cli errors, so fail here first.
python3 -c "
import json, sys
conf = {'bundle': {'createUpdaterArtifacts': True}, 'plugins': {'updater': {'endpoints': []}}}
open(sys.argv[1], 'w').write(json.dumps(conf))
" "$work/tauri.conf.json"
bash "$guard" "$work/tauri.conf.json" >/dev/null 2>&1 \
  && { echo 'a missing pubkey must fail' >&2; exit 1; }

# 6. Malformed or missing inputs fail loudly rather than passing by accident.
printf '{ not json' > "$work/tauri.conf.json"
bash "$guard" "$work/tauri.conf.json" >/dev/null 2>&1 \
  && { echo 'invalid JSON must fail' >&2; exit 1; }

bash "$guard" "$work/absent.json" >/dev/null 2>&1 \
  && { echo 'a missing file must fail' >&2; exit 1; }

# 7. The checked-in configuration is the one that ships; it must satisfy the guard as committed.
bash "$guard" >/dev/null \
  || { echo 'the committed tauri.conf.json must satisfy the guard' >&2; exit 1; }

echo 'desktop updater artifact guard is enforced'
