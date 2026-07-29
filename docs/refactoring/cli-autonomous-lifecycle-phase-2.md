# CLI run vertical slice — autonomous turn lifecycle

This phase moves `/loop` and `/goal` stop policy, queue steering, interrupted
presenter cleanup, generation rebinding, and background normal/expanded/
compaction turn spawning into `run/autonomous.rs`.

The owner preserves the critical generation invariant: every DoneGuard carries
the turn generation so stale completion signals cannot stop a replacement turn;
queued user corrections remain FIFO and receive the next autonomous iteration;
interruption closes workflow state before a replacement prompt begins.

`run.rs` implementation lines reduce from 6,111 to 5,897. The extracted owner
is 222 lines. Warnings-denied CLI Clippy, 444 CLI tests (4 ignored live-network
cases), formatting, and the architecture guard passed.
