#!/usr/bin/env python3
"""Measure owned Rust architecture size and enforce ratcheting guardrails.

The counter reports physical lines while separating inline/external test modules
from implementation. It intentionally uses only the Python standard library so
the check is cheap enough to run on every CI change-detection job.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Iterable


SCHEMA_VERSION = 1
DEFAULT_MAX_IMPLEMENTATION_LINES = 800
DEFAULT_SMALL_FILE_TARGET = 500
TRACKED_THRESHOLDS = (2_000, 5_000, 10_000)

ATTRIBUTE_START = re.compile(r"(?m)^[ \t]*#\s*\[")
CFG_TEST_ATTRIBUTE = re.compile(
    r"^\s*#\s*\[\s*cfg\s*\(\s*test\s*\)\s*\]\s*$",
    re.DOTALL,
)
EXTERNAL_TEST_MODULE = re.compile(
    r"#\s*\[\s*cfg\s*\(\s*test\s*\)\s*\]"
    r"(?:\s*#\s*\[[^\]]*\])*\s*"
    r"(?:pub(?:\([^)]*\))?\s+)?mod\s+([A-Za-z_][A-Za-z0-9_]*)\s*;"
)


@dataclass(frozen=True)
class FileMetrics:
    path: str
    physical_lines: int
    implementation_lines: int
    test_lines: int


def _raw_string_start(text: str, index: int) -> tuple[int, str] | None:
    """Return (opening length, closing delimiter) for a Rust raw string."""
    cursor = index
    if text.startswith("br", cursor):
        cursor += 2
    elif text.startswith("r", cursor):
        cursor += 1
    else:
        return None

    hash_start = cursor
    while cursor < len(text) and text[cursor] == "#":
        cursor += 1
    if cursor >= len(text) or text[cursor] != '"':
        return None

    hashes = text[hash_start:cursor]
    return cursor - index + 1, f'"{hashes}'


def _char_literal_end(text: str, index: int) -> int | None:
    """Return the exclusive end of a Rust char literal, not a lifetime."""
    cursor = index + 1
    if cursor >= len(text) or text[cursor] in "\r\n":
        return None

    if text[cursor] == "\\":
        cursor += 1
        if cursor >= len(text):
            return None
        if text[cursor] == "u" and cursor + 1 < len(text) and text[cursor + 1] == "{":
            closing = text.find("}", cursor + 2)
            if closing == -1:
                return None
            cursor = closing + 1
        else:
            cursor += 1
    else:
        cursor += 1

    if cursor < len(text) and text[cursor] == "'":
        return cursor + 1
    return None


def mask_non_code(text: str) -> str:
    """Mask Rust comments and literals while preserving offsets and newlines."""
    masked = list(text)
    index = 0
    block_depth = 0

    def blank(start: int, end: int) -> None:
        for position in range(start, end):
            if masked[position] not in "\r\n":
                masked[position] = " "

    while index < len(text):
        if block_depth:
            if text.startswith("/*", index):
                blank(index, index + 2)
                block_depth += 1
                index += 2
            elif text.startswith("*/", index):
                blank(index, index + 2)
                block_depth -= 1
                index += 2
            else:
                blank(index, index + 1)
                index += 1
            continue

        if text.startswith("//", index):
            line_end = text.find("\n", index)
            if line_end == -1:
                line_end = len(text)
            blank(index, line_end)
            index = line_end
            continue

        if text.startswith("/*", index):
            blank(index, index + 2)
            block_depth = 1
            index += 2
            continue

        raw = _raw_string_start(text, index)
        if raw is not None:
            opening_length, closing = raw
            closing_start = text.find(closing, index + opening_length)
            end = len(text) if closing_start == -1 else closing_start + len(closing)
            blank(index, end)
            index = end
            continue

        if text[index] == '"':
            cursor = index + 1
            while cursor < len(text):
                if text[cursor] == "\\":
                    cursor += 2
                    continue
                if text[cursor] == '"':
                    cursor += 1
                    break
                cursor += 1
            blank(index, min(cursor, len(text)))
            index = cursor
            continue

        if text[index] == "'":
            char_end = _char_literal_end(text, index)
            if char_end is not None:
                blank(index, char_end)
                index = char_end
                continue

        index += 1

    return "".join(masked)


def _matching_delimiter(code: str, start: int, opening: str, closing: str) -> int | None:
    depth = 0
    for index in range(start, len(code)):
        character = code[index]
        if character == opening:
            depth += 1
        elif character == closing:
            depth -= 1
            if depth == 0:
                return index
    return None


def _next_item_end(code: str, start: int) -> int:
    """Find the end of an item following a cfg(test) attribute."""
    cursor = start
    while cursor < len(code):
        if code[cursor].isspace():
            cursor += 1
            continue

        if code[cursor] == "#":
            bracket = code.find("[", cursor + 1)
            if bracket != -1:
                attribute_end = _matching_delimiter(code, bracket, "[", "]")
                if attribute_end is not None:
                    cursor = attribute_end + 1
                    continue

        break

    for index in range(cursor, len(code)):
        if code[index] == ";":
            return index
        if code[index] == "{":
            item_end = _matching_delimiter(code, index, "{", "}")
            return len(code) - 1 if item_end is None else item_end
    return len(code) - 1


def cfg_test_line_numbers(text: str) -> set[int]:
    """Return zero-based physical line numbers owned by cfg(test) items."""
    code = mask_non_code(text)
    test_lines: set[int] = set()

    for match in ATTRIBUTE_START.finditer(code):
        bracket = code.find("[", match.start())
        if bracket == -1:
            continue
        attribute_end = _matching_delimiter(code, bracket, "[", "]")
        if attribute_end is None:
            continue
        attribute = code[match.start() : attribute_end + 1]
        if CFG_TEST_ATTRIBUTE.fullmatch(attribute) is None:
            continue

        item_end = _next_item_end(code, attribute_end + 1)
        first_line = code.count("\n", 0, match.start())
        last_line = code.count("\n", 0, item_end)
        test_lines.update(range(first_line, last_line + 1))

    return test_lines


def _is_conventional_test_path(relative_path: Path) -> bool:
    parts = relative_path.parts
    name = relative_path.name
    return (
        "tests" in parts
        or name == "tests.rs"
        or name.endswith("_tests.rs")
        or name.startswith("test_")
    )


def _external_test_modules(root: Path, rust_files: Iterable[Path]) -> set[Path]:
    test_modules: set[Path] = set()
    for source_path in rust_files:
        code = mask_non_code(source_path.read_text(encoding="utf-8"))
        for match in EXTERNAL_TEST_MODULE.finditer(code):
            module_name = match.group(1)
            candidates = (
                source_path.parent / f"{module_name}.rs",
                source_path.parent / module_name / "mod.rs",
            )
            for candidate in candidates:
                if candidate.is_file():
                    test_modules.add(candidate.resolve())
                    break
    return test_modules


def collect_metrics(root: Path) -> list[FileMetrics]:
    rust_files = sorted(root.glob("crates/*/src/**/*.rs"))
    rust_files.extend(sorted(root.glob("crates/*/tests/**/*.rs")))
    rust_files = sorted(set(path.resolve() for path in rust_files))
    external_test_modules = _external_test_modules(root, rust_files)

    metrics: list[FileMetrics] = []
    for source_path in rust_files:
        relative = source_path.relative_to(root.resolve())
        text = source_path.read_text(encoding="utf-8")
        physical_lines = len(text.splitlines())
        if _is_conventional_test_path(relative) or source_path in external_test_modules:
            test_lines = physical_lines
        else:
            test_lines = len(cfg_test_line_numbers(text))
        metrics.append(
            FileMetrics(
                path=relative.as_posix(),
                physical_lines=physical_lines,
                implementation_lines=physical_lines - test_lines,
                test_lines=test_lines,
            )
        )
    return metrics


def _implementation_files(metrics: Iterable[FileMetrics]) -> list[FileMetrics]:
    return [metric for metric in metrics if metric.implementation_lines > 0]


def _distribution(metrics: Iterable[FileMetrics]) -> dict[str, int]:
    implementation = _implementation_files(metrics)
    return {
        "implementation_files": len(implementation),
        "at_or_below_500": sum(
            metric.implementation_lines <= DEFAULT_SMALL_FILE_TARGET for metric in implementation
        ),
        "at_or_below_800": sum(
            metric.implementation_lines <= DEFAULT_MAX_IMPLEMENTATION_LINES
            for metric in implementation
        ),
        "above_2000": sum(metric.implementation_lines > 2_000 for metric in implementation),
        "above_5000": sum(metric.implementation_lines > 5_000 for metric in implementation),
        "above_10000": sum(metric.implementation_lines > 10_000 for metric in implementation),
        "implementation_lines": sum(metric.implementation_lines for metric in implementation),
        "test_lines": sum(metric.test_lines for metric in metrics),
    }


def build_baseline(metrics: list[FileMetrics], base_commit: str) -> dict[str, object]:
    implementation = _implementation_files(metrics)
    ratchets = {
        metric.path: metric.implementation_lines
        for metric in implementation
        if metric.implementation_lines > DEFAULT_MAX_IMPLEMENTATION_LINES
    }
    crate_roots = {
        metric.path: metric.implementation_lines
        for metric in implementation
        if Path(metric.path).name in {"lib.rs", "main.rs"}
        and len(Path(metric.path).parts) == 4
    }
    return {
        "schema": SCHEMA_VERSION,
        "base_commit": base_commit,
        "scope": ["crates/*/src/**/*.rs", "crates/*/tests/**/*.rs"],
        "guardrails": {
            "small_file_target": DEFAULT_SMALL_FILE_TARGET,
            "max_implementation_lines": DEFAULT_MAX_IMPLEMENTATION_LINES,
            "tracked_thresholds": list(TRACKED_THRESHOLDS),
        },
        "distribution": _distribution(metrics),
        "ratchets": dict(sorted(ratchets.items())),
        "crate_roots": dict(sorted(crate_roots.items())),
    }


def _fraction_regressed(
    current_numerator: int,
    current_denominator: int,
    baseline_numerator: int,
    baseline_denominator: int,
) -> bool:
    if current_denominator == 0:
        return False
    if baseline_denominator == 0:
        return True
    return current_numerator * baseline_denominator < baseline_numerator * current_denominator


def check_guardrails(metrics: list[FileMetrics], baseline: dict[str, object]) -> list[str]:
    if baseline.get("schema") != SCHEMA_VERSION:
        return [f"unsupported baseline schema: {baseline.get('schema')!r}"]

    current_by_path = {metric.path: metric for metric in _implementation_files(metrics)}
    baseline_ratchets = {
        str(path): int(limit) for path, limit in dict(baseline["ratchets"]).items()
    }
    baseline_roots = {
        str(path): int(limit) for path, limit in dict(baseline["crate_roots"]).items()
    }
    max_lines = int(dict(baseline["guardrails"])["max_implementation_lines"])
    small_target = int(dict(baseline["guardrails"])["small_file_target"])
    baseline_distribution = {
        str(key): int(value) for key, value in dict(baseline["distribution"]).items()
    }
    current_distribution = _distribution(metrics)

    violations: list[str] = []

    for path, metric in sorted(current_by_path.items()):
        if path in baseline_ratchets:
            limit = baseline_ratchets[path]
            if metric.implementation_lines > limit:
                violations.append(
                    f"{path}: {metric.implementation_lines} implementation lines exceeds "
                    f"ratchet {limit}"
                )
        elif metric.implementation_lines > max_lines:
            violations.append(
                f"{path}: new/untracked implementation file has "
                f"{metric.implementation_lines} lines (limit {max_lines})"
            )

        if Path(path).name in {"lib.rs", "main.rs"} and len(Path(path).parts) == 4:
            root_limit = max(baseline_roots.get(path, 0), small_target)
            if metric.implementation_lines > root_limit:
                violations.append(
                    f"{path}: crate root has {metric.implementation_lines} implementation "
                    f"lines (ratchet {root_limit})"
                )

    current_file_count = current_distribution["implementation_files"]
    baseline_file_count = baseline_distribution["implementation_files"]
    for key, threshold in (("at_or_below_500", 500), ("at_or_below_800", 800)):
        if _fraction_regressed(
            current_distribution[key],
            current_file_count,
            baseline_distribution[key],
            baseline_file_count,
        ):
            violations.append(
                f"share of implementation files at or below {threshold} lines regressed: "
                f"{current_distribution[key]}/{current_file_count} vs baseline "
                f"{baseline_distribution[key]}/{baseline_file_count}"
            )

    for threshold in TRACKED_THRESHOLDS:
        key = f"above_{threshold}"
        if current_distribution[key] > baseline_distribution[key]:
            violations.append(
                f"implementation files above {threshold} lines increased: "
                f"{current_distribution[key]} vs baseline {baseline_distribution[key]}"
            )

    return violations


def _git_head(root: Path) -> str:
    result = subprocess.run(
        ["git", "rev-parse", "HEAD"],
        cwd=root,
        check=True,
        text=True,
        capture_output=True,
    )
    return result.stdout.strip()


def _print_report(metrics: list[FileMetrics], baseline: dict[str, object] | None) -> None:
    distribution = _distribution(metrics)
    implementation_files = distribution["implementation_files"]
    print(
        "architecture size: "
        f"{implementation_files} implementation files, "
        f"{distribution['implementation_lines']} implementation lines, "
        f"{distribution['test_lines']} test lines"
    )
    for threshold in (500, 800):
        key = f"at_or_below_{threshold}"
        count = distribution[key]
        share = 100.0 if implementation_files == 0 else count * 100.0 / implementation_files
        print(f"  <= {threshold}: {count}/{implementation_files} ({share:.1f}%)")
    print(
        "  threshold counts: "
        + ", ".join(
            f">{threshold}: {distribution[f'above_{threshold}']}"
            for threshold in TRACKED_THRESHOLDS
        )
    )

    largest = sorted(
        _implementation_files(metrics),
        key=lambda metric: (-metric.implementation_lines, metric.path),
    )[:10]
    print("  largest implementation files:")
    for metric in largest:
        print(
            f"    {metric.implementation_lines:>6} impl + "
            f"{metric.test_lines:>6} test  {metric.path}"
        )

    if baseline is not None:
        print(f"  baseline commit: {baseline['base_commit']}")


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--root",
        type=Path,
        default=Path(__file__).resolve().parents[2],
        help="repository root",
    )
    parser.add_argument(
        "--baseline",
        type=Path,
        default=Path(__file__).with_name("architecture-size-baseline.json"),
        help="baseline JSON path",
    )
    parser.add_argument(
        "--write-baseline",
        action="store_true",
        help="replace the baseline with measurements from the current tree",
    )
    parser.add_argument(
        "--base-commit",
        help="commit recorded in a generated baseline (defaults to current HEAD)",
    )
    parser.add_argument(
        "--json",
        action="store_true",
        help="print current per-file metrics as JSON",
    )
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    root = args.root.resolve()
    baseline_path = args.baseline
    if not baseline_path.is_absolute():
        baseline_path = root / baseline_path

    metrics = collect_metrics(root)
    if args.json:
        print(json.dumps([asdict(metric) for metric in metrics], indent=2))
        return 0

    if args.write_baseline:
        base_commit = args.base_commit or _git_head(root)
        baseline = build_baseline(metrics, base_commit)
        baseline_path.write_text(
            json.dumps(baseline, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
        _print_report(metrics, baseline)
        print(f"wrote {baseline_path}")
        return 0

    try:
        baseline = json.loads(baseline_path.read_text(encoding="utf-8"))
    except FileNotFoundError:
        print(f"missing architecture baseline: {baseline_path}", file=sys.stderr)
        return 2

    _print_report(metrics, baseline)
    violations = check_guardrails(metrics, baseline)
    if violations:
        print("architecture size guard failed:", file=sys.stderr)
        for violation in violations:
            print(f"  - {violation}", file=sys.stderr)
        print(
            "reduce the implementation owner or intentionally ratchet a reviewed baseline; "
            "do not regenerate the baseline to hide growth",
            file=sys.stderr,
        )
        return 1

    print("architecture size guard passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
