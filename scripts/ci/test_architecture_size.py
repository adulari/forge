#!/usr/bin/env python3
"""Unit tests for the architecture size guard."""

from __future__ import annotations

import importlib.util
import sys
import tempfile
import unittest
from pathlib import Path


SCRIPT_PATH = Path(__file__).with_name("architecture_size.py")
SPEC = importlib.util.spec_from_file_location("architecture_size", SCRIPT_PATH)
assert SPEC is not None and SPEC.loader is not None
architecture_size = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = architecture_size
SPEC.loader.exec_module(architecture_size)


class ArchitectureSizeTests(unittest.TestCase):
    def test_masks_literals_comments_and_nested_block_comments(self) -> None:
        source = r'''
fn production() {
    let normal = "{ not structure }";
    let raw = r###"also { not structure }"###;
    let character = '}';
    // }
    /* { outer /* nested } */ } */
}
'''
        masked = architecture_size.mask_non_code(source)

        self.assertEqual(masked.count("{"), 1)
        self.assertEqual(masked.count("}"), 1)
        self.assertEqual(masked.count("\n"), source.count("\n"))

    def test_counts_inline_cfg_test_item_separately(self) -> None:
        source = """\
fn production() {
}

#[cfg(test)]
mod tests {
    #[test]
    fn behavior() {
        assert_eq!("}", "}");
    }
}
"""
        test_lines = architecture_size.cfg_test_line_numbers(source)

        self.assertEqual(test_lines, set(range(3, 10)))
        self.assertEqual(len(source.splitlines()) - len(test_lines), 3)

    def test_discovers_external_cfg_test_module(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source_dir = root / "crates" / "demo" / "src"
            source_dir.mkdir(parents=True)
            (source_dir / "lib.rs").write_text(
                "pub fn production() {}\n#[cfg(test)]\nmod checks;\n",
                encoding="utf-8",
            )
            (source_dir / "checks.rs").write_text(
                "#[test]\nfn behavior() {}\n",
                encoding="utf-8",
            )

            metrics = {
                metric.path: metric for metric in architecture_size.collect_metrics(root)
            }

        self.assertEqual(
            metrics["crates/demo/src/lib.rs"].implementation_lines,
            1,
        )
        self.assertEqual(metrics["crates/demo/src/lib.rs"].test_lines, 2)
        self.assertEqual(metrics["crates/demo/src/checks.rs"].implementation_lines, 0)
        self.assertEqual(metrics["crates/demo/src/checks.rs"].test_lines, 2)

    def test_rejects_growth_in_existing_large_file(self) -> None:
        baseline_metric = architecture_size.FileMetrics("crates/a/src/lib.rs", 900, 900, 0)
        baseline = architecture_size.build_baseline([baseline_metric], "base")
        current = architecture_size.FileMetrics("crates/a/src/lib.rs", 901, 901, 0)

        violations = architecture_size.check_guardrails([current], baseline)

        self.assertTrue(any("exceeds ratchet 900" in item for item in violations))

    def test_rejects_new_large_file(self) -> None:
        baseline_metrics = [
            architecture_size.FileMetrics("crates/a/src/lib.rs", 10, 10, 0),
        ]
        baseline = architecture_size.build_baseline(baseline_metrics, "base")
        current_metrics = [
            *baseline_metrics,
            architecture_size.FileMetrics("crates/a/src/new_owner.rs", 801, 801, 0),
        ]

        violations = architecture_size.check_guardrails(current_metrics, baseline)

        self.assertTrue(any("new/untracked" in item for item in violations))

    def test_allows_deep_extraction_that_improves_distribution(self) -> None:
        baseline_metrics = [
            architecture_size.FileMetrics("crates/a/src/lib.rs", 1_200, 1_200, 0),
            architecture_size.FileMetrics("crates/a/src/small.rs", 100, 100, 0),
        ]
        baseline = architecture_size.build_baseline(baseline_metrics, "base")
        current_metrics = [
            architecture_size.FileMetrics("crates/a/src/lib.rs", 800, 800, 0),
            architecture_size.FileMetrics("crates/a/src/small.rs", 100, 100, 0),
            architecture_size.FileMetrics("crates/a/src/policy.rs", 400, 400, 0),
        ]

        self.assertEqual(
            architecture_size.check_guardrails(current_metrics, baseline),
            [],
        )

    def test_small_crate_root_can_grow_without_crossing_target(self) -> None:
        baseline_metrics = [
            architecture_size.FileMetrics("crates/a/src/lib.rs", 100, 100, 0),
        ]
        baseline = architecture_size.build_baseline(baseline_metrics, "base")
        current_metrics = [
            architecture_size.FileMetrics("crates/a/src/lib.rs", 500, 500, 0),
        ]

        self.assertEqual(
            architecture_size.check_guardrails(current_metrics, baseline),
            [],
        )


if __name__ == "__main__":
    unittest.main()
