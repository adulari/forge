#!/usr/bin/env python3
"""Run Forge's unpinned auto mesh against published native SWE-bench cells."""

from __future__ import annotations

import argparse
import dataclasses
import hashlib
import json
import os
import random
import sqlite3
import subprocess
import tomllib
from collections import defaultdict
from pathlib import Path
from typing import Any, Sequence

import compare_codex_oauth as bench
import compare_codex_oauth_swe as swe

FAMILIES = ("claude", "codex")
DIFFICULTY_RANK = {
    "<15 min fix": 0,
    "15 min - 1 hour": 1,
    "1-4 hours": 2,
}


@dataclasses.dataclass(frozen=True)
class MeshTrial:
    index: int
    instance_id: str
    baselines: tuple[dict[str, Any], ...]

    @property
    def cell_id(self) -> str:
        return f"mesh-auto::{self.instance_id}"

    @property
    def slug(self) -> str:
        safe_instance = self.instance_id.replace("/", "_")
        return f"{self.index:03d}__{safe_instance}__forge-mesh-auto"


def load_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def baseline_cells(
    codex_analysis: Path,
    claude_analysis: Path,
) -> list[dict[str, Any]]:
    cells: list[dict[str, Any]] = []
    specs = (
        (
            "codex",
            codex_analysis,
            "raw_codex_resolved",
            "raw_codex_wall_seconds",
            "raw_codex_total_tokens",
            None,
        ),
        (
            "claude",
            claude_analysis,
            "raw_claude_resolved",
            "raw_claude_wall_seconds",
            "raw_claude_total_tokens",
            "raw_claude_cache_adjusted_tokens_025",
        ),
    )
    for family, path, resolved_key, wall_key, token_key, adjusted_key in specs:
        report = load_json(path)
        rows = report.get("pairs")
        if not isinstance(rows, list) or not rows:
            raise ValueError(f"{path}: missing non-empty pairs")
        for row in rows:
            required = {"model", "instance_id", resolved_key, wall_key, token_key}
            missing = required - set(row)
            if missing:
                raise ValueError(f"{path}: baseline row missing {sorted(missing)}")
            baseline = {
                "resolved": bool(row[resolved_key]),
                "wall_seconds": float(row[wall_key]),
                "total_tokens": int(row[token_key]),
                "cache_adjusted_tokens_025": (
                    float(row[adjusted_key]) if adjusted_key else None
                ),
                "source_analysis": str(path.resolve()),
                "source_pair_id": row.get("pair_id"),
            }
            cells.append(
                {
                    "family": family,
                    "comparator_model": str(row["model"]),
                    "instance_id": str(row["instance_id"]),
                    "dataset_index": row.get("dataset_index"),
                    "difficulty": row.get("difficulty"),
                    "repo": row.get("repo"),
                    "baseline": baseline,
                }
            )

    expected_counts = {"codex": 18, "claude": 6}
    counts = {
        family: sum(cell["family"] == family for cell in cells) for family in FAMILIES
    }
    if counts != expected_counts:
        raise ValueError(
            f"unexpected baseline cell counts: {counts}, expected {expected_counts}"
        )
    ids = {
        (cell["family"], cell["comparator_model"], cell["instance_id"])
        for cell in cells
    }
    if len(ids) != len(cells):
        raise ValueError("duplicate baseline family/model/instance cell")
    return cells


def plan_trials(cells: Sequence[dict[str, Any]], seed: int) -> list[MeshTrial]:
    by_instance: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for cell in cells:
        by_instance[cell["instance_id"]].append(cell)
    ordered_instances = list(by_instance)
    # A quota-limited prefix should cover both native families before Codex-only
    # cells, then increase task difficulty so one expensive arm cannot erase all
    # easier cross-family evidence. Seeded shuffling is the tie-breaker.
    random.Random(seed).shuffle(ordered_instances)
    ordered_instances.sort(
        key=lambda instance_id: (
            -len({cell["family"] for cell in by_instance[instance_id]}),
            DIFFICULTY_RANK.get(
                str(by_instance[instance_id][0].get("difficulty")),
                len(DIFFICULTY_RANK),
            ),
        )
    )
    return [
        MeshTrial(
            index=index,
            instance_id=instance_id,
            baselines=tuple(
                sorted(
                    by_instance[instance_id],
                    key=lambda cell: (cell["family"], cell["comparator_model"]),
                )
            ),
        )
        for index, instance_id in enumerate(ordered_instances, start=1)
    ]


def load_summaries(path: Path) -> list[dict[str, Any]]:
    if not path.exists():
        return []
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


def mesh_argv(forge_bin: Path, prompt: str) -> list[str]:
    # Deliberately no --model and no effort override: this is the regular auto mesh.
    return [
        str(forge_bin),
        "run",
        "--mode",
        "bypass",
        "--output-format",
        "stream-json",
        prompt,
    ]


def mesh_environment(forge_db: Path, tmp_dir: Path) -> dict[str, str]:
    environment = os.environ.copy()
    environment["FORGE_DB"] = str(forge_db)
    environment["TMPDIR"] = str(tmp_dir)
    for key in tuple(environment):
        if key.startswith("FORGE_MESH__"):
            del environment[key]
    return environment


def selected_model(path: Path) -> str | None:
    events, _ = bench.parse_jsonl(path)
    for event in events:
        if event.get("type") == "system" and event.get("subtype") == "routing":
            model = event.get("model")
            return str(model) if model else None
    return None


def provider_from_model(model: str | None) -> str | None:
    if model and "::" in model:
        return model.split("::", 1)[0]
    return None


def route_usage(db_path: Path, session_id: str | None) -> list[dict[str, Any]]:
    if not session_id or not db_path.exists():
        return []
    with sqlite3.connect(db_path) as connection:
        rows = connection.execute(
            """
            WITH RECURSIVE session_tree(id) AS (
                SELECT id FROM session WHERE id = ?
                UNION ALL
                SELECT s.id
                FROM session s
                JOIN session_tree parent ON s.parent_session_id = parent.id
            )
            SELECT
                COALESCE(u.provider, ''),
                COALESCE(u.model, m.model, ''),
                COUNT(*),
                COALESCE(SUM(u.input_tokens), 0),
                COALESCE(SUM(u.cached_input_tokens), 0),
                COALESCE(SUM(u.output_tokens), 0)
            FROM usage u
            JOIN message m ON m.id = u.message_id
            WHERE m.session_id IN (SELECT id FROM session_tree)
            GROUP BY COALESCE(u.provider, ''), COALESCE(u.model, m.model, '')
            ORDER BY COALESCE(u.provider, ''), COALESCE(u.model, m.model, '')
            """,
            (session_id,),
        ).fetchall()
    return [
        {
            "provider": row[0] or provider_from_model(row[1]),
            "model": row[1] or None,
            "provider_calls": int(row[2]),
            "input_tokens": int(row[3]),
            "cached_input_tokens": int(row[4]),
            "output_tokens": int(row[5]),
            "total_tokens": int(row[3]) + int(row[5]),
            "cache_adjusted_tokens_025": round(
                int(row[3]) - 0.75 * int(row[4]) + int(row[5]),
                2,
            ),
        }
        for row in rows
    ]


def latest_subscription_quotas(db_path: Path) -> dict[str, dict[str, Any] | None]:
    result: dict[str, dict[str, Any] | None] = {"claude": None, "codex": None}
    if not db_path.exists():
        return result
    aliases = {
        "claude": ("claude-cli",),
        "codex": ("codex-oauth", "codex-cli"),
    }
    with sqlite3.connect(db_path) as connection:
        for family, providers in aliases.items():
            placeholders = ",".join("?" for _ in providers)
            row = connection.execute(
                f"""
                SELECT provider, fraction_used, resets_at, observed_at
                FROM quota_history
                WHERE provider IN ({placeholders}) AND window_kind = 'weekly'
                ORDER BY observed_at DESC, id DESC
                LIMIT 1
                """,
                providers,
            ).fetchone()
            if row is not None and row[1] is not None:
                result[family] = {
                    "source": "forge-quota-history",
                    "provider": row[0],
                    "used_percent": round(float(row[1]) * 100.0, 3),
                    "window": "weekly",
                    "resets_at": row[2],
                    "observed_at": row[3],
                }
    return result


def capture_mesh_patch(workspace: Path, trial_dir: Path) -> dict[str, Any]:
    excludes = (
        ":(exclude).forge/checkpoints/**",
        ":(exclude).forge/worktrees/**",
        ":(exclude).forge/forge.log",
    )
    subprocess.run(
        ("git", "add", "-N", "--", ".", *excludes),
        cwd=workspace,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=False,
        timeout=30,
    )
    patch = subprocess.run(
        ("git", "diff", "--binary", "--no-ext-diff", "HEAD", "--", ".", *excludes),
        cwd=workspace,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        timeout=30,
        check=False,
    )
    (trial_dir / "changes.patch").write_bytes(patch.stdout)
    (trial_dir / "changes.patch.stderr.log").write_bytes(patch.stderr)
    status = bench.tiny_command(
        ("git", "status", "--short", "--", ".", *excludes),
        cwd=workspace,
    )
    diffstat = bench.tiny_command(
        ("git", "diff", "--stat", "HEAD", "--", ".", *excludes),
        cwd=workspace,
    )
    return {
        "patch_bytes": len(patch.stdout),
        "patch_sha256": hashlib.sha256(patch.stdout).hexdigest(),
        "status": status.splitlines(),
        "diffstat": diffstat,
    }


def run_trial(
    trial: MeshTrial,
    *,
    instance: dict[str, Any],
    dataset_index: int,
    out_root: Path,
    worktree_root: Path,
    forge_bin: Path,
    forge_db: Path,
    timeout_seconds: int,
) -> dict[str, Any]:
    trial_dir = out_root / "trials" / trial.slug
    trial_dir.mkdir(parents=True, exist_ok=False)
    tmp_dir = trial_dir / "tmp"
    tmp_dir.mkdir()

    # One clone root per cell prevents auto-orchestrated child worktrees or build
    # scratch from leaking into another comparator cell.
    workspace = swe.prepare_repo(instance, worktree_root / trial.slug)
    prompt = str(instance["problem_statement"])
    (trial_dir / "prompt.txt").write_text(prompt, encoding="utf-8")
    manifest = {
        "trial": dataclasses.asdict(trial),
        "cell_id": trial.cell_id,
        "dataset_index": dataset_index,
        "repo": instance["repo"],
        "base_commit": instance["base_commit"],
        "difficulty": instance.get("difficulty"),
        "problem_statement_sha256": hashlib.sha256(prompt.encode()).hexdigest(),
        "workspace": str(workspace),
        "tmp_dir": str(tmp_dir),
        "workspace_head_before": bench.tiny_command(
            ("git", "rev-parse", "HEAD"), cwd=workspace
        ),
        "argv": mesh_argv(forge_bin, prompt),
    }
    bench.json_dump(trial_dir / "manifest.json", manifest)

    command_result = bench.run_capture(
        manifest["argv"],
        cwd=workspace,
        env=mesh_environment(forge_db, tmp_dir),
        stdout_path=trial_dir / "events.jsonl",
        stderr_path=trial_dir / "stderr.log",
        timeout_seconds=timeout_seconds,
    )
    bench.json_dump(trial_dir / "process.json", dataclasses.asdict(command_result))
    events_path = trial_dir / "events.jsonl"
    agent = bench.summarize_forge_events(events_path)
    complete = bench.forge_session_tokens(forge_db, agent.get("session_id"))
    if complete is not None:
        agent["stream_tokens"] = agent["tokens"]
        stream_total = agent["stream_tokens"].get("total_tokens")
        agent["tokens"] = complete
        agent["post_stream_tokens"] = (
            complete["total_tokens"] - stream_total
            if isinstance(stream_total, int)
            else None
        )
    agent["selected_model"] = selected_model(events_path)
    agent["route_usage"] = route_usage(forge_db, agent.get("session_id"))

    patch = capture_mesh_patch(workspace, trial_dir)
    patch_text = (trial_dir / "changes.patch").read_text(
        encoding="utf-8",
        errors="replace",
    )
    summary = {
        "trial": {
            "index": trial.index,
            "instance_id": trial.instance_id,
            "arm": "forge-mesh-auto",
        },
        "cell_id": trial.cell_id,
        "baseline_comparators": trial.baselines,
        "dataset_index": dataset_index,
        "repo": instance["repo"],
        "base_commit": instance["base_commit"],
        "difficulty": instance.get("difficulty"),
        "process": dataclasses.asdict(command_result),
        "process_ok": command_result.exit_code == 0 and not command_result.timed_out,
        "agent": agent,
        "patched": bool(patch_text.strip()),
        "patch": patch,
        "quota": latest_subscription_quotas(forge_db),
        "official_resolution": None,
        "manifest": str(trial_dir / "manifest.json"),
        "trial_dir": str(trial_dir),
    }
    bench.json_dump(trial_dir / "summary.json", summary)
    return summary


def write_predictions(rows: Sequence[dict[str, Any]], out_root: Path) -> None:
    predictions_dir = out_root / "predictions"
    predictions_dir.mkdir(parents=True, exist_ok=True)
    path = predictions_dir / "forge-mesh-auto.jsonl"
    with path.open("w", encoding="utf-8") as handle:
        for summary in sorted(rows, key=lambda item: item["dataset_index"]):
            prediction = {
                "instance_id": summary["trial"]["instance_id"],
                "model_name_or_path": "forge-mesh-auto",
                "model_patch": (Path(summary["trial_dir"]) / "changes.patch").read_text(
                    encoding="utf-8", errors="replace"
                ),
            }
            handle.write(json.dumps(prediction, sort_keys=True) + "\n")


def used_percent(quota: dict[str, Any] | None) -> float | None:
    if not quota:
        return None
    value = quota.get("used_percent")
    return float(value) if isinstance(value, (int, float)) else None


def effective_quota(
    summaries: Sequence[dict[str, Any]],
    family: str,
    observed_pct: float,
) -> float:
    values = [observed_pct]
    for summary in summaries:
        value = used_percent((summary.get("quota") or {}).get(family))
        if value is not None:
            values.append(value)
    return max(values)


def mesh_config_snapshot(path: Path) -> dict[str, Any]:
    if not path.is_file():
        raise ValueError(f"Forge config not found: {path}")
    config = tomllib.loads(path.read_text(encoding="utf-8"))
    mesh = config.get("mesh") or {}
    if not isinstance(mesh, dict):
        raise ValueError(f"{path}: [mesh] must be a table")
    return {
        "config_path": str(path.resolve()),
        "config_sha256": bench.sha256_file(path),
        "effective": {
            "auto_discover": mesh.get("auto_discover", True),
            "auto_orchestrate": mesh.get("auto_orchestrate", False),
            "credit_mode": mesh.get("credit_mode", "normal"),
            "default_effort": mesh.get("default_effort"),
            "max_output_tokens": mesh.get("max_output_tokens", 0),
            "failover": mesh.get("failover", True),
        },
    }


def validate_resume(manifest: dict[str, Any], expected: dict[str, Any]) -> None:
    mismatches = {
        key: {"stored": manifest.get(key), "requested": value}
        for key, value in expected.items()
        if manifest.get(key) != value
    }
    if mismatches:
        raise ValueError(
            f"resume manifest mismatch: {json.dumps(mismatches, sort_keys=True)}"
        )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dataset", required=True, type=Path)
    parser.add_argument("--codex-analysis", required=True, type=Path)
    parser.add_argument("--claude-analysis", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("--worktree-root", required=True, type=Path)
    parser.add_argument(
        "--forge-bin",
        type=Path,
        default=bench.REPO_ROOT / "target" / "release" / "forge",
    )
    parser.add_argument(
        "--forge-config",
        type=Path,
        default=Path.home() / ".config" / "forge" / "config.toml",
    )
    parser.add_argument("--seed", type=int, default=20260727)
    parser.add_argument("--timeout-seconds", type=int, default=1800)
    parser.add_argument("--claude-baseline-weekly-pct", required=True, type=float)
    parser.add_argument("--claude-max-weekly-increase-pct", required=True, type=float)
    parser.add_argument("--observed-claude-weekly-pct", required=True, type=float)
    parser.add_argument("--codex-baseline-weekly-pct", required=True, type=float)
    parser.add_argument("--codex-max-weekly-increase-pct", required=True, type=float)
    parser.add_argument("--observed-codex-weekly-pct", required=True, type=float)
    parser.add_argument("--max-new-trials", type=int, default=1)
    parser.add_argument("--resume", action="store_true")
    args = parser.parse_args()

    if args.max_new_trials != 1:
        parser.error(
            "--max-new-trials must be 1 so external quotas refresh after every arm"
        )
    if not args.forge_bin.is_file():
        parser.error(f"Forge binary does not exist: {args.forge_bin}")
    if args.timeout_seconds <= 0:
        parser.error("--timeout-seconds must be positive")

    try:
        instances = swe.load_dataset(args.dataset)
        cells = baseline_cells(args.codex_analysis, args.claude_analysis)
        mesh_config = mesh_config_snapshot(args.forge_config)
    except (
        OSError,
        ValueError,
        json.JSONDecodeError,
        tomllib.TOMLDecodeError,
    ) as error:
        parser.error(str(error))
    by_id = {str(instance["instance_id"]): instance for instance in instances}
    dataset_indexes = {
        str(instance["instance_id"]): index for index, instance in enumerate(instances)
    }
    missing_instances = sorted({cell["instance_id"] for cell in cells} - set(by_id))
    if missing_instances:
        parser.error(f"dataset missing baseline instances: {missing_instances}")

    trials = plan_trials(cells, args.seed)
    claude_stop = args.claude_baseline_weekly_pct + args.claude_max_weekly_increase_pct
    codex_stop = args.codex_baseline_weekly_pct + args.codex_max_weekly_increase_pct
    forge_bin = args.forge_bin.resolve()
    expected_manifest = {
        "dataset": str(args.dataset.resolve()),
        "dataset_sha256": bench.sha256_file(args.dataset),
        "codex_analysis": str(args.codex_analysis.resolve()),
        "codex_analysis_sha256": bench.sha256_file(args.codex_analysis),
        "claude_analysis": str(args.claude_analysis.resolve()),
        "claude_analysis_sha256": bench.sha256_file(args.claude_analysis),
        "forge_binary_sha256": bench.sha256_file(forge_bin),
        "forge_config": mesh_config,
        "seed": args.seed,
        "timeout_seconds": args.timeout_seconds,
        "claude_baseline_weekly_pct": args.claude_baseline_weekly_pct,
        "claude_max_weekly_increase_pct": args.claude_max_weekly_increase_pct,
        "claude_hard_stop_weekly_pct": claude_stop,
        "codex_baseline_weekly_pct": args.codex_baseline_weekly_pct,
        "codex_max_weekly_increase_pct": args.codex_max_weekly_increase_pct,
        "codex_hard_stop_weekly_pct": codex_stop,
    }

    args.out.mkdir(parents=True, exist_ok=True)
    forge_db = args.out / "forge-benchmark.db"
    manifest_path = args.out / "suite-manifest.json"
    index_path = args.out / "trial-index.jsonl"
    if manifest_path.exists():
        if not args.resume:
            parser.error("output already has suite-manifest.json; pass --resume")
        manifest = load_json(manifest_path)
        try:
            validate_resume(manifest, expected_manifest)
        except ValueError as error:
            parser.error(str(error))
    else:
        if args.resume:
            parser.error("cannot --resume without suite-manifest.json")
        if any(args.out.iterdir()):
            parser.error(f"output directory is not empty: {args.out}")
        suite_manifest = {
            "schema_version": 1,
            "created_at": bench.utc_now(),
            **expected_manifest,
            "forge_commit": bench.tiny_command(("git", "rev-parse", "HEAD")),
            "forge_dirty": bool(bench.tiny_command(("git", "status", "--porcelain"))),
            "forge_version": bench.tiny_command((str(forge_bin), "--version")),
            "mode": "regular-full-mesh-auto",
            "model_pin": None,
            "effort_override": None,
            "cells": cells,
            "trial_order": [
                {
                    "index": trial.index,
                    "cell_id": trial.cell_id,
                    "instance_id": trial.instance_id,
                    "comparator_cells": len(trial.baselines),
                    "comparator_families": sorted(
                        {cell["family"] for cell in trial.baselines}
                    ),
                }
                for trial in trials
            ],
        }
        bench.json_dump(manifest_path, suite_manifest)

    summaries = load_summaries(index_path)
    completed = [summary["cell_id"] for summary in summaries]
    planned_prefix = [trial.cell_id for trial in trials[: len(completed)]]
    if completed != planned_prefix:
        parser.error("completed trial order does not match suite manifest")

    quota_check = {
        "observed_at": bench.utc_now(),
        "source": "helm-refreshed",
        "claude_used_percent": args.observed_claude_weekly_pct,
        "claude_hard_stop_weekly_pct": claude_stop,
        "codex_used_percent": args.observed_codex_weekly_pct,
        "codex_hard_stop_weekly_pct": codex_stop,
    }
    with (args.out / "quota-checks.jsonl").open("a", encoding="utf-8") as handle:
        handle.write(json.dumps(quota_check, sort_keys=True) + "\n")

    claude_pct = effective_quota(
        summaries,
        "claude",
        args.observed_claude_weekly_pct,
    )
    codex_pct = effective_quota(summaries, "codex", args.observed_codex_weekly_pct)
    if claude_pct >= claude_stop:
        parser.error(
            f"Claude hard stop reached before next arm: {claude_pct:.3f}% >= "
            f"{claude_stop:.3f}%"
        )
    if codex_pct >= codex_stop:
        parser.error(
            f"Codex hard stop reached before next arm: {codex_pct:.3f}% >= "
            f"{codex_stop:.3f}%"
        )

    remaining = trials[len(summaries) :]
    new_trials = 0
    stop_reason: str | None = None
    if remaining:
        trial = remaining[0]
        print(
            json.dumps(
                {
                    "event": "trial_start",
                    "slug": trial.slug,
                    "cell_id": trial.cell_id,
                    "claude_prior_pct": claude_pct,
                    "claude_stop_pct": claude_stop,
                    "codex_prior_pct": codex_pct,
                    "codex_stop_pct": codex_stop,
                }
            ),
            flush=True,
        )
        summary = run_trial(
            trial,
            instance=by_id[trial.instance_id],
            dataset_index=dataset_indexes[trial.instance_id],
            out_root=args.out,
            worktree_root=args.worktree_root,
            forge_bin=forge_bin,
            forge_db=forge_db,
            timeout_seconds=args.timeout_seconds,
        )
        with index_path.open("a", encoding="utf-8") as handle:
            handle.write(json.dumps(summary, sort_keys=True) + "\n")
        summaries.append(summary)
        write_predictions(summaries, args.out)
        new_trials = 1
        claude_pct = effective_quota(
            summaries,
            "claude",
            args.observed_claude_weekly_pct,
        )
        codex_pct = effective_quota(
            summaries,
            "codex",
            args.observed_codex_weekly_pct,
        )
        print(
            json.dumps(
                {
                    "event": "trial_complete",
                    "slug": trial.slug,
                    "process_ok": summary["process_ok"],
                    "patched": summary["patched"],
                    "selected_model": summary["agent"].get("selected_model"),
                    "route_usage": summary["agent"].get("route_usage"),
                    "tokens": summary["agent"].get("tokens"),
                    "quota": summary.get("quota"),
                }
            ),
            flush=True,
        )
        if claude_pct >= claude_stop:
            stop_reason = (
                f"Claude hard stop reached after {trial.slug}: "
                f"{claude_pct:.3f}% >= {claude_stop:.3f}%"
            )
        elif codex_pct >= codex_stop:
            stop_reason = (
                f"Codex hard stop reached after {trial.slug}: "
                f"{codex_pct:.3f}% >= {codex_stop:.3f}%"
            )
        elif len(summaries) < len(trials):
            stop_reason = (
                f"external quota refresh required after {trial.slug}; "
                "failing closed before another provider call"
            )

    complete = len(summaries) == len(trials)
    run_final = {
        "schema_version": 1,
        "updated_at": bench.utc_now(),
        "planned_trials": len(trials),
        "completed_trials": len(summaries),
        "new_trials_this_segment": new_trials,
        "patched_trials": sum(summary.get("patched", False) for summary in summaries),
        "complete": complete,
        "stop_reason": stop_reason,
        "last_effective_claude_weekly_pct": claude_pct,
        "last_effective_codex_weekly_pct": codex_pct,
        "official_evaluation_required": True,
    }
    bench.json_dump(args.out / "run-final.json", run_final)
    print(json.dumps({"event": "run_complete", **run_final}), flush=True)
    return 0 if complete else 2


if __name__ == "__main__":
    raise SystemExit(main())
