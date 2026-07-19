#!/usr/bin/env bash
# Assert that Forge's portable Linux artifact starts on the documented baseline without pulling in
# a host audio runtime or newer glibc/libstdc++ symbol versions.
# Usage: scripts/check-linux-runtime-deps.sh [path/to/forge]
set -euo pipefail

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "Linux runtime dependency check skipped on $(uname -s)"
  exit 0
fi

binary="${1:-target/release/forge}"
if [[ ! -x "$binary" ]]; then
  echo "Forge binary is missing or not executable: $binary" >&2
  exit 1
fi
if ! command -v readelf >/dev/null 2>&1; then
  echo "readelf is required to verify Linux runtime dependencies" >&2
  exit 1
fi

dynamic_section="$(readelf -d "$binary")"
if [[ "$dynamic_section" == *"libasound.so"* ]]; then
  echo "Forge unexpectedly requires ALSA at process startup:" >&2
  printf '%s\n' "$dynamic_section" >&2
  exit 1
fi

version_info="$(readelf --version-info "$binary")"
max_required() {
  local prefix="$1"
  { grep -oE "${prefix}_[0-9]+(\\.[0-9]+)+" <<<"$version_info" || true; } \
    | sed "s/^${prefix}_//" \
    | sort -Vu \
    | tail -n 1
}
version_exceeds() {
  local actual="$1" maximum="$2"
  [[ "$actual" != "$maximum" && "$(printf '%s\n' "$actual" "$maximum" | sort -V | tail -n 1)" == "$actual" ]]
}

glibc="$(max_required GLIBC)"
glibcxx="$(max_required GLIBCXX)"
if [[ -z "$glibc" ]] || version_exceeds "$glibc" 2.31; then
  echo "Forge requires unsupported glibc $glibc (maximum: 2.31): $binary" >&2
  exit 1
fi
if [[ -n "$glibcxx" ]] && version_exceeds "$glibcxx" 3.4.28; then
  echo "Forge requires unsupported libstdc++ $glibcxx (maximum GLIBCXX: 3.4.28): $binary" >&2
  exit 1
fi
glibcxx="${glibcxx:-not linked}"

machine="$(readelf -h "$binary" | sed -n 's/^ *Machine: *//p')"
host="$(uname -m)"
case "$host:$machine" in
  x86_64:*X86-64*|aarch64:AArch64*)
    "$binary" --version >/dev/null
    startup="startup passed"
    ;;
  *)
    # Cross-architecture release legs execute under QEMU inside the Bullseye build container. The
    # host still performs every static dependency/floor check here.
    startup="startup skipped on $host for $machine"
    ;;
esac

echo "Forge portable runtime check passed ($startup; GLIBC <= $glibc; GLIBCXX <= $glibcxx): $binary"
