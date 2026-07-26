#!/usr/bin/env python3
"""Matched Forge OAuth vs raw Codex CLI benchmark runner.

The runner intentionally uses deterministic repository fixtures and independent
post-run verification.  It preserves every provider event, stderr, patch,
verification log, and machine-readable summary below one run directory.

No credential material is copied into the run directory.  Forge reads its
existing codex-oauth credential from the OS keyring and raw Codex uses its
normal authenticated CLI session.
"""

from __future__ import annotations

import argparse
import dataclasses
import datetime as dt
import hashlib
import json
import os
import random
import shutil
import signal
import sqlite3
import statistics
import subprocess
import sys
import time
from collections import defaultdict
from pathlib import Path
from typing import Any, Iterable, Sequence


REPO_ROOT = Path(__file__).resolve().parents[2]
SCENARIO_ROOT = REPO_ROOT / "scripts" / "manual-e2e" / "scenarios"
DEFAULT_MODELS = ("gpt-5.6-sol", "gpt-5.6-terra", "gpt-5.6-luna")
DEFAULT_SCENARIOS = (
    "multifile-reservations",
    "go-ordered-pipeline",
    "typescript-config-recovery",
    "rust-transaction-ledger",
)
DEFAULT_ARMS = ("forge", "raw-codex")
FORGE_CHECKPOINT_PATHSPEC = ":(exclude).forge/checkpoints/**"


@dataclasses.dataclass(frozen=True)
class Trial:
    pair_index: int
    repeat: int
    model: str
    scenario: str
    arm: str

    @property
    def pair_id(self) -> str:
        return f"{self.model}__{self.scenario}__r{self.repeat}"

    @property
    def slug(self) -> str:
        return f"{self.pair_index:03d}__{self.pair_id}__{self.arm}"


@dataclasses.dataclass
class CommandResult:
    argv: list[str]
    exit_code: int | None
    signal: int | None
    timed_out: bool
    wall_seconds: float
    started_at: str
    ended_at: str


def utc_now() -> str:
    return dt.datetime.now(dt.timezone.utc).isoformat()


def json_dump(path: Path, value: Any) -> None:
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def run_capture(
    argv: Sequence[str],
    *,
    cwd: Path,
    stdout_path: Path,
    stderr_path: Path,
    timeout_seconds: int,
    env: dict[str, str] | None = None,
) -> CommandResult:
    started_at = utc_now()
    started = time.monotonic()
    timed_out = False
    return_code: int | None = None
    with stdout_path.open("wb") as stdout, stderr_path.open("wb") as stderr:
        process = subprocess.Popen(
            list(argv),
            cwd=cwd,
            stdin=subprocess.DEVNULL,
            stdout=stdout,
            stderr=stderr,
            env=env,
            start_new_session=True,
        )
        try:
            return_code = process.wait(timeout=timeout_seconds)
        except subprocess.TimeoutExpired:
            timed_out = True
            os.killpg(process.pid, signal.SIGTERM)
            try:
                return_code = process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                return_code = process.wait(timeout=5)
    wall_seconds = time.monotonic() - started
    return CommandResult(
        argv=list(argv),
        exit_code=return_code if return_code is not None and return_code >= 0 else None,
        signal=-return_code if return_code is not None and return_code < 0 else None,
        timed_out=timed_out,
        wall_seconds=round(wall_seconds, 3),
        started_at=started_at,
        ended_at=utc_now(),
    )


def tiny_command(argv: Sequence[str], *, cwd: Path = REPO_ROOT) -> str:
    completed = subprocess.run(
        list(argv),
        cwd=cwd,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        timeout=30,
        check=False,
    )
    return completed.stdout.strip()


def add_local_git_excludes(workspace: Path, patterns: Sequence[str]) -> None:
    """Append benchmark-local excludes without changing the checked-out tree."""

    exclude_path = workspace / ".git" / "info" / "exclude"
    existing = set(exclude_path.read_text(encoding="utf-8").splitlines())
    missing = [pattern for pattern in patterns if pattern not in existing]
    if not missing:
        return
    with exclude_path.open("a", encoding="utf-8") as handle:
        if exclude_path.stat().st_size:
            handle.write("\n")
        handle.write("\n".join(missing) + "\n")


def initialize_workspace(scenario: str, workspace: Path) -> dict[str, str]:
    source = SCENARIO_ROOT / scenario
    fixture = source / "fixture"
    prompt = source / "prompt.txt"
    if not fixture.is_dir() or not prompt.is_file():
        raise ValueError(f"scenario {scenario!r} has no fixture/prompt")
    shutil.copytree(fixture, workspace)
    commands = (
        ("git", "init", "-q"),
        ("git", "config", "user.email", "harness-bench@local.test"),
        ("git", "config", "user.name", "Harness Benchmark"),
        ("git", "add", "-A"),
        ("git", "commit", "-qm", "benchmark fixture baseline"),
    )
    for command in commands:
        subprocess.run(command, cwd=workspace, check=True, timeout=30)
    # Agent/verifier build products are evidence only through their command logs. Keep them out of
    # the solution patch without changing the model-visible fixture or tracked .gitignore.
    add_local_git_excludes(
        workspace,
        (
            "target/",
            "node_modules/",
            ".pytest_cache/",
            "__pycache__/",
            "*.pyc",
            ".forge/",
        ),
    )
    return {
        "baseline_commit": tiny_command(("git", "rev-parse", "HEAD"), cwd=workspace),
        "baseline_tree": tiny_command(("git", "rev-parse", "HEAD^{tree}"), cwd=workspace),
        "prompt_sha256": sha256_file(prompt),
    }


def forge_argv(forge_bin: Path, model: str, prompt: str) -> list[str]:
    return [
        str(forge_bin),
        "run",
        "--mode",
        "bypass",
        "--output-format",
        "stream-json",
        "--model",
        f"codex-oauth::{model}",
        prompt,
    ]


def forge_environment(forge_db: Path) -> dict[str, str]:
    env = os.environ.copy()
    env.update(
        {
            "FORGE_DB": str(forge_db),
            "FORGE_MESH__DEFAULT_EFFORT": "xhigh",
            "FORGE_MESH__AUTO_DISCOVER": "false",
            "FORGE_MESH__PIN_FAILOVER": "false",
            "FORGE_MESH__FAILOVER": "false",
        }
    )
    return env


def raw_codex_argv(model: str, workspace: Path, prompt: str, profile: str) -> list[str]:
    argv = [
        "codex",
        "exec",
        "--json",
        "--dangerously-bypass-approvals-and-sandbox",
        "--skip-git-repo-check",
        "-m",
        model,
        "-c",
        'model_reasoning_effort="xhigh"',
        "-C",
        str(workspace),
    ]
    if profile == "reduced-config":
        # These are Codex's own isolation flags.  Current 0.145.0 still loads the
        # installed skill catalog; the resulting warning and token cost are kept.
        argv.extend(
            (
                "--ignore-user-config",
                "--ignore-rules",
                "--disable",
                "plugins",
                "--disable",
                "skill_search",
            )
        )
    argv.append(prompt)
    return argv


def parse_jsonl(path: Path) -> tuple[list[dict[str, Any]], int]:
    events: list[dict[str, Any]] = []
    malformed = 0
    if not path.exists():
        return events, malformed
    for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
        if not line.strip():
            continue
        try:
            value = json.loads(line)
        except json.JSONDecodeError:
            malformed += 1
            continue
        if isinstance(value, dict):
            events.append(value)
    return events, malformed


def token_metrics(usage: dict[str, Any] | None) -> dict[str, int | float | None]:
    if not usage:
        return {
            "input_tokens": None,
            "cached_input_tokens": None,
            "uncached_input_tokens": None,
            "output_tokens": None,
            "reasoning_output_tokens": None,
            "total_tokens": None,
            "cache_adjusted_tokens_025": None,
        }
    input_tokens = int(usage.get("input_tokens") or 0)
    cached = int(
        usage.get("cached_input_tokens")
        or usage.get("cache_read_input_tokens")
        or 0
    )
    output = int(usage.get("output_tokens") or 0)
    reasoning = int(usage.get("reasoning_output_tokens") or 0)
    uncached = max(0, input_tokens - cached)
    return {
        "input_tokens": input_tokens,
        # Both current Forge Responses telemetry and Codex 0.145.0 report cached
        # input as a SUBSET of input_tokens.  Never add it to input_tokens.
        "cached_input_tokens": cached,
        "uncached_input_tokens": uncached,
        "output_tokens": output,
        "reasoning_output_tokens": reasoning,
        "total_tokens": input_tokens + output,
        # This is a transparent sensitivity metric, not a claim about the
        # subscription's proprietary quota weighting.
        "cache_adjusted_tokens_025": round(uncached + cached * 0.25 + output, 2),
    }


def summarize_forge_events(path: Path) -> dict[str, Any]:
    events, malformed = parse_jsonl(path)
    usage_event = next(
        (
            event
            for event in reversed(events)
            if event.get("type") == "system" and event.get("subtype") == "usage"
        ),
        None,
    )
    result_event = next(
        (event for event in reversed(events) if event.get("type") == "result"),
        None,
    )
    init_event = next(
        (
            event
            for event in events
            if event.get("type") == "system" and event.get("subtype") == "init"
        ),
        None,
    )
    warnings = [
        event.get("message")
        for event in events
        if event.get("type") == "system"
        and event.get("subtype") in {"warning", "error"}
    ]
    tool_uses = 0
    tool_results = 0
    for event in events:
        message = event.get("message")
        if not isinstance(message, dict):
            continue
        content = message.get("content")
        if not isinstance(content, list):
            continue
        for item in content:
            if not isinstance(item, dict):
                continue
            if item.get("type") == "tool_use":
                tool_uses += 1
            elif item.get("type") == "tool_result":
                tool_results += 1
    return {
        "session_id": (init_event or result_event or {}).get("session_id"),
        "result": (result_event or {}).get("result"),
        "stop_reason": (result_event or {}).get("stop_reason"),
        "warnings": warnings,
        "event_count": len(events),
        "malformed_event_lines": malformed,
        "tool_uses": tool_uses,
        "tool_results": tool_results,
        "tokens": token_metrics((usage_event or {}).get("usage")),
    }


def summarize_raw_events(path: Path) -> dict[str, Any]:
    events, malformed = parse_jsonl(path)
    usage_event = next(
        (event for event in reversed(events) if event.get("type") == "turn.completed"),
        None,
    )
    agent_event = next(
        (
            event
            for event in reversed(events)
            if event.get("type") == "item.completed"
            and (event.get("item") or {}).get("type") == "agent_message"
        ),
        None,
    )
    thread_event = next(
        (event for event in events if event.get("type") == "thread.started"),
        None,
    )
    item_types = [
        (event.get("item") or {}).get("type")
        for event in events
        if event.get("type") == "item.completed"
    ]
    warnings = [
        (event.get("item") or {}).get("message")
        for event in events
        if event.get("type") == "item.completed"
        and (event.get("item") or {}).get("type") == "error"
    ]
    return {
        "session_id": (thread_event or {}).get("thread_id"),
        "result": ((agent_event or {}).get("item") or {}).get("text"),
        "stop_reason": "turn.completed" if usage_event else None,
        "warnings": warnings,
        "event_count": len(events),
        "malformed_event_lines": malformed,
        "tool_uses": sum(
            item_type
            in {
                "command_execution",
                "file_change",
                "mcp_tool_call",
                "web_search",
                "collab_tool_call",
            }
            for item_type in item_types
        ),
        "tool_results": None,
        "tokens": token_metrics((usage_event or {}).get("usage")),
    }


def verification_commands(scenario: str, workspace: Path) -> list[list[str]]:
    if scenario == "multifile-reservations":
        return [[sys.executable, "-m", "unittest", "discover", "-v"]]
    if scenario == "go-ordered-pipeline":
        return [
            ["bash", "-lc", 'test -z "$(gofmt -l pipeline/pipeline.go)"'],
            ["go", "vet", "./..."],
            ["go", "test", "-race", "./..."],
        ]
    if scenario == "typescript-config-recovery":
        # The task requires a fresh strict TypeScript build, not a specifically named `lint`
        # package-script alias. Invoking the existing build script with --noEmit tests the actual
        # contract without rewarding an implementation merely for matching the reference's alias.
        return [["npm", "test"], ["npm", "run", "build", "--", "--noEmit"]]
    if scenario == "rust-transaction-ledger":
        return [
            ["cargo", "fmt", "--check"],
            ["cargo", "clippy", "--all-targets", "--", "-D", "warnings"],
            ["cargo", "test", "--all-targets"],
        ]
    raise ValueError(f"no verifier for {scenario}")


def verify_scenario(
    scenario: str,
    workspace: Path,
    trial_dir: Path,
    timeout_seconds: int,
) -> dict[str, Any]:
    steps: list[dict[str, Any]] = []
    log_path = trial_dir / "verification.log"
    log_path.write_text("", encoding="utf-8")
    all_passed = True
    for index, argv in enumerate(verification_commands(scenario, workspace), start=1):
        stdout_path = trial_dir / f"verification-{index}.stdout.log"
        stderr_path = trial_dir / f"verification-{index}.stderr.log"
        result = run_capture(
            argv,
            cwd=workspace,
            stdout_path=stdout_path,
            stderr_path=stderr_path,
            timeout_seconds=timeout_seconds,
        )
        passed = result.exit_code == 0 and not result.timed_out
        all_passed &= passed
        step = dataclasses.asdict(result) | {"passed": passed}
        steps.append(step)
        with log_path.open("a", encoding="utf-8") as combined:
            combined.write(f"$ {' '.join(argv)}\n")
            combined.write(stdout_path.read_text(encoding="utf-8", errors="replace"))
            combined.write(stderr_path.read_text(encoding="utf-8", errors="replace"))
            combined.write(f"\n[exit={result.exit_code} timeout={result.timed_out}]\n")
    return {"passed": all_passed, "steps": steps}


def capture_patch(workspace: Path, trial_dir: Path) -> dict[str, Any]:
    subprocess.run(
        ("git", "add", "-N", "--", ".", FORGE_CHECKPOINT_PATHSPEC),
        cwd=workspace,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=False,
        timeout=30,
    )
    patch = subprocess.run(
        (
            "git",
            "diff",
            "--binary",
            "--no-ext-diff",
            "HEAD",
            "--",
            ".",
            FORGE_CHECKPOINT_PATHSPEC,
        ),
        cwd=workspace,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        timeout=30,
        check=False,
    )
    (trial_dir / "changes.patch").write_bytes(patch.stdout)
    (trial_dir / "changes.patch.stderr.log").write_bytes(patch.stderr)
    status = tiny_command(
        ("git", "status", "--short", "--", ".", FORGE_CHECKPOINT_PATHSPEC),
        cwd=workspace,
    )
    diffstat = tiny_command(
        ("git", "diff", "--stat", "HEAD", "--", ".", FORGE_CHECKPOINT_PATHSPEC),
        cwd=workspace,
    )
    return {
        "patch_bytes": len(patch.stdout),
        "patch_sha256": hashlib.sha256(patch.stdout).hexdigest(),
        "status": status.splitlines(),
        "diffstat": diffstat,
    }


def raw_quota_for_thread(thread_id: str | None) -> dict[str, Any] | None:
    if not thread_id:
        return None
    codex_root = Path.home() / ".codex"
    candidates = list((codex_root / "sessions").glob(f"**/*{thread_id}*.jsonl"))
    candidates.extend((codex_root / "archived_sessions").glob(f"*{thread_id}*.jsonl"))
    for path in sorted(candidates, key=lambda item: item.stat().st_mtime, reverse=True):
        latest: dict[str, Any] | None = None
        with path.open(encoding="utf-8", errors="replace") as handle:
            for line in handle:
                try:
                    event = json.loads(line)
                except json.JSONDecodeError:
                    continue
                payload = event.get("payload") or {}
                if event.get("type") != "event_msg" or payload.get("type") != "token_count":
                    continue
                rate_limits = (payload.get("info") or {}).get("rate_limits")
                if rate_limits:
                    latest = rate_limits
        if latest:
            primary = latest.get("primary") or {}
            return {
                "source": "raw-codex-rollout",
                "used_percent": primary.get("used_percent"),
                "window_minutes": primary.get("window_minutes"),
                "resets_at": primary.get("resets_at"),
                "plan_type": latest.get("plan_type"),
                "rollout_path": str(path),
                "observed_at": int(path.stat().st_mtime),
            }
    return None


def forge_quota(db_path: Path) -> dict[str, Any] | None:
    if not db_path.exists():
        return None
    with sqlite3.connect(db_path) as connection:
        row = connection.execute(
            """
            SELECT fraction_used, resets_at, observed_at
            FROM quota_history
            WHERE provider = 'codex-oauth' AND window_kind = 'weekly'
            ORDER BY observed_at DESC, id DESC
            LIMIT 1
            """
        ).fetchone()
    if row is None:
        return None
    return {
        "source": "forge-quota-history",
        "used_percent": round(float(row[0]) * 100.0, 3),
        "window_minutes": 7 * 24 * 60,
        "resets_at": row[1],
        "observed_at": row[2],
    }


def forge_session_tokens(db_path: Path, session_id: str | None) -> dict[str, Any] | None:
    """Return complete persisted usage for a Forge session.

    The stream-json usage event is emitted when the user-visible turn finishes. Forge may then run
    a small auxiliary memory extraction before the process exits. The Store ledger is therefore
    the authoritative whole-harness total, while the stream value is retained separately.
    """

    if not session_id or not db_path.exists():
        return None
    with sqlite3.connect(db_path) as connection:
        row = connection.execute(
            """
            SELECT COUNT(*),
                   COALESCE(SUM(u.input_tokens), 0),
                   COALESCE(SUM(u.cached_input_tokens), 0),
                   COALESCE(SUM(u.output_tokens), 0)
            FROM usage u
            JOIN message m ON m.id = u.message_id
            WHERE m.session_id = ?
            """,
            (session_id,),
        ).fetchone()
    if row is None or int(row[0]) == 0:
        return None
    tokens = token_metrics(
        {
            "input_tokens": int(row[1]),
            "cached_input_tokens": int(row[2]),
            "output_tokens": int(row[3]),
        }
    )
    return {"provider_calls": int(row[0]), **tokens}


def refresh_quota_with_forge(
    forge_bin: Path,
    forge_db: Path,
    workspace: Path,
    trial_dir: Path,
) -> tuple[dict[str, Any] | None, dict[str, Any]]:
    """Force the normal bounded Forge quota-refresh path and retain its diagnostics.

    Codex does not attach a rate-limit object to every long-running CLI turn. When that happens,
    guessing from an older percentage would make the overnight cap unsafe. Forge's mesh overview
    performs the same minimal OAuth quota probe used before routing, against the same account.
    """

    result = run_capture(
        (str(forge_bin), "mesh", "--json"),
        cwd=workspace,
        stdout_path=trial_dir / "quota-refresh.stdout.json",
        stderr_path=trial_dir / "quota-refresh.stderr.log",
        timeout_seconds=60,
        env=forge_environment(forge_db),
    )
    return forge_quota(forge_db), dataclasses.asdict(result)


def quota_used_percent(value: dict[str, Any] | None) -> float | None:
    if not value or value.get("used_percent") is None:
        return None
    return float(value["used_percent"])


def plan_trials(
    models: Sequence[str],
    scenarios: Sequence[str],
    repeats: int,
    arms: Sequence[str],
    seed: int,
) -> list[Trial]:
    pairs = [
        (repeat, model, scenario)
        for repeat in range(1, repeats + 1)
        for model in models
        for scenario in scenarios
    ]
    random.Random(seed).shuffle(pairs)
    trials: list[Trial] = []
    for pair_index, (repeat, model, scenario) in enumerate(pairs, start=1):
        ordered_arms = list(arms)
        if pair_index % 2 == 0:
            ordered_arms.reverse()
        trials.extend(
            Trial(pair_index, repeat, model, scenario, arm) for arm in ordered_arms
        )
    return trials


def run_trial(
    trial: Trial,
    *,
    out_root: Path,
    forge_bin: Path,
    forge_db: Path,
    timeout_seconds: int,
    verifier_timeout_seconds: int,
    raw_profile: str,
    dry_run: bool,
) -> dict[str, Any]:
    trial_dir = out_root / "trials" / trial.slug
    workspace = trial_dir / "workspace"
    trial_dir.mkdir(parents=True, exist_ok=False)
    fixture = initialize_workspace(trial.scenario, workspace)
    prompt_path = SCENARIO_ROOT / trial.scenario / "prompt.txt"
    prompt = prompt_path.read_text(encoding="utf-8").strip()
    if trial.arm == "forge":
        argv = forge_argv(forge_bin, trial.model, prompt)
        env = forge_environment(forge_db)
    elif trial.arm == "raw-codex":
        argv = raw_codex_argv(trial.model, workspace, prompt, raw_profile)
        env = os.environ.copy()
    else:
        raise ValueError(f"unknown arm: {trial.arm}")

    manifest = {
        "schema_version": 1,
        "trial": dataclasses.asdict(trial),
        "pair_id": trial.pair_id,
        "forge_commit": tiny_command(("git", "rev-parse", "HEAD")),
        "forge_binary": str(forge_bin),
        "forge_binary_sha256": sha256_file(forge_bin),
        "forge_version": tiny_command((str(forge_bin), "--version")),
        "codex_version": tiny_command(("codex", "--version")),
        "reasoning_effort": "xhigh",
        "raw_profile": raw_profile,
        "fixture": fixture,
        "prompt_path": str(prompt_path.relative_to(REPO_ROOT)),
        "command": argv,
        "created_at": utc_now(),
        "dry_run": dry_run,
    }
    json_dump(trial_dir / "manifest.json", manifest)
    if dry_run:
        summary = {
            "trial": dataclasses.asdict(trial),
            "pair_id": trial.pair_id,
            "dry_run": True,
            "manifest": str(trial_dir / "manifest.json"),
        }
        json_dump(trial_dir / "summary.json", summary)
        return summary

    command_result = run_capture(
        argv,
        cwd=workspace,
        stdout_path=trial_dir / "events.jsonl",
        stderr_path=trial_dir / "stderr.log",
        timeout_seconds=timeout_seconds,
        env=env,
    )
    # Persist this before parsing provider-specific events. A parser regression must never erase
    # whether the expensive model process completed, timed out, or was terminated.
    json_dump(trial_dir / "process.json", dataclasses.asdict(command_result))
    agent = (
        summarize_forge_events(trial_dir / "events.jsonl")
        if trial.arm == "forge"
        else summarize_raw_events(trial_dir / "events.jsonl")
    )
    if trial.arm == "forge":
        complete_tokens = forge_session_tokens(forge_db, agent.get("session_id"))
        if complete_tokens is not None:
            agent["stream_tokens"] = agent["tokens"]
            agent["tokens"] = complete_tokens
            stream_total = agent["stream_tokens"].get("total_tokens")
            complete_total = complete_tokens.get("total_tokens")
            agent["post_stream_tokens"] = (
                complete_total - stream_total
                if complete_total is not None and stream_total is not None
                else None
            )
    verification = verify_scenario(
        trial.scenario,
        workspace,
        trial_dir,
        verifier_timeout_seconds,
    )
    patch = capture_patch(workspace, trial_dir)
    quota = (
        forge_quota(forge_db)
        if trial.arm == "forge"
        else raw_quota_for_thread(agent.get("session_id"))
    )
    quota_refresh = None
    if quota_used_percent(quota) is None:
        quota, quota_refresh = refresh_quota_with_forge(
            forge_bin,
            forge_db,
            workspace,
            trial_dir,
        )
    process_ok = command_result.exit_code == 0 and not command_result.timed_out
    summary = {
        "trial": dataclasses.asdict(trial),
        "pair_id": trial.pair_id,
        "dry_run": False,
        "process": dataclasses.asdict(command_result),
        "agent": agent,
        "verification": verification,
        "patch": patch,
        "quota": quota,
        "quota_refresh": quota_refresh,
        "success": process_ok and verification["passed"],
        "manifest": str(trial_dir / "manifest.json"),
        "trial_dir": str(trial_dir),
    }
    json_dump(trial_dir / "summary.json", summary)
    return summary


def median(values: Iterable[float | int | None]) -> float | None:
    present = [float(value) for value in values if value is not None]
    return round(statistics.median(present), 3) if present else None


def aggregate(summaries: Sequence[dict[str, Any]]) -> dict[str, Any]:
    completed = [summary for summary in summaries if not summary.get("dry_run")]
    groups: dict[tuple[str, str], list[dict[str, Any]]] = defaultdict(list)
    for summary in completed:
        trial = summary["trial"]
        groups[(trial["arm"], trial["model"])].append(summary)

    by_arm_model: list[dict[str, Any]] = []
    for (arm, model), rows in sorted(groups.items()):
        successes = sum(bool(row["success"]) for row in rows)
        total_tokens = [
            row["agent"]["tokens"]["total_tokens"]
            for row in rows
            if row["agent"]["tokens"]["total_tokens"] is not None
        ]
        summed_tokens = sum(total_tokens)
        by_arm_model.append(
            {
                "arm": arm,
                "model": model,
                "trials": len(rows),
                "successes": successes,
                "success_rate": round(successes / len(rows), 4) if rows else None,
                "median_total_tokens": median(total_tokens),
                "sum_total_tokens": summed_tokens,
                "tokens_per_success": (
                    round(summed_tokens / successes, 2) if successes else None
                ),
                "median_cache_adjusted_tokens_025": median(
                    row["agent"]["tokens"]["cache_adjusted_tokens_025"] for row in rows
                ),
                "median_wall_seconds": median(
                    row["process"]["wall_seconds"] for row in rows
                ),
                "complete_token_telemetry": len(total_tokens) == len(rows),
            }
        )

    pairs: dict[str, dict[str, dict[str, Any]]] = defaultdict(dict)
    for summary in completed:
        pairs[summary["pair_id"]][summary["trial"]["arm"]] = summary
    paired_rows: list[dict[str, Any]] = []
    for pair_id, arms in sorted(pairs.items()):
        if not all(arm in arms for arm in DEFAULT_ARMS):
            continue
        forge = arms["forge"]
        raw = arms["raw-codex"]
        forge_tokens = forge["agent"]["tokens"]["total_tokens"]
        raw_tokens = raw["agent"]["tokens"]["total_tokens"]
        token_reduction = None
        if forge_tokens is not None and raw_tokens:
            token_reduction = round((raw_tokens - forge_tokens) / raw_tokens, 4)
        paired_rows.append(
            {
                "pair_id": pair_id,
                "model": forge["trial"]["model"],
                "scenario": forge["trial"]["scenario"],
                "repeat": forge["trial"]["repeat"],
                "forge_success": forge["success"],
                "raw_codex_success": raw["success"],
                "forge_total_tokens": forge_tokens,
                "raw_codex_total_tokens": raw_tokens,
                "forge_token_reduction_vs_raw": token_reduction,
                "forge_wall_seconds": forge["process"]["wall_seconds"],
                "raw_codex_wall_seconds": raw["process"]["wall_seconds"],
            }
        )
    return {
        "generated_at": utc_now(),
        "completed_trials": len(completed),
        "by_arm_model": by_arm_model,
        "paired": paired_rows,
        "paired_summary": {
            "complete_pairs": len(paired_rows),
            "forge_quality_wins": sum(
                row["forge_success"] and not row["raw_codex_success"]
                for row in paired_rows
            ),
            "raw_codex_quality_wins": sum(
                row["raw_codex_success"] and not row["forge_success"]
                for row in paired_rows
            ),
            "quality_ties": sum(
                row["raw_codex_success"] == row["forge_success"] for row in paired_rows
            ),
            "median_forge_token_reduction_vs_raw": median(
                row["forge_token_reduction_vs_raw"] for row in paired_rows
            ),
        },
    }


def render_markdown(report: dict[str, Any]) -> str:
    lines = [
        "# Generated matched-run summary",
        "",
        f"Generated: {report['generated_at']}",
        "",
        "## Arm/model totals",
        "",
        "| arm | model | success | median tokens | tokens/success | median wall s | telemetry |",
        "|---|---|---:|---:|---:|---:|---|",
    ]
    for row in report["by_arm_model"]:
        lines.append(
            "| {arm} | {model} | {successes}/{trials} | {median_total_tokens} | "
            "{tokens_per_success} | {median_wall_seconds} | {telemetry} |".format(
                **row,
                telemetry="complete" if row["complete_token_telemetry"] else "incomplete",
            )
        )
    paired = report["paired_summary"]
    lines.extend(
        [
            "",
            "## Paired summary",
            "",
            f"- Complete pairs: {paired['complete_pairs']}",
            f"- Forge-only verifier passes: {paired['forge_quality_wins']}",
            f"- Raw-Codex-only verifier passes: {paired['raw_codex_quality_wins']}",
            f"- Quality ties: {paired['quality_ties']}",
            "- Median Forge token reduction versus raw: "
            f"{paired['median_forge_token_reduction_vs_raw']}",
            "",
            "Cached input is a subset of `input_tokens` for both arms and is not double-counted.",
            "The cache-adjusted sensitivity metric weights cached input at 0.25, but no claim is",
            "made that this equals OpenAI's private subscription-quota accounting.",
            "",
        ]
    )
    return "\n".join(lines)


def write_aggregate(out_root: Path, summaries: Sequence[dict[str, Any]]) -> dict[str, Any]:
    report = aggregate(summaries)
    json_dump(out_root / "aggregate.json", report)
    (out_root / "aggregate.md").write_text(render_markdown(report), encoding="utf-8")
    return report


def parse_csv(value: str) -> list[str]:
    return [item.strip() for item in value.split(",") if item.strip()]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument(
        "--forge-bin",
        type=Path,
        default=REPO_ROOT / "target" / "debug" / "forge",
    )
    parser.add_argument("--models", default=",".join(DEFAULT_MODELS))
    parser.add_argument("--scenarios", default=",".join(DEFAULT_SCENARIOS))
    parser.add_argument("--arms", default=",".join(DEFAULT_ARMS))
    parser.add_argument("--repeats", type=int, default=1)
    parser.add_argument("--seed", type=int, default=20260726)
    parser.add_argument("--timeout-seconds", type=int, default=1500)
    parser.add_argument("--verifier-timeout-seconds", type=int, default=600)
    parser.add_argument(
        "--raw-profile",
        choices=("native", "reduced-config"),
        default="native",
    )
    parser.add_argument("--baseline-weekly-pct", type=float, required=True)
    parser.add_argument("--max-weekly-increase-pct", type=float, default=30.0)
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()

    models = parse_csv(args.models)
    scenarios = parse_csv(args.scenarios)
    arms = parse_csv(args.arms)
    if args.repeats < 1:
        parser.error("--repeats must be at least 1")
    unknown_models = set(models) - set(DEFAULT_MODELS)
    unknown_scenarios = set(scenarios) - set(DEFAULT_SCENARIOS)
    unknown_arms = set(arms) - set(DEFAULT_ARMS)
    if unknown_models:
        parser.error(f"unsupported models: {sorted(unknown_models)}")
    if unknown_scenarios:
        parser.error(f"unsupported scenarios: {sorted(unknown_scenarios)}")
    if unknown_arms:
        parser.error(f"unsupported arms: {sorted(unknown_arms)}")
    if not args.forge_bin.is_file():
        parser.error(f"Forge binary does not exist: {args.forge_bin}")
    if args.out.exists() and any(args.out.iterdir()):
        parser.error(f"output directory must be absent or empty: {args.out}")

    args.out.mkdir(parents=True, exist_ok=True)
    forge_db = args.out / "forge-benchmark.db"
    trials = plan_trials(models, scenarios, args.repeats, arms, args.seed)
    cap_pct = args.baseline_weekly_pct + args.max_weekly_increase_pct
    suite_manifest = {
        "schema_version": 1,
        "created_at": utc_now(),
        "repo_root": str(REPO_ROOT),
        "forge_commit": tiny_command(("git", "rev-parse", "HEAD")),
        "forge_binary_sha256": sha256_file(args.forge_bin),
        "forge_version": tiny_command((str(args.forge_bin), "--version")),
        "codex_version": tiny_command(("codex", "--version")),
        "models": models,
        "scenarios": scenarios,
        "arms": arms,
        "repeats": args.repeats,
        "seed": args.seed,
        "raw_profile": args.raw_profile,
        "reasoning_effort": "xhigh",
        "baseline_weekly_pct": args.baseline_weekly_pct,
        "max_weekly_increase_pct": args.max_weekly_increase_pct,
        "hard_stop_weekly_pct": cap_pct,
        "trial_order": [dataclasses.asdict(trial) for trial in trials],
        "dry_run": args.dry_run,
    }
    json_dump(args.out / "suite-manifest.json", suite_manifest)

    summaries: list[dict[str, Any]] = []
    index_path = args.out / "trial-index.jsonl"
    stop_reason: str | None = None
    for trial in trials:
        prior_quota = next(
            (
                summary.get("quota")
                for summary in reversed(summaries)
                if summary.get("quota") is not None
            ),
            None,
        )
        prior_pct = quota_used_percent(prior_quota)
        if not args.dry_run and prior_pct is not None and prior_pct >= cap_pct:
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
            out_root=args.out,
            forge_bin=args.forge_bin.resolve(),
            forge_db=forge_db,
            timeout_seconds=args.timeout_seconds,
            verifier_timeout_seconds=args.verifier_timeout_seconds,
            raw_profile=args.raw_profile,
            dry_run=args.dry_run,
        )
        summaries.append(summary)
        with index_path.open("a", encoding="utf-8") as index:
            index.write(json.dumps(summary, sort_keys=True) + "\n")
        report = write_aggregate(args.out, summaries)
        current_pct = quota_used_percent(summary.get("quota"))
        print(
            json.dumps(
                {
                    "event": "trial_complete",
                    "slug": trial.slug,
                    "success": summary.get("success"),
                    "tokens": (summary.get("agent") or {}).get("tokens"),
                    "weekly_pct": current_pct,
                    "aggregate": report["paired_summary"],
                }
            ),
            flush=True,
        )
        if not args.dry_run and current_pct is None:
            stop_reason = (
                f"quota telemetry missing after {trial.slug}; failing closed before "
                "another provider call"
            )
            break
        if not args.dry_run and current_pct >= cap_pct:
            stop_reason = (
                f"weekly hard stop reached after {trial.slug}: "
                f"{current_pct:.3f}% >= {cap_pct:.3f}%"
            )
            break

    final = write_aggregate(args.out, summaries)
    run_status = {
        "completed_at": utc_now(),
        "planned_trials": len(trials),
        "completed_trials": len(summaries),
        "stop_reason": stop_reason,
        "hard_stop_weekly_pct": cap_pct,
        "last_quota": next(
            (
                summary.get("quota")
                for summary in reversed(summaries)
                if summary.get("quota") is not None
            ),
            None,
        ),
        "paired_summary": final["paired_summary"],
    }
    json_dump(args.out / "run-status.json", run_status)
    print(json.dumps({"event": "run_complete", **run_status}), flush=True)
    return 0 if stop_reason is None or "hard stop reached" in stop_reason else 2


if __name__ == "__main__":
    raise SystemExit(main())
