#!/usr/bin/env python3
"""Aggregate matched Forge/native-Claude trials with official SWE-bench reports."""

from __future__ import annotations

import argparse
import json
import math
import statistics
from collections import defaultdict
from pathlib import Path
from typing import Any, Iterable, Sequence

import compare_codex_oauth as bench

ARMS = ("forge", "raw-claude")


def load_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def load_summaries(path: Path) -> list[dict[str, Any]]:
    summaries: list[dict[str, Any]] = []
    with path.open(encoding="utf-8") as handle:
        for line_number, line in enumerate(handle, 1):
            if not line.strip():
                continue
            try:
                summaries.append(json.loads(line))
            except json.JSONDecodeError as error:
                raise ValueError(
                    f"{path}:{line_number}: invalid JSON: {error}"
                ) from error
    return summaries


def report_identity(path: Path) -> tuple[str, str] | None:
    prefix = path.name.split(".", 1)[0]
    if "::" not in prefix:
        return None
    arm, model = prefix.split("::", 1)
    if arm not in ARMS or not model:
        return None
    return arm, model


def official_outcomes(
    evaluation_dirs: Sequence[Path],
    expected: set[tuple[str, str, str]],
) -> tuple[
    dict[tuple[str, str, str], bool],
    dict[tuple[str, str, str], str],
]:
    """Load explicit evaluator outcomes.

    SWE-bench's ``submitted_ids`` can contain every row in a prediction file even
    when ``--instance_ids`` evaluates only one row. It is therefore metadata, not
    an outcome. Only resolved, unresolved, and empty-patch lists are authoritative.
    """

    outcomes: dict[tuple[str, str, str], bool] = {}
    sources: dict[tuple[str, str, str], str] = {}
    report_count = 0

    for evaluation_dir in evaluation_dirs:
        for path in sorted(evaluation_dir.glob("*.json")):
            identity = report_identity(path)
            if identity is None:
                continue
            report = load_json(path)
            if "resolved_ids" not in report:
                continue
            report_count += 1
            arm, model = identity

            error_ids = set(report.get("error_ids") or [])
            if error_ids:
                raise ValueError(
                    f"official evaluator errors in {path}: {sorted(error_ids)}"
                )

            resolved_ids = set(report.get("resolved_ids") or [])
            unresolved_ids = set(report.get("unresolved_ids") or [])
            empty_patch_ids = set(report.get("empty_patch_ids") or [])
            negative_ids = unresolved_ids | empty_patch_ids
            overlap = resolved_ids & negative_ids
            if overlap:
                raise ValueError(
                    f"contradictory official outcomes in {path}: {sorted(overlap)}"
                )

            for instance_id, resolved in (
                *((instance_id, True) for instance_id in resolved_ids),
                *((instance_id, False) for instance_id in negative_ids),
            ):
                key = (arm, model, str(instance_id))
                if key not in expected:
                    continue
                if key in outcomes:
                    raise ValueError(
                        f"duplicate official outcome for {key}: "
                        f"{sources[key]} and {path.resolve()}"
                    )
                outcomes[key] = resolved
                sources[key] = str(path.resolve())

    if report_count == 0:
        raise ValueError(
            "no official SWE-bench reports found; expected files named "
            "'forge::<model>.*.json' and 'raw-claude::<model>.*.json'"
        )

    missing = sorted(expected - set(outcomes))
    if missing:
        raise ValueError(f"missing explicit official outcomes: {missing}")
    return outcomes, sources


def paired_rows(
    summaries: Sequence[dict[str, Any]],
    outcomes: dict[tuple[str, str, str], bool],
    outcome_sources: dict[tuple[str, str, str], str],
) -> list[dict[str, Any]]:
    grouped: dict[str, dict[str, dict[str, Any]]] = defaultdict(dict)
    for summary in summaries:
        trial = summary.get("trial") or {}
        arm = trial.get("arm")
        pair_id = summary.get("pair_id")
        if arm not in ARMS or not isinstance(pair_id, str):
            raise ValueError("trial summary has invalid arm or pair_id")
        if arm in grouped[pair_id]:
            raise ValueError(f"duplicate {arm} summary for {pair_id}")
        grouped[pair_id][arm] = summary

    rows: list[dict[str, Any]] = []
    for pair_id, arms in grouped.items():
        if set(arms) != set(ARMS):
            raise ValueError(f"incomplete matched pair {pair_id}: {sorted(arms)}")

        forge = arms["forge"]
        raw = arms["raw-claude"]
        forge_trial = forge["trial"]
        raw_trial = raw["trial"]
        model = str(forge_trial["model"])
        instance_id = str(forge_trial["instance_id"])
        if raw_trial["model"] != model or raw_trial["instance_id"] != instance_id:
            raise ValueError(f"mismatched pair metadata {pair_id}")

        forge_tokens = forge["agent"]["tokens"].get("total_tokens")
        raw_tokens = raw["agent"]["tokens"].get("total_tokens")
        forge_adjusted = forge["agent"]["tokens"].get("cache_adjusted_tokens_025")
        raw_adjusted = raw["agent"]["tokens"].get("cache_adjusted_tokens_025")
        forge_wall = forge["process"].get("wall_seconds")
        raw_wall = raw["process"].get("wall_seconds")
        numeric = (
            forge_tokens,
            raw_tokens,
            forge_adjusted,
            raw_adjusted,
            forge_wall,
            raw_wall,
        )
        if not all(isinstance(value, (int, float)) for value in numeric):
            raise ValueError(f"incomplete wall/token telemetry {pair_id}")
        if raw_tokens <= 0 or raw_adjusted <= 0 or raw_wall <= 0:
            raise ValueError(f"invalid native-Claude denominator {pair_id}")

        forge_key = ("forge", model, instance_id)
        raw_key = ("raw-claude", model, instance_id)
        rows.append(
            {
                "pair_id": pair_id,
                "dataset_index": forge.get("dataset_index"),
                "model": model,
                "instance_id": instance_id,
                "repo": forge.get("repo"),
                "difficulty": forge.get("difficulty"),
                "forge_resolved": outcomes[forge_key],
                "raw_claude_resolved": outcomes[raw_key],
                "forge_total_tokens": int(forge_tokens),
                "raw_claude_total_tokens": int(raw_tokens),
                "forge_token_reduction_vs_raw": round(
                    (float(raw_tokens) - float(forge_tokens)) / float(raw_tokens),
                    6,
                ),
                "forge_cache_adjusted_tokens_025": float(forge_adjusted),
                "raw_claude_cache_adjusted_tokens_025": float(raw_adjusted),
                "forge_cache_adjusted_reduction_vs_raw": round(
                    (float(raw_adjusted) - float(forge_adjusted)) / float(raw_adjusted),
                    6,
                ),
                "forge_wall_seconds": round(float(forge_wall), 3),
                "raw_claude_wall_seconds": round(float(raw_wall), 3),
                "forge_wall_reduction_vs_raw": round(
                    (float(raw_wall) - float(forge_wall)) / float(raw_wall),
                    6,
                ),
                "forge_provider_calls": forge["agent"]["tokens"].get("provider_calls"),
                "raw_claude_provider_calls": raw["agent"]["tokens"].get(
                    "provider_calls"
                ),
                "forge_report": outcome_sources[forge_key],
                "raw_claude_report": outcome_sources[raw_key],
            }
        )

    return sorted(
        rows, key=lambda row: (row["model"], row["dataset_index"], row["instance_id"])
    )


def median(values: Iterable[float]) -> float:
    return round(float(statistics.median(values)), 6)


def exact_two_sided_sign_test(wins: int, losses: int) -> float:
    trials = wins + losses
    if trials == 0:
        return 1.0
    tail = sum(math.comb(trials, index) for index in range(min(wins, losses) + 1))
    return min(1.0, 2.0 * tail / (2**trials))


def comparative_metric(
    rows: Sequence[dict[str, Any]],
    forge_key: str,
    raw_key: str,
) -> dict[str, Any]:
    forge_total = sum(float(row[forge_key]) for row in rows)
    raw_total = sum(float(row[raw_key]) for row in rows)
    reductions = [
        (float(row[raw_key]) - float(row[forge_key])) / float(row[raw_key])
        for row in rows
    ]
    wins = sum(float(row[forge_key]) < float(row[raw_key]) for row in rows)
    losses = sum(float(row[forge_key]) > float(row[raw_key]) for row in rows)
    return {
        "forge_total": round(forge_total, 3),
        "raw_claude_total": round(raw_total, 3),
        "weighted_forge_reduction": round(
            (raw_total - forge_total) / raw_total if raw_total else 0.0,
            6,
        ),
        "median_paired_forge_reduction": median(reductions) if reductions else 0.0,
        "forge_better_pairs": wins,
        "raw_claude_better_pairs": losses,
        "ties": len(rows) - wins - losses,
        "two_sided_exact_sign_test_p": exact_two_sided_sign_test(wins, losses),
    }


def summarize_rows(rows: Sequence[dict[str, Any]]) -> dict[str, Any]:
    forge_only = sum(
        row["forge_resolved"] and not row["raw_claude_resolved"] for row in rows
    )
    raw_only = sum(
        row["raw_claude_resolved"] and not row["forge_resolved"] for row in rows
    )
    return {
        "complete_pairs": len(rows),
        "quality": {
            "forge_resolved": sum(row["forge_resolved"] for row in rows),
            "raw_claude_resolved": sum(row["raw_claude_resolved"] for row in rows),
            "forge_only_resolved": forge_only,
            "raw_claude_only_resolved": raw_only,
            "both_resolved": sum(
                row["forge_resolved"] and row["raw_claude_resolved"] for row in rows
            ),
            "both_unresolved": sum(
                not row["forge_resolved"] and not row["raw_claude_resolved"]
                for row in rows
            ),
            "two_sided_exact_mcnemar_p": exact_two_sided_sign_test(
                forge_only, raw_only
            ),
        },
        "tokens": comparative_metric(
            rows,
            "forge_total_tokens",
            "raw_claude_total_tokens",
        ),
        "cache_adjusted_tokens_025": comparative_metric(
            rows,
            "forge_cache_adjusted_tokens_025",
            "raw_claude_cache_adjusted_tokens_025",
        ),
        "wall_time": comparative_metric(
            rows,
            "forge_wall_seconds",
            "raw_claude_wall_seconds",
        ),
    }


def grouped_summaries(
    rows: Sequence[dict[str, Any]],
    key: str,
) -> list[dict[str, Any]]:
    grouped: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for row in rows:
        grouped[str(row[key])].append(row)
    return [
        {key: value, **summarize_rows(group)}
        for value, group in sorted(grouped.items())
    ]


def render_markdown(report: dict[str, Any]) -> str:
    all_summary = report["summary"]["all_pairs"]
    matched = report["summary"]["quality_matched_pairs"]
    resolved = report["summary"]["both_resolved_pairs"]
    quality = all_summary["quality"]

    lines = [
        "# Official Forge vs. native Claude Code SWE-bench analysis",
        "",
        f"- Official resolves: Forge {quality['forge_resolved']}/"
        f"{all_summary['complete_pairs']}; native Claude Code "
        f"{quality['raw_claude_resolved']}/{all_summary['complete_pairs']}.",
        f"- Quality-matched pairs: {matched['complete_pairs']}; Forge wall-time reduction "
        f"{matched['wall_time']['weighted_forge_reduction']:.2%}; processed-token reduction "
        f"{matched['tokens']['weighted_forge_reduction']:.2%}.",
        f"- Both-resolved pairs: {resolved['complete_pairs']}; Forge wall-time reduction "
        f"{resolved['wall_time']['weighted_forge_reduction']:.2%}; processed-token reduction "
        f"{resolved['tokens']['weighted_forge_reduction']:.2%}.",
        "",
        "Unconditional totals include fast failures and are retained in the JSON output. "
        "Quality-matched and both-resolved subsets are the meaningful efficiency comparisons.",
        "",
        "## Per pair",
        "",
        "| Model | Difficulty | Instance | Forge | Native | Forge wall | Native wall | "
        "Forge tokens | Native tokens |",
        "|---|---|---|---:|---:|---:|---:|---:|---:|",
    ]
    for row in report["pairs"]:
        lines.append(
            f"| {row['model']} | {row['difficulty']} | {row['instance_id']} | "
            f"{'resolved' if row['forge_resolved'] else 'unresolved'} | "
            f"{'resolved' if row['raw_claude_resolved'] else 'unresolved'} | "
            f"{row['forge_wall_seconds']:.3f}s | {row['raw_claude_wall_seconds']:.3f}s | "
            f"{row['forge_total_tokens']:,} | {row['raw_claude_total_tokens']:,} |"
        )
    return "\n".join(lines) + "\n"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run-dir", type=Path, action="append", required=True)
    parser.add_argument(
        "--evaluation-dir",
        type=Path,
        action="append",
        help="repeat as needed; defaults to every --run-dir",
    )
    parser.add_argument("--out-json", type=Path, default=Path("official-analysis.json"))
    parser.add_argument(
        "--out-markdown",
        type=Path,
        default=Path("official-analysis.md"),
    )
    args = parser.parse_args()

    run_dirs = [path.resolve() for path in args.run_dir]
    evaluation_dirs = [path.resolve() for path in (args.evaluation_dir or args.run_dir)]

    summaries: list[dict[str, Any]] = []
    manifests: list[dict[str, Any]] = []
    run_metadata: list[dict[str, Any]] = []
    for run_dir in run_dirs:
        index_path = run_dir / "trial-index.jsonl"
        manifest_path = run_dir / "suite-manifest.json"
        if not index_path.is_file() or not manifest_path.is_file():
            parser.error(
                f"{run_dir} is missing trial-index.jsonl or suite-manifest.json"
            )
        summaries.extend(load_summaries(index_path))
        manifest = load_json(manifest_path)
        manifests.append(manifest)
        run_metadata.append(
            {
                "run_dir": str(run_dir),
                "dataset": manifest.get("dataset"),
                "dataset_sha256": manifest.get("dataset_sha256"),
                "completed_trials": len(load_summaries(index_path)),
            }
        )

    invariant_keys = (
        "forge_commit",
        "forge_binary_sha256",
        "claude_version",
        "reasoning_effort",
    )
    for key in invariant_keys:
        values = {
            json.dumps(manifest.get(key), sort_keys=True) for manifest in manifests
        }
        if len(values) != 1:
            parser.error(f"run manifests disagree on {key}: {sorted(values)}")

    expected = {
        (
            str(summary["trial"]["arm"]),
            str(summary["trial"]["model"]),
            str(summary["trial"]["instance_id"]),
        )
        for summary in summaries
    }
    try:
        outcomes, sources = official_outcomes(evaluation_dirs, expected)
        rows = paired_rows(summaries, outcomes, sources)
    except ValueError as error:
        parser.error(str(error))

    quality_matched = [
        row for row in rows if row["forge_resolved"] == row["raw_claude_resolved"]
    ]
    both_resolved = [
        row for row in rows if row["forge_resolved"] and row["raw_claude_resolved"]
    ]
    report = {
        "schema_version": 1,
        "generated_at": bench.utc_now(),
        "runs": run_metadata,
        "forge_commit": manifests[0].get("forge_commit"),
        "forge_binary_sha256": manifests[0].get("forge_binary_sha256"),
        "claude_version": manifests[0].get("claude_version"),
        "claude_runtime": manifests[0].get("claude_runtime"),
        "reasoning_effort": manifests[0].get("reasoning_effort"),
        "summary": {
            "all_pairs": summarize_rows(rows),
            "quality_matched_pairs": summarize_rows(quality_matched),
            "both_resolved_pairs": summarize_rows(both_resolved),
        },
        "by_model": grouped_summaries(rows, "model"),
        "by_difficulty": grouped_summaries(rows, "difficulty"),
        "pairs": rows,
    }

    args.out_json.parent.mkdir(parents=True, exist_ok=True)
    args.out_markdown.parent.mkdir(parents=True, exist_ok=True)
    bench.json_dump(args.out_json, report)
    args.out_markdown.write_text(render_markdown(report), encoding="utf-8")
    print(json.dumps(report["summary"], indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
