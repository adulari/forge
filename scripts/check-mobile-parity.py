#!/usr/bin/env python3
"""Check the declared capability boundary for the retained mobile surface."""
from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SOURCE_ROOT = ROOT / "mobile" / "src"
INVENTORY = ROOT / "mobile" / "parity" / "inventory.json"
PLATFORM_MARKERS = re.compile(r"Platform\.OS|isTauri|__TAURI__|@tauri-apps")
SUFFIX_MARKERS = re.compile(r"\.(web|native|ios|android)\.(tsx?|jsx?)$")


def detected_files() -> set[str]:
    found: set[str] = set()
    for path in SOURCE_ROOT.rglob("*"):
        if path.suffix not in {".ts", ".tsx"}:
            continue
        relative = path.relative_to(ROOT).as_posix()
        if PLATFORM_MARKERS.search(path.read_text()) or SUFFIX_MARKERS.search(path.name):
            found.add(relative)
    return found


def load_inventory() -> dict[str, dict[str, str]]:
    rows = json.loads(INVENTORY.read_text())
    if not isinstance(rows, list):
        raise ValueError("inventory must be a JSON array")
    result: dict[str, dict[str, str]] = {}
    for row in rows:
        if not isinstance(row, dict) or not all(isinstance(row.get(key), str) for key in ("file", "platform", "reason")):
            raise ValueError("each inventory row requires string file, platform, and reason")
        file = row["file"]
        if file in result:
            raise ValueError(f"duplicate inventory row: {file}")
        result[file] = row
    return result


def write_inventory() -> None:
    rows = []
    for file in sorted(detected_files()):
        path = ROOT / file
        text = path.read_text()
        if SUFFIX_MARKERS.search(path.name):
            platform = SUFFIX_MARKERS.search(path.name).group(1)
            reason = "platform-specific implementation required by Expo resolution"
        elif "tauri" in text.lower() or "__TAURI__" in text:
            platform = "desktop"
            reason = "Tauri desktop capability boundary"
        else:
            platform = "web/native"
            reason = "shared surface contains platform capability handling"
        rows.append({"file": file, "platform": platform, "reason": reason})
    INVENTORY.parent.mkdir(parents=True, exist_ok=True)
    INVENTORY.write_text(json.dumps(rows, indent=2) + "\n")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--write", action="store_true", help="write the current reviewed inventory")
    args = parser.parse_args()
    if args.write:
        write_inventory()
        return 0
    inventory = load_inventory()
    detected = detected_files()
    declared = set(inventory)
    missing = sorted(detected - declared)
    stale = sorted(declared - detected)
    if missing or stale:
        if missing:
            print("undeclared platform divergence:", *missing, sep="\n  ", file=sys.stderr)
        if stale:
            print("stale parity inventory rows:", *stale, sep="\n  ", file=sys.stderr)
        return 1
    print(f"mobile parity inventory clean: {len(detected)} declared capability files")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
