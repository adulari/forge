#!/usr/bin/env python3
import importlib.util
import json
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).with_name("check-mobile-parity.py")
spec = importlib.util.spec_from_file_location("mobile_parity", SCRIPT)
assert spec and spec.loader
parity = importlib.util.module_from_spec(spec)
spec.loader.exec_module(parity)


class ParityGuardTests(unittest.TestCase):
    def run_fixture(self, source_text: str, inventory: list[dict[str, str]]) -> tuple[dict[str, str], dict[str, dict[str, str]]]:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "mobile" / "src"
            source.mkdir(parents=True)
            file = source / "Screen.tsx"
            file.write_text(source_text, encoding="utf-8")
            inventory_path = root / "mobile" / "parity" / "inventory.json"
            inventory_path.parent.mkdir(parents=True)
            inventory_path.write_text(json.dumps(inventory), encoding="utf-8")
            old_root, old_source, old_inventory = parity.ROOT, parity.SOURCE_ROOT, parity.INVENTORY
            parity.ROOT, parity.SOURCE_ROOT, parity.INVENTORY = root, source, inventory_path
            try:
                return parity.detected_files(), parity.load_inventory()
            finally:
                parity.ROOT, parity.SOURCE_ROOT, parity.INVENTORY = old_root, old_source, old_inventory

    def test_undeclared_behavior_is_detected(self) -> None:
        detected, declared = self.run_fixture(
            'export function Screen() { return Platform.OS === "web" ? <Text>web</Text> : <Text>native</Text>; }',
            [],
        )
        self.assertEqual(detected, {"mobile/src/Screen.tsx": "behavior"})
        self.assertNotIn("mobile/src/Screen.tsx", declared)

    def test_declared_behavior_passes(self) -> None:
        detected, declared = self.run_fixture(
            'export function Screen() { return Platform.OS === "web" ? <Text>web</Text> : <Text>native</Text>; }',
            [{"file": "mobile/src/Screen.tsx", "platform": "web/native", "reason": "reviewed state boundary", "category": "behavior"}],
        )
        self.assertEqual(declared["mobile/src/Screen.tsx"]["category"], detected["mobile/src/Screen.tsx"])

    def test_category_mismatch_is_representable(self) -> None:
        detected, declared = self.run_fixture(
            'const padding = Platform.OS === "web" ? 8 : 4;',
            [{"file": "mobile/src/Screen.tsx", "platform": "web/native", "reason": "inset capability", "category": "behavior"}],
        )
        self.assertEqual(detected["mobile/src/Screen.tsx"], "capability")
        self.assertNotEqual(declared["mobile/src/Screen.tsx"]["category"], detected["mobile/src/Screen.tsx"])


if __name__ == "__main__":
    unittest.main()
