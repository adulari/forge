#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source_binary="$repo_root/target/debug/forge"
cache_root="${XDG_CACHE_HOME:-$HOME/.cache}/forge/mcp-agent"
repo_hash="$(printf '%s' "$repo_root" | cksum | awk '{print $1}')"
stable_binary="$cache_root/forge-$repo_hash"

# A git worktree has no target/ of its own, so fall back to the primary checkout's binary rather
# than demanding a ~360MB debug build per worktree.
if [[ ! -x "$source_binary" ]]; then
    common_dir="$(git -C "$repo_root" rev-parse --git-common-dir 2>/dev/null || true)"
    if [[ -n "$common_dir" ]]; then
        primary="$(cd "$common_dir/.." && pwd)/target/debug/forge"
        [[ -x "$primary" ]] && source_binary="$primary"
    fi
fi
if [[ ! -x "$source_binary" ]]; then
    echo "Forge debug binary not found; run cargo build --bin forge" >&2
    exit 1
fi

mkdir -p "$cache_root"
# Cargo replaces target/debug/forge during builds. Snapshot it outside target so a rebuild cannot
# change the executable path used by this MCP process or trigger a watcher on that build artifact.
# Per-process temp name: every session in this repo shares $cache_root, the binary is ~360MB, and
# the copy takes seconds. A fixed ".new" name let one session mv another's half-written file into
# place and exec a truncated ELF.
tmp="$(mktemp "$cache_root/forge-$repo_hash.XXXXXX")"
install -m 755 "$source_binary" "$tmp"
mv -f "$tmp" "$stable_binary"

# Keep the dev store where .mcp.json used to pin it. A project-local ./.forge store diverges
# from the one every other Forge surface opens, which has already caused a real
# mcp-serve-vs-CLI split-brain bug; do not "simplify" this to $repo_root/.forge.
# Pinned, NOT defaulted. .mcp.json set this unconditionally, and an inherited FORGE_DB is exactly
# how a dev-schema binary once opened the real store and poisoned its migration version.
export FORGE_DB="${XDG_DATA_HOME:-$HOME/.local/share}/forge/forge-dev.db"
mkdir -p "$(dirname "$FORGE_DB")"
exec "$stable_binary" mcp agent --cwd "$repo_root" "$@"
