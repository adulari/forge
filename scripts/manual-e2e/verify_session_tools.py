#!/usr/bin/env python3
"""Validate persisted Forge tool-call envelopes without printing their contents."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sqlite3
from collections import Counter
from pathlib import Path


def parse_arguments() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("database", type=Path)
    parser.add_argument("session_id")
    parser.add_argument(
        "--require-all-ok",
        action="store_true",
        help="reject sessions containing a failed/denied tool outcome",
    )
    parser.add_argument(
        "--deny-external-sources",
        action="store_true",
        help="reject persisted web/GitHub tools and shell commands that access external sources",
    )
    return parser.parse_args()


EXTERNAL_COMMAND_PATTERNS = {
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
        r"\b(?:pip|pip3|npm|pnpm|yarn)\s+(?:install|add)\b|"
        r"\bcargo\s+(?:install|update)\b",
        re.IGNORECASE,
    ),
}


def external_source_findings(
    sequence: int, tool_name: str, arguments: dict[str, object]
) -> list[dict[str, object]]:
    rendered = json.dumps(arguments, sort_keys=True, default=str)
    lowered_name = tool_name.lower()
    kinds: list[str] = []
    if (
        re.search(r"\b(web_search|web_fetch|browser(?:_use)?)\b", lowered_name)
        or "github" in lowered_name
        or "gitlab" in lowered_name
    ):
        kinds.append("external_tool")
    if lowered_name in {"shell", "bash"}:
        command = str(arguments.get("command", arguments.get("cmd", rendered)))
        for kind, pattern in EXTERNAL_COMMAND_PATTERNS.items():
            if pattern.search(command):
                kinds.append(kind)
    digest = hashlib.sha256(f"{tool_name}\n{rendered}".encode()).hexdigest()
    return [
        {
            "message_seq": sequence,
            "tool_name": tool_name,
            "kind": kind,
            "invocation_sha256": digest,
        }
        for kind in kinds
    ]


def main() -> int:
    args = parse_arguments()
    errors: list[str] = []
    envelope_count = 0
    execution_count = 0
    non_ok_execution_count = 0
    integrity_findings: list[dict[str, object]] = []
    tool_result_ids: set[str] = set()
    messages: list[tuple[object, ...]] = []

    uri = f"file:{args.database.resolve()}?mode=ro"
    with sqlite3.connect(uri, uri=True) as database:
        messages = database.execute(
            """
            SELECT id, seq, role, tool_calls_json, tool_call_id
            FROM message
            WHERE session_id = ?
            ORDER BY seq
            """,
            (args.session_id,),
        ).fetchall()
        if not messages:
            errors.append("session has no persisted messages")

        tool_result_ids = {
            tool_call_id
            for _message_id, _seq, role, _envelope, tool_call_id in messages
            if role == "tool" and tool_call_id
        }
        for _message_id, seq, _role, envelope, _tool_call_id in messages:
            if envelope is None:
                continue
            try:
                calls = json.loads(envelope)
            except json.JSONDecodeError as error:
                errors.append(f"message {seq} has invalid tool_calls_json: {error.msg}")
                continue
            if not isinstance(calls, list):
                errors.append(f"message {seq} tool_calls_json is not an array")
                continue

            expected_names: Counter[str] = Counter()
            expected_ids: set[str] = set()
            for index, call in enumerate(calls):
                envelope_count += 1
                if not isinstance(call, dict):
                    errors.append(f"message {seq} call {index} is not an object")
                    continue
                call_id = call.get("id")
                name = call.get("name")
                call_args = call.get("args")
                if not isinstance(call_id, str) or not call_id:
                    errors.append(f"message {seq} call {index} has no valid id")
                else:
                    expected_ids.add(call_id)
                if not isinstance(name, str) or not name:
                    errors.append(f"message {seq} call {index} has no valid name")
                else:
                    expected_names[name] += 1
                if not isinstance(call_args, dict):
                    errors.append(f"message {seq} call {index} args are not an object")

            persisted_rows = database.execute(
                """
                SELECT tool_name, args_json, status
                FROM tool_call
                WHERE message_id = ?
                """,
                (_message_id,),
            ).fetchall()
            execution_count += len(persisted_rows)
            actual_names: Counter[str] = Counter()
            for row_index, (tool_name, args_json, status) in enumerate(persisted_rows):
                actual_names[tool_name] += 1
                try:
                    persisted_args = json.loads(args_json)
                except json.JSONDecodeError as error:
                    errors.append(
                        f"message {seq} execution {row_index} has invalid args_json: {error.msg}"
                    )
                    continue
                if not isinstance(persisted_args, dict):
                    errors.append(
                        f"message {seq} execution {row_index} args are not an object"
                    )
                elif args.deny_external_sources:
                    integrity_findings.extend(
                        external_source_findings(seq, tool_name, persisted_args)
                    )
                if status != "ok":
                    non_ok_execution_count += 1
                if args.require_all_ok and status != "ok":
                    errors.append(
                        f"message {seq} execution {row_index} has non-ok status {status!r}"
                    )
            if actual_names != expected_names:
                errors.append(
                    f"message {seq} envelope/execution names differ: "
                    f"{dict(expected_names)} != {dict(actual_names)}"
                )
            missing_results = expected_ids - tool_result_ids
            if missing_results:
                errors.append(
                    f"message {seq} has {len(missing_results)} unmatched tool result(s)"
                )

    if envelope_count == 0:
        errors.append("session persisted no tool-call envelopes")
    if integrity_findings:
        errors.append(
            f"session used {len(integrity_findings)} external-solution-like tool invocation(s)"
        )

    report = {
        "valid": not errors,
        "database": str(args.database.resolve()),
        "session_id": args.session_id,
        "message_count": len(messages),
        "tool_envelope_count": envelope_count,
        "tool_execution_count": execution_count,
        "non_ok_tool_execution_count": non_ok_execution_count,
        "external_source_findings": integrity_findings,
        "errors": errors,
    }
    print(json.dumps(report, indent=2))
    return 0 if not errors else 1


if __name__ == "__main__":
    raise SystemExit(main())
