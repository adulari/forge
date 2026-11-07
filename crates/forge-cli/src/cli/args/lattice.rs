use clap::Subcommand;

#[derive(Subcommand)]
pub(crate) enum LatticeOp {
    /// (Re)index the working directory into the graph — incremental by file content hash.
    Update {
        /// Root to index (default: current directory).
        path: Option<String>,
        /// Index the root even though it exceeds the indexable-file ceiling. Does not unlock the
        /// home/system directory — that root is never indexable.
        #[arg(long)]
        force: bool,
    },
    /// List every repository root that has rows in the index, flagging roots whose directory is
    /// gone. The store is shared across projects, so a mistaken root (a home directory, a deleted
    /// scratch clone) lingers here until pruned.
    Roots,
    /// Delete an indexed root — its files, symbols, edges and references — and optionally reclaim
    /// the freed disk with VACUUM.
    Prune {
        /// Root to remove (as printed by `forge lattice roots`).
        root: Option<String>,
        /// Remove every root whose directory no longer exists on disk.
        #[arg(long)]
        stale: bool,
        /// Rebuild the database file afterwards to actually return the space to the filesystem.
        /// Requires exclusive access and temporarily needs free space of the database's own size.
        #[arg(long)]
        vacuum: bool,
    },
    /// Find symbols by name (case-insensitive); prints kind, location, and signature.
    Query {
        /// Symbol name or fragment.
        query: String,
    },
    /// Blast radius: everything that references a symbol (transitively) — "what breaks if I change X".
    Impact {
        /// Symbol name.
        symbol: String,
        /// Confine the blast radius to a repo-relative path prefix (e.g. `crates/forge-core`), so a
        /// symbol that exists in several crates doesn't mix their references together.
        #[arg(long)]
        scope: Option<String>,
    },
    /// Shortest call/reference chain of symbol names from A to B.
    Path {
        /// Source symbol name.
        from: String,
        /// Target symbol name.
        to: String,
    },
    /// Decision provenance: who last changed a symbol's definition, when, and in which commit.
    Why {
        /// Symbol name.
        symbol: String,
    },
    /// Compute + store embeddings for nodes lacking them (semantic retrieval). Uses the
    /// `[lattice.embeddings]` backend (ollama by default). Runs automatically on startup when
    /// embeddings are enabled — this is the manual trigger / one-off.
    Embed,
    /// Show index counts (files, symbols, edges, refs).
    Status,
    /// Print a compact, importance-ranked overview of the repo's key definitions, grouped by file
    /// (aider-style repo-map). Selection is by PageRank so high-centrality symbols appear first;
    /// within each file symbols are shown in source order. Output is token-budgeted.
    Map {
        /// Token budget for the map (default: 2000). Larger = more symbols shown.
        #[arg(long, short = 'b')]
        budget: Option<usize>,
    },
}
