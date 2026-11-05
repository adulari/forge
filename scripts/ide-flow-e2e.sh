#!/usr/bin/env bash
# F2 -> j/k -> accept hunk -> revert + undo -> F2 back, against `forge serve --local --mock` in an
# isolated HOME with a scratch git repo. Never touches the real daemon or store.
#   scripts/ide-flow-e2e.sh            (needs target/release/forge, or FORGE_BIN=<path>)
set -uo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export FORGE_BIN="${FORGE_BIN:-$REPO/target/release/forge}"
ROOT="$(mktemp -d)"
export HOME="$ROOT/home" XDG_CONFIG_HOME="$ROOT/home/.config" XDG_DATA_HOME="$ROOT/home/.local/share" XDG_CACHE_HOME="$ROOT/home/.cache"
export FORGE_DB="$ROOT/forge.db"
PORT=$((20000 + RANDOM % 20000))
mkdir -p "$XDG_CONFIG_HOME/forge" "$HOME/.local/share"
printf '[remote]\nport = %s\n[keybinds]\npreset = "forge"\n' "$PORT" > "$XDG_CONFIG_HOME/forge/config.toml"
SCRATCH="$ROOT/repo"
mkdir -p "$SCRATCH/src" && cd "$SCRATCH" && git init -q && git config user.email t@t && git config user.name t
printf 'one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\neleven\ntwelve\n' > src/a.txt
printf 'alpha\nbeta\ngamma\n' > src/b.txt
git add -A && git commit -qm init
sed -i 's/two/TWO/; s/eleven/ELEVEN/' src/a.txt
sed -i 's/beta/BETA/' src/b.txt
cleanup() { [[ -n "${DPID:-}" ]] && kill "$DPID" 2>/dev/null; rm -rf "$ROOT"; }
trap cleanup EXIT
"$FORGE_BIN" serve --local --mock --port "$PORT" >"$ROOT/serve.log" 2>&1 &
DPID=$!
for _ in $(seq 1 60); do [[ -s "$XDG_CONFIG_HOME/forge/serve-token" ]] && curl -fs "http://127.0.0.1:$PORT/$(cat "$XDG_CONFIG_HOME/forge/serve-token")/api/sessions" >/dev/null && break; sleep 0.5; done
TOKEN="$(cat "$XDG_CONFIG_HOME/forge/serve-token")"
# A live session on the project, so the changes route has an agent to attribute them to.
curl -fs -X POST -H 'content-type: application/json' -d "{\"cwd\":\"$SCRATCH\"}" "http://127.0.0.1:$PORT/$TOKEN/api/sessions" >/dev/null
"$REPO/scripts/tui-drive.sh" --cwd "$SCRATCH" --cols "${COLS:-200}" --rows "${ROWS:-50}" "$REPO/scripts/tui-scripts/ide-flow.txt"
exit $?
