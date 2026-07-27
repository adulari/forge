#!/usr/bin/env python3

from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path

import analyze_claude_swe as analyze


class ClaudeAnalysisTests(unittest.TestCase):
    def test_submitted_only_ids_do_not_override_explicit_outcomes(self) -> None:
        expected = {
            ("forge", "sonnet", "medium"),
            ("forge", "sonnet", "easy"),
        }
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "forge::sonnet.medium.json").write_text(
                json.dumps(
                    {
                        "submitted_ids": ["medium", "easy"],
                        "resolved_ids": ["medium"],
                        "unresolved_ids": [],
                        "empty_patch_ids": [],
                        "error_ids": [],
                    }
                ),
                encoding="utf-8",
            )
            (root / "forge::sonnet.easy.json").write_text(
                json.dumps(
                    {
                        "submitted_ids": ["medium", "easy"],
                        "resolved_ids": [],
                        "unresolved_ids": ["easy"],
                        "empty_patch_ids": [],
                        "error_ids": [],
                    }
                ),
                encoding="utf-8",
            )

            outcomes, _ = analyze.official_outcomes([root], expected)

        self.assertTrue(outcomes[("forge", "sonnet", "medium")])
        self.assertFalse(outcomes[("forge", "sonnet", "easy")])

    def test_empty_patch_is_an_explicit_unresolved_outcome(self) -> None:
        expected = {("raw-claude", "sonnet", "hard")}
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "raw-claude::sonnet.hard.json").write_text(
                json.dumps(
                    {
                        "submitted_ids": ["hard"],
                        "resolved_ids": [],
                        "unresolved_ids": [],
                        "empty_patch_ids": ["hard"],
                        "error_ids": [],
                    }
                ),
                encoding="utf-8",
            )

            outcomes, _ = analyze.official_outcomes([root], expected)

        self.assertFalse(outcomes[("raw-claude", "sonnet", "hard")])

    def test_summary_separates_unconditional_and_quality_matched_efficiency(
        self,
    ) -> None:
        rows = [
            {
                "forge_resolved": True,
                "raw_claude_resolved": False,
                "forge_total_tokens": 1000,
                "raw_claude_total_tokens": 10,
                "forge_cache_adjusted_tokens_025": 300,
                "raw_claude_cache_adjusted_tokens_025": 5,
                "forge_wall_seconds": 100,
                "raw_claude_wall_seconds": 1,
            },
            {
                "forge_resolved": True,
                "raw_claude_resolved": True,
                "forge_total_tokens": 60,
                "raw_claude_total_tokens": 100,
                "forge_cache_adjusted_tokens_025": 30,
                "raw_claude_cache_adjusted_tokens_025": 50,
                "forge_wall_seconds": 8,
                "raw_claude_wall_seconds": 10,
            },
        ]

        all_pairs = analyze.summarize_rows(rows)
        matched = analyze.summarize_rows(
            [row for row in rows if row["forge_resolved"] == row["raw_claude_resolved"]]
        )

        self.assertLess(all_pairs["tokens"]["weighted_forge_reduction"], 0)
        self.assertEqual(matched["tokens"]["weighted_forge_reduction"], 0.4)
        self.assertEqual(all_pairs["quality"]["forge_only_resolved"], 1)
        self.assertEqual(all_pairs["quality"]["raw_claude_only_resolved"], 0)


if __name__ == "__main__":
    unittest.main()
