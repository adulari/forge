#!/usr/bin/env bash
# Stand up the local SearXNG that Forge's web_search uses by default.
#
# Why this exists: every keyless search endpoint throttles per IP, and the ones that don't throttle
# lie. Measured from one residential IP on 2026-09-07 — DuckDuckGo answered 1 query then returned
# empty 202s; keyless Brave managed 5 before HTTP 429; keyless Bing never throttled but returned
# plumbers near 1 Microsoft Way for "tokio select macro". A local SearXNG answered 30/30 in 22s
# with the correct docs.rs and tokio.rs pages every time. It is the only free option that is both
# unlimited and accurate, because the quota being spent is your own IP's, spread over 70+ engines.
set -euo pipefail
NAME="${SEARXNG_CONTAINER:-forge-searxng}"
PORT="${SEARXNG_PORT:-8888}"
# Must live under a path the container runtime is allowed to share (Docker Desktop only shares
# $HOME by default, which is why this is not in /tmp).
CONF="${SEARXNG_CONFIG:-$HOME/.config/forge/searxng}"

mkdir -p "$CONF"
if [ ! -f "$CONF/settings.yml" ]; then
  cat > "$CONF/settings.yml" <<EOF
use_default_settings: true
server:
  # Local-only instance: no rate limiter to fight, and not a public endpoint.
  secret_key: "forge-local-searxng"
  limiter: false
  public_instance: false
search:
  formats:
    - html
    - json   # REQUIRED — without this SearXNG serves HTML and Forge cannot parse it
EOF
  echo "[searxng] wrote $CONF/settings.yml"
fi

docker rm -f "$NAME" >/dev/null 2>&1 || true
docker run -d --name "$NAME" --restart unless-stopped \
  -p "$PORT:8080" -v "$CONF:/etc/searxng:rw" searxng/searxng:latest >/dev/null
echo "[searxng] container $NAME starting on :$PORT"

for _ in $(seq 1 30); do
  if [ "$(curl -s -o /dev/null -w '%{http_code}' --max-time 5 \
        "http://localhost:$PORT/search?q=test&format=json" 2>/dev/null)" = "200" ]; then
    echo "[searxng] ready — Forge picks this up automatically on http://localhost:$PORT"
    echo "[searxng] (a different port needs FORGE_SEARXNG_URL=http://localhost:$PORT)"
    exit 0
  fi
  sleep 2
done
echo "[searxng] did not answer JSON within 60s; check: docker logs $NAME" >&2
exit 1
