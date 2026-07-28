#!/usr/bin/env python3
"""Profile Codex and Claude JSONL histories without emitting private prompt text.

The output is deliberately aggregate-only. Session paths are represented by short
SHA-256 identifiers, prompts are classified in memory and discarded, and malformed
or partially-written trailing records are counted instead of failing the scan.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import re
from collections import Counter
from dataclasses import asdict, dataclass, field
from datetime import datetime
from pathlib import Path
from typing import Any, Iterable


CATEGORY_PATTERNS = {
    "benchmark_performance": re.compile(
        r"\b(benchmark|stress|soak|performance|latency|speed|wall ?time|profil|quota|token)\b",
        re.IGNORECASE,
    ),
    "implementation": re.compile(
        r"\b(build|implement|create|add|write|ship|feature|integrat|migrat|refactor)\b",
        re.IGNORECASE,
    ),
    "debugging_fix": re.compile(
        r"\b(fix|bug|broken|fail|error|debug|diagnos|regression|issue|problem)\b",
        re.IGNORECASE,
    ),
    "review_audit": re.compile(
        r"\b(review|audit|inspect|assess|check everything|quality|security|ready)\b",
        re.IGNORECASE,
    ),
    "design_architecture": re.compile(
        r"\b(design|architect|routing|classifier|orchestrat|system|approach|rfc)\b",
        re.IGNORECASE,
    ),
    "release_git_ci": re.compile(
        r"\b(commit|push|merge|branch|worktree|pull request|\bpr\b|release|ci|github)\b",
        re.IGNORECASE,
    ),
    "documentation": re.compile(
        r"\b(document|docs?|readme|report|methodology|publish(ed)?)\b",
        re.IGNORECASE,
    ),
    "operations_deploy": re.compile(
        r"\b(deploy|server|production|daemon|monitor|overnight|continue running|remote)\b",
        re.IGNORECASE,
    ),
    "research_comparison": re.compile(
        r"\b(research|compare|versus|\bvs\b|look (at|into)|investigat|find out)\b",
        re.IGNORECASE,
    ),
}

REPROMPT_PATTERNS = {
    "continuation_persistence": re.compile(
        r"\b(continue|keep going|do not stop|don'?t stop|until|autonomous|overnight|"
        r"run by yourself|finish (it|this|everything)|proceed)\b",
        re.IGNORECASE,
    ),
    "correction_refinement": re.compile(
        r"\b(no[, ]|instead|actually|you missed|not what|make sure|keep this in mind|"
        r"just for reference|i meant|fix all|remove|change)\b",
        re.IGNORECASE,
    ),
    "verification_status": re.compile(
        r"\b(check|verify|status|current|are we ready|is it|how certain|why is|"
        r"does this|tell me)\b",
        re.IGNORECASE,
    ),
    "publish_handoff": re.compile(
        r"\b(commit|push|merge|publish|handoff|paste this|readme|docs?)\b",
        re.IGNORECASE,
    ),
    "scope_expansion": re.compile(
        r"\b(also|another|anything else|other scenarios|along the way|before we|"
        r"after this|same kind|same idea|once again)\b",
        re.IGNORECASE,
    ),
}


@dataclass
class SessionMetrics:
    source: str
    session_key: str
    project: str = "other"
    bytes: int = 0
    records: int = 0
    malformed_records: int = 0
    user_turns: int = 0
    assistant_messages: int = 0
    tool_calls: int = 0
    tool_results: int = 0
    compactions: int = 0
    aborted_turns: int = 0
    completed_turns: int = 0
    state_mutations: int = 0
    first_timestamp: float | None = None
    last_timestamp: float | None = None
    categories: Counter[str] = field(default_factory=Counter)
    reprompts: Counter[str] = field(default_factory=Counter)
    tools: Counter[str] = field(default_factory=Counter)
    models: Counter[str] = field(default_factory=Counter)

    @property
    def duration_seconds(self) -> float:
        if self.first_timestamp is None or self.last_timestamp is None:
            return 0.0
        return max(0.0, self.last_timestamp - self.first_timestamp)


def parse_timestamp(value: Any) -> float | None:
    if not isinstance(value, str):
        return None
    try:
        return datetime.fromisoformat(value.replace("Z", "+00:00")).timestamp()
    except ValueError:
        return None


def update_timestamp(metrics: SessionMetrics, value: Any) -> None:
    timestamp = parse_timestamp(value)
    if timestamp is None:
        return
    if metrics.first_timestamp is None or timestamp < metrics.first_timestamp:
        metrics.first_timestamp = timestamp
    if metrics.last_timestamp is None or timestamp > metrics.last_timestamp:
        metrics.last_timestamp = timestamp


def classify_prompt(metrics: SessionMetrics, text: str) -> None:
    normalized = " ".join(text.split())
    if not normalized:
        return
    for name, pattern in CATEGORY_PATTERNS.items():
        if pattern.search(normalized):
            metrics.categories[name] += 1
    for name, pattern in REPROMPT_PATTERNS.items():
        if pattern.search(normalized):
            metrics.reprompts[name] += 1


def project_bucket(cwd: Any) -> str:
    if not isinstance(cwd, str):
        return "unknown"
    lowered = cwd.lower().rstrip("/")
    if "forge-harness-bench" in lowered or "forge-bench" in lowered:
        return "forge_benchmark_fixture"
    if lowered.endswith("/forge") or "/forge-" in lowered or "/forge/" in lowered:
        return "forge"
    return "other"


def session_key(path: Path) -> str:
    return hashlib.sha256(str(path).encode()).hexdigest()[:12]


def jsonl_records(path: Path, metrics: SessionMetrics) -> Iterable[dict[str, Any]]:
    with path.open("r", encoding="utf-8", errors="replace") as handle:
        for line in handle:
            metrics.records += 1
            try:
                record = json.loads(line)
            except json.JSONDecodeError:
                metrics.malformed_records += 1
                continue
            if isinstance(record, dict):
                yield record


def profile_codex(path: Path) -> SessionMetrics:
    metrics = SessionMetrics("codex", session_key(path), bytes=path.stat().st_size)
    for record in jsonl_records(path, metrics):
        update_timestamp(metrics, record.get("timestamp"))
        kind = record.get("type")
        payload = record.get("payload")
        if not isinstance(payload, dict):
            payload = {}

        if kind == "session_meta":
            metrics.project = project_bucket(payload.get("cwd"))
        elif kind == "turn_context":
            metrics.project = project_bucket(payload.get("cwd"))
            model = payload.get("model")
            if isinstance(model, str):
                metrics.models[model] += 1
        elif kind == "compacted":
            metrics.compactions += 1
        elif kind == "response_item":
            item_type = payload.get("type")
            if item_type in {"function_call", "custom_tool_call", "tool_search_call"}:
                metrics.tool_calls += 1
                name = payload.get("name")
                if isinstance(name, str):
                    metrics.tools[name] += 1
            elif item_type in {
                "function_call_output",
                "custom_tool_call_output",
                "tool_search_output",
            }:
                metrics.tool_results += 1
            elif item_type == "message" and payload.get("role") == "assistant":
                metrics.assistant_messages += 1
        elif kind == "event_msg":
            event_type = payload.get("type")
            if event_type == "user_message":
                metrics.user_turns += 1
                message = payload.get("message")
                if isinstance(message, str):
                    classify_prompt(metrics, message)
            elif event_type == "context_compacted":
                # The dedicated `compacted` record is authoritative when present.
                pass
            elif event_type == "turn_aborted":
                metrics.aborted_turns += 1
            elif event_type == "task_complete":
                metrics.completed_turns += 1
            elif event_type == "patch_apply_end":
                metrics.state_mutations += 1
    return metrics


def claude_prompt_text(record: dict[str, Any]) -> str:
    if record.get("isMeta") is True:
        return ""
    message = record.get("message")
    if not isinstance(message, dict):
        return ""
    content = message.get("content")
    if isinstance(content, str):
        return content
    if not isinstance(content, list):
        return ""
    texts = [
        item.get("text", "")
        for item in content
        if isinstance(item, dict) and item.get("type") == "text"
    ]
    return "\n".join(text for text in texts if isinstance(text, str))


def profile_claude(path: Path) -> SessionMetrics:
    metrics = SessionMetrics("claude", session_key(path), bytes=path.stat().st_size)
    for record in jsonl_records(path, metrics):
        update_timestamp(metrics, record.get("timestamp"))
        metrics.project = max(
            metrics.project,
            project_bucket(record.get("cwd")),
            key=lambda bucket: {"unknown": 0, "other": 1, "forge_benchmark_fixture": 2, "forge": 3}.get(
                bucket, 0
            ),
        )
        kind = record.get("type")
        if kind == "user":
            prompt = claude_prompt_text(record)
            if prompt:
                metrics.user_turns += 1
                classify_prompt(metrics, prompt)
            message = record.get("message")
            content = message.get("content") if isinstance(message, dict) else None
            if isinstance(content, list):
                metrics.tool_results += sum(
                    1
                    for item in content
                    if isinstance(item, dict) and item.get("type") == "tool_result"
                )
        elif kind == "assistant":
            message = record.get("message")
            if not isinstance(message, dict):
                continue
            metrics.assistant_messages += 1
            model = message.get("model")
            if isinstance(model, str):
                metrics.models[model] += 1
            content = message.get("content")
            if isinstance(content, list):
                for item in content:
                    if not isinstance(item, dict) or item.get("type") != "tool_use":
                        continue
                    metrics.tool_calls += 1
                    name = item.get("name")
                    if isinstance(name, str):
                        metrics.tools[name] += 1
        elif kind == "system":
            subtype = record.get("subtype")
            if subtype == "compact_boundary":
                metrics.compactions += 1
            elif subtype == "turn_duration":
                metrics.completed_turns += 1
        elif kind == "file-history-snapshot":
            metrics.state_mutations += 1
    return metrics


def percentile(values: list[float], quantile: float) -> float:
    if not values:
        return 0.0
    ordered = sorted(values)
    position = (len(ordered) - 1) * quantile
    lower = math.floor(position)
    upper = math.ceil(position)
    if lower == upper:
        return ordered[lower]
    return ordered[lower] + (ordered[upper] - ordered[lower]) * (position - lower)


def distribution(sessions: list[SessionMetrics], attribute: str) -> dict[str, float | int]:
    values = [float(getattr(session, attribute)) for session in sessions]
    return {
        "p50": round(percentile(values, 0.50), 2),
        "p90": round(percentile(values, 0.90), 2),
        "p95": round(percentile(values, 0.95), 2),
        "p99": round(percentile(values, 0.99), 2),
        "max": round(max(values, default=0.0), 2),
    }


def summarize(source: str, sessions: list[SessionMetrics]) -> dict[str, Any]:
    categories: Counter[str] = Counter()
    reprompts: Counter[str] = Counter()
    tools: Counter[str] = Counter()
    models: Counter[str] = Counter()
    for session in sessions:
        categories.update(session.categories)
        reprompts.update(session.reprompts)
        tools.update(session.tools)
        models.update(session.models)

    top = sorted(
        sessions,
        key=lambda session: (
            session.user_turns,
            session.compactions,
            session.tool_calls,
            session.bytes,
        ),
        reverse=True,
    )[:12]
    return {
        "source": source,
        "sessions": len(sessions),
        "bytes": sum(session.bytes for session in sessions),
        "records": sum(session.records for session in sessions),
        "malformed_records": sum(session.malformed_records for session in sessions),
        "forge_sessions": sum(session.project == "forge" for session in sessions),
        "benchmark_fixture_sessions": sum(
            session.project == "forge_benchmark_fixture" for session in sessions
        ),
        "totals": {
            field_name: sum(getattr(session, field_name) for session in sessions)
            for field_name in (
                "user_turns",
                "assistant_messages",
                "tool_calls",
                "tool_results",
                "compactions",
                "aborted_turns",
                "completed_turns",
                "state_mutations",
            )
        },
        "session_distributions": {
            field_name: distribution(sessions, field_name)
            for field_name in ("bytes", "user_turns", "tool_calls", "compactions", "duration_seconds")
        },
        "endurance_counts": {
            "at_least_5_turns": sum(session.user_turns >= 5 for session in sessions),
            "at_least_10_turns": sum(session.user_turns >= 10 for session in sessions),
            "at_least_20_turns": sum(session.user_turns >= 20 for session in sessions),
            "at_least_50_turns": sum(session.user_turns >= 50 for session in sessions),
            "at_least_100_turns": sum(session.user_turns >= 100 for session in sessions),
            "with_compaction": sum(session.compactions > 0 for session in sessions),
            "with_abort": sum(session.aborted_turns > 0 for session in sessions),
        },
        "task_categories": dict(categories.most_common()),
        "reprompt_signals": dict(reprompts.most_common()),
        "top_tools": dict(tools.most_common(30)),
        "top_models": dict(models.most_common(20)),
        "largest_endurance_sessions": [
            {
                "session_key": session.session_key,
                "project": session.project,
                "bytes": session.bytes,
                "user_turns": session.user_turns,
                "tool_calls": session.tool_calls,
                "compactions": session.compactions,
                "aborted_turns": session.aborted_turns,
                "completed_turns": session.completed_turns,
                "duration_hours": round(session.duration_seconds / 3600, 2),
            }
            for session in top
        ],
    }


def files(root: Path) -> list[Path]:
    return sorted(path for path in root.rglob("*.jsonl") if path.is_file())


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--codex-root", type=Path, default=Path.home() / ".codex/sessions")
    parser.add_argument("--claude-root", type=Path, default=Path.home() / ".claude/projects")
    parser.add_argument("--source", choices=("all", "codex", "claude"), default="all")
    parser.add_argument("--max-files", type=int)
    args = parser.parse_args()

    result: dict[str, Any] = {
        "privacy": "aggregate-only; no prompt text or filesystem paths are emitted",
    }
    if args.source in {"all", "codex"}:
        paths = files(args.codex_root)
        if args.max_files is not None:
            paths = paths[-args.max_files :]
        result["codex"] = summarize("codex", [profile_codex(path) for path in paths])
    if args.source in {"all", "claude"}:
        paths = files(args.claude_root)
        if args.max_files is not None:
            paths = paths[-args.max_files :]
        result["claude"] = summarize("claude", [profile_claude(path) for path in paths])
    print(json.dumps(result, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
