#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SUITE="$ROOT/scripts/manual-e2e"

usage() {
  echo "usage: $0 <scenario> [--manual|--reference]"
  echo "scenarios:"
  find "$SUITE/scenarios" -mindepth 1 -maxdepth 1 -type d -printf '  %f\n' | sort
}

if [[ $# -lt 1 || "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  exit 0
fi

SCENARIO="$1"
MODE="${2:---auto}"
SCENARIO_DIR="$SUITE/scenarios/$SCENARIO"
if [[ ! -d "$SCENARIO_DIR" || ! -f "$SCENARIO_DIR/prompt.txt" ]]; then
  echo "unknown scenario: $SCENARIO" >&2
  usage >&2
  exit 2
fi

if [[ "$MODE" == "--reference" ]]; then
  if [[ ! -d "$SCENARIO_DIR/reference" ]]; then
    echo "no saved reference for $SCENARIO" >&2
    exit 2
  fi
  echo "$SCENARIO_DIR/reference"
  exit 0
fi

OUT_ROOT="${FORGE_MANUAL_E2E_OUT:-${XDG_DATA_HOME:-$HOME/.local/share}/forge/manual-e2e-runs}"
STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
RUN_DIR="$OUT_ROOT/$SCENARIO-$STAMP-$$"
WORKSPACE="$RUN_DIR/workspace"
mkdir -p "$WORKSPACE"

if [[ -d "$SCENARIO_DIR/fixture" ]]; then
  cp -a "$SCENARIO_DIR/fixture/." "$WORKSPACE/"
fi

git -C "$WORKSPACE" init -q
git -C "$WORKSPACE" config user.email fixture@local.test
git -C "$WORKSPACE" config user.name "Forge Manual E2E"
git -C "$WORKSPACE" add -A
if ! git -C "$WORKSPACE" diff --cached --quiet; then
  git -C "$WORKSPACE" commit -qm "manual E2E baseline"
fi

if [[ -n "${FORGE_BIN:-}" ]]; then
  FORGE_COMMAND="$FORGE_BIN"
elif [[ -x "$ROOT/target/debug/forge" ]]; then
  FORGE_COMMAND="$ROOT/target/debug/forge"
else
  FORGE_COMMAND="$(command -v forge)"
fi

FORGE_CHAT_COMMAND=("$FORGE_COMMAND" chat)
if [[ -n "${FORGE_MODEL:-}" ]]; then
  FORGE_CHAT_COMMAND+=(--model "$FORGE_MODEL")
fi

echo "scenario:  $SCENARIO"
echo "workspace: $WORKSPACE"
echo "prompt:    $SCENARIO_DIR/prompt.txt"
echo "reference: $SCENARIO_DIR/reference"

if [[ "$MODE" == "--manual" ]]; then
  echo
  echo "Paste this prompt into Forge:"
  sed 's/^/  /' "$SCENARIO_DIR/prompt.txt"
  cd "$WORKSPACE"
  exec "${FORGE_CHAT_COMMAND[@]}"
fi

if [[ "$MODE" != "--auto" ]]; then
  echo "unknown mode: $MODE" >&2
  exit 2
fi

if [[ "$SCENARIO" == "interrupt-resume-large-write" ]]; then
  FIRST_SUMMARY="$RUN_DIR/interrupt-summary.jsonl"
  python3 "$SUITE/pty_chat_harness.py" \
    --cwd "$WORKSPACE" \
    --prompt-file "$SCENARIO_DIR/prompt.txt" \
    --log-prefix "$RUN_DIR/interrupt" \
    --timeout "${FORGE_E2E_TIMEOUT:-1500}" \
    --interrupt-after "${FORGE_E2E_INTERRUPT_AFTER:-25}" \
    -- "${FORGE_CHAT_COMMAND[@]}" | tee "$FIRST_SUMMARY"
  jq -e 'select(.interrupt_sent == true and .timed_out == false)' "$FIRST_SUMMARY" >/dev/null
  SESSION_ID="$(jq -er 'select(.session_id != null) | .session_id' "$FIRST_SUMMARY" | tail -1)"

  RESUME_COMMAND=("$FORGE_COMMAND" chat --resume "$SESSION_ID")
  if [[ -n "${FORGE_MODEL:-}" ]]; then
    RESUME_COMMAND+=(--model "$FORGE_MODEL")
  fi
  python3 "$SUITE/pty_chat_harness.py" \
    --cwd "$WORKSPACE" \
    --prompt-file "$SCENARIO_DIR/resume.txt" \
    --log-prefix "$RUN_DIR/resume" \
    --session-id "$SESSION_ID" \
    --timeout "${FORGE_E2E_TIMEOUT:-1500}" \
    -- "${RESUME_COMMAND[@]}"

  FORGE_DB_PATH="${FORGE_DB:-${XDG_DATA_HOME:-$HOME/.local/share}/forge/forge.db}"
  python3 "$SUITE/verify_session_tools.py" "$FORGE_DB_PATH" "$SESSION_ID" --require-all-ok \
    | tee "$RUN_DIR/session-tool-integrity.json"
else
  RUN_SUMMARY="$RUN_DIR/harness-summary.jsonl"
  python3 "$SUITE/pty_chat_harness.py" \
    --cwd "$WORKSPACE" \
    --prompt-file "$SCENARIO_DIR/prompt.txt" \
    --log-prefix "$RUN_DIR/live" \
    --timeout "${FORGE_E2E_TIMEOUT:-1500}" \
    -- "${FORGE_CHAT_COMMAND[@]}" | tee "$RUN_SUMMARY"

  SESSION_ID="$(python3 - "$RUN_SUMMARY" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as summaries:
    records = [json.loads(line) for line in summaries if line.strip()]
session_ids = [record.get("session_id") for record in records if record.get("session_id")]
if not session_ids:
    raise SystemExit("TUI harness did not report a Forge session ID")
print(session_ids[-1])
PY
)"
  FORGE_DB_PATH="${FORGE_DB:-${XDG_DATA_HOME:-$HOME/.local/share}/forge/forge.db}"
  python3 "$SUITE/verify_session_tools.py" "$FORGE_DB_PATH" "$SESSION_ID" \
    | tee "$RUN_DIR/session-tool-integrity.json"
fi

case "$SCENARIO" in
  aetherfront)
    node "$SCENARIO_DIR/verify.js" "$WORKSPACE/index.html" "$RUN_DIR/screenshot.png"
    ;;
  multifile-reservations)
    (cd "$WORKSPACE" && python3 -m unittest discover -v)
    ;;
  go-ordered-pipeline)
    (
      cd "$WORKSPACE"
      UNFORMATTED="$(gofmt -l pipeline/pipeline.go)"
      if [[ -n "$UNFORMATTED" ]]; then
        echo "gofmt required for:" >&2
        echo "$UNFORMATTED" >&2
        exit 1
      fi
      go vet ./...
      go test -race ./...
    )
    ;;
  typescript-config-recovery)
    (cd "$WORKSPACE" && npm test && npm run lint)
    ;;
  rust-transaction-ledger)
    (cd "$WORKSPACE" && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test --all-targets)
    ;;
  interrupt-resume-large-write)
    python3 "$SCENARIO_DIR/verify.py" "$WORKSPACE/interrupted.txt"
    ;;
esac

echo "saved run: $RUN_DIR"
