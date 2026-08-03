#!/usr/bin/env python3
"""Check declared capability and behavioural divergence in the retained mobile surface."""
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
BEHAVIOR_MARKERS = re.compile(
    r"return\s+\(?\s*<|set[A-Z]\w*\(|useState\(|useQuery\(|title=|subtitle=|Text\b|notify|transport|offline|error|empty",
)


def source_files() -> list[Path]:
    return [path for path in SOURCE_ROOT.rglob("*") if path.suffix in {".ts", ".tsx"}]


def divergence_kind(path: Path) -> str | None:
    text = path.read_text()
    if SUFFIX_MARKERS.search(path.name):
        return "capability"
    marker_lines = [index for index, line in enumerate(text.splitlines()) if PLATFORM_MARKERS.search(line)]
    if not marker_lines:
        return None
    lines = text.splitlines()
    for index in marker_lines:
        context = "\n".join(lines[max(0, index - 4): index + 6])
        if BEHAVIOR_MARKERS.search(context):
            return "behavior"
    return "capability"


def detected_files() -> dict[str, str]:
    return {
        path.relative_to(ROOT).as_posix(): kind
        for path in source_files()
        if (kind := divergence_kind(path)) is not None
    }


def load_inventory() -> dict[str, dict[str, str]]:
    rows = json.loads(INVENTORY.read_text())
    if not isinstance(rows, list):
        raise ValueError("inventory must be a JSON array")
    result: dict[str, dict[str, str]] = {}
    for row in rows:
        if not isinstance(row, dict) or not all(isinstance(row.get(key), str) for key in ("file", "platform", "reason", "category")):
            raise ValueError("each inventory row requires string file, platform, reason, and category")
        if row["category"] not in {"capability", "behavior"}:
            raise ValueError(f"invalid parity category for {row.get('file')}: {row['category']}")
        file = row["file"]
        if file in result:
            raise ValueError(f"duplicate inventory row: {file}")
        result[file] = row
    return result


def validate(inventory: dict[str, dict[str, str]], detected: dict[str, str]) -> list[str]:
    errors: list[str] = []
    declared = set(inventory)
    missing = sorted(set(detected) - declared)
    stale = sorted(declared - set(detected))
    wrong_kind = sorted(file for file, kind in detected.items() if inventory.get(file, {}).get("category") != kind)
    if missing: errors.append("undeclared platform divergence: " + ", ".join(missing))
    if stale: errors.append("stale parity inventory rows: " + ", ".join(stale))
    if wrong_kind: errors.append("parity inventory category mismatch: " + ", ".join(wrong_kind))
    return errors
def write_inventory() -> None:
    rows = []
    for file, category in sorted(detected_files().items()):
        path = ROOT / file
        text = path.read_text()
        suffix = SUFFIX_MARKERS.search(path.name)
        if suffix:
            platform = suffix.group(1)
            reason = "platform-specific implementation required by Expo resolution"
        elif "tauri" in text.lower() or "__TAURI__" in text:
            platform = "desktop"
            reason = "Tauri desktop capability boundary"
        else:
            platform = "web/native"
            reason = "shared surface contains platform-specific handling"
        if category == "behavior":
            reason = "declared platform behavior boundary; shared product rules must remain aligned"
        rows.append({"file": file, "platform": platform, "reason": reason, "category": category})
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
    errors = validate(inventory, detected)
    if errors:
        print("\n".join(errors), file=sys.stderr)
        return 1
    behavior = sum(kind == "behavior" for kind in detected.values())
    print(f"mobile parity inventory clean: {len(detected)} declared files ({behavior} behavioral boundaries)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
