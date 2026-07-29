# TUI App split — routing and usage overlays phase

This mechanical TUI phase moves the pure routing/usage overlay data family from
`app.rs` into private `overlays.rs`. `App` retains all event folding, overlay
precedence, key routing, snapshot projection, and rendering; the extracted
structures own only presenter-fed data and a local aggregation helper.

The stable `forge_tui::app::*` paths remain re-exported. No presenter event,
TUI replay, remote projection, route explanation, quota policy, terminal I/O,
or session behavior changes.

`app.rs` reduces from 7,091 to 6,942 implementation lines and the cohesive
owner is 157 lines. TUI warnings-denied Clippy, 235 unit tests, long-session
replay integration, formatting, and the architecture guard passed.
