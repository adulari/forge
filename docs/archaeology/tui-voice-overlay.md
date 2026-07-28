# Code archaeology: TUI voice overlay

## Summary

The `/voice` overlay is pure App presentation state: it tracks download,
recording, transcription, errors, waveform samples, timing, and safe transcript
insertion. The real microphone, model download, and transcription work stays in
the driver. Extracting this owner keeps `App` as the run-loop/state owner while
making voice behavior discoverable without changing replay or terminal I/O.

## Timeline

- **2026-07-19, `f55e984e` / PR #636:** added the voice overlay, waveform,
  local transcription integration, and transcript insertion behavior.
- **2026-07-24, `380155eb` / PR #890:** fixed Linux waveform publication;
  waveform state must therefore remain bounded and driver-fed rather than being
  reconstructed during rendering.

## Load-bearing invariants

- Voice state contains no recorder/thread/model resource, keeping `App` pure,
  cloneable, and replay-safe.
- Waveform storage is bounded and clamps untrusted RMS readings.
- Transcript insertion is cursor-safe, preserves UTF-8 string boundaries via
  Rust string APIs, and adds joining spaces only when needed.
- App continues to own overlay precedence, event application, and rendering;
  driver control-flow does not move.

## Evidence

Focused unit coverage verifies waveform bounding/clamping, elapsed formatting,
and all transcript-insertion spacing, trimming, and stale-cursor cases. The
TUI crate suite includes the retained long-session replay integration test.
