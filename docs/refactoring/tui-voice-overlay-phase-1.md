# TUI App split — voice overlay phase

This mechanical phase follows ADR-0004 and the canonical architecture campaign.
It extracts pure `/voice` overlay state from `app.rs` into private `voice.rs`.

## Ownership and compatibility

`voice.rs` owns voice phase/state, waveform bounds and updates, elapsed labels,
and transcript insertion. `App` retains the voice field, event application,
modal precedence, rendering, and driver coordination. Existing public paths
remain re-exported from `forge_tui::app` and the crate root.

No presenter event, replay format, TUI command, keybinding, terminal I/O,
transcription process, provider request, or persistence behavior changes.

## Result

| Measure | Before | After |
|---|---:|---:|
| `app.rs` implementation lines | 7,208 | 7,091 |
| `voice.rs` implementation lines | — | 123 |
| Workspace implementation files ≤500 | 119/184 | 120/185 |
| Workspace implementation files ≤800 | 149/184 | 150/185 |

The new owner is cohesive and below 500 lines; aggregate scaffolding is
reported by the architecture guard rather than hidden.

## Verification

- warnings-denied TUI Clippy across targets/features
- TUI suite: 235 unit tests plus one long-session replay integration test
- formatting and architecture guard passed

No runtime benchmark is required because no model-visible, provider, session,
persistence, or event-order behavior changed.
