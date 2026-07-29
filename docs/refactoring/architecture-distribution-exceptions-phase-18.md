# Architecture distribution exception ledger

This ledger records the remaining implementation owners above the canonical 800-line target after the deep-domain campaign through phase 18. It does not waive the no-growth ratchets or permit new owners above 800.

## Why the percentage gate cannot be reached by file creation

At the phase-18 measurement, 164 of 243 implementation files are at or below 500 lines and 208 are at or below 800. Adding small files without deleting or shrinking an existing owner would require 547 new files to reach 90% and 457 to reach 95%; that is shallow inflation and is prohibited. Progress must therefore come from replacing existing broad owners with cohesive owners, one independently reviewed domain phase at a time.

## Canonical exceptions retained

The following categories are temporarily canonical because their public interface or state-machine boundary is materially broader than 800 lines and no independently testable deletion boundary was established in this campaign:

- **Composition roots:** `forge-cli` run, Serve, TUI App, Core, Store, Config, and provider CLI roots. These are compatibility/composition surfaces whose remaining code coordinates already-extracted policy owners. They remain frozen by the architecture baseline and must only shrink.
- **Protocol models and command schemas:** CLI `args.rs`, `forge-types`, MCP root, Mesh catalog/root, and Skills root. Splitting these public schemas changes documentation paths, derive ownership, or downstream import surfaces; a future phase requires explicit API/deletion tests first.
- **Long-lived state machines:** run driver/dispatch, Anywhere connector/handoff, Codex OAuth, subagents, capsule, permissions, and shell execution. Their control flow crosses cancellation, replay, or security boundaries. Each requires characterization around crash/cancel/replay behavior before movement.
- **Command/service owners:** models, local, schedule, MCP serve, API serve, benchmark, index, and TUI commands. Each is already a single user-facing command or service owner; further splitting requires a demonstrated subordinate policy boundary rather than line-range movement.

## Completed deep ownership splits

Config editor/provider registries, CLI-provider stream parsing, Serve PWA assets, Anywhere durable state, Anywhere connector routing and command journal, remote upload/voice handlers, core discovery tools, and genai error policy were extracted with focused tests and warnings-denied Clippy. Every new owner is below 800 lines.

## Enforcement

These exceptions are not a regenerated baseline and do not relax CI. Existing per-file ratchets, crate-root ratchets, the prohibition on new files above 800, and distribution non-regression remain authoritative. A follow-up may remove an exception only with history evidence, deletion/interface tests, focused and full gates, and a lower measured owner count.
