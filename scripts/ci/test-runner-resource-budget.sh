#!/usr/bin/env bash
set -euo pipefail

python3 - <<'PY'
from pathlib import Path
import re
import shlex


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
# The Linux release legs build on GitHub-hosted runners (they no longer share the maintainer's
# machine), so the laptop memory caps do not apply. What must hold instead: no release leg or job
# may land back on a self-hosted runner without the budget being reconsidered, and the container
# keeps a pid cap.
if re.search(r"runs-on:[^\n]*self-hosted", release) or release.count("[release,") != 1:
    raise SystemExit("release.yml must stay on GitHub-hosted runners except the package-manifest job")
builder = open("scripts/ci/build-portable-linux.sh", encoding="utf-8").read()
if "--pids-limit" not in builder:
    raise SystemExit("release Docker builder is missing resource limit: --pids-limit")

dispatch_loop = re.search(r"for workflow in ([^;]+); do", release)
if not dispatch_loop:
    raise SystemExit("release.yml is missing the package-manifest check dispatch loop")
for workflow_name in shlex.split(dispatch_loop.group(1)):
    workflow_path = Path(".github/workflows") / workflow_name
    if not workflow_path.is_file():
        raise SystemExit(
            f"release.yml dispatches missing workflow {workflow_path}; "
            "GitHub-managed workflows cannot be dispatched by filename"
        )
    if "workflow_dispatch:" not in workflow_path.read_text():
        raise SystemExit(f"{workflow_path} must support workflow_dispatch")

print("runner resource budgets are enforced")
PY
