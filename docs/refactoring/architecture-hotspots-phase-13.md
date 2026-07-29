# Architecture hotspot completion phase

This phase completes the canonical 5,000-line hotspot objective without adding baseline exceptions.

## Extracted ownership

- Core provider execution is split into model-loop orchestration, provider request/failover policy, stream projection, response persistence, and direct tool execution. Every new Core owner is at or below 800 implementation lines.
- Store synchronization, session/usage/spending, and Lattice persistence are separate owners. The Store root is below 5,000 implementation lines.
- TUI remote projection and rendering are separated from application state. Rendering is divided into live composition, transcript/activity, input, overlays, voice, and status owners. The App root is below 5,000 implementation lines.
- CLI interactive-run support is separated into shell setup, voice capture/transcription, workflow and duel lifecycle, remote projection, remote input/attachments, and restore/session helpers. The run root is below 5,000 implementation lines and every new support owner is below 500 lines.

## Measured scorecard

The final draft measurement reports zero implementation files above 5,000 lines. File distribution improves from 66.2% to 67.0% at or below 500 lines and from 82.8% to 85.0% at or below 800 lines.

The proposed 90%/95% distribution figures are not accepted as a phase-completion gate. Reaching them from the measured tree requires at least 54 additional net owners at or below 500 lines and 24 additional net owners at or below 800 lines. Creating those files mechanically would be shallow inflation, while deep extraction spans unrelated Store, Config, Serve, Provider, Anywhere, and command domains and cannot be truthfully represented as completion of these four hotspot seams. The enforced canonical policy remains the architecture guard: no new owner above 800, no distribution regression, and ratcheted existing hotspots. Repository-wide deepening should continue as independently reviewed domain phases.

## Verification

Focused Core, Store, TUI, and CLI tests pass. Formatting, warnings-denied Clippy for the affected CLI/TUI graph, and the architecture guard pass. No dependency was updated and no architecture baseline was regenerated.
