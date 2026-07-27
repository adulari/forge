#!/usr/bin/env python3
"""Run quota-gated matched Forge-vs-native-Claude SWE-bench trials.

The two arms receive the same model, high effort, task, repository commit, timeout, and Claude
subscription. Runs are resumable in matched-pair-sized segments so an operator can refresh an
external quota authority between pairs. Raw events, stderr, patches, manifests, Forge Store usage,
and Claude rate-limit telemetry are retained. Only the official SWE-bench evaluator decides
whether a patch resolves an instance.
"""

from __future__ import annotations

import argparse
import dataclasses
import hashlib
import json
import os
import sqlite3
import subprocess
from pathlib import Path
from typing import Any, Sequence

import compare_codex_oauth as bench
import compare_codex_oauth_swe as swe


DEFAULT_MODELS = ("opus[1m]", "sonnet")
DEFAULT_ARMS = ("forge", "raw-claude")
CLAUDE_CAPABILITY_BOOLEAN_FIELDS = (
    "supportsEffort",
    "supportsAdaptiveThinking",
    "supportsAutoMode",
    "supportsFastMode",
)


def claude_runtime() -> dict[str, Any]:
    """Read Claude's sanitized, non-billing initialize response.

    This is the same control request Forge uses for model discovery. The raw response also carries
    account and local-runtime details, so this function deliberately retains only model
    capabilities and the separately queried CLI version.
    """

    request_id = "forge-benchmark-capabilities"
    request = {
        "type": "control_request",
        "request_id": request_id,
        "request": {
            "subtype": "initialize",
            "hooks": {},
            "sdkMcpServers": [],
        },
    }
    result = subprocess.run(
        (
            "claude",
            "-p",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--verbose",
            "--no-session-persistence",
            "--tools",
            "",
            "--setting-sources",
            "",
        ),
        input=json.dumps(request) + "\n",
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        timeout=10,
        check=False,
    )
    if result.returncode != 0:
        raise RuntimeError(
            "Claude initialize control request failed "
            f"({result.returncode}): {result.stderr.strip()}"
        )
    response: dict[str, Any] | None = None
    for line in result.stdout.splitlines():
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        if not isinstance(event, dict):
            continue
        envelope = event.get("response") if isinstance(event, dict) else None
        if (
            event.get("type") == "control_response"
            and isinstance(envelope, dict)
            and envelope.get("request_id") == request_id
            and envelope.get("subtype") == "success"
        ):
            payload = envelope.get("response")
            if isinstance(payload, dict):
                response = payload
                break
    if response is None:
        raise RuntimeError(
            "Claude initialize control response was missing or unsuccessful"
        )
    models: list[dict[str, Any]] = []
    for model in response.get("models", []):
        if not isinstance(model, dict):
            continue
        value = model.get("value")
        resolved = model.get("resolvedModel")
        if not isinstance(value, str) or not value:
            continue
        if not isinstance(resolved, str) or not resolved:
            continue
        effort_levels = model.get("supportedEffortLevels")
        sanitized: dict[str, Any] = {
            "value": value,
            "resolvedModel": resolved,
            "supportedEffortLevels": [
                str(level) for level in effort_levels if isinstance(level, str)
            ]
            if isinstance(effort_levels, list)
            else [],
        }
        display_name = model.get("displayName")
        if isinstance(display_name, str) and display_name:
            sanitized["displayName"] = display_name
        for field in CLAUDE_CAPABILITY_BOOLEAN_FIELDS:
            sanitized[field] = bool(model.get(field, False))
        models.append(sanitized)
    if not models:
        raise RuntimeError("Claude initialize response advertised no usable models")
    return {
        "cliVersion": bench.tiny_command(("claude", "--version")),
        "models": models,
    }


def resolve_claude_model(
    runtime: dict[str, Any],
    requested: str,
) -> dict[str, Any] | None:
    for model in runtime.get("models", []):
        if not isinstance(model, dict):
            continue
        value = str(model.get("value") or "")
        resolved = str(model.get("resolvedModel") or "")
        display_name = str(model.get("displayName") or "")
        if (
            value == requested
            or resolved == requested
            or value.partition("[")[0] == requested
            or display_name.partition(" ")[0].lower() == requested.lower()
        ):
            return model
    return None


def forge_argv(forge_bin: Path, model: str, prompt: str) -> list[str]:
    return [
        str(forge_bin),
        "run",
        "--mode",
        "bypass",
        "--output-format",
        "stream-json",
        "--model",
        f"claude-cli::{model}",
        prompt,
    ]


def forge_environment(forge_db: Path) -> dict[str, str]:
    environment = bench.forge_environment(forge_db)
    environment["FORGE_MESH__DEFAULT_EFFORT"] = "high"
    return environment


def raw_claude_argv(model: str, prompt: str) -> list[str]:
    return [
        "claude",
        "-p",
        "--output-format",
        "stream-json",
        "--verbose",
        "--include-partial-messages",
        "--dangerously-skip-permissions",
        "--effort",
        "high",
        "--model",
        model,
        prompt,
    ]


def claude_token_metrics(
    usage: dict[str, Any] | None,
) -> dict[str, int | float | None]:
    """Normalize Claude's additive cache counters into full processed input.

    Claude reports uncached input, cache reads, and cache creation separately. Unlike current
    Codex telemetry, cache reads are not already included in ``input_tokens``.
    """

    if not usage:
        return {
            "input_tokens": None,
            "uncached_input_tokens": None,
            "cache_read_input_tokens": None,
            "cache_creation_input_tokens": None,
            "cached_input_tokens": None,
            "output_tokens": None,
            "total_tokens": None,
            "cache_adjusted_tokens_025": None,
        }
    uncached = int(usage.get("input_tokens") or usage.get("inputTokens") or 0)
    cache_read = int(
        usage.get("cache_read_input_tokens") or usage.get("cacheReadInputTokens") or 0
    )
    cache_creation = int(
        usage.get("cache_creation_input_tokens")
        or usage.get("cacheCreationInputTokens")
        or 0
    )
    output = int(usage.get("output_tokens") or usage.get("outputTokens") or 0)
    full_input = uncached + cache_read + cache_creation
    return {
        "input_tokens": full_input,
        "uncached_input_tokens": uncached,
        "cache_read_input_tokens": cache_read,
        "cache_creation_input_tokens": cache_creation,
        # Forge's common Usage schema retains cache reads as the cached subset.
        "cached_input_tokens": cache_read,
        "output_tokens": output,
        "total_tokens": full_input + output,
        # Transparent sensitivity metric, not Anthropic's private quota formula.
        "cache_adjusted_tokens_025": round(
            uncached + cache_creation + cache_read * 0.25 + output,
            2,
        ),
    }


def quota_from_claude_events(events: Sequence[dict[str, Any]]) -> dict[str, Any] | None:
    for event in reversed(events):
        if event.get("type") != "rate_limit_event":
            continue
        info = event.get("rate_limit_info")
        if not isinstance(info, dict):
            continue
        window = str(info.get("rateLimitType") or "")
        if window not in {"seven_day", "weekly", "7d"}:
            continue
        utilization = info.get("utilization")
        if not isinstance(utilization, (int, float)):
            utilization = info.get("usedFraction")
        if not isinstance(utilization, (int, float)):
            continue
        used_percent = float(utilization)
        if used_percent <= 1.0:
            used_percent *= 100.0
        return {
            "source": "raw-claude-rate-limit-event",
            "used_percent": round(used_percent, 3),
            "window": window,
            "resets_at": info.get("resetsAt"),
            "status": info.get("status"),
            "observed_at": event.get("timestamp"),
        }
    return None


def summarize_raw_claude_events(path: Path) -> dict[str, Any]:
    events, malformed = bench.parse_jsonl(path)
    result_event = next(
        (event for event in reversed(events) if event.get("type") == "result"),
        None,
    )
    result_event = result_event or {}
    item_types: list[str] = []
    tool_results = 0
    for event in events:
        if event.get("type") not in {"assistant", "user"}:
            continue
        message = event.get("message")
        content = message.get("content") if isinstance(message, dict) else None
        if not isinstance(content, list):
            continue
        for item in content:
            if not isinstance(item, dict):
                continue
            item_type = str(item.get("type") or "")
            item_types.append(item_type)
            if item_type == "tool_result":
                tool_results += 1
    warnings = [
        event.get("api_error_status") or event.get("result")
        for event in events
        if event.get("type") == "result" and event.get("is_error")
    ]
    model_usage = result_event.get("modelUsage")
    normalized_model_usage = None
    if isinstance(model_usage, dict):
        normalized_model_usage = {
            str(model): claude_token_metrics(value)
            for model, value in model_usage.items()
            if isinstance(value, dict)
        }
    return {
        "session_id": result_event.get("session_id"),
        "result": result_event.get("result"),
        "stop_reason": result_event.get("subtype"),
        "warnings": warnings,
        "event_count": len(events),
        "malformed_event_lines": malformed,
        "tool_uses": sum(item_type == "tool_use" for item_type in item_types),
        "tool_results": tool_results,
        "provider_calls": result_event.get("num_turns"),
        "tokens": claude_token_metrics(result_event.get("usage")),
        "model_usage": normalized_model_usage,
        "quota": quota_from_claude_events(events),
    }


def forge_claude_quota(db_path: Path) -> dict[str, Any] | None:
    if not db_path.exists():
        return None
    with sqlite3.connect(db_path) as connection:
        row = connection.execute(
            """
            SELECT fraction_used, resets_at, observed_at
            FROM quota_history
            WHERE provider = 'claude-cli' AND window_kind = 'weekly'
            ORDER BY observed_at DESC, id DESC
            LIMIT 1
            """
        ).fetchone()
    if row is None or row[0] is None:
        return None
    return {
        "source": "forge-claude-quota-history",
        "used_percent": round(float(row[0]) * 100.0, 3),
        "window": "weekly",
        "resets_at": row[1],
        "observed_at": row[2],
    }


def run_trial(
    trial: swe.SweTrial,
    *,
    instance: dict[str, Any],
    dataset_index: int,
    out_root: Path,
    worktree_root: Path,
    forge_bin: Path,
    timeout_seconds: int,
) -> dict[str, Any]:
    trial_dir = out_root / "trials" / trial.slug
    trial_dir.mkdir(parents=True, exist_ok=False)
    forge_db = swe.trial_forge_db(trial_dir)
    workspace = swe.prepare_repo(instance, worktree_root)
    prompt = swe.benchmark_prompt(str(instance["problem_statement"]))
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
        "workspace_head": bench.tiny_command(
            ("git", "rev-parse", "HEAD"), cwd=workspace
        ),
    }
    bench.json_dump(trial_dir / "manifest.json", manifest)
    argv = (
        forge_argv(forge_bin, trial.model, prompt)
        if trial.arm == "forge"
        else raw_claude_argv(trial.model, prompt)
    )
    environment = (
        forge_environment(forge_db) if trial.arm == "forge" else os.environ.copy()
    )
    if trial.arm == "forge":
        environment["FORGE_PERSISTENT_BRIDGE"] = "1"
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
        else summarize_raw_claude_events(trial_dir / "events.jsonl")
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
        base_ref=swe.benchmark_base_ref(workspace),
    )
    patch_text = (trial_dir / "changes.patch").read_text(
        encoding="utf-8",
        errors="replace",
    )
    quota = forge_claude_quota(forge_db) if trial.arm == "forge" else agent.get("quota")
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
        "process_ok": process_ok,
        "official_resolution": None,
        "manifest": str(trial_dir / "manifest.json"),
        "trial_dir": str(trial_dir),
    }
    bench.json_dump(trial_dir / "summary.json", summary)
    return summary


def load_summaries(path: Path) -> list[dict[str, Any]]:
    if not path.exists():
        return []
    summaries: list[dict[str, Any]] = []
    for line_number, line in enumerate(
        path.read_text(encoding="utf-8").splitlines(),
        start=1,
    ):
        if not line.strip():
            continue
        try:
            value = json.loads(line)
        except json.JSONDecodeError as error:
            raise ValueError(f"{path}:{line_number}: invalid JSON: {error}") from error
        if not isinstance(value, dict):
            raise ValueError(f"{path}:{line_number}: summary is not an object")
        summaries.append(value)
    return summaries


def effective_quota(
    summaries: Sequence[dict[str, Any]],
    observed_weekly_pct: float,
) -> float:
    values = [observed_weekly_pct]
    values.extend(
        value
        for summary in summaries
        if (value := bench.quota_used_percent(summary.get("quota"))) is not None
    )
    return max(values)


def validate_resume(
    manifest: dict[str, Any],
    *,
    dataset: Path,
    forge_bin: Path,
    models: Sequence[str],
    arms: Sequence[str],
    seed: int,
    baseline_weekly_pct: float,
    max_weekly_increase_pct: float,
    runtime: dict[str, Any],
    resolved_models: dict[str, str],
) -> None:
    expected = {
        "dataset_sha256": bench.sha256_file(dataset),
        "forge_binary_sha256": bench.sha256_file(forge_bin),
        "models": list(models),
        "arms": list(arms),
        "seed": seed,
        "baseline_weekly_pct": baseline_weekly_pct,
        "max_weekly_increase_pct": max_weekly_increase_pct,
        "claude_runtime": runtime,
        "resolved_models": resolved_models,
    }
    mismatches = {
        key: {"expected": value, "actual": manifest.get(key)}
        for key, value in expected.items()
        if manifest.get(key) != value
    }
    if mismatches:
        raise ValueError(f"resume manifest mismatch: {json.dumps(mismatches)}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dataset", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("--worktree-root", required=True, type=Path)
    parser.add_argument(
        "--forge-bin",
        type=Path,
        default=bench.REPO_ROOT / "target" / "release" / "forge",
    )
    parser.add_argument("--models", default=",".join(DEFAULT_MODELS))
    parser.add_argument("--arms", default=",".join(DEFAULT_ARMS))
    parser.add_argument("--seed", type=int, default=20260727)
    parser.add_argument("--timeout-seconds", type=int, default=1500)
    parser.add_argument("--baseline-weekly-pct", required=True, type=float)
    parser.add_argument("--max-weekly-increase-pct", type=float, default=9.0)
    parser.add_argument(
        "--observed-weekly-pct",
        required=True,
        type=float,
        help="fresh externally observed weekly utilization (Helm for the official run)",
    )
    parser.add_argument(
        "--max-new-trials",
        type=int,
        default=1,
        help="run exactly one additional provider arm before an external refresh",
    )
    parser.add_argument("--resume", action="store_true")
    args = parser.parse_args()

    models = bench.parse_csv(args.models)
    arms = bench.parse_csv(args.arms)
    if set(models) - set(DEFAULT_MODELS):
        parser.error(f"unsupported models: {sorted(set(models) - set(DEFAULT_MODELS))}")
    unknown_arms = set(arms) - set(DEFAULT_ARMS)
    if unknown_arms:
        parser.error(f"unsupported arms: {sorted(unknown_arms)}")
    if not arms:
        parser.error("at least one arm is required")
    if args.max_new_trials != 1:
        parser.error(
            "--max-new-trials must be 1 so external quotas refresh after every arm"
        )
    if not args.forge_bin.is_file():
        parser.error(f"Forge binary does not exist: {args.forge_bin}")

    try:
        runtime = claude_runtime()
    except (OSError, subprocess.SubprocessError, RuntimeError) as error:
        parser.error(str(error))
    resolved_models: dict[str, str] = {}
    for model in models:
        capability = resolve_claude_model(runtime, model)
        if capability is None:
            parser.error(
                f"Claude initialize did not advertise requested model {model!r}"
            )
        if not capability.get("supportsEffort") or "high" not in capability.get(
            "supportedEffortLevels", []
        ):
            parser.error(f"Claude model {model!r} does not advertise high effort")
        resolved_models[model] = str(capability["resolvedModel"])

    instances = swe.load_dataset(args.dataset)
    by_id = {str(instance["instance_id"]): instance for instance in instances}
    dataset_indexes = {
        str(instance["instance_id"]): index for index, instance in enumerate(instances)
    }
    trials = swe.plan_trials(models, instances, arms, args.seed)
    cap_pct = args.baseline_weekly_pct + args.max_weekly_increase_pct
    args.out.mkdir(parents=True, exist_ok=True)
    manifest_path = args.out / "suite-manifest.json"
    index_path = args.out / "trial-index.jsonl"

    if manifest_path.exists():
        if not args.resume:
            parser.error("output contains a run; pass --resume to continue it")
        suite_manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        validate_resume(
            suite_manifest,
            dataset=args.dataset,
            forge_bin=args.forge_bin,
            models=models,
            arms=arms,
            seed=args.seed,
            baseline_weekly_pct=args.baseline_weekly_pct,
            max_weekly_increase_pct=args.max_weekly_increase_pct,
            runtime=runtime,
            resolved_models=resolved_models,
        )
    else:
        if args.resume:
            parser.error("cannot resume: suite-manifest.json does not exist")
        if any(args.out.iterdir()):
            parser.error(f"output directory must be empty: {args.out}")
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
            "forge_dirty": bool(bench.tiny_command(("git", "status", "--porcelain"))),
            "forge_binary_sha256": bench.sha256_file(args.forge_bin),
            "forge_version": bench.tiny_command((str(args.forge_bin), "--version")),
            "claude_version": runtime["cliVersion"],
            "claude_runtime": runtime,
            "models": models,
            "resolved_models": resolved_models,
            "arms": arms,
            "seed": args.seed,
            "reasoning_effort": "high",
            "raw_profile": "native",
            "timeout_seconds": args.timeout_seconds,
            "baseline_weekly_pct": args.baseline_weekly_pct,
            "max_weekly_increase_pct": args.max_weekly_increase_pct,
            "hard_stop_weekly_pct": cap_pct,
            "trial_order": [dataclasses.asdict(trial) for trial in trials],
        }
        bench.json_dump(manifest_path, suite_manifest)

    summaries = load_summaries(index_path)
    completed_trials = [summary.get("trial") for summary in summaries]
    planned_prefix = [
        dataclasses.asdict(trial) for trial in trials[: len(completed_trials)]
    ]
    if completed_trials != planned_prefix:
        parser.error("completed trial order does not match the suite manifest")
    remaining = trials[len(summaries) :]
    remaining = remaining[: args.max_new_trials]

    quota_check = {
        "observed_at": bench.utc_now(),
        "source": "helm-refreshed",
        "used_percent": args.observed_weekly_pct,
        "hard_stop_weekly_pct": cap_pct,
        "completed_trials_before_segment": len(summaries),
    }
    with (args.out / "quota-checks.jsonl").open("a", encoding="utf-8") as handle:
        handle.write(json.dumps(quota_check, sort_keys=True) + "\n")

    stop_reason: str | None = None
    quota_warnings: list[str] = []
    new_summaries = 0
    last_seen_pct = effective_quota(summaries, args.observed_weekly_pct)
    for trial in remaining:
        if last_seen_pct >= cap_pct:
            stop_reason = (
                f"weekly hard stop reached before {trial.slug}: "
                f"{last_seen_pct:.3f}% >= {cap_pct:.3f}%"
            )
            break
        print(
            json.dumps(
                {
                    "event": "trial_start",
                    "slug": trial.slug,
                    "pair_id": trial.pair_id,
                    "prior_weekly_pct": last_seen_pct,
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
        )
        summaries.append(summary)
        new_summaries += 1
        with index_path.open("a", encoding="utf-8") as index:
            index.write(json.dumps(summary, sort_keys=True) + "\n")
        swe.write_predictions(args.out, summaries, models, arms)
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
        if current_pct + 1e-9 < last_seen_pct:
            warning = (
                f"quota telemetry was lower than the conservative effective value after "
                f"{trial.slug}: {current_pct:.3f}% < {last_seen_pct:.3f}%; retained the "
                "higher value"
            )
            quota_warnings.append(warning)
            print(
                json.dumps({"event": "quota_warning", "message": warning}), flush=True
            )
        last_seen_pct = max(last_seen_pct, current_pct)
        if last_seen_pct >= cap_pct:
            stop_reason = (
                f"weekly hard stop reached after {trial.slug}: "
                f"{last_seen_pct:.3f}% >= {cap_pct:.3f}%"
            )
            break

    completed = len(summaries) == len(trials)
    paused = (
        stop_reason is None
        and not completed
        and args.max_new_trials is not None
        and new_summaries == len(remaining)
    )
    final = {
        "schema_version": 1,
        "updated_at": bench.utc_now(),
        "planned_trials": len(trials),
        "completed_trials": len(summaries),
        "new_trials_this_segment": new_summaries,
        "patched_trials": sum(summary["patched"] for summary in summaries),
        "complete": completed,
        "paused_for_external_quota_refresh": paused,
        "stop_reason": stop_reason,
        "last_quota": next(
            (
                summary.get("quota")
                for summary in reversed(summaries)
                if summary.get("quota") is not None
            ),
            None,
        ),
        "last_effective_weekly_pct": last_seen_pct,
        "quota_warnings": quota_warnings,
        "official_evaluation_required": True,
    }
    bench.json_dump(args.out / "run-final.json", final)
    print(json.dumps({"event": "run_complete", **final}), flush=True)
    return 2 if stop_reason is not None else 0


if __name__ == "__main__":
    raise SystemExit(main())
