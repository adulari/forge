#!/usr/bin/env python3
"""Strict acceptance verifier for the interrupt/resume large-write scenario."""

from __future__ import annotations

import json
import sys
from pathlib import Path


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: verify.py interrupted.txt", file=sys.stderr)
        return 2

    artifact = Path(sys.argv[1])
    if not artifact.is_file():
        print(f"missing artifact: {artifact}", file=sys.stderr)
        return 1

    lines = artifact.read_text(encoding="utf-8").splitlines()
    errors: list[str] = []
    if len(lines) != 321:
        errors.append(f"expected 321 lines, found {len(lines)}")
    if not lines or lines[0] != "INTERRUPT_RESUME_PROBE":
        errors.append("missing exact INTERRUPT_RESUME_PROBE header")

    for number in range(1, 321):
        index = number
        if index >= len(lines):
            errors.append(f"missing record {number:04d}")
            continue
        prefix = f"{number:04d}:"
        line = lines[index]
        if not line.startswith(prefix):
            errors.append(f"line {index + 1} must start with {prefix!r}")
            continue
        payload = line[len(prefix) :].strip()
        if len(payload) < 90:
            errors.append(f"record {number:04d} payload is only {len(payload)} characters")
        if not payload.isalnum():
            errors.append(f"record {number:04d} payload is not strictly alphanumeric")

    report = {
        "valid": not errors,
        "path": str(artifact.resolve()),
        "line_count": len(lines),
        "first_record": lines[1][:20] if len(lines) > 1 else None,
        "last_record": lines[-1][:20] if lines else None,
        "errors": errors[:20],
    }
    print(json.dumps(report, indent=2))
    return 0 if not errors else 1


if __name__ == "__main__":
    raise SystemExit(main())
