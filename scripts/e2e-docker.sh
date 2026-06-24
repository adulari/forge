#!/usr/bin/env bash
# Real-turn cross-distro E2E for Linux, locally, via Docker — no VM.
#
# Why: Forge is developed on Arch but users run mainstream distros (Ubuntu/Debian/Fedora) and WSL.
# Most cross-platform bugs (glibc version, missing git, a dead/absent Secret Service that hung
# `forge chat`, PATH quirks) only reproduce off Arch. This drives a REAL `forge run` turn in each
# distro against the host's ollama, so you can confirm functionality + diagnose breakage yourself.
#
# What it does:
#   1. Builds the CURRENT code into a portable glibc binary inside a rust container (so you test the
#      code you're editing, not a release). Cached cargo + a separate target dir keep re-runs fast
#      and never clobber your host `target/`.
#   2. Runs `forge run "<prompt>" --model <ollama-model>` in ubuntu/debian/fedora, talking to the
#      host's ollama over the host network. Asserts: completes under the timeout, exits 0, prints a
#      non-empty answer, and DOESN'T emit a panic / "Resolver error" / "No usable model".
#   3. A no-Secret-Service container (the WSL keyring-hang condition): asserts startup still
#      COMPLETES within the timeout (the 800ms keyring probe must bound it, not hang forever).
#
# Usage:
#   scripts/e2e-docker.sh                 # build + test ubuntu/debian/fedora against host ollama
#   E2E_MODEL=ollama::llama3.2 scripts/e2e-docker.sh
#   E2E_PROMPT='Reply with exactly: PONG' scripts/e2e-docker.sh
#   scripts/e2e-docker.sh --no-build      # reuse the last container-built binary
#
# Requires: docker, a running host ollama with the model pulled (`ollama pull llama3.2`).
set -uo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODEL="${E2E_MODEL:-ollama::llama3.2}"
PROMPT="${E2E_PROMPT:-Reply with exactly the single word: PONG}"
TIMEOUT="${E2E_TIMEOUT:-90}"
IMAGES=("ubuntu:24.04" "debian:12" "fedora:40")
BUILD=1
[[ "${1:-}" == "--no-build" ]] && BUILD=0

command -v docker >/dev/null || { echo "docker is required" >&2; exit 2; }

# The container-built binary lands here (a separate target dir so the host dev build is untouched).
BIN="$REPO/target-e2e/release/forge"

if [[ "$BUILD" == 1 ]]; then
  echo "── building forge (current code) in a glibc container … this is the slow step (cached after first run)" >&2
  docker run --rm \
    -v "$REPO":/src:ro \
    -v forge-e2e-target:/out \
    -v forge-e2e-cargo:/usr/local/cargo/registry \
    -e CARGO_TARGET_DIR=/out \
    -w /src rust:1.85 \
    bash -c "cargo build --release -p forge-cli" || { echo "container build failed" >&2; exit 1; }
  # Copy the built binary out of the named volume to a host path we can bind read-only.
  mkdir -p "$REPO/target-e2e/release"
  docker run --rm -v forge-e2e-target:/out -v "$REPO/target-e2e/release":/host alpine \
    sh -c "cp /out/release/forge /host/forge" || { echo "copy-out failed" >&2; exit 1; }
fi
[[ -x "$BIN" ]] || { echo "no binary at $BIN (run without --no-build first)" >&2; exit 1; }

PASS=0; FAIL=0
check() { # name, output, rc
  local name="$1" out="$2" rc="$3" bad=""
  echo "$out" | grep -qiE "panic|Resolver error|No usable model|RUST_BACKTRACE" && bad="error markers in output"
  [[ "$rc" -eq 124 ]] && bad="TIMED OUT (hang)"
  [[ "$rc" -ne 0 && "$rc" -ne 124 ]] && bad="nonzero exit ($rc)"
  [[ -z "$(echo "$out" | tr -d '[:space:]')" ]] && bad="empty output"
  if [[ -n "$bad" ]]; then
    echo "  ✗ $name — $bad"; echo "$out" | tail -8 | sed 's/^/      /'; FAIL=$((FAIL+1))
  else
    echo "  ✓ $name"; PASS=$((PASS+1))
  fi
}

echo "── real-turn across distros (model: $MODEL, host ollama)"
for img in "${IMAGES[@]}"; do
  out=$(docker run --rm --network host -e HOME=/root \
    -v "$BIN":/usr/local/bin/forge:ro \
    "$img" timeout "$TIMEOUT" forge run "$PROMPT" --model "$MODEL" 2>&1); rc=$?
  check "$img real turn" "$out" "$rc"
done

echo "── keyring-hang guard (no Secret Service / D-Bus — the WSL condition)"
# A bare distro container has no org.freedesktop.secrets; `forge doctor` exercises the keyring path
# at startup. It must COMPLETE within the timeout (probe-bounded), not hang.
out=$(docker run --rm --network host -e HOME=/root \
  -v "$BIN":/usr/local/bin/forge:ro \
  ubuntu:24.04 timeout 30 forge doctor 2>&1); rc=$?
[[ "$rc" -eq 124 ]] && { echo "  ✗ doctor HUNG with no keyring (the 800ms probe didn't bound it)"; FAIL=$((FAIL+1)); } \
                    || { echo "  ✓ doctor completes with no keyring (rc=$rc)"; PASS=$((PASS+1)); }

echo
echo "e2e-docker: $PASS passed, $FAIL failed"
exit $(( FAIL > 0 ? 1 : 0 ))
