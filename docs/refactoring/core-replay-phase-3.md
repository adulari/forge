# Core transcript replay extraction

This focused Core phase moves the single persisted-message → presenter-replay
mapping into private `replay.rs`.

The mapping remains shared by live model-facing replay and Store-backed full
history replay. It preserves visible-message filtering, user/assistant/tool
ordering, tool-call/result name matching, first-line result summaries, error
color classification, and the sole user-visible compaction marker.

Core root implementation lines reduce from 10,478 to 10,424; `replay.rs` is 61
lines. Focused replay coverage, long-session endurance (including compaction),
warnings-denied Core Clippy, formatting, and the architecture guard passed.
No presenter event order, persistence behavior, or TUI replay format changed.
