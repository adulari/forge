#!/usr/bin/env bash
set -euo pipefail

# Guards scripts/ci/appimage-wayland-backend.sh, which is what stops the AppImage shipping with
# GDK_BACKEND pinned to x11 or pre-empting the app's own choice. It runs at package time inside the
# desktop release workflow, where a
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

# 1. The x11 pin is removed without replacement, and the rest of the hook survives. Setting any
# backend here would make the app mistake it for the user's choice.
appdir="$work/ok"
make_hook "$appdir"
bash "$fixer" "$appdir" >/dev/null
hook="$appdir/apprun-hooks/linuxdeploy-plugin-gtk.sh"
grep -q 'GDK_BACKEND=' "$hook" \
  && { echo 'the patched hook must not assign GDK_BACKEND' >&2; exit 1; }
grep -q 'WEBKIT_DISABLE_DMABUF_RENDERER' "$hook" \
  && { echo 'the patched hook must leave the renderer to the app' >&2; exit 1; }
grep -q 'XDG_DATA_DIRS' "$hook" \
  || { echo 'unrelated hook lines must be preserved' >&2; exit 1; }

# 1b. Sourced on a Wayland session, an X11 host and with an explicit user value, the hook leaves
# the user's environment exactly as it found it.
for setup in 'WAYLAND_DISPLAY=wayland-0' 'DISPLAY=:0'; do
  env -u GDK_BACKEND -u WAYLAND_DISPLAY -u DISPLAY -u WEBKIT_DISABLE_DMABUF_RENDERER "$setup" \
    bash -c 'source "$1"; [[ -z ${GDK_BACKEND:-} && -z ${WEBKIT_DISABLE_DMABUF_RENDERER:-} ]]' _ "$hook" \
    || { echo "the hook must not set the backend or renderer ($setup)" >&2; exit 1; }
done
env GDK_BACKEND=x11 bash -c 'source "$1"; [[ $GDK_BACKEND == x11 ]]' _ "$hook" \
  || { echo 'a user-chosen backend must survive the hook' >&2; exit 1; }

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
