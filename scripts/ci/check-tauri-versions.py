#!/usr/bin/env python3
"""Fail when the tauri crate and @tauri-apps/api differ in major.minor.

`tauri build` refuses that mismatch, but only the release workflow runs it, so a PR that moves
one side (a new plugin needing a newer tauri, say) merged green and broke every desktop release.
"""
import json
import re
import sys
from pathlib import Path

root = Path(__file__).resolve().parents[2]
lock = (root / "mobile/src-tauri/Cargo.lock").read_text()
crate = re.search(r'\[\[package\]\]\nname = "tauri"\nversion = "([^"]+)"', lock)
npm = json.loads((root / "mobile/package-lock.json").read_text())
api = npm["packages"].get("node_modules/@tauri-apps/api", {}).get("version")
if not crate or not api:
    sys.exit("could not read the tauri crate or @tauri-apps/api version")


def minor(version):
    return ".".join(version.split(".")[:2])


if minor(crate.group(1)) != minor(api):
    sys.exit(
        f"tauri crate {crate.group(1)} and @tauri-apps/api {api} differ in major.minor; "
        "tauri build refuses this. Move both to the same minor."
    )
print(f"tauri crate {crate.group(1)} matches @tauri-apps/api {api}")
