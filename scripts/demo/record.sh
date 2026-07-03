#!/usr/bin/env bash
# Re-record the README demo GIFs.
#
#   scripts/demo/record.sh            # stage + record every tape
#   scripts/demo/record.sh hero tui   # stage + record only these tapes
#
# Requires: vhs (with ttyd + ffmpeg + a headless Chrome/Chromium for its renderer) and a
# "JetBrainsMono Nerd Font" install. GIFs land in docs/assets/. Recordings are offline and
# deterministic (see setup.sh) — `mesh.tape` alone reflects the recording machine's catalog.
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
STAGE="$REPO/scripts/demo/.stage"

command -v vhs >/dev/null || { echo "vhs is required: https://github.com/charmbracelet/vhs" >&2; exit 2; }

if [[ $# -gt 0 ]]; then
  names=("$@")
else
  names=()
  for tape in "$REPO"/scripts/demo/tapes/*.tape; do names+=("$(basename "$tape" .tape)"); done
fi

cd "$REPO"
for name in "${names[@]}"; do
  # Re-stage before every tape: each recording starts from the same seeded state (a chat tape
  # writing mock-note.txt must not become the file's blame owner in the provenance tape).
  "$REPO/scripts/demo/setup.sh"
  echo "── recording $name.tape ──"
  vhs "$STAGE/tapes/$name.tape"
done

# Keep the README fast to load: palette-optimize anything ffmpeg can shrink.
for gif in docs/assets/forge-demo.gif docs/assets/demo-*.gif; do
  [[ -f "$gif" ]] || continue
  tmp="$gif.opt.gif"
  if ffmpeg -y -v error -i "$gif" \
      -vf "fps=16,split[a][b];[a]palettegen=max_colors=128[p];[b][p]paletteuse=dither=bayer:bayer_scale=5" \
      "$tmp" && [[ -s "$tmp" && $(stat -c%s "$tmp") -lt $(stat -c%s "$gif") ]]; then
    mv "$tmp" "$gif"
  else
    rm -f "$tmp"
  fi
done

ls -sh docs/assets/forge-demo.gif docs/assets/demo-*.gif 2>/dev/null
