# Serve editable configuration ownership phase

## History and boundary

The mobile scalar configuration editor landed in `82e5a7f6` and was extended for lists and permission rules in `f561f841` and `5be59a7e`. These commits share one domain rule: the client may view and mutate only the descriptor-backed settings exported by `forge-config`; arbitrary dotted keys are never accepted.

This phase moves request/scope models, descriptor projection, field-kind mapping, allowlisted mutation, reset behavior, and response refresh to `serve/serve_config.rs`. Serve retains only route registration. Characterization proves every descriptor is projected exactly once under its writable key and enum options retain their authored order. Serve serializes its read-modify-write mutations with a process-wide lock so concurrent HTTP updates cannot overwrite one another. Independent review also found that `reset_config_value` documentation claimed a cross-scope reset the implementation never performed; the contract is now documented truthfully as a selected-scope reset. Complex JSON descriptors are intentionally retained: the mobile editor added explicit validated JSON support for permissions/statusline/hooks/keybinds/providers in the cited feature history, while MCP remains excluded behind its dedicated API.

## Measured intent

The new owner is below 500 implementation lines and deletes the complete configuration editor implementation from the Serve composition root. The canonical architecture guard, focused tests, warnings-denied Clippy, and independent review are required before commit. This remains an intermediate phase and does not waive or claim the 90%/95% terminal gates.
