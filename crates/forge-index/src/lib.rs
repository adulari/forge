//! Lattice — Forge's native code-intelligence subsystem (docs/features/code-intelligence.md).
//! PR1: tree-sitter structural extraction (Rust) persisted into the shared SQLite store,
//! incremental by file content hash, queryable via the `forge lattice` CLI. Later PRs add
//! resolved call/reference edges, auto-retrieval injection into the turn, embeddings, and a
//! file watcher.

use std::path::Path;
use std::sync::Arc;

use forge_store::{LatticeEdgeRow, LatticeFileRow, LatticeNodeRow, Store, StoreError};
use sha2::{Digest, Sha256};

mod extract;

pub use extract::{extract_rust, NodeKind, RawNode};

#[derive(Debug, thiserror::Error)]
pub enum LatticeError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("io: {0}")]
    Io(String),
}

/// The code-intelligence graph for one repository root, backed by the shared [`Store`].
pub struct Lattice {
    store: Arc<Store>,
    /// Canonical root path, used to namespace symbols and compute repo-relative paths.
    repo_root: String,
}

/// What an `update` did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UpdateStats {
    pub files_indexed: usize,
    pub files_skipped: usize,
    pub symbols: usize,
}

/// A symbol returned from a query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeHit {
    pub name: String,
    pub kind: String,
    pub qualname: Option<String>,
    pub signature: Option<String>,
    pub rel_path: String,
    pub line: i64,
}

/// Index-wide counts for `forge lattice status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexStatus {
    pub files: i64,
    pub nodes: i64,
    pub edges: i64,
}

impl Lattice {
    /// Open the Lattice for `repo_root` (canonicalized so identity is stable regardless of how
    /// the path was spelled).
    pub fn new(store: Arc<Store>, repo_root: &Path) -> Self {
        let repo_root = std::fs::canonicalize(repo_root)
            .unwrap_or_else(|_| repo_root.to_path_buf())
            .to_string_lossy()
            .into_owned();
        Self { store, repo_root }
    }

    /// Incrementally (re)index every supported source file under the root. Files whose content
    /// hash is unchanged since the last run are skipped without re-parsing.
    pub fn update(&self) -> Result<UpdateStats, LatticeError> {
        let mut stats = UpdateStats::default();
        let root = Path::new(&self.repo_root).to_path_buf();
        let mut stack = vec![root];
        while let Some(dir) = stack.pop() {
            let entries = match std::fs::read_dir(&dir) {
                Ok(e) => e,
                Err(_) => continue,
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if path.is_dir() {
                    if is_skippable_dir(&name) {
                        continue;
                    }
                    stack.push(path);
                } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                    self.index_file(&path, &mut stats)?;
                }
            }
        }
        Ok(stats)
    }

    /// (Re)index a single file (e.g. after the agent edits it). No-op for unsupported files.
    pub fn reindex_path(&self, path: &Path) -> Result<(), LatticeError> {
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            return Ok(());
        }
        let mut stats = UpdateStats::default();
        self.index_file(path, &mut stats)
    }

    fn index_file(&self, path: &Path, stats: &mut UpdateStats) -> Result<(), LatticeError> {
        let rel = self.rel_path(path);
        let src = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(_) => return Ok(()), // unreadable (e.g. non-UTF8) — skip, don't fail the whole run
        };
        let hash = sha_hex(src.as_bytes());
        if self
            .store
            .lattice_file_hash(&self.repo_root, &rel)?
            .as_deref()
            == Some(hash.as_str())
        {
            stats.files_skipped += 1;
            return Ok(());
        }

        let file_id = sha_hex(format!("{}\0{}", self.repo_root, rel).as_bytes());
        let raw = extract_rust(&src);
        let mut node_ids: Vec<String> = Vec::with_capacity(raw.len());
        let mut nodes = Vec::with_capacity(raw.len());
        for n in &raw {
            let id = sha_hex(
                format!(
                    "{}\0{}\0{}\0{}\0{}",
                    self.repo_root,
                    rel,
                    n.kind.as_str(),
                    n.qualname,
                    n.line_start
                )
                .as_bytes(),
            );
            node_ids.push(id.clone());
            nodes.push(LatticeNodeRow {
                id,
                file_id: file_id.clone(),
                kind: n.kind.as_str().to_string(),
                name: n.name.clone(),
                qualname: Some(n.qualname.clone()),
                signature: n.signature.clone(),
                span_start: n.span_start as i64,
                span_end: n.span_end as i64,
                line_start: n.line_start as i64,
            });
        }
        // `contains` edges: enclosing symbol → nested symbol (impl→method, mod→item, …).
        let mut edges = Vec::new();
        for (i, n) in raw.iter().enumerate() {
            if let Some(p) = n.parent {
                let id = sha_hex(format!("{}\0contains\0{}", node_ids[p], node_ids[i]).as_bytes());
                edges.push(LatticeEdgeRow {
                    id,
                    src_id: node_ids[p].clone(),
                    dst_id: node_ids[i].clone(),
                    kind: "contains".to_string(),
                    unresolved_name: None,
                });
            }
        }

        let file = LatticeFileRow {
            id: file_id,
            repo_root: self.repo_root.clone(),
            rel_path: rel,
            lang: "rust".to_string(),
            content_hash: hash,
            parse_status: "ok".to_string(),
        };
        self.store.replace_lattice_file(&file, &nodes, &edges)?;
        stats.files_indexed += 1;
        stats.symbols += nodes.len();
        Ok(())
    }

    /// Symbols whose name matches `query` (case-insensitive), best-first.
    pub fn query(&self, query: &str, limit: usize) -> Result<Vec<NodeHit>, LatticeError> {
        let rows = self.store.lattice_nodes_by_name(query, limit)?;
        let mut hits = Vec::with_capacity(rows.len());
        for r in rows {
            let rel_path = self
                .store
                .lattice_file_path(&r.file_id)?
                .unwrap_or_default();
            hits.push(NodeHit {
                name: r.name,
                kind: r.kind,
                qualname: r.qualname,
                signature: r.signature,
                rel_path,
                line: r.line_start,
            });
        }
        Ok(hits)
    }

    pub fn status(&self) -> Result<IndexStatus, LatticeError> {
        let (files, nodes, edges) = self.store.lattice_counts()?;
        Ok(IndexStatus {
            files,
            nodes,
            edges,
        })
    }

    fn rel_path(&self, path: &Path) -> String {
        let canon = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        canon
            .strip_prefix(&self.repo_root)
            .unwrap_or(&canon)
            .to_string_lossy()
            .replace('\\', "/")
    }
}

/// Directories never worth indexing — build output, VCS, dependencies, and dotdirs.
fn is_skippable_dir(name: &str) -> bool {
    matches!(name, "target" | "node_modules" | ".git" | "graphify-out") || name.starts_with('.')
}

fn sha_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    let digest = h.finalize();
    // 128 bits of hex is plenty to avoid collisions across one repo's symbols.
    digest.iter().take(16).map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static N: AtomicUsize = AtomicUsize::new(0);

    struct Tmp {
        root: std::path::PathBuf,
    }
    impl Tmp {
        fn new() -> Tmp {
            let n = N.fetch_add(1, Ordering::SeqCst);
            let root =
                std::env::temp_dir().join(format!("forge-lattice-{}-{n}", std::process::id()));
            std::fs::create_dir_all(root.join("src")).unwrap();
            std::fs::create_dir_all(root.join("target/debug")).unwrap();
            Tmp { root }
        }
        fn write(&self, rel: &str, content: &str) {
            let p = self.root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, content).unwrap();
        }
    }
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn lattice(root: &Path) -> Lattice {
        let store = Arc::new(Store::open_in_memory().unwrap());
        Lattice::new(store, root)
    }

    #[test]
    fn indexes_rust_files_and_queries_symbols() {
        let t = Tmp::new();
        t.write(
            "src/lib.rs",
            "pub struct Session { id: String }\nimpl Session { pub fn run_turn(&self) {} }\n",
        );
        // A file under target/ must be ignored.
        t.write("target/debug/built.rs", "pub fn should_not_index() {}");
        let lat = lattice(&t.root);

        let stats = lat.update().unwrap();
        assert_eq!(stats.files_indexed, 1, "only src/lib.rs, not target/");
        assert!(stats.symbols >= 3, "struct + impl + method: {stats:?}");

        let hits = lat.query("run_turn", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].kind, "method");
        assert_eq!(hits[0].rel_path, "src/lib.rs");
        assert!(lat.query("should_not_index", 10).unwrap().is_empty());
    }

    #[test]
    fn reindex_is_incremental_on_unchanged_hash() {
        let t = Tmp::new();
        t.write("src/a.rs", "pub fn alpha() {}");
        let lat = lattice(&t.root);

        let first = lat.update().unwrap();
        assert_eq!(first.files_indexed, 1);
        assert_eq!(first.files_skipped, 0);

        // Nothing changed → the second pass skips, re-parses nothing.
        let second = lat.update().unwrap();
        assert_eq!(second.files_indexed, 0);
        assert_eq!(second.files_skipped, 1);

        // Edit the file → it re-indexes, and the new symbol is queryable, the old one gone.
        t.write("src/a.rs", "pub fn beta() {}");
        let third = lat.update().unwrap();
        assert_eq!(third.files_indexed, 1);
        assert!(lat.query("beta", 10).unwrap().len() == 1);
        assert!(
            lat.query("alpha", 10).unwrap().is_empty(),
            "stale symbol removed"
        );
    }

    #[test]
    fn status_reports_counts() {
        let t = Tmp::new();
        t.write("src/lib.rs", "pub fn one() {}\npub fn two() {}");
        let lat = lattice(&t.root);
        lat.update().unwrap();
        let s = lat.status().unwrap();
        assert_eq!(s.files, 1);
        assert_eq!(s.nodes, 2);
    }
}
