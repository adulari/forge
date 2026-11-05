#!/usr/bin/env bash
set -euo pipefail

# Make the packaged AppImage launch on the same backend as the direct binary.
#
# linuxdeploy-plugin-gtk writes AppDir/apprun-hooks/linuxdeploy-plugin-gtk.sh, and that generated
# hook contains:
#
#     export GDK_BACKEND=x11 # Crash with Wayland backend on Wayland - We tested it without it
#                            # and ended up with this: https://github.com/tauri-apps/tauri/issues/8541
#
# So every AppImage user has been running through XWayland — a different rendering and input stack
# from the direct binary, which runs native Wayland. That is the defect in docs/known-issues.md.
#
# Verified on a Hyprland/wlroots session against the shipped v2.12.1 AppImage:
#
#   GDK_BACKEND=wayland alone                     -> Gdk-Message: Error 71 (Protocol error)
#                                                    dispatching to Wayland display, exit 1
#   GDK_BACKEND=wayland + dmabuf renderer off     -> ran to the 15s timeout, no crash, and
#                                                    `hyprctl clients` reported
#                                                    class=forge-desktop xwayland=False
#
# The upstream x11 pin is therefore treating a symptom: the crash is WebKitGTK's dmabuf renderer,
# not the Wayland backend itself. The pin is removed and the app (linux_render.rs) chooses the
# backend and disables that renderer at startup, identically for the AppImage and the direct
# binary. AppRun must not export GDK_BACKEND at all: the app treats any value present at startup as
# the user's own choice and never overrides it, and it applies its one XWayland fallback (Wayland +
# proprietary NVIDIA + WebKitGTK 2.54) only while the variable is unset.

appdir=${1:?usage: appimage-wayland-backend.sh <AppDir>}
hook="$appdir/apprun-hooks/linuxdeploy-plugin-gtk.sh"

if [[ ! -f $hook ]]; then
  echo "appimage-wayland-backend: no GTK hook at $hook" >&2
  exit 1
fi

if ! grep -q '^export GDK_BACKEND=x11' "$hook"; then
  # Upstream changed the hook. Fail loudly rather than silently shipping XWayland again.
  echo "appimage-wayland-backend: expected 'export GDK_BACKEND=x11' in $hook; refusing to guess" >&2
  exit 1
fi

python3 - "$hook" <<'PY'
import sys
from pathlib import Path

hook = Path(sys.argv[1])
text = hook.read_text()
replacement = (
    '# Patched by scripts/ci/appimage-wayland-backend.sh: GDK_BACKEND is deliberately NOT set here.\n'
    '# Forge picks the backend and the WebKitGTK workarounds itself at startup\n'
    '# (mobile/src-tauri/src/linux_render.rs), and it can only tell a value the user chose from one\n'
    '# chosen for them if nothing in AppRun sets it.'
)
lines = text.splitlines()
for index, line in enumerate(lines):
    if line.startswith('export GDK_BACKEND=x11'):
        lines[index] = replacement
        break
hook.write_text('\n'.join(lines) + '\n')
PY

echo "appimage-wayland-backend: patched $hook to prefer Wayland with the dmabuf renderer disabled"
