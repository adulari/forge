# Serve model catalog ownership phase

## History and boundary

The Serve model catalog evolved from the mobile model picker in `110c5fd0` and gained health, benchmark, pricing, and context metadata in `a3673b2e`. Its cohesive policy is provider-grouped catalog projection: load the cached discovery catalog, apply configured and Store-fetched pricing, join Store health/context observations, and derive the client tier.

This phase moves model response types, catalog joins, tier derivation, and handler to `serve/serve_models.rs`. Serve retains only route registration. The owner test pins the precedence rule `frontier > paid/subscription > trivial`; cached-catalog-unavailable behavior remains an explicit empty response. Independent review found and fixed the omission of Store-fetched model prices; health/context and benchmark joins remain sourced from their existing authoritative Store/catalog APIs.

The `tier` field is a display bucket, not Mesh routing eligibility: it deliberately summarizes frontier/cost/subscription identity for the mobile catalog and does not claim to replace task classification or ranked route selection.

## Measured result

- Serve root: 2,514 to 2,406 implementation lines.
- New model owner: below 500 implementation lines.
- Repository distribution: 222/301 (73.8%) at or below 500 and 280/301 (93.0%) at or below 800.
- Eight owners remain above 2,000; none exceeds 5,000.

Focused model tests, warnings-denied Clippy, formatting, and the architecture guard pass. This intermediate phase does not waive or claim the canonical 90%/95% terminal gates.
