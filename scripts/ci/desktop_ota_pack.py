#!/usr/bin/env python3
"""Pack the desktop web export into a desktop OTA bundle and its manifest.

    desktop_ota_pack.py pack  --dist mobile/dist --runtime R --bundle-id SHA \
                              --created-at EPOCH --min-app-version 0.0.0 --out DIR
    desktop_ota_pack.py prune --assets assets.json [--keep 5]

`pack` writes `bundle-<runtime>-<id>.zip` and `manifest-<runtime>.json` into DIR. The zip is
deterministic (sorted entries, fixed timestamps), so the same export always has the same sha256.
Signing is a separate step (`tauri signer sign`) so this script never sees the private key.

`prune` reads a JSON list of release assets ({"name", "createdAt"}) and prints the names of the
asset files to delete: all but the newest `--keep` bundles of each runtime, together with their
signatures. Manifests are never listed.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
import zipfile
from pathlib import Path

RUNTIME = re.compile(r"^[A-Za-z0-9.-]{1,64}$")
BUNDLE_ID = re.compile(r"^[0-9a-f]{7,64}$")
SEMVER = re.compile(r"^\d+\.\d+\.\d+$")
BUNDLE_ASSET = re.compile(r"^bundle-(?P<runtime>[A-Za-z0-9.-]+)-(?P<id>[0-9a-f]{7,64})\.zip(\.sig)?$")
ZIP_EPOCH = (1980, 1, 1, 0, 0, 0)


def pack(dist: Path, runtime: str, bundle_id: str, created_at: int, min_app_version: str, out: Path) -> dict:
    if not RUNTIME.match(runtime):
        raise SystemExit(f"runtime version {runtime!r} is not a plain token")
    if not BUNDLE_ID.match(bundle_id):
        raise SystemExit(f"bundle id {bundle_id!r} is not a commit hash")
    if not SEMVER.match(min_app_version):
        raise SystemExit(f"min app version {min_app_version!r} is not x.y.z")
    if not (dist / "index.html").is_file():
        raise SystemExit(f"{dist} has no index.html; run the web export first")

    out.mkdir(parents=True, exist_ok=True)
    bundle_path = out / f"bundle-{runtime}-{bundle_id}.zip"
    files = sorted(path for path in dist.rglob("*") if path.is_file())
    with zipfile.ZipFile(bundle_path, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as archive:
        for path in files:
            info = zipfile.ZipInfo(path.relative_to(dist).as_posix(), ZIP_EPOCH)
            info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = 0o644 << 16
            archive.writestr(info, path.read_bytes())

    data = bundle_path.read_bytes()
    manifest = {
        "runtime_version": runtime,
        "bundle_id": bundle_id,
        "created_at": created_at,
        "sha256": hashlib.sha256(data).hexdigest(),
        "size": len(data),
        "min_app_version": min_app_version,
    }
    # No trailing newline and a fixed byte layout: the signature covers exactly these bytes.
    (out / f"manifest-{runtime}.json").write_text(json.dumps(manifest, indent=2))
    return manifest


def prune(assets: list[dict], keep: int) -> list[str]:
    runtimes: dict[str, dict[str, list[dict]]] = {}
    for asset in assets:
        match = BUNDLE_ASSET.match(asset["name"])
        if match:
            by_id = runtimes.setdefault(match["runtime"], {})
            by_id.setdefault(match["id"], []).append(asset)

    doomed: list[str] = []
    for by_id in runtimes.values():
        newest_first = sorted(
            by_id.items(),
            key=lambda item: max(asset.get("createdAt", "") for asset in item[1]),
            reverse=True,
        )
        for _, stale in newest_first[keep:]:
            doomed.extend(asset["name"] for asset in stale)
    return sorted(doomed)


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    pack_cmd = commands.add_parser("pack")
    pack_cmd.add_argument("--dist", type=Path, required=True)
    pack_cmd.add_argument("--runtime", required=True)
    pack_cmd.add_argument("--bundle-id", required=True)
    pack_cmd.add_argument("--created-at", type=int, required=True)
    pack_cmd.add_argument("--min-app-version", default="0.0.0")
    pack_cmd.add_argument("--out", type=Path, required=True)
    prune_cmd = commands.add_parser("prune")
    prune_cmd.add_argument("--assets", type=Path, required=True)
    prune_cmd.add_argument("--keep", type=int, default=5)
    args = parser.parse_args(argv)

    if args.command == "pack":
        manifest = pack(args.dist, args.runtime, args.bundle_id, args.created_at, args.min_app_version, args.out)
        print(json.dumps(manifest, indent=2))
    else:
        for name in prune(json.loads(args.assets.read_text()), args.keep):
            print(name)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
