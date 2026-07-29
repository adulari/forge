# Code archaeology: Lattice graph traversal

## Boundary

`forge-index/src/graph.rs` owns the questions that require *walking* the indexed reference edges:
PageRank centrality, the reverse-dependency closure (`impact` / `impact_in_scope`), the connection
path between two symbols, and git provenance for a definition (`why` plus the `git blame` /
`git show` helpers). The index root keeps extraction, storage, incremental update, embeddings, and
retrieval.

## The caveat that makes it one owner

tree-sitter references are keyed by *name* with no cross-crate binding. An unscoped walk on a
symbol that exists in several crates therefore mixes their results together — which is why the
scoped variants exist and why `view` reaches for them. That constraint applies to every traversal
in this module and to nothing outside it.

## Interface

The traversals stay inherent methods on `Lattice`, so no caller changed. `parse_blame_sha` and
`parse_show_meta` are `pub(crate)` for the root's existing porcelain-parsing tests; everything
else stays private.

## Characterization

The crate's existing tests — impact scoping, PageRank behaviour, blame/show parsing — continue to
exercise this module unchanged through `Lattice`'s public surface, and the whole crate suite
passes.
