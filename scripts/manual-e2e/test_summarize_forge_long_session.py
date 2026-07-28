import datetime as dt
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))

import summarize_forge_long_session as summary


class ForgeLongSessionSummaryTests(unittest.TestCase):
    def test_prompt_spans_require_all_prompts_in_order(self) -> None:
        messages = [
            {"seq": 1, "role": "user", "content": "first"},
            {"seq": 2, "role": "assistant", "content": "done"},
            {"seq": 3, "role": "user", "content": "second"},
        ]

        self.assertEqual(
            summary.prompt_spans(messages, ["first", "second"]), [(1, 3), (3, None)]
        )
        with self.assertRaises(RuntimeError):
            summary.prompt_spans(messages, ["second", "first"])

    def test_token_totals_include_raw_and_cache_sensitivities(self) -> None:
        rows = [
            {
                "input_tokens": 1_000,
                "cached_input_tokens": 800,
                "output_tokens": 100,
            },
            {
                "input_tokens": 500,
                "cached_input_tokens": 400,
                "output_tokens": 50,
            },
        ]

        totals = summary.token_totals(rows)

        self.assertEqual(totals["raw_tokens"], 1_650)
        self.assertEqual(totals["cache_adjusted_tokens_025"], 750)
        self.assertEqual(totals["cache_zero_credit_tokens"], 450)

    def test_quota_result_reports_arm_and_cumulative_deltas(self) -> None:
        observed = dt.datetime.now(dt.timezone.utc).isoformat()

        result = summary.quota_result(
            baseline=36,
            cap=10,
            before=37,
            after=38,
            before_observed_at=observed,
            after_observed_at=observed,
        )

        self.assertEqual(result["arm_delta_points"], 1)
        self.assertEqual(result["cumulative_delta_points"], 2)
        self.assertTrue(result["within_cap"])
        self.assertTrue(result["valid"])

        decreasing = summary.quota_result(
            baseline=36,
            cap=10,
            before=38,
            after=37,
            before_observed_at=observed,
            after_observed_at=observed,
        )
        self.assertFalse(decreasing["valid"])
        self.assertIn("decreased", " ".join(decreasing["errors"]))

        for baseline, cap, before, after in [
            (96, 5, 96, 96),
            (36, float("nan"), 36, 36),
            (36, 5, float("nan"), 36),
        ]:
            with self.subTest(
                baseline=baseline, cap=cap, before=before, after=after
            ):
                self.assertFalse(
                    summary.quota_result(
                        baseline=baseline,
                        cap=cap,
                        before=before,
                        after=after,
                        before_observed_at=observed,
                        after_observed_at=observed,
                    )["valid"]
                )

    def test_hidden_result_requires_every_adversarial_invariant(self) -> None:
        complete = {
            "contention_requests": 100,
            "contention_winners": 1,
            "duplicate_requests": 100,
            "concurrent_cancellations": 100,
            "interleaved_reserve_cancels": 100,
            "interleaving_verified": True,
            "rollback_verified": True,
            "cancellation_rollback_verified": True,
        }
        self.assertTrue(summary.hidden_result_passed(complete))
        incomplete = {**complete, "contention_winners": 2}
        self.assertFalse(summary.hidden_result_passed(incomplete))
        not_interleaved = {**complete, "interleaving_verified": False}
        self.assertFalse(summary.hidden_result_passed(not_interleaved))

    def test_failed_hidden_verifier_is_retained_as_structured_result(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            log = Path(temporary) / "hidden-tests.log"
            log.write_text(
                "Traceback (most recent call last):\n"
                "reservations.models.NotFound: unknown\n",
                encoding="utf-8",
            )

            result = summary.hidden_result(log)

            self.assertFalse(result["verification_completed"])
            self.assertIn("before emitting", result["error"])
            self.assertEqual(len(result["sha256"]), 64)

    def test_original_test_evidence_requires_replay_methods_and_support(self) -> None:
        base = {
            "test:test_service.py:Tests.test_contract": "original body",
            "support:test_service.py:Tests.setUp": "original setup",
        }
        replay = {"exit_code": 0, "timed_out": False, "tests_skipped": 0}

        strengthened = {
            **base,
            "test:test_service.py:Tests.test_new_case": "new body",
        }
        evidence = summary.original_test_evidence(base, strengthened, replay)
        self.assertTrue(evidence["original_tests_not_weakened"])
        self.assertTrue(evidence["original_tests_preserved"])
        self.assertEqual(evidence["original_test_count"], 1)
        self.assertEqual(evidence["final_test_count"], 2)

        changed_body = {**strengthened, next(iter(base)): "strengthened body"}
        evidence = summary.original_test_evidence(base, changed_body, replay)
        self.assertFalse(evidence["original_tests_preserved"])
        self.assertTrue(
            evidence["original_tests_not_weakened"],
            "the untouched source suite replay, method presence, and support fingerprint "
            "prove behavior even when an original test body was strengthened",
        )

        missing_method = {"support:test_service.py:Tests.setUp": "original setup"}
        self.assertFalse(
            summary.original_test_evidence(
                base, missing_method, replay
            )["original_tests_not_weakened"]
        )
        changed_support = {
            "test:test_service.py:Tests.test_contract": "original body",
            "support:test_service.py:Tests.setUp": "weakened setup",
        }
        self.assertFalse(
            summary.original_test_evidence(
                base, changed_support, replay
            )["original_tests_not_weakened"]
        )

    def test_harness_records_merge_resumed_turns_without_hiding_failed_attempt(self) -> None:
        merged = summary.merge_harness_records(
            [
                {
                    "session_id": "session-1",
                    "elapsed_s": 301,
                    "turns_expected": 6,
                    "turns": [
                        {"turn": 1, "completed": True, "elapsed_s": 114},
                    ],
                    "operator_interrupted": True,
                    "prompt_dispatch_failed": True,
                },
                {
                    "session_id": "session-1",
                    "elapsed_s": 700,
                    "quota_gate_wait_s": 12.5,
                    "quota_gates": [{"turn": 2, "codex_weekly_percent": 8.0}],
                    "turns_expected": 6,
                    "turns": [
                        {"turn": number, "completed": True, "elapsed_s": 100}
                        for number in range(2, 7)
                    ],
                },
            ]
        )

        self.assertEqual(merged["turns_completed"], 6)
        self.assertEqual(
            [turn["turn"] for turn in merged["turns"]], [1, 2, 3, 4, 5, 6]
        )
        self.assertEqual(merged["attempt_elapsed_s"], 1_001)
        self.assertEqual(merged["quota_gate_wait_s"], 12.5)
        self.assertEqual(merged["quota_gates"][0]["turn"], 2)
        self.assertTrue(merged["attempts"][0]["operator_interrupted"])
        self.assertTrue(merged["attempts"][0]["prompt_dispatch_failed"])


if __name__ == "__main__":
    unittest.main()
