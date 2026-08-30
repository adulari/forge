#!/usr/bin/env bash
# Shim wrapped around the linuxdeploy AppImage plugin by the desktop release workflow. linuxdeploy
# invokes the plugin without --appdir as a probe, so that path must fall through to the real
# plugin instead of failing — under `set -euo pipefail` an early exit 1 here fails the whole
# packaging step before the AppImage is ever built.
set -euo pipefail

appdir=""
previous=""
for argument in "$@"; do
  [[ $previous == --appdir ]] && appdir="$argument"
  [[ $argument == --appdir=* ]] && appdir="${argument#--appdir=}"
  previous="$argument"
done
if [[ -n "$appdir" ]]; then
  bash "${FORGE_APPIMAGE_BACKEND_FIX:?}" "$appdir"
else
  echo 'appimage plugin invoked without --appdir; skipping backend fix and delegating to the real plugin' >&2
fi
exec "${FORGE_LINUXDEPLOY_PLUGIN_ROOT:?}/AppRun" "$@"
