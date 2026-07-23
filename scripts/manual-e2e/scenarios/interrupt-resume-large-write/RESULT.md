# Interrupt/resume large-write probe

This fault-injection scenario interrupts a real Forge TUI turn while the model is generating a
large `write_file` call, then resumes the same persisted session with a recovery prompt. The
acceptance verifier requires exactly 321 lines, the exact header, monotonically numbered `0001`
through `0320` records, and an alphanumeric payload of at least 90 characters on every record.

Each run retains the interrupted and resumed raw terminal logs, timelines, session ID, final file,
artifact verifier output, and credential-free persisted tool-envelope integrity report under
Forge's persistent `manual-e2e-runs/interrupt-resume-large-write-*` directory.

## Verified live result (2026-07-23)

Using `qwencloud::qwen3.8-max-preview`, Forge interrupted the initial turn after 25 seconds and
resumed the exact persisted session. The resumed large `write_file` stream stayed on one healthy
provider attempt for 602.6 seconds, streamed 36.3k output tokens with no model-output cap, and
surfaced 8,536 buffered provider events in the TUI instead of tripping the former 120-second core
idle watchdog. The final artifact passed all 321-line, `0001`–`0320`, and payload-length checks;
all 6 tool-call envelopes and all 6 execution records were valid and matched. The successful run
is retained locally as `interrupt-resume-large-write-20260723T023432Z-1797693`.
