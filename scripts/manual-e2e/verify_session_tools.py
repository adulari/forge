#!/usr/bin/env python3
"""Validate persisted Forge tool-call envelopes without printing their contents."""

from __future__ import annotations

import argparse
import json
import sqlite3
from collections import Counter
from pathlib import Path


def parse_arguments() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("database", type=Path)
    parser.add_argument("session_id")
    return parser.parse_args()


def main() -> int:
    args = parse_arguments()
    errors: list[str] = []
    envelope_count = 0
    execution_count = 0
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
                    errors.append(f"message {seq} execution {row_index} args are not an object")
                if status != "ok":
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
                errors.append(f"message {seq} has {len(missing_results)} unmatched tool result(s)")

    if envelope_count == 0:
        errors.append("session persisted no tool-call envelopes")

    report = {
        "valid": not errors,
        "database": str(args.database.resolve()),
        "session_id": args.session_id,
        "message_count": len(messages),
        "tool_envelope_count": envelope_count,
        "tool_execution_count": execution_count,
        "errors": errors,
    }
    print(json.dumps(report, indent=2))
    return 0 if not errors else 1


if __name__ == "__main__":
    raise SystemExit(main())
