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
visible HUD, and zero runtime exceptions. The HTML, screenshot, TUI timeline, and session remain
under `scripts/.manual-e2e-out/aetherfront-20260723T025111Z-1833126/`.
