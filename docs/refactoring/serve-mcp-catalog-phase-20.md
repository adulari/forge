# Serve MCP catalog ownership phase

## History and boundary

The MCP control-surface catalog and project-scoped create endpoint landed together in `f251d404`; Machined added editable-source projection and mutation of explicit project/user `mcp.toml` owners in `60f82a28`. Their shared rule is ownership of persisted MCP configuration: the API may list every layered server, but it may mutate only explicit TOML sources and must never materialize imported read-only entries into another source of truth.

This phase moves request models, catalog projection, project-scoped creation, layered toggle ownership, transport validation, and persistence safety to `serve/serve_mcp.rs`. Serve retains only route composition. The mobile create contract intentionally remains project-scoped: its request has never carried a scope and `AddMcpServerSheet` describes adding the current workspace's server. Toggles continue to locate project scope before user scope, matching the pre-extraction behavior.

## Characterization and review fixes

Pure tests pin the accepted persisted server-name alphabet and HTTP(S) transport contract. Mutation tests prove a missing catalog starts empty and malformed TOML is refused rather than overwritten. Independent review identified the prior malformed-file overwrite risk and prefix-only URL validation; this phase fixes both by parsing URLs and loading mutable TOML strictly. Existing CLI/Serve test suites cover router composition while these owner tests cover deletion-critical persistence policy.

## Measured result

- Serve root: 3,327 to 3,113 implementation lines.
- New MCP owner: below 500 implementation lines.
- Repository distribution: 217/296 (73.3%) at or below 500 and 275/296 (92.9%) at or below 800.
- Eight owners remain above 2,000; none exceeds 5,000.

This remains an intermediate phase. It does not waive or claim the 90%/95% terminal gates, regenerate the baseline, or enable auto-merge.
