#!/usr/bin/env bash
# linuxdeploy probes its plugins with no --appdir. The shim used to `exit 1` on that path, which
# under `set -euo pipefail` killed the whole packaging step — that single line is why the
# linux-x86_64 leg never produced an AppImage and three desktop releases stayed unpublished
# drafts. Pin the probe path so it cannot regress.
set -euo pipefail

shim="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/appimage-plugin-shim.sh"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

mkdir -p "$work/plugin-root"
cat > "$work/plugin-root/AppRun" <<'STUB'
#!/usr/bin/env bash
echo "AppRun invoked: $*"
exit 0
STUB
chmod 0755 "$work/plugin-root/AppRun"

printf '#!/usr/bin/env bash\ntouch "%s/backend-fix-ran"\n' "$work" > "$work/backend-fix.sh"
chmod 0755 "$work/backend-fix.sh"

export FORGE_LINUXDEPLOY_PLUGIN_ROOT="$work/plugin-root"
export FORGE_APPIMAGE_BACKEND_FIX="$work/backend-fix.sh"

# A probe with no --appdir must succeed and still delegate to the real plugin.
output="$(bash "$shim" --plugin-api-version)"
[[ "$output" == "AppRun invoked: --plugin-api-version" ]] || {
  echo "probe did not delegate to AppRun; got: $output" >&2
  exit 1
}
[[ -e "$work/backend-fix-ran" ]] && {
  echo "backend fix ran for a probe that carries no appdir" >&2
  exit 1
}

# A real packaging call still applies the backend fix, then delegates.
output="$(bash "$shim" --appdir "$work/AppDir")"
[[ -e "$work/backend-fix-ran" ]] || {
  echo "backend fix did not run for an --appdir invocation" >&2
  exit 1
}
[[ "$output" == "AppRun invoked: --appdir $work/AppDir" ]] || {
  echo "appdir call did not delegate to AppRun; got: $output" >&2
  exit 1
}

echo "appimage plugin shim: 2 passed"
