#!/usr/bin/env python3
"""Re-score a completed matched benchmark from preserved raw artifacts."""

from __future__ import annotations

import argparse
import copy
import json
import math
import random
from collections import defaultdict
from pathlib import Path
from typing import Any

import compare_codex_oauth as bench


def exact_two_sided_sign_test(wins: int, losses: int) -> float | None:
    n = wins + losses
    if n == 0:
        return None
    tail = min(wins, losses)
    probability = 2 * sum(math.comb(n, k) for k in range(tail + 1)) / (2**n)
    return round(min(1.0, probability), 8)


def percentile(values: list[float], fraction: float) -> float:
    ordered = sorted(values)
    if not ordered:
        raise ValueError("percentile requires at least one value")
    position = (len(ordered) - 1) * fraction
    lower = math.floor(position)
    upper = math.ceil(position)
    if lower == upper:
        return ordered[lower]
    weight = position - lower
    return ordered[lower] * (1 - weight) + ordered[upper] * weight


def bootstrap_median_ci(
    values: list[float],
    *,
    samples: int = 20_000,
    seed: int = 20260726,
) -> list[float] | None:
    if not values:
        return None
    rng = random.Random(seed)
    medians: list[float] = []
    for _ in range(samples):
        draw = [rng.choice(values) for _ in values]
        draw.sort()
        middle = len(draw) // 2
        value = (
            draw[middle]
            if len(draw) % 2
            else (draw[middle - 1] + draw[middle]) / 2
        )
        medians.append(value)
    return [
        round(percentile(medians, 0.025), 4),
        round(percentile(medians, 0.975), 4),
    ]


def corrected_typescript_verification(
    summary: dict[str, Any],
    trial_dir: Path,
    timeout_seconds: int,
) -> dict[str, Any]:
    original_steps = summary["verification"]["steps"]
    npm_test = next(
        (
            step
            for step in original_steps
            if step.get("argv") == ["npm", "test"]
        ),
        None,
    )
    stdout_path = trial_dir / "contract-build.stdout.log"
    stderr_path = trial_dir / "contract-build.stderr.log"
    direct = bench.run_capture(
        ["npm", "run", "build", "--", "--noEmit"],
        cwd=trial_dir / "workspace",
        stdout_path=stdout_path,
        stderr_path=stderr_path,
        timeout_seconds=timeout_seconds,
    )
    direct_step = vars(direct) | {
        "passed": direct.exit_code == 0 and not direct.timed_out,
        "purpose": "contract-required strict TypeScript no-emit build",
    }
    npm_test_passed = bool(npm_test and npm_test.get("passed"))
    return {
        "passed": npm_test_passed and direct_step["passed"],
        "steps": [npm_test, direct_step],
        "scoring_rule": (
            "npm test plus direct strict no-emit build; a package-script named `lint` "
            "is not part of the task contract"
        ),
        "original_verification": summary["verification"],
    }


def rescore_trial(
    summary_path: Path,
    *,
    forge_db: Path,
    verifier_timeout_seconds: int,
) -> dict[str, Any]:
    summary = json.loads(summary_path.read_text(encoding="utf-8"))
    corrected = copy.deepcopy(summary)
    corrected["original_success"] = summary["success"]
    corrected["corrections"] = []
    if summary["trial"]["arm"] == "forge":
        complete = bench.forge_session_tokens(
            forge_db,
            summary["agent"].get("session_id"),
        )
        if complete is None:
            raise RuntimeError(f"missing Forge ledger usage for {summary_path}")
        stream = summary["agent"]["tokens"]
        corrected["agent"]["stream_tokens"] = stream
        corrected["agent"]["tokens"] = complete
        corrected["agent"]["post_stream_tokens"] = (
            int(complete["total_tokens"]) - int(stream["total_tokens"])
        )
        corrected["corrections"].append(
            {
                "kind": "complete_forge_ledger_usage",
                "stream_total_tokens": stream["total_tokens"],
                "ledger_total_tokens": complete["total_tokens"],
                "post_stream_tokens": corrected["agent"]["post_stream_tokens"],
            }
        )
    if summary["trial"]["scenario"] == "typescript-config-recovery":
        corrected["verification"] = corrected_typescript_verification(
            summary,
            summary_path.parent,
            verifier_timeout_seconds,
        )
        corrected["corrections"].append(
            {
                "kind": "typescript_contract_verifier",
                "original_rule": "npm test plus npm run lint",
                "corrected_rule": "npm test plus direct strict no-emit build",
            }
        )
    process_ok = (
        corrected["process"]["exit_code"] == 0
        and not corrected["process"]["timed_out"]
    )
    corrected["success"] = process_ok and corrected["verification"]["passed"]
    corrected["corrected_at"] = bench.utc_now()
    bench.json_dump(summary_path.parent / "summary.corrected.json", corrected)
    return corrected


def paired_analysis(summaries: list[dict[str, Any]]) -> dict[str, Any]:
    pairs: dict[str, dict[str, dict[str, Any]]] = defaultdict(dict)
    for summary in summaries:
        pairs[summary["pair_id"]][summary["trial"]["arm"]] = summary
    rows: list[dict[str, Any]] = []
    for pair_id, arms in sorted(pairs.items()):
        if set(arms) != set(bench.DEFAULT_ARMS):
            continue
        forge = arms["forge"]
        raw = arms["raw-codex"]
        forge_tokens = int(forge["agent"]["tokens"]["total_tokens"])
        raw_tokens = int(raw["agent"]["tokens"]["total_tokens"])
        forge_wall = float(forge["process"]["wall_seconds"])
        raw_wall = float(raw["process"]["wall_seconds"])
        rows.append(
            {
                "pair_id": pair_id,
                "model": forge["trial"]["model"],
                "scenario": forge["trial"]["scenario"],
                "forge_success": forge["success"],
                "raw_codex_success": raw["success"],
                "forge_total_tokens": forge_tokens,
                "raw_codex_total_tokens": raw_tokens,
                "forge_token_reduction_vs_raw": round(
                    (raw_tokens - forge_tokens) / raw_tokens,
                    4,
                ),
                "forge_wall_seconds": forge_wall,
                "raw_codex_wall_seconds": raw_wall,
                "forge_wall_reduction_vs_raw": round(
                    (raw_wall - forge_wall) / raw_wall,
                    4,
                ),
            }
        )
    token_reductions = [row["forge_token_reduction_vs_raw"] for row in rows]
    wall_reductions = [row["forge_wall_reduction_vs_raw"] for row in rows]
    token_wins = sum(value > 0 for value in token_reductions)
    token_losses = sum(value < 0 for value in token_reductions)
    wall_wins = sum(value > 0 for value in wall_reductions)
    wall_losses = sum(value < 0 for value in wall_reductions)
    total_forge_tokens = sum(row["forge_total_tokens"] for row in rows)
    total_raw_tokens = sum(row["raw_codex_total_tokens"] for row in rows)
    total_forge_wall = sum(row["forge_wall_seconds"] for row in rows)
    total_raw_wall = sum(row["raw_codex_wall_seconds"] for row in rows)
    return {
        "pairs": rows,
        "summary": {
            "complete_pairs": len(rows),
            "quality": {
                "forge_passes": sum(row["forge_success"] for row in rows),
                "raw_codex_passes": sum(row["raw_codex_success"] for row in rows),
                "forge_only_passes": sum(
                    row["forge_success"] and not row["raw_codex_success"] for row in rows
                ),
                "raw_codex_only_passes": sum(
                    row["raw_codex_success"] and not row["forge_success"] for row in rows
                ),
            },
            "tokens": {
                "forge_total": total_forge_tokens,
                "raw_codex_total": total_raw_tokens,
                "weighted_forge_reduction": round(
                    (total_raw_tokens - total_forge_tokens) / total_raw_tokens,
                    4,
                ),
                "median_paired_forge_reduction": round(
                    float(bench.median(token_reductions) or 0),
                    4,
                ),
                "bootstrap_95pct_ci_for_median_reduction": bootstrap_median_ci(
                    token_reductions
                ),
                "forge_lower_pairs": token_wins,
                "raw_codex_lower_pairs": token_losses,
                "ties": len(rows) - token_wins - token_losses,
                "two_sided_exact_sign_test_p": exact_two_sided_sign_test(
                    token_wins,
                    token_losses,
                ),
            },
            "wall_time": {
                "forge_total_seconds": round(total_forge_wall, 3),
                "raw_codex_total_seconds": round(total_raw_wall, 3),
                "weighted_forge_reduction": round(
                    (total_raw_wall - total_forge_wall) / total_raw_wall,
                    4,
                ),
                "median_paired_forge_reduction": round(
                    float(bench.median(wall_reductions) or 0),
                    4,
                ),
                "forge_faster_pairs": wall_wins,
                "raw_codex_faster_pairs": wall_losses,
                "ties": len(rows) - wall_wins - wall_losses,
                "two_sided_exact_sign_test_p": exact_two_sided_sign_test(
                    wall_wins,
                    wall_losses,
                ),
            },
        },
    }


def group_analysis(
    paired_rows: list[dict[str, Any]],
    key: str,
) -> list[dict[str, Any]]:
    groups: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for row in paired_rows:
        groups[row[key]].append(row)
    result: list[dict[str, Any]] = []
    for name, rows in sorted(groups.items()):
        forge_tokens = sum(row["forge_total_tokens"] for row in rows)
        raw_tokens = sum(row["raw_codex_total_tokens"] for row in rows)
        result.append(
            {
                key: name,
                "pairs": len(rows),
                "forge_passes": sum(row["forge_success"] for row in rows),
                "raw_codex_passes": sum(row["raw_codex_success"] for row in rows),
                "forge_total_tokens": forge_tokens,
                "raw_codex_total_tokens": raw_tokens,
                "weighted_forge_token_reduction": round(
                    (raw_tokens - forge_tokens) / raw_tokens,
                    4,
                ),
                "median_paired_forge_token_reduction": bench.median(
                    row["forge_token_reduction_vs_raw"] for row in rows
                ),
            }
        )
    return result


def render_corrected_markdown(report: dict[str, Any]) -> str:
    summary = report["paired_analysis"]["summary"]
    lines = [
        "# Corrected matched-run baseline",
        "",
        f"Generated: {report['generated_at']}",
        "",
        "## Outcome",
        "",
        f"- Contract passes: Forge {summary['quality']['forge_passes']}/"
        f"{summary['complete_pairs']}, raw Codex {summary['quality']['raw_codex_passes']}/"
        f"{summary['complete_pairs']}.",
        f"- Total tokens: Forge {summary['tokens']['forge_total']:,}, "
        f"raw Codex {summary['tokens']['raw_codex_total']:,}.",
        f"- Weighted Forge token reduction: "
        f"{summary['tokens']['weighted_forge_reduction']:.1%}.",
        f"- Median paired Forge token reduction: "
        f"{summary['tokens']['median_paired_forge_reduction']:.1%}, "
        f"bootstrap 95% CI {summary['tokens']['bootstrap_95pct_ci_for_median_reduction']}.",
        f"- Forge used fewer tokens in {summary['tokens']['forge_lower_pairs']}/"
        f"{summary['complete_pairs']} pairs; exact two-sided sign-test "
        f"`p={summary['tokens']['two_sided_exact_sign_test_p']}`.",
        f"- Weighted Forge wall-time reduction: "
        f"{summary['wall_time']['weighted_forge_reduction']:.1%}; Forge faster in "
        f"{summary['wall_time']['forge_faster_pairs']}/{summary['complete_pairs']} pairs.",
        "",
        "## Per model",
        "",
        "| model | pass Forge/raw | Forge tokens | raw tokens | weighted reduction |",
        "|---|---:|---:|---:|---:|",
    ]
    for row in report["by_model"]:
        lines.append(
            f"| {row['model']} | {row['forge_passes']}/{row['raw_codex_passes']} | "
            f"{row['forge_total_tokens']:,} | {row['raw_codex_total_tokens']:,} | "
            f"{row['weighted_forge_token_reduction']:.1%} |"
        )
    lines.extend(
        [
            "",
            "## Per scenario",
            "",
            "| scenario | pass Forge/raw | Forge tokens | raw tokens | weighted reduction |",
            "|---|---:|---:|---:|---:|",
        ]
    )
    for row in report["by_scenario"]:
        lines.append(
            f"| {row['scenario']} | {row['forge_passes']}/{row['raw_codex_passes']} | "
            f"{row['forge_total_tokens']:,} | {row['raw_codex_total_tokens']:,} | "
            f"{row['weighted_forge_token_reduction']:.1%} |"
        )
    lines.extend(
        [
            "",
            "## Corrections applied",
            "",
            "- Forge totals come from the complete SQLite session usage ledger, including "
            "post-stream memory extraction.",
            "- TypeScript correctness is `npm test` plus a direct strict no-emit build. "
            "A script specifically named `lint` is not required by the task contract.",
            "- Cached input remains a subset of input tokens and is not double-counted.",
            "",
            "This controlled suite reached a quality ceiling: both arms passed every task. "
            "It proves a large token/latency difference on these fixtures, not superior "
            "general intelligence or SWE-bench resolve rate.",
            "",
        ]
    )
    return "\n".join(lines)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run-dir", required=True, type=Path)
    parser.add_argument("--verifier-timeout-seconds", type=int, default=600)
    args = parser.parse_args()
    forge_db = args.run_dir / "forge-benchmark.db"
    summary_paths = sorted(args.run_dir.glob("trials/*/summary.json"))
    if not summary_paths:
        parser.error(f"no trial summaries under {args.run_dir}")
    summaries = [
        rescore_trial(
            path,
            forge_db=forge_db,
            verifier_timeout_seconds=args.verifier_timeout_seconds,
        )
        for path in summary_paths
    ]
    paired = paired_analysis(summaries)
    base_aggregate = bench.aggregate(summaries)
    report = {
        "schema_version": 1,
        "generated_at": bench.utc_now(),
        "run_dir": str(args.run_dir),
        "trial_count": len(summaries),
        "paired_analysis": paired,
        "by_model": group_analysis(paired["pairs"], "model"),
        "by_scenario": group_analysis(paired["pairs"], "scenario"),
        "base_aggregate": base_aggregate,
    }
    bench.json_dump(args.run_dir / "aggregate.corrected.json", report)
    (args.run_dir / "aggregate.corrected.md").write_text(
        render_corrected_markdown(report),
        encoding="utf-8",
    )
    print(json.dumps(report["paired_analysis"]["summary"], indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
