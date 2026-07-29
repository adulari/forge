# Code archaeology: shell model-output rendering

## Boundary

`shell/output.rs` owns the conversion of captured process bytes into safe model-facing text. Process spawning, timeouts, sandbox setup, and process-tree cleanup remain in `shell.rs`.

## Invariants

- NUL-containing output is summarized as binary rather than embedded in a tool response.
- Terminal CSI, OSC, and charset escape sequences do not leak control payloads into model context.
- Stderr remains visibly attributed after stream rendering.
- Token-budget truncation preserves UTF-8 boundaries, both head and tail context, and never exceeds the advertised output budget.

## Characterization

`rendering_removes_terminal_control_data_and_preserves_stderr` and `truncation_is_utf8_safe_and_retains_both_ends` protect the module interface. Existing shell integration tests exercise the renderer on real command output.
