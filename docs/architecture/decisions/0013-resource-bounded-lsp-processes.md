# ADR-0013: Resource-bounded LSP processes

- **Status:** Accepted
- **Date:** 2026-07-29
- **Deciders:** Forge maintainers

## Context

Forge keeps language servers alive between post-edit diagnostic requests. Lifecycle hardening already
limited the number of retained roots and reaped idle processes, but it did not constrain one live
analyzer or its descendants. During the architecture campaign, individual `rust-analyzer` processes
reached approximately 3.35–4.0 GiB RSS. A controlled reproduction on the Forge workspace showed the
same failure mode: initialization launched 37 Cargo/build-script processes, reached 3.7 GiB aggregate
RSS and approximately 14 cores, and crossed a 2 GiB safety cutoff in 2.1 seconds.

Disabling `[lsp]` avoided overload but removed live diagnostics. Limiting only the number of roots
also did not help when one analyzer was sufficient to exhaust the machine.

## Decision

Forge applies three complementary controls:

1. At most one language-server process tree is live per Forge process. Four lazy root slots remain
   available, but only one may own the process-wide permit.
2. Every live language-server tree has a resident-memory guard. Forge samples the server and all of
   its descendants every 250 ms and terminates children before the server when the tree exceeds
   `[lsp].memory_limit_mb` (default 2048 MiB). Setting the value to `0` explicitly disables the hard
   guard. Idle servers are reaped after `[lsp].idle_timeout_secs` (default 120 seconds).
3. Rust Analyzer receives a lightweight initialization profile:
   - one analyzer worker and one cache-priming worker;
   - cache priming and check-on-save disabled;
   - build scripts and procedural macros disabled while dependency type information stays loaded;
   - expected `macro-error` noise from disabled procedural macros suppressed;
   - the syntax-tree LRU reduced from 128 entries to 32;
   - `CARGO_BUILD_JOBS=1` and `RAYON_NUM_THREADS=1` inherited by analyzer children.

This profile still provides on-demand syntax and semantic diagnostics with dependency type
information. Full build-script, procedural-macro, and all-target validation remains the
responsibility of Forge's explicit `cargo check`/Clippy/test pipeline, where it is visible, bounded,
and intentionally invoked instead of duplicated after every edit.

A cold analyzer gets up to twelve seconds for its first diagnostic. A diagnostic timeout keeps the
healthy server warm and returns no result for that best-effort request. Late publications older than
the document version Forge just synchronized are ignored, preventing a timed-out request from
surfacing stale results on the next edit. Broken pipes, EOF, rejected initialization, and
resource-guard termination still drop the process and use the existing exponential recovery
cooldown.

## Evidence

The final profile was exercised against this workspace with Rust Analyzer 1.96.0:

- initialization: 0.29 seconds;
- intentional type-error diagnostics: 9.03 seconds;
- peak/final process-tree RSS over 60.79 seconds: 1675.7 MiB;
- peak CPU: approximately one core;
- average CPU over the run: 13.6% of one core;
- maximum process-tree size: four processes;
- the 2 GiB guard did not fire.

The same probe before dependency/build isolation reached 3.7 GiB, 37 processes, and approximately
14 cores. Unit coverage also verifies process-tree discovery, initialization options, the
process-wide live-server limit, idle reaping, timeout reuse, and actual termination of an
over-budget fake server tree.

## Alternatives considered

- **Keep LSP disabled.** Rejected because it makes the feature safe only by making it unavailable.
- **Rely on analyzer-count and idle limits.** Rejected because the measured incident required only
  one analyzer.
- **Use Linux cgroups or systemd limits.** Useful for deployment hardening, but not portable to
  macOS and Windows and too coarse when Forge also owns model and tool subprocesses.
- **Use `RLIMIT_AS`.** Rejected because virtual-memory mappings are not a portable proxy for
  resident memory and can terminate analyzers far below the intended RSS budget.
- **Run unrestricted Rust Analyzer behind only the RSS guard.** Rejected because it repeatedly pays
  the overload cost before termination and never becomes useful on constrained machines.

## Consequences

- LSP can be enabled on constrained workstations without allowing one analyzer to exceed 2 GiB or
  consume all cores.
- Rust diagnostics intentionally favor fast workspace-local feedback over full Cargo fidelity.
  The explicit Rust quality suite remains authoritative.
- A very large non-Rust language server may need a higher `memory_limit_mb`; the setting is
  operator-controlled.
- Only one concurrent Forge session receives live LSP service. Other sessions degrade to empty
  best-effort diagnostics until the permit becomes available, while normal model/tool work
  continues.
