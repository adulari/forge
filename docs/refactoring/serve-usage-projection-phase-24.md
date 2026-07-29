# Serve usage projection ownership phase

## History and boundary

The mobile usage page landed as one projection domain in `6fd57c30`; QwenCloud quota integration extended it in `a3673b2e`. The owner combines weekly/session provider usage, provider transport kind, and active subscription windows into the client wire shape.

This phase moves the complete projection and aggregation policy to `serve/serve_usage.rs`, with characterization for token/cost conservation and provider grouping. The Store projection now selects the newest observation across the shared Codex alias group before applying reset and five-minute freshness filtering, so an expired newest sample cannot fall back to an older misleading bar. The TypeScript wire mirror now includes the already-emitted `cachedInputTokens` field. The legacy single-session `/remote` usage endpoint remains intentionally synthetic because that server has no Store seam; the durable, Store-backed contract is the daemon `/api/usage` route.

## Measured intent

The new owner is below 500 implementation lines and deletes the usage response models, aggregation, classification, and handler from Serve. Focused CLI/Store/mobile checks, warnings-denied Clippy, architecture guard, and independent review are required before commit. This remains intermediate and does not waive the 90%/95% terminal gates.
