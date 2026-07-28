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
if [[ ! -d "$SCENARIO_DIR" || \
  ( ! -f "$SCENARIO_DIR/prompt.txt" && ! -f "$SCENARIO_DIR/prompts.json" ) ]]; then
  echo "unknown scenario: $SCENARIO" >&2
  usage >&2
  exit 2
fi
if [[ -f "$SCENARIO_DIR/prompts.json" ]]; then
  PROMPT_SOURCE="$SCENARIO_DIR/prompts.json"
  HARNESS_PROMPT_ARGS=(--prompt-sequence-file "$PROMPT_SOURCE")
  E2E_TOTAL_TIMEOUT="${FORGE_E2E_TIMEOUT:-9000}"
  HARNESS_TURN_TIMEOUT_ARGS=(--turn-timeout "${FORGE_E2E_TURN_TIMEOUT:-1500}")
else
  PROMPT_SOURCE="$SCENARIO_DIR/prompt.txt"
  HARNESS_PROMPT_ARGS=(--prompt-file "$PROMPT_SOURCE")
  E2E_TOTAL_TIMEOUT="${FORGE_E2E_TIMEOUT:-1500}"
  HARNESS_TURN_TIMEOUT_ARGS=()
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

FIXTURE_DIR="$SCENARIO_DIR/fixture"
if [[ -f "$SCENARIO_DIR/fixture.source" ]]; then
  FIXTURE_SOURCE="$(<"$SCENARIO_DIR/fixture.source")"
  FIXTURE_DIR="$(realpath -m "$SCENARIO_DIR/$FIXTURE_SOURCE")"
  SCENARIO_ROOT="$(realpath "$SUITE/scenarios")"
  if [[ "$FIXTURE_DIR" != "$SCENARIO_ROOT/"* ]]; then
    echo "fixture.source escapes the scenario suite: $FIXTURE_SOURCE" >&2
    exit 2
  fi
  if [[ ! -d "$FIXTURE_DIR" ]]; then
    echo "fixture.source does not resolve to a directory: $FIXTURE_SOURCE" >&2
    exit 2
  fi
fi
if [[ -d "$FIXTURE_DIR" ]]; then
  cp -a "$FIXTURE_DIR/." "$WORKSPACE/"
  # A prior local test run may have left ignored interpreter caches inside a shared fixture.
  # They are not benchmark source and must never enter the synthetic one-commit base.
  find "$WORKSPACE" -type f \( -name '*.pyc' -o -name '*.pyo' \) -delete
  find "$WORKSPACE" -depth -type d -name __pycache__ -empty -delete
else
  echo "scenario fixture is missing: $FIXTURE_DIR" >&2
  exit 2
fi

if [[ -e "$WORKSPACE/.git" ]]; then
  echo "scenario fixture unexpectedly contains Git metadata; refusing an unisolated run" >&2
  exit 2
fi
git -C "$WORKSPACE" init -q
git -C "$WORKSPACE" config user.email fixture@local.test
git -C "$WORKSPACE" config user.name "Forge Manual E2E"
git -C "$WORKSPACE" add -A
if ! git -C "$WORKSPACE" diff --cached --quiet; then
  git -C "$WORKSPACE" commit -qm "manual E2E baseline"
fi
BASE_COMMIT="$(git -C "$WORKSPACE" rev-parse HEAD)"
BASE_TREE="$(git -C "$WORKSPACE" rev-parse "$BASE_COMMIT^{tree}")"
if [[ "$(git -C "$WORKSPACE" rev-list --all --count)" != "1" ]] || \
  [[ -n "$(git -C "$WORKSPACE" remote)" ]]; then
  echo "history isolation failed: expected one reachable synthetic commit and no remotes" >&2
  exit 2
fi
{
  echo ".forge/"
  echo ".claude/"
  echo "__pycache__/"
  echo "*.pyc"
  echo "*.pyo"
} >>"$WORKSPACE/.git/info/exclude"

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

python3 - "$RUN_DIR/run-manifest.json" "$SCENARIO" "$PROMPT_SOURCE" "$WORKSPACE" \
  "$BASE_COMMIT" "$BASE_TREE" "$FORGE_COMMAND" "${FORGE_MODEL:-}" <<'PY'
import hashlib
import json
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path

path, scenario, prompt_source, workspace, base, base_tree, forge_bin, model = sys.argv[1:]
prompt_path = Path(prompt_source)
if prompt_path.suffix == ".json":
    prompts = json.loads(prompt_path.read_text(encoding="utf-8"))
else:
    prompts = [prompt_path.read_text(encoding="utf-8")]
manifest = {
    "schema_version": 1,
    "created_at": datetime.now(timezone.utc).isoformat(),
    "scenario": scenario,
    "prompt_sha256": [hashlib.sha256(prompt.encode()).hexdigest() for prompt in prompts],
    "workspace": str(Path(workspace).resolve()),
    "synthetic_base_commit": base,
    "synthetic_base_tree": base_tree,
    "reachable_commit_count": 1,
    "remote_count": 0,
    "forge_binary": str(Path(forge_bin).resolve()),
    "forge_binary_sha256": hashlib.sha256(Path(forge_bin).read_bytes()).hexdigest(),
    "forge_version": subprocess.run(
        [forge_bin, "--version"],
        check=False,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
    ).stdout.strip(),
    "model_override": model or None,
    "effort_override": None,
}
path = Path(path)
temporary = path.with_suffix(path.suffix + ".tmp")
temporary.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8")
temporary.replace(path)
PY

echo "scenario:  $SCENARIO"
echo "workspace: $WORKSPACE"
echo "prompt:    $PROMPT_SOURCE"
echo "reference: $SCENARIO_DIR/reference"

if [[ "$MODE" == "--manual" ]]; then
  echo
  if [[ -f "$SCENARIO_DIR/prompts.json" ]]; then
    echo "Send these prompts to one Forge session in order:"
    python3 - "$SCENARIO_DIR/prompts.json" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as prompts:
    for index, prompt in enumerate(json.load(prompts), 1):
        print(f"\n  TURN {index}\n")
        print("\n".join(f"  {line}" for line in prompt.splitlines()))
PY
  else
    echo "Paste this prompt into Forge:"
    sed 's/^/  /' "$SCENARIO_DIR/prompt.txt"
  fi
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
    "${HARNESS_PROMPT_ARGS[@]}" \
    --log-prefix "$RUN_DIR/live" \
    --timeout "$E2E_TOTAL_TIMEOUT" \
    "${HARNESS_TURN_TIMEOUT_ARGS[@]}" \
    -- "${FORGE_CHAT_COMMAND[@]}" | tee "$RUN_SUMMARY"

  SESSION_ID="$(python3 - "$RUN_SUMMARY" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as summaries:
    records = [json.loads(line) for line in summaries if line.strip()]
if not records:
    raise SystemExit("TUI harness did not report a summary")
latest = records[-1]
if latest.get("timed_out") or latest.get("turns_completed") != latest.get("turns_expected"):
    raise SystemExit(
        "TUI harness did not finish every prompt: "
        f"{latest.get('turns_completed')}/{latest.get('turns_expected')}"
    )
session_ids = [record.get("session_id") for record in records if record.get("session_id")]
if not session_ids:
    raise SystemExit("TUI harness did not report a Forge session ID")
print(session_ids[-1])
PY
)"
  FORGE_DB_PATH="${FORGE_DB:-${XDG_DATA_HOME:-$HOME/.local/share}/forge/forge.db}"
  TOOL_VERIFY_ARGS=()
  if [[ "$SCENARIO" == "long-session-reservations" ]]; then
    TOOL_VERIFY_ARGS=(--require-all-ok --deny-external-sources)
  fi
  python3 "$SUITE/verify_session_tools.py" "$FORGE_DB_PATH" "$SESSION_ID" \
    "${TOOL_VERIFY_ARGS[@]}" \
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
  long-session-reservations)
    (cd "$WORKSPACE" && python3 -m unittest discover -v 2>&1) \
      | tee "$RUN_DIR/visible-tests.log"
    python3 "$SCENARIO_DIR/verify.py" "$WORKSPACE" 2>&1 \
      | tee "$RUN_DIR/hidden-tests.log"
    ;;
  interrupt-resume-large-write)
    python3 "$SCENARIO_DIR/verify.py" "$WORKSPACE/interrupted.txt"
    ;;
esac

PATCH_PATHS=(
  .
  ':(exclude).forge/**'
  ':(exclude).claude/**'
  ':(exclude)**/__pycache__/**'
  ':(exclude)**/*.pyc'
  ':(exclude)**/*.pyo'
)
git -C "$WORKSPACE" add -N -- "${PATCH_PATHS[@]}"
git -C "$WORKSPACE" diff --binary "$BASE_COMMIT" -- "${PATCH_PATHS[@]}" \
  >"$RUN_DIR/changes.patch"
git -C "$WORKSPACE" diff --check "$BASE_COMMIT" -- "${PATCH_PATHS[@]}"
git -C "$WORKSPACE" status --short -- "${PATCH_PATHS[@]}" >"$RUN_DIR/git-status.txt"

echo "saved run: $RUN_DIR"
