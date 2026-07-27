#!/usr/bin/env python3
"""Run matched Forge-vs-Codex trials on an official SWE-bench dataset subset.

This runner preserves the same model prompt, repository commit, reasoning effort, timeout, and
authenticated Codex account across arms. It produces standard prediction JSONL files for the
official SWE-bench Docker evaluator and retains raw events, stderr, patches, quota telemetry, and
per-trial summaries. Correctness is deliberately not guessed here; only the official evaluator
decides whether a patch resolves an instance.
"""

from __future__ import annotations

import argparse
import dataclasses
import hashlib
import json
import os
import random
import shutil
import subprocess
from collections import defaultdict
from pathlib import Path
from typing import Any, Sequence

import compare_codex_oauth as bench


BENCHMARK_BASE_REF = "refs/forge-benchmark/base"
BENCHMARK_ORIGIN_MARKER = "forge-benchmark-origin-base"
BENCHMARK_INTEGRITY_PREAMBLE = """\
Benchmark integrity rules:
- Solve the task from the checked-out repository contents and the problem statement.
- Do not access the network, remote repositories, issue trackers, pull requests, or external search.
- Do not search Git history for a later fix. This repository intentionally contains only a
  synthetic base commit; treat it as the complete available history.
"""


@dataclasses.dataclass(frozen=True)
class SweTrial:
    index: int
    model: str
    instance_id: str
    arm: str

    @property
    def pair_id(self) -> str:
        return f"{self.model}__{self.instance_id}"

    @property
    def slug(self) -> str:
        safe_model = self.model.replace("/", "_")
        safe_instance = self.instance_id.replace("/", "_")
        return f"{self.index:03d}__{safe_model}__{safe_instance}__{self.arm}"


def load_dataset(path: Path) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    for line_number, line in enumerate(
        path.read_text(encoding="utf-8").splitlines(),
        start=1,
    ):
        if not line.strip():
            continue
        value = json.loads(line)
        required = {"instance_id", "repo", "base_commit", "problem_statement"}
        missing = required - set(value)
        if missing:
            raise ValueError(f"{path}:{line_number}: missing {sorted(missing)}")
        rows.append(value)
    if not rows:
        raise ValueError(f"empty SWE-bench dataset: {path}")
    ids = [row["instance_id"] for row in rows]
    if len(ids) != len(set(ids)):
        raise ValueError("SWE-bench subset contains duplicate instance_id values")
    return rows


def plan_trials(
    models: Sequence[str],
    instances: Sequence[dict[str, Any]],
    arms: Sequence[str],
    seed: int,
) -> list[SweTrial]:
    pairs = [
        (model, str(instance["instance_id"]))
        for model in models
        for instance in instances
    ]
    random.Random(seed).shuffle(pairs)
    trials: list[SweTrial] = []
    for pair_number, (model, instance_id) in enumerate(pairs):
        ordered_arms = list(arms)
        if pair_number % 2 == 1:
            ordered_arms.reverse()
        for arm in ordered_arms:
            trials.append(
                SweTrial(
                    index=pair_number + 1,
                    model=model,
                    instance_id=instance_id,
                    arm=arm,
                )
            )
    return trials


def run_git(
    args: Sequence[str],
    *,
    cwd: Path,
    timeout_seconds: int = 300,
) -> subprocess.CompletedProcess[bytes]:
    return subprocess.run(
        ("git", *args),
        cwd=cwd,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        timeout=timeout_seconds,
        check=False,
    )


def require_git_success(
    result: subprocess.CompletedProcess[bytes],
    *,
    operation: str,
) -> None:
    if result.returncode != 0:
        stderr = result.stderr.decode("utf-8", errors="replace").strip()
        raise RuntimeError(f"{operation} failed ({result.returncode}): {stderr}")


def benchmark_prompt(problem_statement: str) -> str:
    return f"{BENCHMARK_INTEGRITY_PREAMBLE}\n{problem_statement}"


def benchmark_base_ref(workspace: Path) -> str:
    exists = run_git(("show-ref", "--verify", "--quiet", BENCHMARK_BASE_REF), cwd=workspace)
    if exists.returncode != 0:
        raise RuntimeError(f"{workspace}: missing isolated benchmark base ref")
    return BENCHMARK_BASE_REF


def trial_forge_db(trial_dir: Path) -> Path:
    """Return a fresh Forge state database path scoped to one benchmark cell."""

    return trial_dir / "forge-benchmark.db"


def isolate_repo_history(workspace: Path, original_base: str) -> None:
    """Replace the upstream object database with one synthetic base-tree commit."""

    original_tree = bench.tiny_command(
        ("git", "rev-parse", f"{original_base}^{{tree}}"),
        cwd=workspace,
    )
    git_dir = workspace / ".git"
    if not git_dir.is_dir():
        raise RuntimeError(f"refusing to replace non-directory Git metadata: {git_dir}")
    shutil.rmtree(git_dir)

    require_git_success(run_git(("init", "--quiet"), cwd=workspace), operation="initializing")
    require_git_success(
        run_git(("config", "user.email", "benchmark@forge.invalid"), cwd=workspace),
        operation="configuring benchmark Git email",
    )
    require_git_success(
        run_git(("config", "user.name", "Forge Benchmark"), cwd=workspace),
        operation="configuring benchmark Git name",
    )
    require_git_success(
        run_git(("add", "--force", "--all"), cwd=workspace),
        operation="staging isolated benchmark base",
    )
    require_git_success(
        run_git(
            ("commit", "--quiet", "--no-gpg-sign", "-m", "benchmark: isolated base"),
            cwd=workspace,
            timeout_seconds=900,
        ),
        operation="committing isolated benchmark base",
    )
    synthetic_base = bench.tiny_command(("git", "rev-parse", "HEAD"), cwd=workspace)
    synthetic_tree = bench.tiny_command(
        ("git", "rev-parse", f"{synthetic_base}^{{tree}}"),
        cwd=workspace,
    )
    if synthetic_tree != original_tree:
        raise RuntimeError(
            f"isolated base tree mismatch for {workspace}: "
            f"{synthetic_tree} != {original_tree}"
        )
    require_git_success(
        run_git(("update-ref", BENCHMARK_BASE_REF, synthetic_base), cwd=workspace),
        operation="recording isolated benchmark base",
    )
    (workspace / ".git" / BENCHMARK_ORIGIN_MARKER).write_text(
        f"{original_base}\n",
        encoding="utf-8",
    )
    require_git_success(
        run_git(("checkout", "--quiet", "--detach", synthetic_base), cwd=workspace),
        operation="detaching isolated benchmark base",
    )


def prepare_repo(instance: dict[str, Any], worktree_root: Path) -> Path:
    """Create a history-isolated base tree, then restore it before every arm."""

    worktree_root.mkdir(parents=True, exist_ok=True)
    root = worktree_root.resolve()
    safe_id = str(instance["instance_id"]).replace("/", "_")
    workspace = (root / safe_id).resolve()
    if workspace.parent != root:
        raise RuntimeError(f"unsafe SWE-bench workspace path: {workspace}")
    if not workspace.exists():
        clone = subprocess.run(
            (
                "git",
                "clone",
                "--filter=blob:none",
                "--no-checkout",
                "--quiet",
                f"https://github.com/{instance['repo']}.git",
                str(workspace),
            ),
            cwd=root,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=900,
            check=False,
        )
        require_git_success(clone, operation=f"cloning {instance['repo']}")
    if not (workspace / ".git").is_dir():
        raise RuntimeError(f"refusing to reset non-Git path: {workspace}")

    base_commit = str(instance["base_commit"])
    marker = workspace / ".git" / BENCHMARK_ORIGIN_MARKER
    if marker.exists():
        recorded_base = marker.read_text(encoding="utf-8").strip()
        if recorded_base != base_commit:
            raise RuntimeError(
                f"isolated base mismatch for {instance['instance_id']}: "
                f"{recorded_base} != {base_commit}"
            )
        reset = run_git(("reset", "--hard", "--quiet", BENCHMARK_BASE_REF), cwd=workspace)
        require_git_success(reset, operation=f"resetting isolated base {base_commit}")
    else:
        exists = run_git(("cat-file", "-e", f"{base_commit}^{{commit}}"), cwd=workspace)
        if exists.returncode != 0:
            fetched = run_git(
                ("fetch", "--quiet", "origin", base_commit),
                cwd=workspace,
                timeout_seconds=900,
            )
            require_git_success(fetched, operation=f"fetching {base_commit}")
        checkout = run_git(("checkout", "--quiet", "--detach", base_commit), cwd=workspace)
        require_git_success(checkout, operation=f"checking out {base_commit}")
        reset = run_git(("reset", "--hard", "--quiet", base_commit), cwd=workspace)
        require_git_success(reset, operation=f"resetting {base_commit}")
        clean = run_git(("clean", "-ffdx", "--quiet"), cwd=workspace)
        require_git_success(clean, operation=f"cleaning {workspace}")
        isolate_repo_history(workspace, base_commit)

    clean = run_git(("clean", "-ffdx", "--quiet"), cwd=workspace)
    require_git_success(clean, operation=f"cleaning isolated workspace {workspace}")
    head = bench.tiny_command(("git", "rev-parse", "HEAD"), cwd=workspace)
    isolated_base = bench.tiny_command(
        ("git", "rev-parse", BENCHMARK_BASE_REF),
        cwd=workspace,
    )
    if head != isolated_base:
        checkout = run_git(
            ("checkout", "--quiet", "--detach", BENCHMARK_BASE_REF),
            cwd=workspace,
        )
        require_git_success(checkout, operation=f"detaching isolated base {base_commit}")
    bench.add_local_git_excludes(
        workspace,
        (".forge/checkpoints/", ".forge/forge.log"),
    )
    reachable = bench.tiny_command(("git", "rev-list", "--all", "--count"), cwd=workspace)
    if reachable != "1":
        raise RuntimeError(
            f"history isolation failed for {instance['instance_id']}: "
            f"{reachable} reachable commits"
        )
    return workspace


def write_predictions(
    out_root: Path,
    summaries: Sequence[dict[str, Any]],
    models: Sequence[str],
    arms: Sequence[str],
) -> None:
    grouped: dict[tuple[str, str], list[dict[str, Any]]] = defaultdict(list)
    for summary in summaries:
        trial = summary["trial"]
        grouped[(trial["model"], trial["arm"])].append(summary)
    predictions_dir = out_root / "predictions"
    predictions_dir.mkdir(parents=True, exist_ok=True)
    for model in models:
        for arm in arms:
            rows = sorted(
                grouped.get((model, arm), []),
                key=lambda summary: summary["dataset_index"],
            )
            path = predictions_dir / f"{arm}__{model}.jsonl"
            with path.open("w", encoding="utf-8") as handle:
                for summary in rows:
                    prediction = {
                        "instance_id": summary["trial"]["instance_id"],
                        "model_name_or_path": f"{arm}::{model}",
                        "model_patch": (
                            Path(summary["trial_dir"]) / "changes.patch"
                        ).read_text(encoding="utf-8", errors="replace"),
                    }
                    handle.write(json.dumps(prediction, sort_keys=True) + "\n")


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
                raise ValueError(f"{path}:{line_number}: invalid JSON: {error}") from error
    return summaries


def effective_quota(
    summaries: Sequence[dict[str, Any]],
    observed_weekly_pct: float,
) -> float:
    values = [observed_weekly_pct]
    for summary in summaries:
        value = bench.quota_used_percent(summary.get("quota"))
        if value is not None:
            values.append(value)
    return max(values)


def validate_resume(manifest: dict[str, Any], expected: dict[str, Any]) -> None:
    for key, value in expected.items():
        if manifest.get(key) != value:
            raise ValueError(
                f"resume manifest mismatch for {key}: {manifest.get(key)!r} != {value!r}"
            )


def run_trial(
    trial: SweTrial,
    *,
    instance: dict[str, Any],
    dataset_index: int,
    out_root: Path,
    worktree_root: Path,
    forge_bin: Path,
    timeout_seconds: int,
    reasoning_effort: str,
) -> dict[str, Any]:
    trial_dir = out_root / "trials" / trial.slug
    trial_dir.mkdir(parents=True, exist_ok=False)
    forge_db = trial_forge_db(trial_dir)
    workspace = prepare_repo(instance, worktree_root)
    prompt = benchmark_prompt(str(instance["problem_statement"]))
    (trial_dir / "prompt.txt").write_text(prompt, encoding="utf-8")
    manifest = {
        "trial": dataclasses.asdict(trial),
        "pair_id": trial.pair_id,
        "dataset_index": dataset_index,
        "repo": instance["repo"],
        "base_commit": instance["base_commit"],
        "difficulty": instance.get("difficulty"),
        "problem_statement_sha256": hashlib.sha256(prompt.encode()).hexdigest(),
        "workspace": str(workspace),
        "workspace_head": bench.tiny_command(("git", "rev-parse", "HEAD"), cwd=workspace),
    }
    bench.json_dump(trial_dir / "manifest.json", manifest)
    argv = (
        bench.forge_argv(forge_bin, trial.model, prompt)
        if trial.arm == "forge"
        else bench.raw_codex_argv(
            trial.model,
            workspace,
            prompt,
            "native",
            reasoning_effort,
        )
    )
    environment = (
        bench.forge_environment(forge_db, reasoning_effort)
        if trial.arm == "forge"
        else os.environ.copy()
    )
    command_result = bench.run_capture(
        argv,
        cwd=workspace,
        stdout_path=trial_dir / "events.jsonl",
        stderr_path=trial_dir / "stderr.log",
        timeout_seconds=timeout_seconds,
        env=environment,
    )
    bench.json_dump(trial_dir / "process.json", dataclasses.asdict(command_result))
    agent = (
        bench.summarize_forge_events(trial_dir / "events.jsonl")
        if trial.arm == "forge"
        else bench.summarize_raw_events(trial_dir / "events.jsonl")
    )
    if trial.arm == "forge":
        complete = bench.forge_session_tokens(forge_db, agent.get("session_id"))
        if complete is not None:
            agent["stream_tokens"] = agent["tokens"]
            agent["tokens"] = complete
            stream_total = agent["stream_tokens"].get("total_tokens")
            ledger_total = complete.get("total_tokens")
            agent["post_stream_tokens"] = (
                ledger_total - stream_total
                if stream_total is not None and ledger_total is not None
                else None
            )
    patch = bench.capture_patch(
        workspace,
        trial_dir,
        base_ref=benchmark_base_ref(workspace),
    )
    patch_text = (trial_dir / "changes.patch").read_text(
        encoding="utf-8",
        errors="replace",
    )
    quota = (
        bench.forge_quota(forge_db)
        if trial.arm == "forge"
        else bench.raw_quota_for_thread(agent.get("session_id"))
    )
    quota_refresh = None
    if bench.quota_used_percent(quota) is None:
        quota, quota_refresh = bench.refresh_quota_with_forge(
            forge_bin,
            forge_db,
            workspace,
            trial_dir,
        )
    process_ok = command_result.exit_code == 0 and not command_result.timed_out
    summary = {
        "trial": dataclasses.asdict(trial),
        "pair_id": trial.pair_id,
        "dataset_index": dataset_index,
        "repo": instance["repo"],
        "base_commit": instance["base_commit"],
        "difficulty": instance.get("difficulty"),
        "process": dataclasses.asdict(command_result),
        "agent": agent,
        "patch": patch,
        "patched": bool(patch_text.strip()),
        "quota": quota,
        "quota_refresh": quota_refresh,
        "process_ok": process_ok,
        "official_resolution": None,
        "manifest": str(trial_dir / "manifest.json"),
        "trial_dir": str(trial_dir),
    }
    bench.json_dump(trial_dir / "summary.json", summary)
    return summary


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dataset", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("--worktree-root", required=True, type=Path)
    parser.add_argument(
        "--forge-bin",
        type=Path,
        default=bench.REPO_ROOT / "target" / "debug" / "forge",
    )
    parser.add_argument("--models", default=",".join(bench.DEFAULT_MODELS))
    parser.add_argument("--arms", default=",".join(bench.DEFAULT_ARMS))
    parser.add_argument("--seed", type=int, default=20260726)
    parser.add_argument("--timeout-seconds", type=int, default=1500)
    parser.add_argument(
        "--reasoning-effort",
        choices=("low", "medium", "high", "xhigh"),
        default="high",
    )
    parser.add_argument("--baseline-weekly-pct", required=True, type=float)
    parser.add_argument("--max-weekly-increase-pct", type=float, default=30.0)
    parser.add_argument("--observed-weekly-pct", required=True, type=float)
    parser.add_argument("--max-new-trials", type=int, default=1)
    parser.add_argument("--resume", action="store_true")
    args = parser.parse_args()

    models = bench.parse_csv(args.models)
    arms = bench.parse_csv(args.arms)
    unknown_models = set(models) - set(bench.DEFAULT_MODELS)
    unknown_arms = set(arms) - set(bench.DEFAULT_ARMS)
    if unknown_models:
        parser.error(f"unsupported models: {sorted(unknown_models)}")
    if unknown_arms:
        parser.error(f"unsupported arms: {sorted(unknown_arms)}")
    if args.max_new_trials != 1:
        parser.error(
            "--max-new-trials must be 1 so external quotas refresh after every arm"
        )
    if not args.forge_bin.is_file():
        parser.error(f"Forge binary does not exist: {args.forge_bin}")

    instances = load_dataset(args.dataset)
    by_id = {str(instance["instance_id"]): instance for instance in instances}
    dataset_indexes = {
        str(instance["instance_id"]): index
        for index, instance in enumerate(instances)
    }
    trials = plan_trials(models, instances, arms, args.seed)
    cap_pct = args.baseline_weekly_pct + args.max_weekly_increase_pct
    args.out.mkdir(parents=True, exist_ok=True)
    expected_manifest = {
        "schema_version": 1,
        "dataset": str(args.dataset.resolve()),
        "dataset_sha256": bench.sha256_file(args.dataset),
        "instances": [
            {
                "instance_id": instance["instance_id"],
                "repo": instance["repo"],
                "base_commit": instance["base_commit"],
                "difficulty": instance.get("difficulty"),
            }
            for instance in instances
        ],
        "forge_commit": bench.tiny_command(("git", "rev-parse", "HEAD")),
        "forge_binary_sha256": bench.sha256_file(args.forge_bin),
        "forge_version": bench.tiny_command((str(args.forge_bin), "--version")),
        "codex_version": bench.tiny_command(("codex", "--version")),
        "models": models,
        "arms": arms,
        "seed": args.seed,
        "reasoning_effort": args.reasoning_effort,
        "raw_profile": "native",
        "timeout_seconds": args.timeout_seconds,
        "baseline_weekly_pct": args.baseline_weekly_pct,
        "max_weekly_increase_pct": args.max_weekly_increase_pct,
        "hard_stop_weekly_pct": cap_pct,
        "trial_order": [dataclasses.asdict(trial) for trial in trials],
    }
    manifest_path = args.out / "suite-manifest.json"
    index_path = args.out / "trial-index.jsonl"
    if args.resume:
        if not manifest_path.is_file():
            parser.error("--resume requires an existing suite-manifest.json")
        try:
            validate_resume(
                json.loads(manifest_path.read_text(encoding="utf-8")),
                expected_manifest,
            )
        except (ValueError, json.JSONDecodeError) as error:
            parser.error(str(error))
    else:
        unexpected = list(args.out.iterdir())
        if unexpected:
            parser.error(f"output directory is not empty: {args.out}")
        bench.json_dump(
            manifest_path,
            {"created_at": bench.utc_now(), **expected_manifest},
        )

    try:
        summaries = load_summaries(index_path)
    except ValueError as error:
        parser.error(str(error))
    completed_order = [
        (
            summary["trial"]["model"],
            summary["trial"]["instance_id"],
            summary["trial"]["arm"],
        )
        for summary in summaries
    ]
    planned_order = [
        (trial.model, trial.instance_id, trial.arm)
        for trial in trials[: len(summaries)]
    ]
    if completed_order != planned_order:
        parser.error("completed trial order does not match suite manifest")
    write_predictions(args.out, summaries, models, arms)

    quota_check = {
        "checked_at": bench.utc_now(),
        "observed_weekly_pct": args.observed_weekly_pct,
        "hard_stop_weekly_pct": cap_pct,
    }
    with (args.out / "quota-checks.jsonl").open("a", encoding="utf-8") as handle:
        handle.write(json.dumps(quota_check, sort_keys=True) + "\n")

    stop_reason: str | None = None
    prior_pct = effective_quota(summaries, args.observed_weekly_pct)
    if prior_pct >= cap_pct:
        stop_reason = (
            f"weekly hard stop reached before next arm: "
            f"{prior_pct:.3f}% >= {cap_pct:.3f}%"
        )
    elif len(summaries) < len(trials):
        trial = trials[len(summaries)]
        print(
            json.dumps(
                {
                    "event": "trial_start",
                    "slug": trial.slug,
                    "pair_id": trial.pair_id,
                    "prior_weekly_pct": prior_pct,
                    "hard_stop_weekly_pct": cap_pct,
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
            forge_bin=args.forge_bin.resolve(),
            timeout_seconds=args.timeout_seconds,
            reasoning_effort=args.reasoning_effort,
        )
        summaries.append(summary)
        with index_path.open("a", encoding="utf-8") as index:
            index.write(json.dumps(summary, sort_keys=True) + "\n")
        write_predictions(args.out, summaries, models, arms)
        current_pct = effective_quota(summaries, args.observed_weekly_pct)
        print(
            json.dumps(
                {
                    "event": "trial_complete",
                    "slug": trial.slug,
                    "process_ok": summary["process_ok"],
                    "patched": summary["patched"],
                    "tokens": summary["agent"]["tokens"],
                    "weekly_pct": current_pct,
                }
            ),
            flush=True,
        )
        if current_pct >= cap_pct:
            stop_reason = (
                f"weekly hard stop reached after {trial.slug}: "
                f"{current_pct:.3f}% >= {cap_pct:.3f}%"
            )
        elif len(summaries) < len(trials):
            stop_reason = (
                f"external quota refresh required after {trial.slug}; "
                "failing closed before another provider call"
            )

    complete = len(summaries) == len(trials)
    final = {
        "schema_version": 1,
        "updated_at": bench.utc_now(),
        "planned_trials": len(trials),
        "completed_trials": len(summaries),
        "patched_trials": sum(summary["patched"] for summary in summaries),
        "complete": complete,
        "stop_reason": stop_reason,
        "last_effective_weekly_pct": effective_quota(
            summaries,
            args.observed_weekly_pct,
        ),
        "official_evaluation_required": True,
    }
    bench.json_dump(args.out / "run-final.json", final)
    print(json.dumps({"event": "run_complete", **final}), flush=True)
    return 0 if complete else 2


if __name__ == "__main__":
    raise SystemExit(main())
