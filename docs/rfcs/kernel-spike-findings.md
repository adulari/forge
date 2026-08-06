# Kernel spike — round 1 findings (brokering + persistence)

Date: 2026-08-06. Runnable prototype: `docs/rfcs/kernel-spike/` (`python host.py <allowed-dir>`).
Scope of this round: prove or disprove the **mechanism** the RFC's decision rests on — a persistent
Python control process whose side effects can only leave through a typed host broker, with a
provable deny path and state that survives a kernel restart. Criterion 1 (token/wall-clock win on
a representative task set) is NOT covered here; it needs the real harness and is round 2.

## What was built

A ~90-line Python kernel process plus a host that plays Forge's session core. Line-JSON on stdio:
the host sends `exec`; the kernel runs code in a persistent namespace; `forge.write_file`,
`forge.read_file`, and `forge.shell` inside that namespace each emit a typed `host_request` and
block until the host answers. The host applies a policy (writes/reads allowed only under one
directory, shell denied outright) exactly where Forge's permission broker would sit.

## Results (measured, 4 runs)

| Claim | Result |
|---|---|
| Python state survives across separate tool calls | PASS — `x` computed in call 1, printed in call 2 |
| Brokered write + read round-trip | PASS |
| A denied write **fails inside the script** and no file appears | PASS — `PermissionError: host denied fs.write: write outside allowed dir refused`, target absent afterwards |
| Denied shell | PASS |
| Script can catch a denial and keep running | PASS |
| State survives a full kernel restart via snapshot/restore | PASS — pickle blob, 132 B for this namespace |
| Direct `open()` bypasses the broker | **FAIL — leak confirmed** |

Timings on this host (Python 3.14): first `exec` round-trip 27–34 ms across runs, including
interpreter warmup and a 1M-element sum. `startup=0ms` in the output is **not** a readiness
measure — `Popen` returns before the interpreter is usable; treat first-exec as the real number.
Restart+restore was likewise sub-millisecond at this namespace size and says nothing about a
realistic one.

## The finding that matters

**Brokering the `forge.*` API is necessary but not sufficient.** Model-written Python can call
`open()`, `os.system`, or `subprocess` directly and never touch the broker — confirmed by the last
check, which wrote a file the policy would have refused. So the RFC's ADR-0008 guarantee cannot
rest on the API surface alone. Any real implementation needs OS-level confinement of the kernel
process (Landlock on Linux — Forge already uses it for `[shell] scoped_cargo_target`; seccomp for
`execve`; equivalents elsewhere), with the broker as the *only* path back out for anything the
sandbox denies. That is a design constraint discovered by measurement, not a blocker: it means the
kernel work depends on the sandbox work, and the sandbox layer should be specified first.

Two implications for the full-kernel decision:

1. Sequencing changes — confinement precedes the kernel, not the other way round.
2. Portability cost is real: Landlock is Linux-only. macOS/Windows either get a weaker guarantee
   (documented) or the kernel tool stays opt-in there. This needs a call before round 2.

## Round 2 (not yet run)

- Criterion 1: token/wall-clock comparison, scripted vs discrete tools, on a representative task
  set inside the real harness.
- Interpreter choice: this spike used the system `python3`. The single-binary question
  (wasm-sandboxed interpreter vs external process) is open and interacts directly with the
  confinement finding above.
- Compaction/detach survival against Forge's actual session lifecycle, not a synthetic restart.
