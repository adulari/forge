#!/usr/bin/env python3
from __future__ import annotations

import hashlib
import json
import tempfile
import unittest
import zipfile
from pathlib import Path

import desktop_ota_pack as ota


class PackTests(unittest.TestCase):
    def export(self, root: Path) -> Path:
        dist = root / "dist"
        (dist / "_expo/static/js").mkdir(parents=True)
        (dist / "index.html").write_text("<html></html>")
        (dist / "_expo/static/js/app.js").write_text("console.log(1)")
        return dist

    def test_manifest_describes_the_bundle_and_zip_is_deterministic(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            dist = self.export(root)
            first = ota.pack(dist, "0123456789abcdef", "abcdef0123", 1_700_000_000, "0.0.0", root / "a")
            second = ota.pack(dist, "0123456789abcdef", "abcdef0123", 1_700_000_000, "0.0.0", root / "b")
            self.assertEqual(first, second)
            bundle = root / "a" / "bundle-0123456789abcdef-abcdef0123.zip"
            self.assertEqual(first["sha256"], hashlib.sha256(bundle.read_bytes()).hexdigest())
            self.assertEqual(first["size"], bundle.stat().st_size)
            self.assertEqual(set(first), {"runtime_version", "bundle_id", "created_at", "sha256", "size", "min_app_version"})
            on_disk = json.loads((root / "a" / "manifest-0123456789abcdef.json").read_text())
            self.assertEqual(on_disk, first)
            with zipfile.ZipFile(bundle) as archive:
                self.assertEqual(sorted(archive.namelist()), ["_expo/static/js/app.js", "index.html"])

    def test_refuses_a_dist_without_index_and_malformed_identifiers(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            with self.assertRaises(SystemExit):
                ota.pack(root, "rt", "abcdef0123", 1, "0.0.0", root / "out")
            dist = self.export(root)
            for runtime, bundle_id, minimum in [("../x", "abcdef0123", "0.0.0"), ("rt", "../etc", "0.0.0"), ("rt", "abcdef0123", "v1")]:
                with self.assertRaises(SystemExit):
                    ota.pack(dist, runtime, bundle_id, 1, minimum, root / "out")


class PruneTests(unittest.TestCase):
    def asset(self, runtime: str, bundle_id: str, day: int, suffix: str = ".zip") -> dict:
        return {"name": f"bundle-{runtime}-{bundle_id}{suffix}", "createdAt": f"2026-10-{day:02d}T00:00:00Z"}

    def test_keeps_the_newest_per_runtime_with_their_signatures_and_never_lists_manifests(self):
        assets = [{"name": "manifest-rtA.json", "createdAt": "2026-10-01T00:00:00Z"}]
        for day in range(1, 8):
            bundle_id = f"{day:07x}"
            assets += [self.asset("rtA", bundle_id, day), self.asset("rtA", bundle_id, day, ".zip.sig")]
        assets += [self.asset("rtB", "bbbbbbb", 2), self.asset("rtB", "bbbbbbb", 2, ".zip.sig")]
        doomed = ota.prune(assets, 5)
        self.assertEqual(
            doomed,
            sorted(
                [f"bundle-rtA-{day:07x}.zip" for day in (1, 2)] + [f"bundle-rtA-{day:07x}.zip.sig" for day in (1, 2)]
            ),
        )
        self.assertNotIn("manifest-rtA.json", doomed)

    def test_ignores_unrelated_assets(self):
        self.assertEqual(ota.prune([{"name": "forge-linux.tar.gz", "createdAt": "x"}], 1), [])


if __name__ == "__main__":
    unittest.main()
