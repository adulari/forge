# Forge-wide refactor campaign closeout — 2026-07

## Disposition

This document closes the bounded refactor campaign carried by PR
[#932](https://github.com/Adulari/forge/pull/932). The branch contains 109 commits and changes
259 files relative to `main`. It is a substantial architecture and reliability improvement, but it
does **not** satisfy every numerical target from the original campaign brief. In particular, this
closeout must not be cited as evidence of 100% code coverage or of the 90%/95% file-size
distribution targets.

The campaign was closed for merge after the operator directed the work to stop the serial
micro-extraction loop, reduce workstation load, fix supported release blockers, and document the
remaining gap honestly.

## Delivered architecture work

The campaign replaced broad mixed-responsibility owners with cohesive private modules across the
main runtime:

- Core session lifecycle, history, context-program construction, replay, controls, tool dispatch,
  orchestration, and completeness/quality policy gained explicit owners.
- Model Mesh classification and routing-context policy were separated from catalog and execution
  plumbing.
- CLI autonomous-run support, session restoration, remote input, workflow/duel support, shell
  setup, and voice support were separated from the interactive composition root.
- TUI overlay and voice-overlay behavior moved out of the central app owner.
- Store handoff persistence and synchronization-journal policy gained focused owners.
- Configuration path policy, provider/model discovery, Codex quota projection, bridge budgeting,
  subagent execution, API wire translation, tool edit matching, notebook handling, permission
  decomposition, queue drain support, and imported Claude policy were isolated.
- Serve workflow, MCP catalog, project discovery, configuration editing, changelog, usage, and
  model-catalog domains gained private owners with characterization tests.

The phase archaeology and boundary evidence live in the other documents under
`docs/refactoring/`.

## Measured architecture result

The canonical architecture-size tool reports:

| Measure | Closeout result | Original target | Status |
|---|---:|---:|---|
| Implementation files at or below 500 LOC | 222/301 (73.8%) | at least 90% | Not reached |
| Implementation files at or below 800 LOC | 280/301 (93.0%) | at least 95% | Not reached |
| Implementation owners above 2,000 LOC | 8 | deep extraction or narrow exception | Partially reduced |
| Implementation owners above 5,000 LOC | 0 | 0 | Reached |
| Implementation owners above 10,000 LOC | 0 | 0 | Reached |

With a fixed 301-file denominator, another 49 existing owners would need to move to 500 LOC or
below and another 6 would need to move to 800 LOC or below. New extracted files also change the
denominator, so those figures are lower bounds rather than a completion plan.

The largest remaining implementation owners at closeout are:

| Owner | Implementation LOC |
|---|---:|
| `crates/forge-cli/src/cli/commands/run.rs` | 4,136 |
| `crates/forge-core/src/lib.rs` | 3,964 |
| `crates/forge-tui/src/app.rs` | 3,338 |
| `crates/forge-config/src/lib.rs` | 2,991 |
| `crates/forge-provider/src/cli_provider.rs` | 2,928 |
| `crates/forge-cli/src/serve.rs` | 2,434 |
| `crates/forge-cli/src/anywhere/mod.rs` | 2,079 |
| `crates/forge-cli/src/remote.rs` | 2,060 |

The architecture guard passes because the campaign ratcheted the checked-in baseline, introduced
no implementation owner above 5,000 LOC, and documented the remaining exceptions. This is not the
same claim as reaching the long-term distribution targets.

## Reliability and correctness fixes found during the refactor

Independent reviews and characterization work found and fixed defects beyond file movement,
including:

- OAuth pasted callbacks can no longer bypass CSRF-state validation.
- LSP initialization, document synchronization, stderr sanitization, recovery cooldown, EOF
  recovery, global process permits, idle reaping, and failed-process cleanup were hardened.
- Timed-out and dropped Serve session drivers are explicitly aborted instead of becoming detached
  tasks that retain full session state.
- Serve prunes unexpectedly completed drivers and performs bounded joins during daemon shutdown.
- Queue repository validation, gate exit handling, failed-task branches, and assay semantics were
  corrected.
- MCP dynamic client registration and device-flow separation were hardened.
- Explicit session model pins survive reservation pressure.
- Context-window derivation no longer borrows unrelated provider or subscription metadata.
- Claude import policy, scopes, aliases, MCP destinations, allowlists, marker reconciliation,
  serialization, and error propagation were hardened.
- Serve MCP mutation preserves malformed user catalogs instead of overwriting them.
- Project browsing rejects relative-path ambiguity and canonical symlink escapes.
- Configuration mutation is serialized and its scoped-reset contract is documented accurately.
- Usage database failures no longer become plausible zero values; Codex alias/freshness semantics,
  Gemini classification, and TypeScript wire parity were corrected.
- Serve model projection includes Store-fetched pricing.

## Memory incident and operational state

Two campaign runs overloaded the workstation:

- A `rust-analyzer` process was OOM-killed at approximately 3.35 GiB anonymous RSS.
- A later analyzer reached approximately 4.0 GiB RSS.
- Before detached-driver cleanup was installed, `earlyoom` terminated the Forge daemon at
  approximately 6.74 GiB RSS.
- The systemd unit peaked between approximately 15.4 and 16.6 GiB including children and used
  roughly 2.1 GiB swap.

The product now generates `OOMPolicy=continue`, bounds live analyzer processes process-wide,
replaces per-request idle timers with per-slot lifecycle handling, reaps idle analyzers, and aborts
retained driver/turn tasks deterministically. Those changes materially reduce unbounded retention,
but they do not provide a portable resident-memory ceiling for one analyzer.

For the closeout and post-merge verification on this workstation, user configuration therefore
uses:

```toml
[lsp]
enabled = false

[mesh.subagents]
max_agents = 2
max_concurrency = 1
```

This prevents another analyzer-driven overload while keeping Forge Anywhere usable. Re-enabling
LSP should be treated as an explicit operator decision until a cross-platform resident-memory
budget exists.

## Coverage and validation truth

`cargo llvm-cov`/`cargo-llvm-cov` is not installed in the campaign environment, so line, branch, and
function coverage percentages were not measured. Passing tests do not imply 100% coverage.
Consequently:

- no 100% coverage claim is made;
- no claim that every behavior in the entire codebase was refactored is made;
- phase-specific characterization and deletion-critical tests are the evidence for changed
  boundaries.

The merge gate is the repository's normal required CI plus one consolidated local verification:
formatting, warnings-denied Clippy, workspace tests, architecture-size tests/guard, and
`git diff --check`. The final PR check state and published-main parity are recorded in the PR and
merge history rather than frozen into this document while checks are still running.

## Remaining work

The following are explicit follow-ups, not hidden completion claims:

1. Continue deep domain extraction for the eight owners above 2,000 implementation LOC.
2. Reach the 90% at-or-below-500 and 95% at-or-below-800 distributions without shallow wrapper
   files or line-count slicing.
3. Add and run a maintained coverage job if a numerical coverage target is required.
4. Add a portable aggregate resident-memory budget for LSP children before enabling LSP by default
   on constrained machines.
5. Repeat long-session memory endurance with LSP both disabled and explicitly enabled under that
   future budget.

This closeout intentionally leaves those gaps visible so a future campaign can start from measured
facts rather than from a false “100% complete” label.

## Post-closeout LSP resource follow-up (2026-07-29)

The LSP safety gap above was addressed by
[ADR-0013](../architecture/decisions/0013-resource-bounded-lsp-processes.md). Forge now combines a
single process-wide live-analyzer permit, a configurable 2 GiB process-tree RSS guard, 120-second
idle reaping, and a lightweight Rust Analyzer profile. A real-workspace probe reduced the measured
cold-start peak from 3.7 GiB/37 processes/approximately 14 cores to 1675.7 MiB/four processes/
approximately one core while still reporting an intentional Rust type error in 9.03 seconds. The
final RSS remained at the measured peak through the 60.79-second run and the 2 GiB guard did not
fire.

The operational workaround `[lsp] enabled = false` is therefore no longer required for this
workstation once a build containing ADR-0013 is installed. The historical measurements and
workaround remain above to preserve an accurate record of the original campaign state.
