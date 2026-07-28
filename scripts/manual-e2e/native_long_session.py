#!/usr/bin/env python3
"""Run one history-continuous native CLI stress turn at a time.

The runner deliberately separates provider turns from quota refreshes. An external
quota authority (Helm in the maintained benchmark) records a fresh observation
before each call and immediately after it. The next call fails closed when that
observation is absent, stale, or at the configured cumulative cap.
"""

from __future__ import annotations

import argparse
import ast
import datetime as dt
import hashlib
import json
import os
import re
import shutil
import signal
import subprocess
import sys
import time
import uuid
from pathlib import Path
from typing import Any, Sequence


ROOT = Path(__file__).resolve().parents[2]
SUITE = ROOT / "scripts" / "manual-e2e"
DEFAULT_SCENARIO = "long-session-reservations"
STATE_FILE = "run-state.json"
CLAUDE_BENCH_SETTINGS = json.dumps(
    {
        "sandbox": {
            "enabled": True,
            "autoAllowBashIfSandboxed": True,
            "network": {
                "allowedDomains": [],
                "strictAllowlist": True,
            },
        }
    },
    separators=(",", ":"),
    sort_keys=True,
)
INTEGRITY_PATTERNS = {
    "web_tool": re.compile(r"\b(WebFetch|WebSearch|browser(_use)?)\b", re.IGNORECASE),
    "url": re.compile(r"https?://", re.IGNORECASE),
    "network_cli": re.compile(
        r"(^|[\s;&|])(?:/usr/bin/|/bin/)?(?:curl|wget|aria2c|ftp|ssh|scp)\b",
        re.IGNORECASE,
    ),
    "remote_git": re.compile(
        r"\bgit\s+(?:clone|fetch|pull|ls-remote)\b|\bgh\s+",
        re.IGNORECASE,
    ),
    "package_network": re.compile(
        r"\b(?:pip|pip3|npm|pnpm|yarn)\s+(?:install|add)\b|\bcargo\s+(?:install|update)\b",
        re.IGNORECASE,
    ),
}
PATCH_PATHS = (
    ".",
    ":(exclude).forge/**",
    ":(exclude).claude/**",
    ":(exclude)**/__pycache__/**",
    ":(exclude)**/*.pyc",
    ":(exclude)**/*.pyo",
)
LOCAL_USAGE_ERROR = re.compile(
    r"(?im)^error: (?:unexpected argument|unrecognized (?:argument|option)|"
    r"invalid value|required arguments? (?:were|was) not provided).*$"
)
LOCAL_USAGE_BANNER = re.compile(r"(?im)^Usage:\s+(?:codex|claude)\b")


def utc_now() -> str:
    return dt.datetime.now(dt.timezone.utc).isoformat()


def dump_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(
        json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    temporary.replace(path)


def load_json(path: Path) -> Any:
    return json.loads(path.read_text(encoding="utf-8"))


def tiny_command(argv: Sequence[str], *, cwd: Path | None = None) -> str:
    completed = subprocess.run(
        argv,
        cwd=cwd,
        check=False,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        timeout=20,
    )
    return completed.stdout.strip()


def git(
    workspace: Path, *args: str, check: bool = True
) -> subprocess.CompletedProcess[bytes]:
    completed = subprocess.run(
        ("git", "-C", str(workspace), *args),
        check=False,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    if check and completed.returncode != 0:
        raise RuntimeError(
            f"git {' '.join(args)} failed: "
            f"{completed.stderr.decode('utf-8', 'replace').strip()}"
        )
    return completed


def prompt_digest(prompt: str) -> str:
    return hashlib.sha256(prompt.encode()).hexdigest()


def prepare_workspace(scenario_dir: Path, workspace: Path) -> str:
    source_marker = scenario_dir / "fixture.source"
    fixture = scenario_dir / "fixture"
    if source_marker.exists():
        fixture = (
            scenario_dir / source_marker.read_text(encoding="utf-8").strip()
        ).resolve()
        scenario_root = (SUITE / "scenarios").resolve()
        if scenario_root not in fixture.parents:
            raise ValueError(f"fixture.source escapes the scenario suite: {fixture}")
    if not fixture.is_dir():
        raise FileNotFoundError(f"scenario fixture is missing: {fixture}")

    shutil.copytree(
        fixture,
        workspace,
        ignore=shutil.ignore_patterns(
            ".git",
            ".forge",
            ".claude",
            "__pycache__",
            "*.pyc",
            "*.pyo",
        ),
    )
    git(workspace, "init", "-q")
    git(workspace, "config", "user.email", "native-stress@local.test")
    git(workspace, "config", "user.name", "Native Stress Baseline")
    git(workspace, "add", "-A")
    git(workspace, "commit", "-qm", "native stress synthetic baseline")
    baseline = tiny_command(("git", "rev-parse", "HEAD"), cwd=workspace)
    git(workspace, "remote", "remove", "origin", check=False)
    exclude = workspace / ".git" / "info" / "exclude"
    exclude.write_text(
        ".forge/\n.claude/\n__pycache__/\n*.pyc\n",
        encoding="utf-8",
    )
    reachable = tiny_command(("git", "rev-list", "--all", "--count"), cwd=workspace)
    remotes = tiny_command(("git", "remote"), cwd=workspace)
    if reachable != "1" or remotes:
        raise RuntimeError(
            f"history isolation failed: reachable={reachable!r}, remotes={remotes!r}"
        )
    return baseline


def codex_argv(
    model: str,
    effort: str,
    prompt: str,
    workspace: Path,
    session_id: str | None,
) -> list[str]:
    common = [
        "--json",
        "--disable",
        "standalone_web_search",
        "--skip-git-repo-check",
        "-m",
        model,
        "-c",
        f'model_reasoning_effort="{effort}"',
        "-c",
        'sandbox_mode="workspace-write"',
    ]
    if session_id is None:
        return [
            "codex",
            "exec",
            *common,
            "--sandbox",
            "workspace-write",
            "-C",
            str(workspace),
            prompt,
        ]
    return [
        "codex",
        "exec",
        "resume",
        *common,
        session_id,
        prompt,
    ]


def claude_argv(
    model: str,
    effort: str,
    prompt: str,
    session_id: str,
    first_turn: bool,
) -> list[str]:
    argv = [
        "claude",
        "-p",
        "--output-format",
        "stream-json",
        "--verbose",
        "--include-partial-messages",
        "--dangerously-skip-permissions",
        "--disallowedTools",
        "WebFetch,WebSearch",
        "--strict-mcp-config",
        "--settings",
        CLAUDE_BENCH_SETTINGS,
        "--effort",
        effort,
        "--model",
        model,
    ]
    if first_turn:
        argv.extend(("--session-id", session_id))
    else:
        argv.extend(("--resume", session_id))
    argv.append(prompt)
    return argv


def parse_jsonl(path: Path) -> tuple[list[dict[str, Any]], int]:
    events: list[dict[str, Any]] = []
    malformed = 0
    if not path.exists():
        return events, malformed
    with path.open(encoding="utf-8", errors="replace") as handle:
        for line in handle:
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


def codex_tokens(usage: dict[str, Any] | None) -> dict[str, float | int | None]:
    if not usage:
        return empty_tokens()
    input_tokens = int(usage.get("input_tokens") or 0)
    cached = int(usage.get("cached_input_tokens") or 0)
    output = int(usage.get("output_tokens") or 0)
    return token_metrics(input_tokens, cached, output)


def claude_tokens(usage: dict[str, Any] | None) -> dict[str, float | int | None]:
    if not usage:
        return empty_tokens()
    uncached = int(usage.get("input_tokens") or 0)
    cache_read = int(usage.get("cache_read_input_tokens") or 0)
    cache_creation = int(usage.get("cache_creation_input_tokens") or 0)
    output = int(usage.get("output_tokens") or 0)
    result = token_metrics(uncached + cache_read + cache_creation, cache_read, output)
    result["cache_creation_input_tokens"] = cache_creation
    return result


def empty_tokens() -> dict[str, float | int | None]:
    return {
        "input_tokens": None,
        "cached_input_tokens": None,
        "uncached_input_tokens": None,
        "output_tokens": None,
        "total_tokens": None,
        "cache_adjusted_tokens_025": None,
        "cache_zero_credit_tokens": None,
    }


def token_metrics(
    input_tokens: int, cached: int, output: int
) -> dict[str, float | int]:
    uncached = max(0, input_tokens - cached)
    return {
        "input_tokens": input_tokens,
        "cached_input_tokens": cached,
        "uncached_input_tokens": uncached,
        "output_tokens": output,
        "total_tokens": input_tokens + output,
        # Sensitivity metric used by Forge's canonical harness benchmark.
        "cache_adjusted_tokens_025": round(uncached + cached * 0.25 + output, 2),
        # Lower bound showing only newly processed input plus output.
        "cache_zero_credit_tokens": uncached + output,
    }


def summarize_codex(path: Path) -> dict[str, Any]:
    events, malformed = parse_jsonl(path)
    started = next(
        (event for event in events if event.get("type") == "thread.started"), {}
    )
    completed = next(
        (event for event in reversed(events) if event.get("type") == "turn.completed"),
        {},
    )
    final = next(
        (
            event.get("item", {}).get("text")
            for event in reversed(events)
            if event.get("type") == "item.completed"
            and event.get("item", {}).get("type") == "agent_message"
        ),
        None,
    )
    tool_calls = sum(
        event.get("type") == "item.completed"
        and event.get("item", {}).get("type")
        in {"command_execution", "mcp_tool_call", "file_change"}
        for event in events
    )
    return {
        "session_id": started.get("thread_id"),
        "completed": bool(completed),
        "result": final,
        "event_count": len(events),
        "malformed_event_lines": malformed,
        "tool_calls": tool_calls,
        "tokens": codex_tokens(completed.get("usage")),
    }


def codex_rollout_context(session_id: str | None) -> dict[str, Any]:
    if not session_id:
        return {}
    roots = [
        Path.home() / ".codex" / "sessions",
        Path.home() / ".codex" / "archived_sessions",
    ]
    candidates = [
        path
        for root in roots
        if root.exists()
        for path in root.glob(f"**/*{session_id}*.jsonl")
    ]
    for path in sorted(candidates, key=lambda item: item.stat().st_mtime, reverse=True):
        latest: dict[str, Any] = {}
        with path.open(encoding="utf-8", errors="replace") as handle:
            for line in handle:
                try:
                    event = json.loads(line)
                except json.JSONDecodeError:
                    continue
                if event.get("type") != "turn_context":
                    continue
                payload = event.get("payload")
                if isinstance(payload, dict):
                    latest = {
                        "resolved_model": payload.get("model"),
                        "resolved_effort": payload.get("effort")
                        or payload.get("model_reasoning_effort"),
                    }
        if latest:
            return latest
    return {}


def summarize_claude(path: Path) -> dict[str, Any]:
    events, malformed = parse_jsonl(path)
    result = next(
        (event for event in reversed(events) if event.get("type") == "result"),
        {},
    )
    tool_calls = 0
    for event in events:
        if event.get("type") != "assistant":
            continue
        content = (event.get("message") or {}).get("content")
        if isinstance(content, list):
            tool_calls += sum(
                isinstance(item, dict) and item.get("type") == "tool_use"
                for item in content
            )
    return {
        "session_id": result.get("session_id"),
        "completed": bool(result) and not bool(result.get("is_error")),
        "result": result.get("result"),
        "resolved_models": sorted((result.get("modelUsage") or {}).keys()),
        "provider_calls": result.get("num_turns"),
        "event_count": len(events),
        "malformed_event_lines": malformed,
        "tool_calls": tool_calls,
        "tokens": claude_tokens(result.get("usage")),
    }


def run_capture(
    argv: Sequence[str],
    *,
    cwd: Path,
    stdout_path: Path,
    stderr_path: Path,
    timeout: int,
) -> dict[str, Any]:
    started_wall = utc_now()
    started = time.monotonic()
    with stdout_path.open("wb") as stdout, stderr_path.open("wb") as stderr:
        process = subprocess.Popen(
            argv,
            cwd=cwd,
            stdout=stdout,
            stderr=stderr,
            start_new_session=True,
        )
        timed_out = False
        try:
            exit_code = process.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            timed_out = True
            os.killpg(process.pid, signal.SIGTERM)
            try:
                exit_code = process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                exit_code = process.wait()
    return {
        "started_at": started_wall,
        "ended_at": utc_now(),
        "wall_seconds": round(time.monotonic() - started, 3),
        "exit_code": exit_code,
        "timed_out": timed_out,
        "argv": list(argv),
    }


def parse_observed_at(value: Any, label: str) -> dt.datetime:
    if not value:
        raise ValueError(f"{label} is missing")
    observed = dt.datetime.fromisoformat(str(value).replace("Z", "+00:00"))
    if observed.tzinfo is None:
        raise ValueError(f"{label} must include a timezone")
    return observed.astimezone(dt.timezone.utc)


def quota_is_fresh(
    state: dict[str, Any],
    max_age_seconds: int,
    *,
    for_paid_call: bool = True,
) -> tuple[bool, str]:
    observations = state.get("quota_observations") or []
    if not observations:
        return False, "no external quota observation recorded"
    latest = observations[-1]
    if int(latest.get("after_turn", -1)) != len(state.get("turns") or []):
        return (
            False,
            "a post-turn external quota refresh is required before another paid call",
        )
    try:
        recorded = parse_observed_at(latest.get("recorded_at"), "recorded_at")
        externally_observed = parse_observed_at(
            latest.get("external_observed_at"), "external_observed_at"
        )
    except (TypeError, ValueError) as error:
        return False, f"invalid quota observation: {error}"
    now = dt.datetime.now(dt.timezone.utc)
    recorded_age = (now - recorded).total_seconds()
    external_age = (now - externally_observed).total_seconds()
    if recorded_age < -60 or external_age < -60:
        return False, "quota observation is dated in the future"
    if recorded_age > max_age_seconds or external_age > max_age_seconds:
        return (
            False,
            "latest quota observation is stale "
            f"(recorded={recorded_age:.1f}s, external={external_age:.1f}s)",
        )
    turns = state.get("turns") or []
    if turns:
        try:
            last_turn_ended = parse_observed_at(
                turns[-1]["process"].get("ended_at"), "last turn ended_at"
            )
        except (KeyError, TypeError, ValueError) as error:
            return False, f"invalid retained turn timestamp: {error}"
        # Provider rate-limit headers are emitted immediately before CLI teardown; allow a few
        # seconds for process cleanup while still rejecting any pre-turn or mid-turn sample.
        if externally_observed + dt.timedelta(seconds=5) < last_turn_ended:
            return (
                False,
                "external quota observation predates the completed provider turn",
            )
    weekly = float(latest["weekly_utilization_percent"])
    hard_stop = float(state["quota_baseline_percent"]) + float(
        state["quota_cap_delta_points"]
    )
    if weekly > hard_stop:
        return (
            False,
            f"weekly utilization {weekly:.1f}% exceeded hard stop {hard_stop:.1f}%",
        )
    if for_paid_call and weekly >= hard_stop:
        return (
            False,
            f"weekly utilization {weekly:.1f}% reached hard stop {hard_stop:.1f}%",
        )
    if weekly < float(state["quota_baseline_percent"]):
        return False, "quota observation predates or contradicts the recorded baseline"
    if state.get("quota_cap_violation"):
        return False, "a prior external observation exceeded the cumulative quota cap"
    if for_paid_call and state.get("quota_cap_reached"):
        return False, "the cumulative quota cap was already reached"
    return True, ""


def is_preflight_failure(turn: dict[str, Any]) -> bool:
    tokens = (turn.get("agent") or {}).get("tokens") or {}
    process = turn.get("process") or {}
    stderr_path = Path(str(turn.get("turn_dir") or "")) / "stderr.log"
    try:
        stderr = stderr_path.read_text(encoding="utf-8", errors="replace")
    except OSError:
        stderr = ""
    return (
        not bool(turn.get("success"))
        and int((turn.get("agent") or {}).get("event_count") or 0) == 0
        and all(value is None for value in tokens.values())
        and (turn.get("agent") or {}).get("session_id") is None
        and process.get("exit_code") == 2
        and not bool(process.get("timed_out"))
        and float(process.get("wall_seconds") or 0.0) <= 5.0
        and bool(LOCAL_USAGE_ERROR.search(stderr))
        and bool(LOCAL_USAGE_BANNER.search(stderr))
    )


def recover_preflight_failures(state: dict[str, Any]) -> bool:
    """Move zero-event CLI/parser failures aside without erasing their audit record."""
    retained: list[dict[str, Any]] = []
    preflight = state.setdefault("preflight_failures", [])
    changed = False
    for turn in state.get("turns") or []:
        if is_preflight_failure(turn):
            if not any(
                item.get("turn_dir") == turn.get("turn_dir") for item in preflight
            ):
                preflight.append(turn)
            changed = True
        else:
            retained.append(turn)
    if changed:
        state["turns"] = retained
        state["next_turn"] = len(retained)
        state.pop("failed_turn", None)
    return changed


def recover(args: argparse.Namespace) -> int:
    """Persist recovery of zero-provider preflight failures without starting a paid arm."""

    state_path = args.run_dir.resolve() / STATE_FILE
    state = load_json(state_path)
    changed = recover_preflight_failures(state)
    if changed:
        dump_json(state_path, state)
    print(
        json.dumps(
            {
                "changed": changed,
                "next_turn": state["next_turn"],
                "preflight_failures": len(state.get("preflight_failures") or []),
            },
            sort_keys=True,
        )
    )
    return 0


def is_zero_token_auth_failure(turn: dict[str, Any]) -> bool:
    agent = turn.get("agent") or {}
    process = turn.get("process") or {}
    tokens = agent.get("tokens") or {}
    result = str(agent.get("result") or "")
    return (
        not bool(turn.get("success"))
        and process.get("exit_code") == 1
        and not bool(process.get("timed_out"))
        and 0 < int(agent.get("event_count") or 0)
        and int(agent.get("provider_calls") or 0) <= 1
        and float(tokens.get("total_tokens") or 0) == 0
        and "failed to authenticate" in result.lower()
    )


def claude_authenticated() -> bool:
    completed = subprocess.run(
        ("claude", "auth", "status", "--json"),
        check=False,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        text=True,
        timeout=20,
    )
    if completed.returncode != 0:
        return False
    try:
        status = json.loads(completed.stdout)
    except json.JSONDecodeError:
        return False
    return status.get("loggedIn") is True


def recover_auth(args: argparse.Namespace) -> int:
    """Unlock one proven zero-token auth failure only after auth is restored."""

    state_path = args.run_dir.resolve() / STATE_FILE
    state = load_json(state_path)
    failure = state.get("paid_failure")
    if not isinstance(failure, dict) or not is_zero_token_auth_failure(failure):
        raise RuntimeError("no proven zero-token authentication failure is retained")
    if state.get("provider") != "claude" or not claude_authenticated():
        raise RuntimeError("Claude authentication is not restored")
    failures = state.setdefault("authentication_failures", [])
    if not any(item.get("turn_dir") == failure.get("turn_dir") for item in failures):
        failures.append(failure)
    state.pop("paid_failure", None)
    dump_json(state_path, state)
    print(
        json.dumps(
            {
                "recovered": True,
                "next_turn": state["next_turn"],
                "authentication_failures": len(failures),
            },
            sort_keys=True,
        )
    )
    return 0


def prepare(args: argparse.Namespace) -> int:
    scenario_dir = SUITE / "scenarios" / args.scenario
    prompts_path = scenario_dir / "prompts.json"
    prompts = load_json(prompts_path)
    if (
        not isinstance(prompts, list)
        or len(prompts) != 6
        or not all(isinstance(prompt, str) and prompt.strip() for prompt in prompts)
    ):
        raise ValueError("the matched stress scenario must contain exactly six prompts")

    stamp = dt.datetime.now(dt.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    run_dir = (
        args.out_root.resolve()
        / f"{args.scenario}-native-{args.provider}-{stamp}-{os.getpid()}"
    )
    run_dir.mkdir(parents=True)
    workspace = run_dir / "workspace"
    baseline = prepare_workspace(scenario_dir, workspace)
    session_id = str(uuid.uuid4()) if args.provider == "claude" else None
    version = tiny_command(
        ("codex", "--version") if args.provider == "codex" else ("claude", "--version")
    )
    state = {
        "schema_version": 1,
        "provider": args.provider,
        "requested_model": args.model,
        "effort": args.effort,
        "cli_version": version,
        "scenario": args.scenario,
        "scenario_prompt_sha256": [prompt_digest(prompt) for prompt in prompts],
        "workspace": str(workspace),
        "synthetic_base_commit": baseline,
        "synthetic_base_tree": tiny_command(
            ("git", "rev-parse", f"{baseline}^{{tree}}"), cwd=workspace
        ),
        "reachable_commit_count": 1,
        "remote_count": 0,
        "session_id": session_id,
        "next_turn": 0,
        "turns": [],
        "preflight_failures": [],
        "authentication_failures": [],
        "quota_baseline_percent": args.quota_baseline,
        "quota_cap_delta_points": args.quota_cap,
        "quota_observations": [],
        "created_at": utc_now(),
    }
    dump_json(run_dir / STATE_FILE, state)
    print(run_dir)
    return 0


def record_quota(args: argparse.Namespace) -> int:
    state_path = args.run_dir.resolve() / STATE_FILE
    state = load_json(state_path)
    requested_cap = getattr(args, "quota_cap", None)
    if requested_cap is not None:
        current_cap = float(state["quota_cap_delta_points"])
        requested_cap = float(requested_cap)
        if requested_cap <= 0:
            raise ValueError("quota cap delta must be positive")
        if requested_cap > current_cap:
            raise ValueError(
                "refusing to loosen an active run's cumulative quota cap "
                f"from {current_cap:g} to {requested_cap:g} percentage points"
            )
        state["quota_cap_delta_points"] = requested_cap
    observation = {
        "recorded_at": utc_now(),
        "external_observed_at": args.external_observed_at,
        "weekly_utilization_percent": args.weekly,
        "source": args.source,
        "after_turn": len(state["turns"]),
    }
    state["quota_observations"].append(observation)
    hard_stop = float(state["quota_baseline_percent"]) + float(
        state["quota_cap_delta_points"]
    )
    if args.weekly > hard_stop:
        state["quota_cap_violation"] = {
            "observed": args.weekly,
            "hard_stop": hard_stop,
            "recorded_at": observation["recorded_at"],
        }
    elif args.weekly >= hard_stop:
        state["quota_cap_reached"] = {
            "observed": args.weekly,
            "hard_stop": hard_stop,
            "recorded_at": observation["recorded_at"],
        }
    dump_json(state_path, state)
    print(json.dumps(observation, sort_keys=True))
    return 0


def run_turn(args: argparse.Namespace) -> int:
    run_dir = args.run_dir.resolve()
    state_path = run_dir / STATE_FILE
    state = load_json(state_path)
    if recover_preflight_failures(state):
        dump_json(state_path, state)
    if state.get("paid_failure"):
        raise RuntimeError(
            "a provider-started attempt failed; refusing to duplicate the paid call"
        )
    valid, reason = quota_is_fresh(state, args.quota_max_age)
    if not valid:
        raise RuntimeError(f"paid call denied: {reason}")
    turn_index = int(state["next_turn"])
    prompts = load_json(SUITE / "scenarios" / state["scenario"] / "prompts.json")
    if turn_index >= len(prompts):
        raise RuntimeError("all six turns already completed")
    if len(state["turns"]) != turn_index:
        raise RuntimeError(
            "run state is inconsistent; refusing to duplicate a provider call"
        )

    prior_attempts = sum(
        int(attempt.get("turn") or 0) == turn_index + 1
        for attempt in state.get("preflight_failures") or []
    )
    directory_name = (
        f"{turn_index + 1:02d}"
        if prior_attempts == 0
        else f"{turn_index + 1:02d}-attempt-{prior_attempts + 1:02d}"
    )
    turn_dir = run_dir / "turns" / directory_name
    turn_dir.mkdir(parents=True, exist_ok=False)
    prompt = prompts[turn_index]
    workspace = Path(state["workspace"])
    if state["provider"] == "codex":
        argv = codex_argv(
            state["requested_model"],
            state["effort"],
            prompt,
            workspace,
            state.get("session_id"),
        )
    else:
        argv = claude_argv(
            state["requested_model"],
            state["effort"],
            prompt,
            state["session_id"],
            turn_index == 0,
        )
    dump_json(
        turn_dir / "manifest.json",
        {
            "turn": turn_index + 1,
            "prompt_sha256": prompt_digest(prompt),
            "session_id_before": state.get("session_id"),
            "workspace_head": tiny_command(("git", "rev-parse", "HEAD"), cwd=workspace),
            "argv": argv,
        },
    )
    process = run_capture(
        argv,
        cwd=workspace,
        stdout_path=turn_dir / "events.jsonl",
        stderr_path=turn_dir / "stderr.log",
        timeout=args.timeout,
    )
    agent = (
        summarize_codex(turn_dir / "events.jsonl")
        if state["provider"] == "codex"
        else summarize_claude(turn_dir / "events.jsonl")
    )
    if turn_index == 0 and state["provider"] == "codex":
        state["session_id"] = agent.get("session_id")
    if state["provider"] == "codex":
        agent.update(codex_rollout_context(state.get("session_id")))
    same_session = bool(state.get("session_id")) and agent.get(
        "session_id"
    ) == state.get("session_id")
    success = (
        process["exit_code"] == 0
        and not process["timed_out"]
        and agent["completed"]
        and same_session
    )
    turn = {
        "turn": turn_index + 1,
        "process": process,
        "agent": agent,
        "same_session": same_session,
        "success": success,
        "turn_dir": str(turn_dir),
    }
    dump_json(turn_dir / "summary.json", turn)
    if success:
        state["turns"].append(turn)
        state["next_turn"] = turn_index + 1
    else:
        if is_preflight_failure(turn):
            state.setdefault("preflight_failures", []).append(turn)
        else:
            state["paid_failure"] = turn
    dump_json(state_path, state)
    print(json.dumps(turn, sort_keys=True))
    return 0 if success else 1


def tool_invocations(event: dict[str, Any]) -> list[tuple[str, str]]:
    """Extract executed tool names/inputs without scanning prose or tool output."""
    invocations: list[tuple[str, str]] = []
    if event.get("type") == "item.completed":
        item = event.get("item") or {}
        item_type = str(item.get("type") or "")
        if item_type == "command_execution":
            invocations.append(("command", str(item.get("command") or "")))
        elif item_type in {"web_search", "web_fetch"}:
            invocations.append(("external_tool", item_type))
        elif item_type in {"mcp_tool_call", "dynamic_tool_call"}:
            identity = "::".join(
                str(item.get(key) or "") for key in ("server", "name", "tool_name")
            )
            arguments = item.get("arguments", item.get("input", item.get("args", {})))
            invocations.append(
                (
                    "tool",
                    f"{identity}\n{json.dumps(arguments, sort_keys=True, default=str)}",
                )
            )
    elif event.get("type") == "assistant":
        content = (event.get("message") or {}).get("content")
        if isinstance(content, list):
            for item in content:
                if not isinstance(item, dict) or item.get("type") != "tool_use":
                    continue
                name = str(item.get("name") or "")
                value = item.get("input", {})
                rendered = json.dumps(value, sort_keys=True, default=str)
                kind = "command" if name.lower() in {"bash", "shell"} else "tool"
                invocation = (
                    str(value.get("command", value.get("cmd", rendered)))
                    if kind == "command" and isinstance(value, dict)
                    else f"{name}\n{rendered}"
                )
                invocations.append((kind, invocation))
    return invocations


def integrity_kinds(kind: str, text: str) -> list[str]:
    findings: list[str] = []
    lowered = text.lower()
    if kind == "tool" and (
        re.search(r"\b(webfetch|websearch|browser(?:_use)?)\b", text, re.IGNORECASE)
        or any(marker in lowered for marker in ("mcp__github", "github::", "gitlab::"))
    ):
        findings.append("external_tool")
    if kind == "command":
        for label in ("url", "network_cli", "remote_git", "package_network"):
            if INTEGRITY_PATTERNS[label].search(text):
                findings.append(label)
    return findings


def integrity_audit(run_dir: Path) -> dict[str, Any]:
    findings: list[dict[str, Any]] = []
    events_seen = 0
    for path in sorted((run_dir / "turns").glob("*/events.jsonl")):
        events, _ = parse_jsonl(path)
        events_seen += len(events)
        manifest_path = path.parent / "manifest.json"
        if manifest_path.exists():
            turn_number = int(load_json(manifest_path)["turn"])
        else:
            match = re.match(r"\d+", path.parent.name)
            if match is None:
                raise ValueError(
                    f"cannot derive turn number for integrity evidence at {path}"
                )
            turn_number = int(match.group())
        for line_number, event in enumerate(events, 1):
            for invocation_kind, text in tool_invocations(event):
                for finding_kind in integrity_kinds(invocation_kind, text):
                    findings.append(
                        {
                            "turn": turn_number,
                            "event": line_number,
                            "kind": finding_kind,
                            "invocation_kind": invocation_kind,
                            "text_sha256": hashlib.sha256(text.encode()).hexdigest(),
                        }
                    )
    return {
        "valid": not findings,
        "events_audited": events_seen,
        "findings": findings,
    }


def public_signatures(root: Path) -> dict[str, list[str]]:
    signatures: dict[str, list[str]] = {}
    for path in sorted((root / "reservations").glob("*.py")):
        rows: list[str] = []
        tree = ast.parse(path.read_text(encoding="utf-8"))

        class SignatureVisitor(ast.NodeVisitor):
            def __init__(self) -> None:
                self.classes: list[str] = []

            def visit_ClassDef(self, node: ast.ClassDef) -> None:
                if not node.name.startswith("_"):
                    identity = ".".join([*self.classes, node.name])
                    bases = ",".join(
                        ast.dump(base, include_attributes=False) for base in node.bases
                    )
                    decorators = ",".join(
                        ast.dump(value, include_attributes=False)
                        for value in node.decorator_list
                    )
                    rows.append(
                        f"class:{identity}:bases={bases}:decorators={decorators}"
                    )
                self.classes.append(node.name)
                for member in node.body:
                    self.visit(member)
                self.classes.pop()

            def visit_FunctionDef(self, node: ast.FunctionDef) -> None:
                self.record_function(node, "def")

            def visit_AsyncFunctionDef(self, node: ast.AsyncFunctionDef) -> None:
                self.record_function(node, "async-def")

            def record_function(
                self, node: ast.FunctionDef | ast.AsyncFunctionDef, kind: str
            ) -> None:
                if node.name.startswith("_"):
                    return
                identity = ".".join([*self.classes, node.name])
                arguments = ast.dump(node.args, include_attributes=False)
                returns = (
                    ast.dump(node.returns, include_attributes=False)
                    if node.returns is not None
                    else ""
                )
                decorators = ",".join(
                    ast.dump(value, include_attributes=False)
                    for value in node.decorator_list
                )
                rows.append(
                    f"{kind}:{identity}:args={arguments}:returns={returns}:"
                    f"decorators={decorators}"
                )

        SignatureVisitor().visit(tree)
        signatures[path.name] = sorted(rows)
    return signatures


def original_tests(root: Path) -> dict[str, str]:
    result: dict[str, str] = {}
    for path in sorted((root / "tests").glob("test*.py")):
        tree = ast.parse(path.read_text(encoding="utf-8"))
        for node in tree.body:
            if isinstance(
                node, (ast.FunctionDef, ast.AsyncFunctionDef)
            ) and node.name.startswith("test"):
                result[f"test:{path.name}:{node.name}"] = ast.dump(
                    node, include_attributes=False
                )
            elif isinstance(node, ast.ClassDef):
                class_metadata = (
                    tuple(
                        ast.dump(base, include_attributes=False) for base in node.bases
                    ),
                    tuple(
                        ast.dump(keyword, include_attributes=False)
                        for keyword in node.keywords
                    ),
                    tuple(
                        ast.dump(decorator, include_attributes=False)
                        for decorator in node.decorator_list
                    ),
                )
                result[f"class-meta:{path.name}:{node.name}"] = repr(class_metadata)
                for member in node.body:
                    if isinstance(
                        member, (ast.FunctionDef, ast.AsyncFunctionDef)
                    ) and member.name.startswith("test"):
                        result[f"test:{path.name}:{node.name}.{member.name}"] = (
                            ast.dump(member, include_attributes=False)
                        )
    return result


def preserved_test_contract(root: Path) -> dict[str, str]:
    """Fingerprint original tests plus their setup/helper methods.

    Test methods alone are insufficient evidence that a suite was not weakened: changing
    ``setUp`` or a class helper can make the same assertions exercise much less. Add every
    pre-existing non-test function and method to the subset comparison while still allowing the
    agent to append new tests and helpers.
    """

    result = original_tests(root)
    for path in sorted((root / "tests").glob("test*.py")):
        tree = ast.parse(path.read_text(encoding="utf-8"))
        for node in tree.body:
            if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
                if not node.name.startswith("test"):
                    result[f"support:{path.name}:{node.name}"] = ast.dump(
                        node, include_attributes=False
                    )
                continue
            if not isinstance(node, ast.ClassDef):
                continue
            for member in node.body:
                if isinstance(member, (ast.FunctionDef, ast.AsyncFunctionDef)):
                    if not member.name.startswith("test"):
                        result[f"support:{path.name}:{node.name}.{member.name}"] = ast.dump(
                            member, include_attributes=False
                        )
    return result


def run_verifier(argv: Sequence[str], *, cwd: Path, path: Path) -> dict[str, Any]:
    started = time.monotonic()
    process = subprocess.Popen(
        argv,
        cwd=cwd,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        start_new_session=True,
    )
    timed_out = False
    try:
        stdout, _ = process.communicate(timeout=120)
    except subprocess.TimeoutExpired:
        timed_out = True
        os.killpg(process.pid, signal.SIGTERM)
        try:
            stdout, _ = process.communicate(timeout=10)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            stdout, _ = process.communicate()
    path.write_bytes(stdout)
    output = stdout.decode("utf-8", "replace")
    return {
        "argv": list(argv),
        "exit_code": process.returncode,
        "timed_out": timed_out,
        "wall_seconds": round(time.monotonic() - started, 3),
        "output_sha256": hashlib.sha256(stdout).hexdigest(),
        "tests_run": (
            int(match.group(1))
            if (match := re.search(r"Ran\s+(\d+)\s+tests?", output))
            else None
        ),
        "tests_skipped": (
            int(match.group(1)) if (match := re.search(r"skipped=(\d+)", output)) else 0
        ),
    }


def capture_patch(workspace: Path, run_dir: Path, baseline: str) -> dict[str, Any]:
    git(workspace, "add", "-N", "--", *PATCH_PATHS)
    patch = git(workspace, "diff", "--binary", baseline, "--", *PATCH_PATHS)
    patch_path = run_dir / "changes.patch"
    patch_path.write_bytes(patch.stdout)
    status = tiny_command(
        ("git", "status", "--short", "--", *PATCH_PATHS), cwd=workspace
    ).splitlines()
    (run_dir / "git-status.txt").write_text(
        "".join(f"{row}\n" for row in status),
        encoding="utf-8",
    )
    return {
        "bytes": len(patch.stdout),
        "sha256": hashlib.sha256(patch.stdout).hexdigest(),
        "diff_check": (
            git(
                workspace,
                "diff",
                "--check",
                baseline,
                "--",
                *PATCH_PATHS,
            ).returncode
            == 0
        ),
        "status": status,
    }


def aggregate_tokens(turns: list[dict[str, Any]]) -> dict[str, Any]:
    fields = [
        "input_tokens",
        "cached_input_tokens",
        "uncached_input_tokens",
        "output_tokens",
        "total_tokens",
        "cache_adjusted_tokens_025",
        "cache_zero_credit_tokens",
    ]
    totals: dict[str, Any] = {}
    for field in fields:
        values = [turn["agent"]["tokens"].get(field) for turn in turns]
        totals[field] = (
            round(sum(float(value) for value in values), 2)
            if values and all(value is not None for value in values)
            else None
        )
    totals["turns_reported"] = sum(
        turn["agent"]["tokens"].get("total_tokens") is not None for turn in turns
    )
    totals["turns_expected"] = len(turns)
    totals["complete"] = totals["turns_reported"] == len(turns)
    return totals


def finalize(args: argparse.Namespace) -> int:
    run_dir = args.run_dir.resolve()
    state = load_json(run_dir / STATE_FILE)
    if state["next_turn"] != 6 or len(state["turns"]) != 6:
        raise RuntimeError("cannot finalize an incomplete six-turn native session")
    if not all(turn["success"] for turn in state["turns"]):
        raise RuntimeError("cannot finalize: at least one retained turn failed")
    fresh, reason = quota_is_fresh(state, 86_400, for_paid_call=False)
    if not fresh:
        raise RuntimeError(f"cannot finalize before the final quota refresh: {reason}")
    workspace = Path(state["workspace"])
    scenario_dir = SUITE / "scenarios" / state["scenario"]
    fixture_marker = scenario_dir / "fixture.source"
    fixture = (
        (scenario_dir / fixture_marker.read_text(encoding="utf-8").strip()).resolve()
        if fixture_marker.exists()
        else scenario_dir / "fixture"
    )

    visible = run_verifier(
        (sys.executable, "-m", "unittest", "discover", "-v"),
        cwd=workspace,
        path=run_dir / "visible-tests.log",
    )
    original_visible = run_verifier(
        (
            sys.executable,
            "-m",
            "unittest",
            "discover",
            "-s",
            str(fixture / "tests"),
            "-v",
        ),
        cwd=workspace,
        path=run_dir / "original-tests-replay.log",
    )
    hidden = run_verifier(
        (sys.executable, str(scenario_dir / "verify.py"), str(workspace)),
        cwd=workspace,
        path=run_dir / "hidden-tests.log",
    )
    patch = capture_patch(workspace, run_dir, state["synthetic_base_commit"])
    integrity = integrity_audit(run_dir)
    remotes = tiny_command(("git", "remote"), cwd=workspace).splitlines()
    base_is_ancestor = (
        git(
            workspace,
            "merge-base",
            "--is-ancestor",
            state["synthetic_base_commit"],
            "HEAD",
            check=False,
        ).returncode
        == 0
    )
    signatures_unchanged = public_signatures(fixture) == public_signatures(workspace)
    base_tests = preserved_test_contract(fixture)
    final_tests = preserved_test_contract(workspace)
    preserved_tests = all(
        final_tests.get(name) == value for name, value in base_tests.items()
    )
    original_test_methods_present = all(
        name in final_tests for name in base_tests if name.startswith("test:")
    )
    original_test_support_preserved = all(
        final_tests.get(name) == value
        for name, value in base_tests.items()
        if not name.startswith("test:")
    )
    original_tests_not_weakened = (
        original_visible["exit_code"] == 0
        and not original_visible["timed_out"]
        and original_visible["tests_skipped"] == 0
        and original_test_methods_present
        and original_test_support_preserved
    )
    original_test_count = sum(name.startswith("test:") for name in base_tests)
    final_test_count = sum(name.startswith("test:") for name in final_tests)
    tokens = aggregate_tokens(state["turns"])
    all_passed = (
        visible["exit_code"] == 0
        and not visible["timed_out"]
        and visible["tests_skipped"] == 0
        and original_tests_not_weakened
        and hidden["exit_code"] == 0
        and not hidden["timed_out"]
        and patch["diff_check"]
        and integrity["valid"]
        and not remotes
        and base_is_ancestor
        and signatures_unchanged
        and tokens["complete"]
    )
    result = {
        "provider": state["provider"],
        "requested_model": state["requested_model"],
        "effort": state["effort"],
        "cli_version": state["cli_version"],
        "resolved_models": sorted(
            {
                model
                for turn in state["turns"]
                for model in (
                    turn["agent"].get("resolved_models", [])
                    + (
                        [turn["agent"]["resolved_model"]]
                        if turn["agent"].get("resolved_model")
                        else []
                    )
                )
            }
        ),
        "resolved_efforts": sorted(
            {
                turn["agent"]["resolved_effort"]
                for turn in state["turns"]
                if turn["agent"].get("resolved_effort")
            }
        ),
        "session_id": state["session_id"],
        "same_session_all_turns": all(turn["same_session"] for turn in state["turns"]),
        "turns_completed": sum(turn["success"] for turn in state["turns"]),
        "turn_wall_seconds": [
            turn["process"]["wall_seconds"] for turn in state["turns"]
        ],
        "total_turn_wall_seconds": round(
            sum(turn["process"]["wall_seconds"] for turn in state["turns"]), 3
        ),
        "preflight_failures": state.get("preflight_failures") or [],
        "authentication_failures": state.get("authentication_failures") or [],
        "total_attempt_wall_seconds": round(
            sum(turn["process"]["wall_seconds"] for turn in state["turns"])
            + sum(
                attempt["process"]["wall_seconds"]
                for attempt in state.get("preflight_failures") or []
            )
            + sum(
                attempt["process"]["wall_seconds"]
                for attempt in state.get("authentication_failures") or []
            ),
            3,
        ),
        "tokens": tokens,
        "visible_tests": visible,
        "original_tests_replay": original_visible,
        "hidden_tests": hidden,
        "public_signatures_unchanged": signatures_unchanged,
        "original_tests_preserved": preserved_tests,
        "original_test_methods_present": original_test_methods_present,
        "original_test_support_preserved": original_test_support_preserved,
        "original_tests_not_weakened": original_tests_not_weakened,
        "original_test_count": original_test_count,
        "final_test_count": final_test_count,
        "workspace_remotes": remotes,
        "synthetic_base_is_ancestor": base_is_ancestor,
        "synthetic_base_tree": state.get("synthetic_base_tree")
        or tiny_command(
            (
                "git",
                "rev-parse",
                f"{state['synthetic_base_commit']}^{{tree}}",
            ),
            cwd=workspace,
        ),
        "patch": patch,
        "integrity": integrity,
        "quota": {
            "baseline_percent": state["quota_baseline_percent"],
            "cap_delta_points": state["quota_cap_delta_points"],
            "observations": state["quota_observations"],
            "final_delta_points": (
                float(state["quota_observations"][-1]["weekly_utilization_percent"])
                - float(state["quota_baseline_percent"])
            ),
        },
        "all_acceptance_passed": all_passed,
        "finalized_at": utc_now(),
    }
    dump_json(run_dir / "final-summary.json", result)
    print(json.dumps(result, sort_keys=True))
    return 0 if all_passed else 1


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser()
    commands = root.add_subparsers(dest="command", required=True)

    prepare_parser = commands.add_parser("prepare")
    prepare_parser.add_argument(
        "--provider", choices=("codex", "claude"), required=True
    )
    prepare_parser.add_argument("--model", required=True)
    prepare_parser.add_argument(
        "--effort", choices=("low", "medium", "high"), default="high"
    )
    prepare_parser.add_argument("--scenario", default=DEFAULT_SCENARIO)
    prepare_parser.add_argument("--out-root", type=Path, required=True)
    prepare_parser.add_argument("--quota-baseline", type=float, required=True)
    prepare_parser.add_argument("--quota-cap", type=float, required=True)
    prepare_parser.set_defaults(handler=prepare)

    quota_parser = commands.add_parser("record-quota")
    quota_parser.add_argument("--run-dir", type=Path, required=True)
    quota_parser.add_argument("--weekly", type=float, required=True)
    quota_parser.add_argument(
        "--source", default="Helm refresh_usage + get_claude_limits"
    )
    quota_parser.add_argument("--external-observed-at")
    quota_parser.add_argument(
        "--quota-cap",
        type=float,
        help="optionally tighten (never loosen) this active run's delta cap",
    )
    quota_parser.set_defaults(handler=record_quota)

    recover_parser = commands.add_parser("recover")
    recover_parser.add_argument("--run-dir", type=Path, required=True)
    recover_parser.set_defaults(handler=recover)

    auth_recover_parser = commands.add_parser("recover-auth")
    auth_recover_parser.add_argument("--run-dir", type=Path, required=True)
    auth_recover_parser.set_defaults(handler=recover_auth)

    turn_parser = commands.add_parser("turn")
    turn_parser.add_argument("--run-dir", type=Path, required=True)
    turn_parser.add_argument("--timeout", type=int, default=1500)
    turn_parser.add_argument("--quota-max-age", type=int, default=900)
    turn_parser.set_defaults(handler=run_turn)

    final_parser = commands.add_parser("finalize")
    final_parser.add_argument("--run-dir", type=Path, required=True)
    final_parser.set_defaults(handler=finalize)
    return root


def main() -> int:
    args = parser().parse_args()
    return int(args.handler(args))


if __name__ == "__main__":
    raise SystemExit(main())
