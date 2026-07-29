# Code archaeology: plugin command grammar

## Boundary

`cli/args/plugins.rs` owns the complete marketplace-aware plugin lifecycle grammar: install, list, remove, update, and marketplace registration. Runtime package resolution, lockfile mutation, and source fetching remain in MCP/plugin command handlers.

## Interface

`PluginCmd` and `PluginMarketplaceCmd` are the nested Clap/dispatch contract. The `add` install alias, optional marketplace selector, and marketplace `--ref` spelling are stable CLI compatibility surface.

## Characterization

Root CLI parser tests validate installation, list/update optional arguments, marketplace add/remove,
and compatibility aliases. Handler tests cover lockfile and marketplace behavior; the extraction
preserves the enum variants and composition-root re-exports.
