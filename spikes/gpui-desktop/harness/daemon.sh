#!/bin/bash
D=$HOME/.cache/forge-spike-gpui/daemon
env -i PATH=/usr/bin:/bin HOME=$D/home XDG_CONFIG_HOME=$D/config XDG_DATA_HOME=$D/data XDG_CACHE_HOME=$D/cache FORGE_NO_REPORTS=1 FORGE_MOCK_STREAM_DELAY_MS=20 \
  setsid $HOME/.cache/forge-apptest/ide-daemon/forge-bin-c serve --local --port 7611 --mock > $D/serve.log 2>&1 < /dev/null &
echo $! > $D/pid
