# Aetherfront reference

Open `reference/index.html` directly in a browser to play **Aetherfront: Last Bastion**. The saved
reference is a 42 KB single-file Canvas RTS generated during a real Forge TUI run. It has six unit
types, six building types, three control points, fog of war, resource harvesting, production,
selection and commands, pause/settings, victory/defeat state, and local best statistics.

Automated acceptance uses `verify.js` and requires Chrome plus the bundled Chrome remote-interface
module available on this development machine. The original run passed with zero browser exceptions.
The verifier normalizes multiple valid implementations instead of requiring the reference game's
private variable names. Every run retains a screenshot and adjacent `*.verification.json` gameplay
report.

## Verified unpinned-mesh result (2026-07-23)

An unpinned live TUI run routed the parent through the Codex subscription and delegated independent
design/verification work to Codex and NVIDIA NIM models. It produced a distinct 47 KB playable
implementation. The implementation-independent browser verifier confirmed its embedded self-check
(6 unit types, 7 building types, 3 abilities, and 3 control points), tutorial round trip, start,
simulation advance, both armies, economy/world entities, pause/resume, selection and movement,
visible HUD, and zero runtime exceptions. The HTML, screenshot, TUI timeline, and session are
retained as `aetherfront-20260723T025111Z-1833126` in Forge's persistent `manual-e2e-runs/`
directory.

## Second verified unpinned-mesh result (2026-07-23)

A second fresh run produced another distinct 46.5 KB implementation through an unpinned Codex
mesh, using two parent models plus independent `game-designer` and `qa-reviewer` children. Forge
preserved an eight-minute child provider wait with an explicit no-event warning, then completed a
13,006-byte `write_file` and four coherent `append_file` operations without truncation or malformed
JSON. All 23 parent and 1 child tool executions were OK; usage recorded 25.4k output tokens and
215.7k cached input tokens.

The first outer browser run exposed verifier overfitting rather than a game defect: this game keeps
state in an IIFE-local `game`, exports `window.AETHERFRONT_SELF_CHECK`, uses `#field` for its Canvas,
and names its pause overlay `#pauseMenu`. The verifier now injects a scope-local behavior adapter,
accepts object or function self-checks, normalizes selection/movement and overlay variants, and
requires a nonzero Canvas. Both the committed reference and the new game independently pass with
live simulation advancement, both armies, 6 building types, 8 crystal fields, exactly 3 control
points, pause/resume, selection/movement, tutorial behavior, a 1440×757 Canvas, and zero browser
exceptions. The playable result, screenshot, reports, session, and timeline are retained as
`aetherfront-20260723T034609Z-1910381` in Forge's persistent `manual-e2e-runs/` directory.
