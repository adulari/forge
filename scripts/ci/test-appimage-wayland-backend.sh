#!/usr/bin/env bash
set -euo pipefail

# Guards scripts/ci/appimage-wayland-backend.sh, which is what stops the AppImage shipping with
# GDK_BACKEND pinned to x11. It runs at package time inside the desktop release workflow, where a
# silent no-op would ship an XWayland build again without anyone noticing.

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
fixer="$script_dir/appimage-wayland-backend.sh"
work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT

make_hook() {
  mkdir -p "$1/apprun-hooks"
  cat > "$1/apprun-hooks/linuxdeploy-plugin-gtk.sh" <<'HOOK'
#! /usr/bin/env bash
export GTK_THEME="$APPIMAGE_GTK_THEME"
export GDK_BACKEND=x11 # Crash with Wayland backend on Wayland - We tested it without it and ended up with this: https://github.com/tauri-apps/tauri/issues/8541
export XDG_DATA_DIRS="$APPDIR/usr/share:/usr/share:$XDG_DATA_DIRS"
HOOK
}

# 1. The x11 pin is replaced, both values stay overridable, and the rest of the hook survives.
appdir="$work/ok"
make_hook "$appdir"
bash "$fixer" "$appdir" >/dev/null
hook="$appdir/apprun-hooks/linuxdeploy-plugin-gtk.sh"
grep -q '^if \[\[ -n \${WAYLAND_DISPLAY:-} && -z \${GDK_BACKEND:-} \]\]; then$' "$hook" \
  || { echo 'expected a Wayland-session guard around the GDK_BACKEND default' >&2; exit 1; }
grep -q '^  export GDK_BACKEND=wayland$' "$hook" \
  || { echo 'expected the Wayland-preferring GDK_BACKEND export' >&2; exit 1; }
grep -q '^  export WEBKIT_DISABLE_DMABUF_RENDERER="${WEBKIT_DISABLE_DMABUF_RENDERER:-1}"$' "$hook" \
  || { echo 'expected the dmabuf renderer to be disabled' >&2; exit 1; }
grep -q '^export GDK_BACKEND=x11' "$hook" \
  && { echo 'the x11 pin survived the patch' >&2; exit 1; }
grep -q 'XDG_DATA_DIRS' "$hook" \
  || { echo 'unrelated hook lines must be preserved' >&2; exit 1; }

# 1b. The default must not break X11-only hosts. With no Wayland display and no explicit backend,
# the hook leaves GDK_BACKEND unset so GTK can select X11 normally.
if env -u WAYLAND_DISPLAY -u GDK_BACKEND bash -c 'source "$1"; [[ -z ${GDK_BACKEND:-} ]]' _ "$hook"; then
  :
else
  echo 'X11-only environments must not be forced onto the Wayland backend' >&2
  exit 1
fi

# 1c. A Wayland session receives the native backend default when the caller has not overridden it.
if env -u GDK_BACKEND WAYLAND_DISPLAY=wayland-0 bash -c 'source "$1"; [[ $GDK_BACKEND == wayland ]]' _ "$hook"; then
  :
else
  echo 'Wayland sessions must receive the native backend default' >&2
  exit 1
fi

# 1d. The dmabuf workaround exists for the Wayland crash (Gdk Error 71) only. X11 ships today with
# the renderer enabled, so an X11 host must not silently lose it.
if env -u WAYLAND_DISPLAY -u GDK_BACKEND -u WEBKIT_DISABLE_DMABUF_RENDERER \
    bash -c 'source "$1"; [[ -z ${WEBKIT_DISABLE_DMABUF_RENDERER:-} ]]' _ "$hook"; then
  :
else
  echo 'an X11-only host must keep the dmabuf renderer enabled' >&2
  exit 1
fi

# 1e. A caller who selects Wayland explicitly still needs the workaround, even though the hook did
# not choose the backend for them.
if env -u WEBKIT_DISABLE_DMABUF_RENDERER GDK_BACKEND=wayland \
    bash -c 'source "$1"; [[ ${WEBKIT_DISABLE_DMABUF_RENDERER:-} == 1 ]]' _ "$hook"; then
  :
else
  echo 'an explicit Wayland backend must still disable the dmabuf renderer' >&2
  exit 1
fi

# 2. A hook without the expected pin must fail loudly. If linuxdeploy changes its template, the
#    build has to stop rather than quietly ship XWayland again.
appdir="$work/changed"
mkdir -p "$appdir/apprun-hooks"
printf '#! /usr/bin/env bash\nexport GTK_THEME=x\n' > "$appdir/apprun-hooks/linuxdeploy-plugin-gtk.sh"
if bash "$fixer" "$appdir" >/dev/null 2>&1; then
  echo 'a changed upstream hook must be rejected, not silently accepted' >&2
  exit 1
fi

# 3. A missing hook is an error too.
appdir="$work/missing"
mkdir -p "$appdir"
if bash "$fixer" "$appdir" >/dev/null 2>&1; then
  echo 'a missing GTK hook must be rejected' >&2
  exit 1
fi

echo 'appimage wayland backend fix is enforced'
