#!/bin/bash
# GPUI spike off-screen on Xvfb :87 against the isolated mock daemon. Usage: run-gpui.sh [ide|chat] ; prints pid
H=$HOME/.cache/forge-spike-gpui
IDS=$(python3 -c "import json;print(json.load(open('$H/ids.json'))['${CHAT_KEY:-long}'])")
rm -rf $H/spikedir; mkdir -p $H/spikedir
env -u WAYLAND_DISPLAY -u HYPRLAND_INSTANCE_SIGNATURE DISPLAY=:87 XDG_CONFIG_HOME=$H/daemon/config SPIKE_DIR=$H/spikedir SPIKE_VIEW=${1:-ide} SPIKE_CHAT_SESSION=$IDS ${EXTRA_ENV} \
  setsid ${BIN:-$HOME/.cache/forge-target-spike-gpui/release/forge-gpui-spike} > $H/spikedir/app.log 2>&1 < /dev/null &
echo $!
