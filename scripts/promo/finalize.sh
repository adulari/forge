#!/usr/bin/env bash
# Mux the synthesized soundtracks into the silent Remotion renders and place
# the final deliverables in docs/assets/.
#
# Full pipeline:
#   npm run render:main && npm run render:teaser && npm run render:vertical
#   python3 audio/make_audio.py
#   ./finalize.sh
set -euo pipefail
cd "$(dirname "$0")"

ASSETS=../../docs/assets
mkdir -p "$ASSETS"

mux() {
  local video=$1 audio=$2 out=$3
  ffmpeg -hide_banner -loglevel error -y \
    -i "docs-out/$video" -i "audio/$audio" \
    -map 0:v -map 1:a -c:v copy -c:a copy -movflags +faststart -shortest \
    "$ASSETS/$out"
  echo "wrote $ASSETS/$out ($(du -h "$ASSETS/$out" | cut -f1))"
}

mux forge-promo.mp4 promo.m4a forge-promo.mp4
mux forge-promo-teaser.mp4 teaser.m4a forge-promo-teaser.mp4
mux forge-promo-vertical.mp4 vertical.m4a forge-promo-vertical.mp4
