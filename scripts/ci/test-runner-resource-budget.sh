#!/usr/bin/env bash
set -euo pipefail

python3 - <<'PY'
from pathlib import Path
import re


def workflow(path: str) -> str:
    return Path(path).read_text()


def job_block(text: str, name: str) -> str:
    match = re.search(
        rf"(?ms)^  {re.escape(name)}:\n(.*?)(?=^  [A-Za-z0-9_-]+:\n|\Z)", text
    )
    if not match:
        raise SystemExit(f"missing job {name}")
    return match.group(1)


heavy_jobs = {
    ".github/workflows/ci.yml": ("clippy", "test", "release-build"),
    ".github/workflows/security.yml": ("audit", "deny"),
    ".github/workflows/mobile-typecheck.yml": ("tauri",),
}

for path, jobs in heavy_jobs.items():
    text = workflow(path)
    if 'CARGO_BUILD_JOBS: "4"' not in text:
        raise SystemExit(f"{path} must cap Cargo parallelism at four jobs")
    for job in jobs:
        block = job_block(text, job)
        if "runs-on: [self-hosted, linux, x64, heavy]" not in block:
            raise SystemExit(f"{path} job {job} must run on the serialized heavy runner")

release = workflow(".github/workflows/release.yml")
if 'CARGO_BUILD_JOBS: "4"' not in release:
    raise SystemExit("release.yml must cap Cargo parallelism at four jobs")
for option in (
    "--memory-reservation 6g",
    "--memory 8g",
    "--memory-swap 9g",
    "--cpus 4",
    "--pids-limit 2048",
):
    if option not in release:
        raise SystemExit(f"release Docker builder is missing resource limit: {option}")

print("runner resource budgets are enforced")
PY
