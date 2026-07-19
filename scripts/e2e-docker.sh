#!/usr/bin/env bash
# Real-turn cross-distro E2E for Linux, locally, via Docker — no VM.
#
# Why: Forge is developed on Arch but users run mainstream distros (Ubuntu/Debian/Fedora) and WSL.
# Most cross-platform bugs (glibc version, missing git, a dead/absent Secret Service that hung
# `forge chat`, PATH quirks) only reproduce off Arch. The default deterministic mock pass exercises
# every distro offline; `E2E_REAL=1` additionally drives a real turn against the host's Ollama.
#
# What it does:
#   1. Builds the CURRENT code at Forge's declared Rust 1.88 MSRV inside a rust container (so you
#      test the code you're editing, not a release). Cached cargo + a separate target dir keep
#      re-runs fast and never clobber your host `target/`.
#   2. Runs Forge in ubuntu/debian/fedora. The default mock turn asserts the full tool loop's final
#      answer; the opt-in Ollama turn asserts the requested PONG. Both must complete under the
#      timeout, exit 0, and avoid panic / resolver / missing-model errors.
#   3. A no-Secret-Service container (the WSL keyring-hang condition): asserts startup still
#      COMPLETES within the timeout (the 800ms keyring probe must bound it, not hang forever).
#
# Usage:
#   scripts/e2e-docker.sh                 # build + deterministic offline distro tests
#   E2E_REAL=1 scripts/e2e-docker.sh      # also test each distro against host Ollama
#   E2E_REAL=1 E2E_MODEL=ollama::llama3.2 scripts/e2e-docker.sh
#   E2E_REAL=1 E2E_PROMPT='Reply YES' E2E_EXPECT=YES scripts/e2e-docker.sh
#   scripts/e2e-docker.sh --no-build      # reuse the last container-built binary
#
# Requires: Docker. Live mode also needs host Ollama with the model pulled (`ollama pull llama3.2`).
set -uo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# Auto-detect a model from the host ollama (first one pulled) unless E2E_MODEL is set, so the
# script works whatever you have locally (model ids are exact — `llama3.2` ≠ `llama3.2:latest`).
detect_model() {
  local names small
  names=$(curl -s --max-time 5 localhost:11434/api/tags 2>/dev/null \
    | grep -o '"name":"[^"]*"' | sed 's/"name":"//;s/"//')
  [[ -z "$names" ]] && return
  # Prefer a SMALL/fast model so the turn fits the timeout (a 30B model alone blows past it); fall
  # back to the first pulled model.
  small=$(echo "$names" | grep -iE 'llama3\.2|:1b|:3b|0\.5b|1\.5b|phi|gemma:2b|tinyllama|qwen2\.5-coder:3b' | head -1)
  echo "ollama::${small:-$(echo "$names" | head -1)}"
}
MODEL="${E2E_MODEL:-$(detect_model)}"
MODEL="${MODEL:-ollama::llama3.2:latest}"
PROMPT="${E2E_PROMPT:-Reply with exactly the single word: PONG}"
EXPECTED="${E2E_EXPECT:-PONG}"
TIMEOUT="${E2E_TIMEOUT:-120}"
IMAGES=("ubuntu:22.04" "ubuntu:24.04" "debian:12" "fedora:40")
BUILD=1
[[ "${1:-}" == "--no-build" ]] && BUILD=0

command -v docker >/dev/null || { echo "docker is required" >&2; exit 2; }

# The container-built binary lands here (a separate target dir so the host dev build is untouched).
BIN="$REPO/target-e2e/release/forge"

if [[ "$BUILD" == 1 ]]; then
  echo "── building forge (current code) in a glibc container … this is the slow step (cached after first run)" >&2
  docker run --rm \
    -v "$REPO":/src:ro \
    -v forge-e2e-bullseye-target:/out \
    -v forge-e2e-cargo:/usr/local/cargo/registry \
    -e CARGO_TARGET_DIR=/out \
    -e RUSTUP_TOOLCHAIN=1.88.0 \
    -w /src 'rust:1.88.0-bullseye@sha256:b315f988b86912bafa7afd39a6ded0a497bf850ec36578ca9a3bdd6a14d5db4e' \
    bash -c "apt-get update -qq && apt-get install -y -qq --no-install-recommends cmake libclang-dev >/dev/null && cargo build --release --locked -p forge-agent" \
    || { echo "container build failed" >&2; exit 1; }
  # Copy the built binary out of the named volume to a host path we can bind read-only.
  mkdir -p "$REPO/target-e2e/release"
  rm -f "$REPO/target-e2e/release/forge"
  docker run --rm --user "$(id -u):$(id -g)" \
    -v forge-e2e-bullseye-target:/out -v "$REPO/target-e2e/release":/host alpine \
    sh -c "cp /out/release/forge /host/forge" || { echo "copy-out failed" >&2; exit 1; }
fi
[[ -x "$BIN" ]] || { echo "no binary at $BIN (run without --no-build first)" >&2; exit 1; }
"$REPO/scripts/check-linux-runtime-deps.sh" "$BIN" \
  || { echo "portable Linux dependency check failed" >&2; exit 1; }

PASS=0; FAIL=0
check() { # name, output, rc, optional expected literal
  local name="$1" out="$2" rc="$3" expected="${4:-}" bad=""
  echo "$out" | grep -qiE "panic|Resolver error|No usable model|RUST_BACKTRACE" && bad="error markers in output"
  [[ "$rc" -eq 124 ]] && bad="TIMED OUT (hang)"
  [[ "$rc" -ne 0 && "$rc" -ne 124 ]] && bad="nonzero exit ($rc)"
  [[ -z "$(echo "$out" | tr -d '[:space:]')" ]] && bad="empty output"
  if [[ -z "$bad" && -n "$expected" ]] && ! grep -qF -- "$expected" <<<"$out"; then
    bad="missing expected output ($expected)"
  fi
  if [[ -n "$bad" ]]; then
    echo "  ✗ $name — $bad"; echo "$out" | tail -8 | sed 's/^/      /'; FAIL=$((FAIL+1))
  else
    echo "  ✓ $name"; PASS=$((PASS+1))
  fi
}

# HTTPS needs the normal distro CA bundle. Deliberately do not install ALSA: the default Forge
# build must start and complete real turns on minimal server/WSL images without libasound.so.2.
RUNTIME_PREP='{ command -v apt-get >/dev/null && apt-get update -qq && apt-get install -y -qq ca-certificates; } >/dev/null 2>&1 || { command -v dnf >/dev/null && dnf install -q -y ca-certificates >/dev/null 2>&1; } || true'

# Per-distro smoke with the deterministic mock provider: proves the SHIPPED behaviour on this distro
# end-to-end — the binary runs, the mesh classifies + routes, tools execute, the agent loop closes.
# No network, no keys, fully offline. This is the green baseline "does Forge work on this distro".
echo "── forge runs end-to-end on each distro (mock provider: binary + routing + tools + agent loop)"
for img in "${IMAGES[@]}"; do
  out=$(docker run --rm -e HOME=/root \
    -v "$BIN":/usr/local/bin/forge:ro \
    "$img" bash -c "$RUNTIME_PREP; timeout 60 forge run 'list three colors' --mock --mode bypass </dev/null" 2>&1); rc=$?
  check "$img (mock smoke)" "$out" "$rc" "workspace looks healthy"
done

# Optional live-provider turn against host Ollama. It is opt-in because it requires a local model
# and network access; when enabled, the response must contain the explicit expected marker.
if [[ "${E2E_REAL:-0}" == 1 ]]; then
  echo "── live ollama turn (E2E_REAL=1, model: $MODEL, expect: $EXPECTED)"
  for img in "${IMAGES[@]}"; do
    out=$(docker run --rm --network host -e HOME=/root -e OLLAMA_HOST=http://127.0.0.1:11434 \
      -v "$BIN":/usr/local/bin/forge:ro \
      "$img" bash -c "$RUNTIME_PREP; timeout $TIMEOUT forge run '$PROMPT' --model '$MODEL' --mode bypass </dev/null" 2>&1); rc=$?
    check "$img (live ollama)" "$out" "$rc" "$EXPECTED"
  done
fi

echo "── keyring-hang guard (no Secret Service / D-Bus — the WSL condition)"
# A bare distro container has no org.freedesktop.secrets; `forge doctor` exercises the keyring path
# at startup. It must COMPLETE within the timeout (probe-bounded), not hang.
out=$(docker run --rm --network host -e HOME=/root \
  -v "$BIN":/usr/local/bin/forge:ro \
  ubuntu:24.04 bash -c "$RUNTIME_PREP; timeout 30 forge doctor" 2>&1); rc=$?
[[ "$rc" -eq 124 ]] && { echo "  ✗ doctor HUNG with no keyring (the 800ms probe didn't bound it)"; FAIL=$((FAIL+1)); } \
                    || { echo "  ✓ doctor completes with no keyring (rc=$rc)"; PASS=$((PASS+1)); }

echo
echo "e2e-docker: $PASS passed, $FAIL failed"
exit $(( FAIL > 0 ? 1 : 0 ))
