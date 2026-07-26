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
import subprocess
from collections import defaultdict
from pathlib import Path
from typing import Any, Sequence

import compare_codex_oauth as bench


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


def prepare_repo(instance: dict[str, Any], worktree_root: Path) -> Path:
    """Clone once, then restore the exact official base commit before every arm."""

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
    bench.add_local_git_excludes(
        workspace,
        (".forge/checkpoints/", ".forge/forge.log"),
    )
    actual = bench.tiny_command(("git", "rev-parse", "HEAD"), cwd=workspace)
    if actual != base_commit:
        raise RuntimeError(
            f"base commit mismatch for {instance['instance_id']}: {actual} != {base_commit}"
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


def run_trial(
    trial: SweTrial,
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
    workspace = prepare_repo(instance, worktree_root)
    prompt = str(instance["problem_statement"])
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
        else bench.raw_codex_argv(trial.model, workspace, prompt, "native")
    )
    environment = (
        bench.forge_environment(forge_db) if trial.arm == "forge" else os.environ.copy()
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
    patch = bench.capture_patch(workspace, trial_dir)
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
    parser.add_argument("--baseline-weekly-pct", required=True, type=float)
    parser.add_argument("--max-weekly-increase-pct", type=float, default=30.0)
    args = parser.parse_args()

    models = bench.parse_csv(args.models)
    arms = bench.parse_csv(args.arms)
    unknown_models = set(models) - set(bench.DEFAULT_MODELS)
    unknown_arms = set(arms) - set(bench.DEFAULT_ARMS)
    if unknown_models:
        parser.error(f"unsupported models: {sorted(unknown_models)}")
    if unknown_arms:
        parser.error(f"unsupported arms: {sorted(unknown_arms)}")
    if not args.forge_bin.is_file():
        parser.error(f"Forge binary does not exist: {args.forge_bin}")
    if args.out.exists() and any(args.out.iterdir()):
        parser.error(f"output directory must be absent or empty: {args.out}")

    instances = load_dataset(args.dataset)
    by_id = {str(instance["instance_id"]): instance for instance in instances}
    dataset_indexes = {
        str(instance["instance_id"]): index
        for index, instance in enumerate(instances)
    }
    trials = plan_trials(models, instances, arms, args.seed)
    cap_pct = args.baseline_weekly_pct + args.max_weekly_increase_pct
    args.out.mkdir(parents=True, exist_ok=True)
    forge_db = args.out / "forge-benchmark.db"
    suite_manifest = {
        "schema_version": 1,
        "created_at": bench.utc_now(),
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
        "reasoning_effort": "xhigh",
        "raw_profile": "native",
        "timeout_seconds": args.timeout_seconds,
        "baseline_weekly_pct": args.baseline_weekly_pct,
        "max_weekly_increase_pct": args.max_weekly_increase_pct,
        "hard_stop_weekly_pct": cap_pct,
        "trial_order": [dataclasses.asdict(trial) for trial in trials],
    }
    bench.json_dump(args.out / "suite-manifest.json", suite_manifest)

    summaries: list[dict[str, Any]] = []
    stop_reason: str | None = None
    index_path = args.out / "trial-index.jsonl"
    for trial in trials:
        prior_quota = next(
            (
                summary.get("quota")
                for summary in reversed(summaries)
                if summary.get("quota") is not None
            ),
            None,
        )
        prior_pct = bench.quota_used_percent(prior_quota)
        if prior_pct is not None and prior_pct >= cap_pct:
            stop_reason = (
                f"weekly hard stop reached before {trial.slug}: "
                f"{prior_pct:.3f}% >= {cap_pct:.3f}%"
            )
            break
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
            forge_db=forge_db,
            timeout_seconds=args.timeout_seconds,
        )
        summaries.append(summary)
        with index_path.open("a", encoding="utf-8") as index:
            index.write(json.dumps(summary, sort_keys=True) + "\n")
        write_predictions(args.out, summaries, models, arms)
        current_pct = bench.quota_used_percent(summary.get("quota"))
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
        if current_pct is None:
            stop_reason = (
                f"quota telemetry missing after {trial.slug}; failing closed before "
                "another provider call"
            )
            break
        if current_pct >= cap_pct:
            stop_reason = (
                f"weekly hard stop reached after {trial.slug}: "
                f"{current_pct:.3f}% >= {cap_pct:.3f}%"
            )
            break

    final = {
        "schema_version": 1,
        "completed_at": bench.utc_now(),
        "planned_trials": len(trials),
        "completed_trials": len(summaries),
        "patched_trials": sum(summary["patched"] for summary in summaries),
        "stop_reason": stop_reason,
        "last_quota": next(
            (
                summary.get("quota")
                for summary in reversed(summaries)
                if summary.get("quota") is not None
            ),
            None,
        ),
        "official_evaluation_required": True,
    }
    bench.json_dump(args.out / "run-final.json", final)
    print(json.dumps({"event": "run_complete", **final}), flush=True)
    return 0 if stop_reason is None else 2


if __name__ == "__main__":
    raise SystemExit(main())
