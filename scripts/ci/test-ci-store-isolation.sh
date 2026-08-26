#!/usr/bin/env bash
set -euo pipefail

# Every `cargo test` step in CI must pin FORGE_DB to a per-job path.
#
# The runners are PERSISTENT and self-hosted, so an unset FORGE_DB lets the tests that open the
# default store reach one shared file that outlives the job and is common to every branch. A branch
# carrying a new migration upgrades it, and afterwards every PR built from an older main fails with
# `database schema version N is newer than this build supports` — a failure with nothing to do with
# the change under test. This guard exists because that happened (2026-08-07: runner store at v26,
# main at v25, nine `run::driver` tests failing on a docs-only PR).

python3 - <<'PY'
from pathlib import Path
import re
import sys

failures = []

for path in sorted(Path(".github/workflows").glob("*.yml")):
    text = path.read_text()
    # Each `- run:` step, with whatever indented lines follow it (its `env:` block included).
    for match in re.finditer(r"(?ms)^(\s+)- (?:name:.*?\n\1  )?run: (.*?)(?=^\1- |\Z)", text):
        body = match.group(0)
        command = match.group(2)
        if "cargo test" not in command:
            continue
        if "FORGE_DB" not in body:
            first = command.strip().splitlines()[0]
            failures.append(f"{path}: `{first}` runs cargo test without pinning FORGE_DB")

if failures:
    print("CI would let tests open the shared runner store:", file=sys.stderr)
    for failure in failures:
        print(f"  {failure}", file=sys.stderr)
    print(
        "\nAdd `env: { FORGE_DB: ${{ runner.temp }}/<name>.db }` to the step. See the comment on\n"
        "the test job in ci.yml, and 'Never let a dev build touch the real store' in AGENTS.md.",
        file=sys.stderr,
    )
    raise SystemExit(1)

print("every CI cargo test step pins FORGE_DB")
PY
