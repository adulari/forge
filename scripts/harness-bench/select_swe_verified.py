#!/usr/bin/env python3
"""Select a deterministic, repo-diverse SWE-bench Verified difficulty sample."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
from typing import Any


DEFAULT_BANDS = ("<15 min fix", "15 min - 1 hour", "1-4 hours")


def load_rows(path: Path) -> list[dict[str, Any]]:
    rows = [
        json.loads(line)
        for line in path.read_text(encoding="utf-8").splitlines()
        if line.strip()
    ]
    required = {"instance_id", "repo", "difficulty"}
    for index, row in enumerate(rows, start=1):
        missing = required - set(row)
        if missing:
            raise ValueError(f"{path}:{index}: missing {sorted(missing)}")
    return rows


def select_rows(
    rows: list[dict[str, Any]],
    *,
    seed: int,
    per_band: int,
    bands: tuple[str, ...] = DEFAULT_BANDS,
) -> list[dict[str, Any]]:
    selected: list[dict[str, Any]] = []
    used_repos: set[str] = set()
    for band in bands:
        candidates = [row for row in rows if row["difficulty"] == band]
        candidates.sort(
            key=lambda row: hashlib.sha256(
                f"{seed}:{band}:{row['instance_id']}".encode()
            ).hexdigest()
        )
        chosen: list[dict[str, Any]] = []
        for row in candidates:
            if row["repo"] in used_repos:
                continue
            chosen.append(row)
            used_repos.add(row["repo"])
            if len(chosen) == per_band:
                break
        if len(chosen) != per_band:
            raise ValueError(
                f"could select only {len(chosen)}/{per_band} unique repos for {band}"
            )
        selected.extend(chosen)
    return selected


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("--seed", type=int, default=20260726)
    parser.add_argument("--per-band", type=int, default=2)
    args = parser.parse_args()
    if args.per_band < 1:
        parser.error("--per-band must be at least 1")
    rows = load_rows(args.source)
    selected = select_rows(rows, seed=args.seed, per_band=args.per_band)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(
        "".join(json.dumps(row, sort_keys=True) + "\n" for row in selected),
        encoding="utf-8",
    )
    source_hash = hashlib.sha256(args.source.read_bytes()).hexdigest()
    manifest = {
        "schema_version": 1,
        "source": str(args.source.resolve()),
        "source_sha256": source_hash,
        "seed": args.seed,
        "per_band": args.per_band,
        "difficulty_bands": list(DEFAULT_BANDS),
        "excluded_band": ">4 hours",
        "algorithm": (
            "per difficulty band, ascending sha256(seed:difficulty:instance_id), "
            "first candidates whose repos were not already selected"
        ),
        "selected": [
            {
                "instance_id": row["instance_id"],
                "repo": row["repo"],
                "difficulty": row["difficulty"],
            }
            for row in selected
        ],
    }
    manifest_path = args.out.with_suffix(".manifest.json")
    manifest_path.write_text(
        json.dumps(manifest, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    print(json.dumps(manifest, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
