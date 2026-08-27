#!/usr/bin/env bash
set -euo pipefail

# Guards scripts/ci/desktop-version-guard.sh. The guard only earns its place if it fails on the
# drift it exists to catch, so that case is tested first.

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
guard="$script_dir/desktop-version-guard.sh"
work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT

write_manifest() {
  cat > "$1" <<TOML
[workspace]
members = ["crates/*"]

[workspace.package]
version = "$2"
edition = "2021"
TOML
}

write_conf() { printf '{\n  "version": "%s",\n  "productName": "Forge"\n}\n' "$2" > "$1"; }

# 1. Matching versions pass.
write_manifest "$work/Cargo.toml" 2.12.2
write_conf "$work/tauri.conf.json" 2.12.2
bash "$guard" "$work/Cargo.toml" "$work/tauri.conf.json" >/dev/null \
  || { echo 'matching versions must pass' >&2; exit 1; }

# 2. The real defect: the bundle left behind at an older version.
write_conf "$work/tauri.conf.json" 2.6.6
if output=$(bash "$guard" "$work/Cargo.toml" "$work/tauri.conf.json" 2>&1); then
  echo 'a drifted bundle version must fail' >&2
  exit 1
fi
grep -q '2.6.6' <<<"$output" || { echo 'the error must name the drifted version' >&2; exit 1; }
grep -q '2.12.2' <<<"$output" || { echo 'the error must name the workspace version' >&2; exit 1; }

# 3. A per-crate version must not be mistaken for the workspace one. Without the section-scoped
#    match this passes by comparing against the wrong number entirely.
cat > "$work/Cargo.toml" <<'TOML'
[package]
name = "decoy"
version = "2.6.6"

[workspace.package]
version = "2.12.2"
TOML
write_conf "$work/tauri.conf.json" 2.6.6
bash "$guard" "$work/Cargo.toml" "$work/tauri.conf.json" >/dev/null 2>&1 \
  && { echo 'a [package] version must not satisfy the workspace comparison' >&2; exit 1; }

# 4. Malformed or missing inputs fail loudly rather than passing by accident.
write_manifest "$work/Cargo.toml" 2.12.2
printf '{ not json' > "$work/tauri.conf.json"
bash "$guard" "$work/Cargo.toml" "$work/tauri.conf.json" >/dev/null 2>&1 \
  && { echo 'invalid JSON must fail' >&2; exit 1; }

printf '{"productName":"Forge"}' > "$work/tauri.conf.json"
bash "$guard" "$work/Cargo.toml" "$work/tauri.conf.json" >/dev/null 2>&1 \
  && { echo 'a missing version field must fail' >&2; exit 1; }

bash "$guard" "$work/Cargo.toml" "$work/absent.json" >/dev/null 2>&1 \
  && { echo 'a missing file must fail' >&2; exit 1; }

echo 'desktop version guard is enforced'
