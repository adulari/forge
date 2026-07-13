#!/bin/sh
# Forge desktop installer. Downloads the matching Tauri bundle from GitHub Releases.
# curl -fsSL https://raw.githubusercontent.com/Adulari/forge/main/install-desktop.sh | sh
set -eu
REPO="Adulari/forge"
err() { printf 'install-desktop: %s\n' "$1" >&2; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || err "required tool not found: $1"; }
need uname
if command -v curl >/dev/null 2>&1; then dl() { curl -fsSL "$1" -o "$2"; }; fetch() { curl -fsSL "$1"; }; elif command -v wget >/dev/null 2>&1; then dl() { wget -qO "$2" "$1"; }; fetch() { wget -qO - "$1"; }; else err 'need curl or wget'; fi
os=$(uname -s); arch=$(uname -m)
case "$os:$arch" in
  Darwin:arm64|Darwin:aarch64) asset="Forge-desktop-macos-aarch64.dmg"; kind=mac-dmg ;;
  Darwin:x86_64) asset="Forge-desktop-macos-x86_64.dmg"; kind=mac-dmg ;;
  Linux:x86_64|Linux:amd64) asset="Forge-desktop-linux-x86_64.AppImage"; kind=appimage ;;
  Linux:arm64|Linux:aarch64) asset="Forge-desktop-linux-aarch64.AppImage"; kind=appimage ;;
  *) err "unsupported platform: $os/$arch (desktop builds: macOS and Linux x86_64/aarch64)" ;;
esac
version=${FORGE_VERSION:-}
[ -n "$version" ] || version=$(fetch "https://api.github.com/repos/$REPO/releases/latest" | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -1)
[ -n "$version" ] || err 'could not resolve latest release tag'
base="https://github.com/$REPO/releases/download/$version"; tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT
printf 'install-desktop: downloading %s %s...\n' "$asset" "$version" >&2
dl "$base/$asset" "$tmp/$asset" || err "download failed: $base/$asset"
case "$kind" in
  mac-dmg)
    need hdiutil
    mount=$(hdiutil attach "$tmp/$asset" -nobrowse -plist | sed -n 's:.*<string>\([^<]*\)</string>.*:\1:p' | tail -1)
    [ -d "$mount" ] || err 'could not mount desktop disk image'
    app=$(find "$mount" -maxdepth 1 -name '*.app' -print -quit)
    [ -n "$app" ] || { hdiutil detach "$mount" >/dev/null; err 'disk image contains no .app'; }
    dest=/Applications
    [ -w "$dest" ] || dest="$HOME/Applications"
    mkdir -p "$dest"
    rm -rf "$dest/Forge.app"
    cp -R "$app" "$dest/Forge.app"
    hdiutil detach "$mount" >/dev/null || true
    printf 'install-desktop: installed Forge.app to %s\n' "$dest" >&2 ;;
  appimage)
    dest="${FORGE_DESKTOP_DIR:-$HOME/.local/bin}"; data="${XDG_DATA_HOME:-$HOME/.local/share}"
    mkdir -p "$dest" "$data/applications" "$data/icons/hicolor/256x256/apps"
    appimg="$tmp/$asset"; chmod 0755 "$appimg"
    appdir="$data/forge-desktop"
    # Prefer running the EXTRACTED app via its AppRun rather than the AppImage directly: the
    # bundled type-2 runtime fails to self-mount on some modern kernels ("fuse: memory allocation
    # failed") even though the squashfs payload is perfectly valid, so a plain `Exec=...AppImage`
    # silently never opens. Extraction sidesteps the runtime entirely.
    rm -rf "$appdir"; extracted=""
    # 1) The AppImage's own extractor (no extra deps) — works on most hosts.
    if ( cd "$tmp" && "$appimg" --appimage-extract >/dev/null 2>&1 ) && [ -d "$tmp/squashfs-root" ]; then
      mv "$tmp/squashfs-root" "$appdir"; extracted=1
    # 2) Fallback: the runtime's extractor is broken too on some hosts; use system unsquashfs at the
    #    payload offset (--appimage-offset is a plain ELF read, so it works even when mounting doesn't).
    elif command -v unsquashfs >/dev/null 2>&1; then
      off=$("$appimg" --appimage-offset 2>/dev/null || echo 0)
      [ "${off:-0}" -gt 0 ] && unsquashfs -f -o "$off" -d "$appdir" "$appimg" >/dev/null 2>&1 && extracted=1
    fi
    if [ -n "$extracted" ] && [ -x "$appdir/AppRun" ]; then
      cat > "$dest/forge-desktop" <<EOF
#!/bin/sh
# Launch the extracted app via AppRun (the bundled AppImage runtime fails on some kernels).
exec "$appdir/AppRun" "\$@"
EOF
      chmod 0755 "$dest/forge-desktop"
      exec_line="$dest/forge-desktop"
      # Drop any stale AppImage from an older install so it doesn't shadow the launcher.
      rm -f "$dest/forge-desktop.AppImage"
      printf 'install-desktop: installed Forge (extracted) to %s, launcher %s\n' "$appdir" "$dest/forge-desktop" >&2
    else
      # Last resort: install the AppImage as-is (extraction unavailable, e.g. no unsquashfs).
      cp "$appimg" "$dest/forge-desktop.AppImage"; chmod 0755 "$dest/forge-desktop.AppImage"
      exec_line="$dest/forge-desktop.AppImage"
      printf 'install-desktop: installed Forge AppImage to %s (could not extract; install squashfs-tools if it will not launch)\n' "$dest/forge-desktop.AppImage" >&2
    fi
    # The release companion icon is optional; the desktop entry remains launcher-visible without it.
    dl "$base/Forge-desktop-linux-icon.png" "$data/icons/hicolor/256x256/apps/forge-desktop.png" 2>/dev/null || true
    cat > "$data/applications/forge-desktop.desktop" <<EOF
[Desktop Entry]
Name=Forge
Comment=Forge desktop app
Exec=$exec_line
Icon=forge-desktop
Terminal=false
Type=Application
Categories=Development;
EOF
    printf 'install-desktop: desktop entry installed\n' >&2 ;;
esac
