#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source_binary="$repo_root/target/debug/forge"
cache_root="${XDG_CACHE_HOME:-$HOME/.cache}/forge/mcp-agent"
repo_hash="$(printf '%s' "$repo_root" | cksum | awk '{print $1}')"
stable_binary="$cache_root/forge-$repo_hash"

if [[ ! -x "$source_binary" ]]; then
    echo "Forge debug binary not found; run cargo build --bin forge" >&2
    exit 1
fi

mkdir -p "$cache_root"
# Cargo replaces target/debug/forge during builds. Snapshot it outside target so a rebuild cannot
# change the executable path used by this MCP process or trigger a watcher on that build artifact.
install -m 755 "$source_binary" "$stable_binary.new"
mv -f "$stable_binary.new" "$stable_binary"

# Keep the dev store where .mcp.json used to pin it. A project-local ./.forge store diverges
# from the one every other Forge surface opens, which has already caused a real
# mcp-serve-vs-CLI split-brain bug; do not "simplify" this to $repo_root/.forge.
export FORGE_DB="${FORGE_DB:-${XDG_DATA_HOME:-$HOME/.local/share}/forge/forge-dev.db}"
mkdir -p "$(dirname "$FORGE_DB")"
exec "$stable_binary" mcp agent --cwd "$repo_root" "$@"
