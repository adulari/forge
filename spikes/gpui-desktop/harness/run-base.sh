#!/bin/bash
# Baseline: current Forge Desktop (Tauri + RN-web) off-screen on Xvfb :87, private D-Bus, mock daemon only.
H=$HOME/.cache/forge-spike-gpui; I=$H/inst/base; rm -rf $I; mkdir -p $I/config/forge $I/data $I/cache $I/run; chmod 700 $I/run
cp -f $H/daemon/config/forge/serve-state.json $I/config/forge/ && chmod 600 $I/config/forge/serve-state.json
pgrep -f "Xvfb :87" >/dev/null || { Xvfb :87 -screen 0 2560x1440x24 -nolisten tcp >/dev/null 2>&1 & sleep 1; }
BIN=${1:-$HOME/.cache/forge-target-spike-gpui-base/release/forge-desktop}
env -u WAYLAND_DISPLAY -u HYPRLAND_INSTANCE_SIGNATURE -u DBUS_SESSION_BUS_ADDRESS DISPLAY=:87 GDK_BACKEND=x11 WEBKIT_DISABLE_DMABUF_RENDERER=1 FORGE_NO_REPORTS=1 \
 XDG_CONFIG_HOME=$I/config XDG_DATA_HOME=$I/data XDG_CACHE_HOME=$I/cache XDG_RUNTIME_DIR=$I/run \
 WEBKIT_INSPECTOR_HTTP_SERVER=127.0.0.1:9361 FORGE_DESKTOP_AUTO_UPDATE=0 FORGE_DESKTOP_OTA=0 \
 setsid dbus-run-session -- $BIN > $I/app.log 2>&1 < /dev/null &
echo started
