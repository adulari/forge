#!/usr/bin/env python3
"""Audit benchmark cells and emit clean native baselines for mesh studies."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
from pathlib import Path
from typing import Any, Iterable

import compare_codex_oauth as bench


INTEGRITY_PREFIX = "Benchmark integrity rules:"
COMPARABLE_TIMEOUT_SECONDS = 1_500
HISTORY_COMMAND = re.compile(
    r"(?i)(?:^|[;&|(\s])git\s+"
    r"(?:--[\w-]+\s+)*(log|show|reflog|branch|remote|fetch|pull|push|clone|"
    r"submodule|ls-remote|rev-list)"
    r"(?:\s|$)"
)
NETWORK_COMMAND = re.compile(
    r"(?i)(?:^|[;&|(\s])(?:curl|wget|gh|ssh|scp|sftp|telnet|nc|ncat)\s"
)
EXTERNAL_TOOL_NAMES = {
    "fetch",
    "web",
    "web_fetch",
    "web_search",
    "webfetch",
    "websearch",
}
COMMAND_TOOL_NAMES = {
    "bash",
    "command_execution",
    "exec",
    "exec_command",
    "run_command",
    "shell",
}


def load_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def load_json_lines(path: Path) -> list[dict[str, Any]]:
    if not path.is_file():
        return []
    rows: list[dict[str, Any]] = []
    for line in path.read_text(encoding="utf-8").splitlines():
        if line.strip():
            value = json.loads(line)
            if isinstance(value, dict):
                rows.append(value)
    return rows


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def dataset_evidence(manifest: dict[str, Any]) -> dict[str, Any]:
    value = manifest.get("dataset")
    path = Path(value) if isinstance(value, str) and value else None
    if path is None or not path.is_file():
        return {
            "path": str(path) if path else None,
            "exists": False,
            "sha256": None,
            "metadata_matches": False,
            "instances": {},
        }
    payload = path.read_bytes()
    instances: dict[str, dict[str, Any]] = {}
    for line in payload.decode("utf-8").splitlines():
        if not line.strip():
            continue
        row = json.loads(line)
        if isinstance(row, dict) and row.get("instance_id"):
            instances[str(row["instance_id"])] = {
                **row,
                "_dataset_index": len(instances),
            }
    digest = sha256_bytes(payload)
    return {
        "path": str(path.resolve()),
        "exists": True,
        "sha256": digest,
        "metadata_matches": digest == manifest.get("dataset_sha256"),
        "instances": instances,
    }


def compact(value: Any, limit: int = 240) -> str:
    text = value if isinstance(value, str) else json.dumps(value, sort_keys=True)
    text = " ".join(text.split())
    return text if len(text) <= limit else f"{text[: limit - 1]}…"


def normalized_tool_name(name: str) -> str:
    return name.lower().rsplit("__", 1)[-1]


def event_tool_records(event: dict[str, Any]) -> Iterable[tuple[str, Any]]:
    item = event.get("item")
    if isinstance(item, dict) and item.get("type") in {
        "command_execution",
        "mcp_tool_call",
    }:
        yield (
            str(item.get("name") or item["type"]),
            item.get("command")
            or item.get("arguments")
            or item.get("input")
            or item,
        )
    for container in (event.get("message"), event):
        if not isinstance(container, dict):
            continue
        content = container.get("content")
        if not isinstance(content, list):
            continue
        for block in content:
            if isinstance(block, dict) and block.get("type") == "tool_use":
                yield str(block.get("name", "tool_use")), block.get("input", {})


def command_text(name: str, tool_input: Any) -> str | None:
    if normalized_tool_name(name) not in COMMAND_TOOL_NAMES:
        return None
    if isinstance(tool_input, str):
        return tool_input
    if not isinstance(tool_input, dict):
        return None
    value = tool_input.get("cmd") or tool_input.get("command")
    if isinstance(value, list):
        return " ".join(str(part) for part in value)
    return value if isinstance(value, str) else None


def audit_events(path: Path) -> dict[str, Any]:
    if not path.is_file():
        return {
            "event_file": False,
            "malformed": 0,
            "tool_calls": 0,
            "findings": [{"kind": "missing_events"}],
            "flagged_tools": [],
        }
    findings: list[dict[str, Any]] = []
    tools: list[dict[str, Any]] = []
    malformed = 0
    with path.open(encoding="utf-8", errors="replace") as handle:
        for line_number, line in enumerate(handle, 1):
            if not line.strip():
                continue
            try:
                event = json.loads(line)
            except json.JSONDecodeError:
                malformed += 1
                continue
            for name, tool_input in event_tool_records(event):
                record = {
                    "line": line_number,
                    "name": name,
                    "input": compact(tool_input, 800),
                }
                tools.append(record)
                normalized = normalized_tool_name(name)
                if normalized in EXTERNAL_TOOL_NAMES:
                    findings.append(
                        {
                            "kind": "external_solution_tool",
                            "line": line_number,
                            "tool": name,
                            "input": compact(tool_input),
                        }
                    )
                    continue
                command = command_text(name, tool_input)
                if command is None:
                    continue
                history_match = HISTORY_COMMAND.search(command)
                if history_match:
                    findings.append(
                        {
                            "kind": "git_history_or_remote",
                            "operation": history_match.group(1).lower(),
                            "line": line_number,
                            "tool": name,
                            "input": compact(command),
                        }
                    )
                if NETWORK_COMMAND.search(command):
                    findings.append(
                        {
                            "kind": "network_command",
                            "line": line_number,
                            "tool": name,
                            "input": compact(command),
                        }
                    )
    flagged_lines = {finding.get("line") for finding in findings}
    return {
        "event_file": True,
        "malformed": malformed,
        "tool_calls": len(tools),
        "findings": findings,
        "flagged_tools": [tool for tool in tools if tool["line"] in flagged_lines],
    }


def official_reports(run_dir: Path) -> dict[str, list[dict[str, Any]]]:
    reports: dict[str, list[dict[str, Any]]] = {}
    for report_path in run_dir.glob("**/logs/run_evaluation/**/report.json"):
        try:
            payload = load_json(report_path)
        except (OSError, json.JSONDecodeError):
            continue
        for instance_id, result in payload.items():
            if not isinstance(result, dict):
                continue
            reports.setdefault(instance_id, []).append(
                {
                    "path": str(report_path.resolve()),
                    "model_name": report_path.parent.parent.name,
                    "resolved": result.get("resolved"),
                    "patch_applied": result.get("patch_successfully_applied"),
                }
            )
    return reports


def expected_report_name(arm: str, model: str | None) -> str:
    return arm if arm == "forge-mesh-auto" else f"{arm}::{model}"


def report_status(
    reports: dict[str, list[dict[str, Any]]],
    *,
    instance_id: str,
    arm: str,
    model: str | None,
) -> dict[str, Any]:
    matches = [
        report
        for report in reports.get(instance_id, [])
        if report["model_name"] == expected_report_name(arm, model)
    ]
    unique = {(row["resolved"], row["patch_applied"]) for row in matches}
    if len(matches) == 1 and len(unique) == 1:
        resolved, patch_applied = next(iter(unique))
        return {
            "count": 1,
            "resolved": resolved,
            "patch_applied": patch_applied,
            "paths": [matches[0]["path"]],
        }
    return {
        "count": len(matches),
        "resolved": None,
        "patch_applied": None,
        "paths": [row["path"] for row in matches],
        "ambiguous": len(unique) > 1,
    }


def selected_identity(agent: dict[str, Any]) -> dict[str, Any]:
    model_usage = agent.get("model_usage")
    route_usage = agent.get("route_usage")
    return {
        "selected_model": agent.get("selected_model"),
        "model_usage_names": (
            sorted(model_usage) if isinstance(model_usage, dict) else []
        ),
        "route_models": (
            sorted(
                {
                    str(row["model"])
                    for row in route_usage
                    if isinstance(row, dict) and row.get("model")
                }
            )
            if isinstance(route_usage, list)
            else []
        ),
    }


def expected_process_identity(
    *,
    arm: str,
    model: str | None,
    argv: list[Any],
) -> dict[str, Any]:
    text = [str(part) for part in argv]
    expected_model: str | None = None
    expected_effort: str | None = None
    model_flag: str | None = None
    if arm == "forge":
        model_flag = "--model"
        if model:
            provider = "claude-cli" if model in {"opus[1m]", "sonnet"} else "codex-oauth"
            expected_model = f"{provider}::{model}"
    elif arm == "raw-codex":
        model_flag = "-m"
        expected_model = model
        expected_effort = 'model_reasoning_effort="high"'
    elif arm == "raw-claude":
        model_flag = "--model"
        expected_model = model
        expected_effort = "high"

    actual_model: str | None = None
    if model_flag in text:
        index = text.index(model_flag)
        if index + 1 < len(text):
            actual_model = text[index + 1]
    if arm == "raw-codex":
        effort_matches = expected_effort in text
    elif arm == "raw-claude":
        effort_matches = (
            "--effort" in text
            and text.index("--effort") + 1 < len(text)
            and text[text.index("--effort") + 1] == expected_effort
        )
    else:
        effort_matches = True
    return {
        "expected_model": expected_model,
        "actual_model": actual_model,
        "model_matches": expected_model == actual_model,
        "effort_matches": effort_matches,
        "unpinned_mesh": arm != "forge-mesh-auto" or "--model" not in text,
    }


def patch_capture(summary_path: Path, summary: dict[str, Any]) -> dict[str, Any]:
    patch_path = summary_path.with_name("changes.patch")
    expected = summary.get("patch") or {}
    if not patch_path.is_file():
        return {
            "path": str(patch_path.resolve()),
            "exists": False,
            "bytes": None,
            "sha256": None,
            "metadata_matches": False,
        }
    payload = patch_path.read_bytes()
    size = len(payload)
    digest = sha256_bytes(payload)
    return {
        "path": str(patch_path.resolve()),
        "exists": True,
        "bytes": size,
        "sha256": digest,
        "metadata_matches": (
            size > 0
            and size == expected.get("patch_bytes")
            and digest == expected.get("patch_sha256")
        ),
    }


def audit_run(run_dir: Path) -> dict[str, Any]:
    manifest_path = run_dir / "suite-manifest.json"
    manifest = load_json(manifest_path) if manifest_path.is_file() else {}
    dataset = dataset_evidence(manifest)
    quota_checks = load_json_lines(run_dir / "quota-checks.jsonl")
    reports = official_reports(run_dir)
    cells: list[dict[str, Any]] = []
    for cell_index, summary_path in enumerate(
        sorted(run_dir.glob("trials/*/summary.json"))
    ):
        summary = load_json(summary_path)
        trial = summary.get("trial") or {}
        agent = summary.get("agent") or {}
        process = summary.get("process") or {}
        trial_manifest_path = Path(
            summary.get("manifest") or summary_path.with_name("manifest.json")
        )
        trial_manifest = (
            load_json(trial_manifest_path) if trial_manifest_path.is_file() else {}
        )
        prompt_path = summary_path.with_name("prompt.txt")
        prompt = (
            prompt_path.read_bytes().decode("utf-8", errors="replace")
            if prompt_path.is_file()
            else ""
        )
        instance_id = str(
            trial.get("instance_id")
            or summary.get("instance_id")
            or (trial_manifest.get("trial") or {}).get("instance_id")
            or ""
        )
        arm = str(trial.get("arm") or "")
        model = trial.get("model")
        tokens = agent.get("tokens") or {}
        process_identity = expected_process_identity(
            arm=arm,
            model=model,
            argv=process.get("argv") or [],
        )
        captured_patch = patch_capture(summary_path, summary)
        official = report_status(
            reports,
            instance_id=instance_id,
            arm=arm,
            model=model,
        )
        trace = audit_events(summary_path.with_name("events.jsonl"))
        reasons: list[str] = []
        history_safe = "history-safe" in run_dir.name
        if not history_safe:
            reasons.append("non_isolated_or_legacy_suite")
        suite_instances = {
            str(row.get("instance_id")): row
            for row in manifest.get("instances", [])
            if isinstance(row, dict) and row.get("instance_id")
        }
        dataset_instance = dataset["instances"].get(instance_id)
        suite_instance = suite_instances.get(instance_id) or dataset_instance
        if not dataset["metadata_matches"]:
            reasons.append("dataset_identity_mismatch")
        if suite_instance is None:
            reasons.append("task_not_declared_in_suite")
        elif any(
            summary.get(key) != suite_instance.get(key)
            for key in ("base_commit", "repo", "difficulty")
        ):
            reasons.append("task_or_base_not_comparable")
        if dataset_instance is not None:
            problem_statement = str(dataset_instance.get("problem_statement") or "")
            if (
                summary.get("dataset_index") != dataset_instance["_dataset_index"]
                or trial_manifest.get("problem_statement_sha256")
                != sha256_bytes(prompt.encode("utf-8"))
                or not prompt.endswith(problem_statement)
            ):
                reasons.append("task_prompt_or_dataset_index_mismatch")
        declared_trial = next(
            (
                row
                for row in manifest.get("trial_order", [])
                if isinstance(row, dict) and row.get("index") == trial.get("index")
            ),
            None,
        )
        if (
            declared_trial is None
            or declared_trial.get("instance_id") != instance_id
            or (
                declared_trial.get("arm") is not None
                and declared_trial.get("arm") != arm
            )
            or (
                declared_trial.get("model") is not None
                and declared_trial.get("model") != model
            )
        ):
            reasons.append("trial_not_declared_in_order")
        if manifest.get("timeout_seconds") != COMPARABLE_TIMEOUT_SECONDS:
            reasons.append("timeout_not_comparable")
        if arm == "forge-mesh-auto":
            if (
                manifest.get("mode") != "regular-full-mesh-auto"
                or manifest.get("model_pin") is not None
                or manifest.get("effort_override") is not None
            ):
                reasons.append("mesh_not_regular_unpinned_auto")
            if not process_identity["unpinned_mesh"]:
                reasons.append("mesh_process_was_model_pinned")
        elif manifest.get("reasoning_effort") != "high":
            reasons.append("effort_not_regular_high")
        if arm != "forge-mesh-auto":
            if arm not in manifest.get("arms", []):
                reasons.append("arm_not_declared_in_suite")
            if model not in manifest.get("models", []):
                reasons.append("model_not_declared_in_suite")
            if not process_identity["model_matches"]:
                reasons.append("process_model_mismatch")
            if not process_identity["effort_matches"]:
                reasons.append("process_effort_mismatch")
        if arm == "raw-codex" and not manifest.get("codex_version"):
            reasons.append("missing_codex_version")
        if arm == "raw-claude":
            if not manifest.get("claude_version"):
                reasons.append("missing_claude_version")
            if not (manifest.get("resolved_models") or {}).get(str(model)):
                reasons.append("missing_resolved_claude_model")
        if arm in {"forge", "forge-mesh-auto"}:
            if not manifest.get("forge_version"):
                reasons.append("missing_forge_version")
            if not re.fullmatch(
                r"[0-9a-f]{64}",
                str(manifest.get("forge_binary_sha256") or ""),
            ):
                reasons.append("missing_forge_binary_identity")
        if not prompt.startswith(INTEGRITY_PREFIX):
            reasons.append("missing_integrity_preamble")
        if trace["findings"]:
            reasons.append("trace_integrity_violation")
        if trace["malformed"]:
            reasons.append("malformed_trace_events")
        if not summary.get("process_ok") or process.get("timed_out"):
            reasons.append("provider_process_invalid")
        if official["count"] != 1:
            reasons.append("missing_or_ambiguous_official_evaluation")
        if official["patch_applied"] is not True:
            reasons.append("patch_not_applied_by_official_evaluator")
        if not summary.get("patched"):
            reasons.append("missing_captured_patch")
        if not captured_patch["metadata_matches"]:
            reasons.append("captured_patch_metadata_mismatch")
        if summary.get("base_commit") != trial_manifest.get("base_commit"):
            reasons.append("base_commit_manifest_mismatch")
        cells.append(
            {
                "run": run_dir.name,
                "run_dir": str(run_dir.resolve()),
                "slug": summary_path.parent.name,
                "arm": arm,
                "requested_model": model,
                "identity": selected_identity(agent),
                "process_identity": process_identity,
                "instance_id": instance_id,
                "dataset_index": summary.get("dataset_index"),
                "repo": summary.get("repo"),
                "difficulty": summary.get("difficulty"),
                "base_commit": summary.get("base_commit"),
                "workspace_head": trial_manifest.get("workspace_head")
                or trial_manifest.get("workspace_head_before"),
                "integrity_preamble": prompt.startswith(INTEGRITY_PREFIX),
                "process_ok": summary.get("process_ok"),
                "timed_out": process.get("timed_out"),
                "wall_seconds": process.get("wall_seconds"),
                "patched": summary.get("patched"),
                "patch_bytes": (summary.get("patch") or {}).get("patch_bytes"),
                "patch_capture": captured_patch,
                "tokens_raw": tokens.get("total_tokens"),
                "tokens_cache_adjusted_025": tokens.get(
                    "cache_adjusted_tokens_025"
                ),
                "quota_observation": {
                    "pre_arm_external_refresh": (
                        quota_checks[cell_index]
                        if cell_index < len(quota_checks)
                        else None
                    ),
                    "post_arm_provider_snapshot": summary.get("quota"),
                },
                "official": official,
                "trace": trace,
                "validity": {
                    "status": "valid" if not reasons else "invalid_superseded",
                    "reasons": reasons,
                },
            }
        )
    return {
        "run_dir": str(run_dir.resolve()),
        "suite": manifest,
        "dataset_evidence": {
            key: value for key, value in dataset.items() if key != "instances"
        },
        "cells": cells,
    }


def native_baselines(payload: dict[str, Any], family: str) -> dict[str, Any]:
    specs = {
        "codex": (
            "raw-codex",
            "raw_codex_resolved",
            "raw_codex_wall_seconds",
            "raw_codex_total_tokens",
            "raw_codex_cache_adjusted_tokens_025",
        ),
        "claude": (
            "raw-claude",
            "raw_claude_resolved",
            "raw_claude_wall_seconds",
            "raw_claude_total_tokens",
            "raw_claude_cache_adjusted_tokens_025",
        ),
    }
    arm, resolved_key, wall_key, token_key, adjusted_key = specs[family]
    cells = [
        cell
        for run in payload["runs"]
        for cell in run["cells"]
        if cell["arm"] == arm and cell["validity"]["status"] == "valid"
    ]
    identities: set[tuple[str, str]] = set()
    pairs: list[dict[str, Any]] = []
    for cell in cells:
        identity = (str(cell["requested_model"]), cell["instance_id"])
        if identity in identities:
            raise ValueError(f"duplicate valid native {family} cell: {identity}")
        identities.add(identity)
        pairs.append(
            {
                "pair_id": f"{identity[0]}__{identity[1]}",
                "model": identity[0],
                "instance_id": identity[1],
                "dataset_index": cell["dataset_index"],
                "repo": cell["repo"],
                "difficulty": cell["difficulty"],
                resolved_key: cell["official"]["resolved"],
                wall_key: cell["wall_seconds"],
                token_key: cell["tokens_raw"],
                adjusted_key: cell["tokens_cache_adjusted_025"],
                "source_run": cell["run_dir"],
                "source_cell": cell["slug"],
            }
        )
    if not pairs:
        raise ValueError(f"no valid native {family} cells")
    return {
        "schema_version": 1,
        "family": family,
        "generated_at": bench.utc_now(),
        "integrity_source": payload["out_path"],
        "pairs": sorted(pairs, key=lambda row: (row["model"], row["instance_id"])),
    }


def render_markdown(payload: dict[str, Any]) -> str:
    cells = [cell for run in payload["runs"] for cell in run["cells"]]
    valid = sum(cell["validity"]["status"] == "valid" for cell in cells)
    lines = [
        "# Benchmark cell validity ledger",
        "",
        f"- Cells audited: {len(cells)}",
        f"- Valid and reusable: {valid}",
        f"- Invalid or superseded: {len(cells) - valid}",
        "",
        "“Valid and reusable” is a methodology judgment, not a headline-selection "
        "instruction or a successful solve. Preserve duplicate clean cells, but "
        "select them by a declared version/order policy rather than observed speed "
        "or quality.",
        "",
        "| Run | Cell | Status | Official | Wall s | Raw tokens | Cache-adjusted | Integrity findings | Reasons |",
        "|---|---|---|---:|---:|---:|---:|---|---|",
    ]
    for cell in cells:
        findings = ", ".join(
            sorted({row["kind"] for row in cell["trace"]["findings"]})
        ) or "none"
        reasons = ", ".join(cell["validity"]["reasons"]) or "clean and comparable"
        lines.append(
            "| "
            + " | ".join(
                [
                    cell["run"],
                    cell["slug"],
                    cell["validity"]["status"],
                    str(cell["official"]["resolved"]).lower(),
                    str(cell["wall_seconds"]),
                    str(cell["tokens_raw"]),
                    str(cell["tokens_cache_adjusted_025"]),
                    findings,
                    reasons,
                ]
            )
            + " |"
        )
    lines.extend(
        [
            "",
            "The earlier contaminated headline outcomes are not a controlled estimate of "
            "contamination's effect. The clean replacement also changed Codex effort from "
            "`xhigh` to `high`, added an integrity preamble, isolated Git history, and is "
            "subject to model stochasticity.",
            "",
        ]
    )
    return "\n".join(lines)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("runs", nargs="+", type=Path)
    parser.add_argument("--out-json", required=True, type=Path)
    parser.add_argument("--out-markdown", type=Path)
    parser.add_argument("--codex-native-out", type=Path)
    parser.add_argument("--claude-native-out", type=Path)
    args = parser.parse_args()

    payload = {
        "schema_version": 1,
        "generated_at": bench.utc_now(),
        "out_path": str(args.out_json.resolve()),
        "runs": [audit_run(path.resolve()) for path in args.runs],
    }
    bench.json_dump(args.out_json, payload)
    if args.out_markdown:
        args.out_markdown.write_text(render_markdown(payload), encoding="utf-8")
    for family, path in (
        ("codex", args.codex_native_out),
        ("claude", args.claude_native_out),
    ):
        if path:
            bench.json_dump(path, native_baselines(payload, family))
    cells = [cell for run in payload["runs"] for cell in run["cells"]]
    print(
        json.dumps(
            {
                "cells": len(cells),
                "valid": sum(
                    cell["validity"]["status"] == "valid" for cell in cells
                ),
                "invalid_superseded": sum(
                    cell["validity"]["status"] != "valid" for cell in cells
                ),
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
