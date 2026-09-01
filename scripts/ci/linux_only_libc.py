#!/usr/bin/env python3
"""Reject glibc-only libc symbols that are not gated to Linux.

The macOS release legs are the only place these are compiled, and release.yml is
workflow_dispatch-only, so a `#[cfg(unix)]` block calling a glibc-only accessor passes every PR
check and then fails the release. v2.13.3's aarch64-apple-darwin leg died exactly this way:

    error[E0425]: cannot find function `__errno_location` in crate `libc`

`libc` exposes these names only for Linux/gnu targets; Apple spells the same concepts differently
(`__error` for errno). A use must therefore sit under a cfg that names Linux, not merely `unix`.
Portable std equivalents are usually better still: `std::io::Error::last_os_error()` for errno.

Exit 0 when clean; print `guard failed: ...` lines and exit 1 otherwise.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]

# Names `libc` defines for linux-gnu and not for Apple. Extend as new ones are reached for.
LINUX_ONLY = (
    "__errno_location",
    "gettid",
    "memfd_create",
    "pidfd_open",
    "syscall",
    "prctl",
    "epoll_create1",
    "eventfd",
    "inotify_init1",
    "statx",
)

USE = re.compile(r"\blibc::(" + "|".join(LINUX_ONLY) + r")\b")
CFG = re.compile(r"#\s*!?\[\s*cfg")
LINUX = re.compile(r'target_os\s*=\s*"linux"|target_env\s*=\s*"gnu"')
# How far above a use to look for the cfg attribute that gates it.
LOOKBACK = 12


def offenders(path: Path, text: str) -> list[str]:
    lines = text.splitlines()
    found = []
    for number, line in enumerate(lines, start=1):
        match = USE.search(line)
        if not match or line.lstrip().startswith("//"):
            continue
        window = lines[max(0, number - 1 - LOOKBACK) : number - 1]
        gated = any(CFG.search(above) and LINUX.search(above) for above in window)
        if not gated:
            rel = path.relative_to(REPO_ROOT) if path.is_relative_to(REPO_ROOT) else path
            found.append(
                f"guard failed: {rel}:{number} uses libc::{match.group(1)}, which libc defines "
                'only for Linux; gate it with #[cfg(target_os = "linux")] or use a portable '
                "equivalent, or the macOS release legs will not compile"
            )
    return found


def main(argv: list[str]) -> int:
    if argv:
        paths = [Path(a).resolve() for a in argv]
    else:
        paths = sorted((REPO_ROOT / "crates").rglob("*.rs"))

    failures = []
    for path in paths:
        failures.extend(offenders(path, path.read_text(encoding="utf-8", errors="replace")))

    for failure in failures:
        print(failure, file=sys.stderr)
    if failures:
        return 1
    print(f"linux-only libc guard: {len(paths)} files clean")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
