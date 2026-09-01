#!/usr/bin/env python3
"""Guards scripts/ci/linux_only_libc.py. Case 1 is the code that broke the v2.13.3 macOS leg."""

from __future__ import annotations

import subprocess
import sys
import tempfile
from pathlib import Path

GUARD = Path(__file__).resolve().with_name("linux_only_libc.py")


def run(source: str | None = None) -> subprocess.CompletedProcess[str]:
    args = [sys.executable, str(GUARD)]
    if source is None:
        return subprocess.run(args, capture_output=True, text=True)
    with tempfile.TemporaryDirectory() as work:
        path = Path(work) / "probe.rs"
        path.write_text(source)
        return subprocess.run([*args, str(path)], capture_output=True, text=True)


def check(condition: bool, message: str) -> None:
    if not condition:
        raise SystemExit(f"test failed: {message}")


# 1. The real defect: a glibc-only accessor reached from a merely-unix cfg.
result = run(
    """
pub fn pid_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        unsafe { libc::kill(pid as i32, 0) == 0 || *libc::__errno_location() == libc::EPERM }
    }
}
"""
)
check(result.returncode == 1, "a cfg(unix) use of __errno_location must fail")
check("__errno_location" in result.stderr, f"the error must name the symbol: {result.stderr}")

# 2. The fix that shipped: errno read through std, no platform symbol at all.
result = run(
    """
    unsafe { libc::kill(pid as i32, 0) == 0 }
        || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
"""
)
check(result.returncode == 0, f"the portable form must pass: {result.stderr}")

# 3. Correctly gated Linux-only code stays allowed.
result = run(
    """
#[cfg(target_os = "linux")]
fn tid() -> i64 {
    unsafe { libc::gettid() as i64 }
}
"""
)
check(result.returncode == 0, f"a linux-gated use must pass: {result.stderr}")

# 4. A gate far above the use is not a gate this guard can see; it must not pass by accident.
result = run("\n".join(['#[cfg(target_os = "linux")]', *["// filler"] * 14, "libc::gettid();"]))
check(result.returncode == 1, "a cfg beyond the lookback window must not count as gating")

# 5. Commented-out code is not compiled and must not fail the build.
result = run("// unsafe { libc::__errno_location() };\n")
check(result.returncode == 0, f"a comment must not fail: {result.stderr}")

# 6. The committed tree is what ships; it must satisfy the guard as committed.
result = run()
check(result.returncode == 0, f"the committed crates must satisfy the guard: {result.stderr}")

print("linux-only libc guard is enforced")
