#!/usr/bin/env bash
set -euo pipefail
test -f seed.txt
test "$(cat mock-note.txt)" = "deterministic mock output"
