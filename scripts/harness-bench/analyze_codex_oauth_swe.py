#!/usr/bin/env python3
"""Aggregate matched Codex OAuth trials with official SWE-bench reports."""

from __future__ import annotations

import argparse
import json
import statistics
from collections import defaultdict
from pathlib import Path
from typing import Any, Iterable

import compare_codex_oauth as bench
from rescore_codex_oauth import bootstrap_median_ci, exact_two_sided_sign_test


ARMS = ("forge", "raw-codex")


def load_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def load_summaries(path: Path) -> list[dict[str, Any]]:
    summaries: list[dict[str, Any]] = []
    with path.open(encoding="utf-8") as handle:
        for line_number, line in enumerate(handle, 1):
            if not line.strip():
                continue
            try:
                summary = json.loads(line)
            except json.JSONDecodeError as error:
                raise ValueError(f"{path}:{line_number}: invalid JSON: {error}") from error
            summaries.append(summary)
    return summaries


def median(values: Iterable[float]) -> float:
    return round(float(statistics.median(values)), 4)


def find_official_reports(
    evaluation_dir: Path,
    combinations: set[tuple[str, str]],
) -> dict[tuple[str, str], dict[str, Any]]:
    reports: dict[tuple[str, str], dict[str, Any]] = {}
    for arm, model in sorted(combinations):
        matches = sorted(evaluation_dir.glob(f"{arm}::{model}.*.json"))
        if len(matches) != 1:
            raise ValueError(
                f"expected one official report for {arm}/{model}, found {len(matches)}"
            )
        report = load_json(matches[0])
        report["_path"] = str(matches[0].resolve())
        reports[(arm, model)] = report
    return reports


def official_outcomes(
    reports: dict[tuple[str, str], dict[str, Any]],
) -> dict[tuple[str, str, str], bool]:
    outcomes: dict[tuple[str, str, str], bool] = {}
    for (arm, model), report in reports.items():
        errors = report.get("error_ids") or []
        if errors:
            raise ValueError(f"official evaluator errors for {arm}/{model}: {errors}")
        submitted = set(report.get("submitted_ids") or [])
        resolved = set(report.get("resolved_ids") or [])
        unresolved = set(report.get("unresolved_ids") or [])
        if resolved | unresolved != submitted:
            raise ValueError(f"incomplete official outcomes for {arm}/{model}")
        for instance_id in sorted(submitted):
            outcomes[(arm, model, instance_id)] = instance_id in resolved
    return outcomes


def paired_rows(
    summaries: list[dict[str, Any]],
    outcomes: dict[tuple[str, str, str], bool],
) -> list[dict[str, Any]]:
    grouped: dict[str, dict[str, dict[str, Any]]] = defaultdict(dict)
    for summary in summaries:
        trial = summary["trial"]
        grouped[summary["pair_id"]][trial["arm"]] = summary

    rows: list[dict[str, Any]] = []
    for pair_id, arms in grouped.items():
        if set(arms) != set(ARMS):
            raise ValueError(f"incomplete matched pair {pair_id}: {sorted(arms)}")
        forge = arms["forge"]
        raw = arms["raw-codex"]
        model = str(forge["trial"]["model"])
        instance_id = str(forge["trial"]["instance_id"])
        if raw["trial"]["model"] != model or raw["trial"]["instance_id"] != instance_id:
            raise ValueError(f"mismatched pair metadata for {pair_id}")
        forge_tokens = forge["agent"]["tokens"].get("total_tokens")
        raw_tokens = raw["agent"]["tokens"].get("total_tokens")
        if not isinstance(forge_tokens, int) or not isinstance(raw_tokens, int):
            raise ValueError(f"incomplete token telemetry for {pair_id}")
        forge_wall = forge["process"].get("wall_seconds")
        raw_wall = raw["process"].get("wall_seconds")
        if not isinstance(forge_wall, (int, float)) or not isinstance(
            raw_wall, (int, float)
        ):
            raise ValueError(f"incomplete wall telemetry for {pair_id}")
        forge_key = ("forge", model, instance_id)
        raw_key = ("raw-codex", model, instance_id)
        if forge_key not in outcomes or raw_key not in outcomes:
            raise ValueError(f"missing official outcome for {pair_id}")
        rows.append(
            {
                "pair_id": pair_id,
                "dataset_index": forge["dataset_index"],
                "model": model,
                "instance_id": instance_id,
                "repo": forge["repo"],
                "difficulty": forge.get("difficulty"),
                "forge_resolved": outcomes[forge_key],
                "raw_codex_resolved": outcomes[raw_key],
                "forge_total_tokens": forge_tokens,
                "raw_codex_total_tokens": raw_tokens,
                "forge_token_reduction_vs_raw": round(
                    (raw_tokens - forge_tokens) / raw_tokens,
                    4,
                ),
                "forge_wall_seconds": round(float(forge_wall), 3),
                "raw_codex_wall_seconds": round(float(raw_wall), 3),
                "forge_wall_reduction_vs_raw": round(
                    (float(raw_wall) - float(forge_wall)) / float(raw_wall),
                    4,
                ),
                "forge_provider_calls": forge["agent"]["tokens"].get("provider_calls"),
                "raw_codex_provider_calls": raw["agent"]["tokens"].get(
                    "provider_calls"
                ),
            }
        )
    return sorted(rows, key=lambda row: (row["model"], row["dataset_index"]))


def summarize_rows(rows: list[dict[str, Any]]) -> dict[str, Any]:
    if not rows:
        raise ValueError("no complete matched pairs")
    forge_resolved = sum(row["forge_resolved"] for row in rows)
    raw_resolved = sum(row["raw_codex_resolved"] for row in rows)
    forge_only = sum(
        row["forge_resolved"] and not row["raw_codex_resolved"] for row in rows
    )
    raw_only = sum(
        row["raw_codex_resolved"] and not row["forge_resolved"] for row in rows
    )
    token_reductions = [row["forge_token_reduction_vs_raw"] for row in rows]
    token_wins = sum(value > 0 for value in token_reductions)
    token_losses = sum(value < 0 for value in token_reductions)
    wall_reductions = [row["forge_wall_reduction_vs_raw"] for row in rows]
    wall_wins = sum(value > 0 for value in wall_reductions)
    wall_losses = sum(value < 0 for value in wall_reductions)
    forge_tokens = sum(row["forge_total_tokens"] for row in rows)
    raw_tokens = sum(row["raw_codex_total_tokens"] for row in rows)
    forge_wall = sum(row["forge_wall_seconds"] for row in rows)
    raw_wall = sum(row["raw_codex_wall_seconds"] for row in rows)
    return {
        "complete_pairs": len(rows),
        "quality": {
            "forge_resolved": forge_resolved,
            "raw_codex_resolved": raw_resolved,
            "forge_only_resolved": forge_only,
            "raw_codex_only_resolved": raw_only,
            "both_resolved": sum(
                row["forge_resolved"] and row["raw_codex_resolved"] for row in rows
            ),
            "neither_resolved": sum(
                not row["forge_resolved"] and not row["raw_codex_resolved"]
                for row in rows
            ),
            "two_sided_exact_mcnemar_p": exact_two_sided_sign_test(
                forge_only, raw_only
            ),
        },
        "tokens": {
            "forge_total": forge_tokens,
            "raw_codex_total": raw_tokens,
            "weighted_forge_reduction": round(
                (raw_tokens - forge_tokens) / raw_tokens,
                4,
            ),
            "forge_tokens_per_resolved": (
                round(forge_tokens / forge_resolved, 2) if forge_resolved else None
            ),
            "raw_codex_tokens_per_resolved": (
                round(raw_tokens / raw_resolved, 2) if raw_resolved else None
            ),
            "median_paired_forge_reduction": median(token_reductions),
            "bootstrap_95pct_ci_for_median_reduction": bootstrap_median_ci(
                token_reductions
            ),
            "forge_lower_pairs": token_wins,
            "raw_codex_lower_pairs": token_losses,
            "ties": len(rows) - token_wins - token_losses,
            "two_sided_exact_sign_test_p": exact_two_sided_sign_test(
                token_wins, token_losses
            ),
        },
        "wall_time": {
            "forge_total_seconds": round(forge_wall, 3),
            "raw_codex_total_seconds": round(raw_wall, 3),
            "weighted_forge_reduction": round(
                (raw_wall - forge_wall) / raw_wall,
                4,
            ),
            "median_paired_forge_reduction": median(wall_reductions),
            "forge_faster_pairs": wall_wins,
            "raw_codex_faster_pairs": wall_losses,
            "ties": len(rows) - wall_wins - wall_losses,
            "two_sided_exact_sign_test_p": exact_two_sided_sign_test(
                wall_wins, wall_losses
            ),
        },
    }


def grouped_summaries(
    rows: list[dict[str, Any]],
    key: str,
) -> list[dict[str, Any]]:
    grouped: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for row in rows:
        grouped[str(row[key])].append(row)
    result: list[dict[str, Any]] = []
    for value, group in sorted(grouped.items()):
        summary = summarize_rows(group)
        result.append({key: value, **summary})
    return result


def render_markdown(report: dict[str, Any]) -> str:
    summary = report["summary"]
    quality = summary["quality"]
    tokens = summary["tokens"]
    wall = summary["wall_time"]
    lines = [
        "# Official SWE-bench matched-run analysis",
        "",
        f"- Complete matched pairs: {summary['complete_pairs']}",
        (
            f"- Resolved: Forge {quality['forge_resolved']}/{summary['complete_pairs']}; "
            f"raw Codex {quality['raw_codex_resolved']}/{summary['complete_pairs']}"
        ),
        (
            f"- Discordant: Forge-only {quality['forge_only_resolved']}; "
            f"raw-only {quality['raw_codex_only_resolved']}; "
            f"exact two-sided McNemar/sign p={quality['two_sided_exact_mcnemar_p']}"
        ),
        (
            f"- Tokens: Forge {tokens['forge_total']:,}; raw Codex "
            f"{tokens['raw_codex_total']:,}; weighted Forge reduction "
            f"{tokens['weighted_forge_reduction']:.2%}"
        ),
        (
            f"- Wall: Forge {wall['forge_total_seconds']:.3f}s; raw Codex "
            f"{wall['raw_codex_total_seconds']:.3f}s; weighted Forge reduction "
            f"{wall['weighted_forge_reduction']:.2%}"
        ),
        "",
        "## By model",
        "",
        "| Model | Forge resolved | Raw resolved | Forge tokens | Raw tokens | Forge token reduction |",
        "|---|---:|---:|---:|---:|---:|",
    ]
    for group in report["by_model"]:
        group_summary = group["tokens"]
        lines.append(
            f"| {group['model']} | {group['quality']['forge_resolved']}/{group['complete_pairs']} "
            f"| {group['quality']['raw_codex_resolved']}/{group['complete_pairs']} "
            f"| {group_summary['forge_total']:,} | {group_summary['raw_codex_total']:,} "
            f"| {group_summary['weighted_forge_reduction']:.2%} |"
        )
    lines.extend(
        [
            "",
            "## Per pair",
            "",
            "| Model | Instance | Forge | Raw | Forge tokens | Raw tokens | Reduction |",
            "|---|---|---:|---:|---:|---:|---:|",
        ]
    )
    for row in report["pairs"]:
        lines.append(
            f"| {row['model']} | {row['instance_id']} | "
            f"{'resolved' if row['forge_resolved'] else 'unresolved'} | "
            f"{'resolved' if row['raw_codex_resolved'] else 'unresolved'} | "
            f"{row['forge_total_tokens']:,} | {row['raw_codex_total_tokens']:,} | "
            f"{row['forge_token_reduction_vs_raw']:.2%} |"
        )
    return "\n".join(lines) + "\n"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run-dir", required=True, type=Path)
    parser.add_argument("--evaluation-dir", required=True, type=Path)
    parser.add_argument("--out-json", type=Path)
    parser.add_argument("--out-markdown", type=Path)
    args = parser.parse_args()

    run_dir = args.run_dir.resolve()
    evaluation_dir = args.evaluation_dir.resolve()
    summaries = load_summaries(run_dir / "trial-index.jsonl")
    combinations = {
        (summary["trial"]["arm"], summary["trial"]["model"]) for summary in summaries
    }
    reports = find_official_reports(evaluation_dir, combinations)
    outcomes = official_outcomes(reports)
    rows = paired_rows(summaries, outcomes)
    run_final = load_json(run_dir / "run-final.json")
    suite_manifest = load_json(run_dir / "suite-manifest.json")
    report = {
        "schema_version": 1,
        "generated_at": bench.utc_now(),
        "run_dir": str(run_dir),
        "evaluation_dir": str(evaluation_dir),
        "dataset": suite_manifest["dataset"],
        "dataset_sha256": suite_manifest["dataset_sha256"],
        "forge_commit": suite_manifest["forge_commit"],
        "forge_binary_sha256": suite_manifest["forge_binary_sha256"],
        "last_quota": run_final.get("last_quota"),
        "official_reports": {
            f"{arm}::{model}": report["_path"]
            for (arm, model), report in sorted(reports.items())
        },
        "pairs": rows,
        "summary": summarize_rows(rows),
        "by_model": grouped_summaries(rows, "model"),
        "by_instance": grouped_summaries(rows, "instance_id"),
    }
    out_json = args.out_json or run_dir / "official-analysis.json"
    out_markdown = args.out_markdown or run_dir / "official-analysis.md"
    bench.json_dump(out_json, report)
    out_markdown.write_text(render_markdown(report), encoding="utf-8")
    print(json.dumps(report["summary"], indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
