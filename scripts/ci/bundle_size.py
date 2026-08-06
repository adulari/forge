#!/usr/bin/env python3
"""Measure the exported web bundle and enforce a ratcheting size budget.

The bundle is what a user waits on: roughly 400 ms of a measured 995 ms cold start
to first paint is bundle fetch, parse, and module evaluation (docs/performance/
desktop-baseline.md). Nothing else in CI notices if it doubles, so growth is
invisible until someone measures startup again by hand.

This reads an existing `expo export -p web` output rather than producing one, so it
is free to run wherever the export already happens. Standard library only, like
`architecture_size.py`, whose ratchet convention it deliberately mirrors: a rise
past the tolerance fails and has to be accepted explicitly, so bundle growth is a
committed decision instead of a drift nobody sees.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

DEFAULT_DIST = Path("mobile/dist")
DEFAULT_BASELINE = Path("scripts/ci/bundle-size-baseline.json")
# Enough headroom that a dependency bump or a normal feature does not trip the guard,
# tight enough that the Lucide barrel (585 KB, 10.5% of the bundle) would have.
DEFAULT_TOLERANCE = 0.05


def measure(dist: Path) -> dict[str, int]:
    """Total emitted JavaScript, and the largest single entry chunk within it."""
    scripts = sorted(dist.glob("_expo/static/js/web/*.js"))
    if not scripts:
        raise SystemExit(
            f"no exported bundle under {dist}/_expo/static/js/web — run `expo export -p web` first"
        )
    sizes = {path: path.stat().st_size for path in scripts}
    return {
        "entry_bytes": max(sizes.values()),
        "total_js_bytes": sum(sizes.values()),
        "chunk_count": len(sizes),
    }


def load_baseline(path: Path) -> dict[str, int]:
    if not path.exists():
        raise SystemExit(f"no baseline at {path} — create one with --write-baseline")
    return json.loads(path.read_text())


def report(measured: dict[str, int], baseline: dict[str, int], tolerance: float) -> int:
    failures = 0
    for key in ("entry_bytes", "total_js_bytes"):
        now = measured[key]
        was = baseline.get(key)
        if was is None:
            print(f"{key}: {now:,} (no baseline entry — recorded on next --write-baseline)")
            continue
        ceiling = int(was * (1 + tolerance))
        delta = now - was
        pct = (delta / was * 100) if was else 0.0
        status = "ok" if now <= ceiling else "OVER BUDGET"
        print(f"{key}: {now:,} vs baseline {was:,} ({delta:+,} / {pct:+.1f}%) — {status}")
        if now > ceiling:
            failures += 1
    if failures:
        print()
        print("The bundle grew past its budget. If that is intended, re-record it:")
        print("  python3 scripts/ci/bundle_size.py --write-baseline")
        print("and say in the PR why the bundle needed to grow.")
    return failures


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dist", type=Path, default=DEFAULT_DIST)
    parser.add_argument("--baseline", type=Path, default=DEFAULT_BASELINE)
    parser.add_argument("--tolerance", type=float, default=DEFAULT_TOLERANCE)
    parser.add_argument(
        "--write-baseline",
        action="store_true",
        help="record the current measurement as the accepted budget",
    )
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    measured = measure(args.dist)
    if args.write_baseline:
        args.baseline.write_text(json.dumps(measured, indent=2, sort_keys=True) + "\n")
        print(f"wrote {args.baseline}: {json.dumps(measured, sort_keys=True)}")
        return 0
    return 1 if report(measured, load_baseline(args.baseline), args.tolerance) else 0


if __name__ == "__main__":
    raise SystemExit(main())
