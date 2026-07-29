# Serve workflow-library ownership phase

## History and boundary

The workflow library entered Serve with the Hearth control surface (`cdc0e20d`) and gained typed arguments and durable run projection in the Machined phases (`60f82a28`, `42225e49`). Those changes evolved as one domain: discover `.forge/workflows/*.js`, parse authored metadata, scope durable run history to the canonical workspace, and project an honest API row. The HTTP composition root only selects the session workspace and registers the route.

This phase moves that complete policy and its parser/status unit tests to `serve/serve_workflows.rs`. The existing real-router characterization remains in `serve.rs` to prove route wiring, workspace isolation, ordering, live-run semantics, and empty history for an unrun workflow. The old parser, workflow response models, history conversion, and handler are deleted from the root; the new owner is substantive rather than a wrapper.

## Measured result

- Serve root: 3,627 to 3,327 implementation lines.
- New workflow owner: below 500 implementation lines.
- Repository distribution: 216/295 (73.2%) at or below 500 and 274/295 (92.9%) at or below 800.
- Owners above 2,000 remain eight; no owner exceeds 5,000.

This is an intermediate deep-domain phase. It does not claim the canonical 90%/95% terminal distribution gates, regenerate the baseline, or enable auto-merge.

## Verification

Focused workflow and Serve tests, warnings-denied all-target/all-feature CLI Clippy, formatting, and the architecture guard pass. Independent review confirmed the history-backed deep seam and requested explicit imports plus escaped-title parser coverage; both are included before commit.
