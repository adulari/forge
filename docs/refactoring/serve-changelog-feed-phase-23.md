# Serve changelog feed ownership phase

## History and boundary

Machined introduced the compiled “What’s New” endpoint in `60f82a28`; the release audit in `07da3d17` hardened its shipped-only semantics. The complete domain is the embedded release artifact, bounded query, Keep-a-Changelog parser, and release projection.

This phase moves that domain and all parser fixtures to `serve/serve_changelog.rs`. The include path is adjusted for the nested module and still resolves the repository-root `CHANGELOG.md` at compile time, never a user workspace at runtime. Serve retains only token-scoped route registration and the shared JSON response primitive. Independent review passed behavior parity and embedded-path correctness.

## Measured result

- Serve root: 2,759 to 2,656 implementation lines.
- New changelog owner: below 500 implementation lines.
- Repository distribution: 220/299 (73.6%) at or below 500 and 278/299 (93.0%) at or below 800.
- Eight owners remain above 2,000; none exceeds 5,000.

Focused parser tests, warnings-denied Clippy, formatting, and the architecture guard pass. This remains intermediate and does not waive or claim the 90%/95% terminal gates.
