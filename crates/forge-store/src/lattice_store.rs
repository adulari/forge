//! Lattice graph persistence operations.

use super::*;

impl Store {
    /// The stored content hash for a file, or `None` if it hasn't been indexed — the
    /// incremental-update gate (skip files whose hash is unchanged).
    pub fn lattice_file_hash(&self, repo_root: &str, rel_path: &str) -> Result<Option<String>> {
        let conn = self.lock()?;
        let hash = conn
            .query_row(
                "SELECT content_hash FROM lattice_file WHERE repo_root = ?1 AND rel_path = ?2",
                (repo_root, rel_path),
                |r| r.get::<_, String>(0),
            )
            .optional()?;
        Ok(hash)
    }

    /// Insert or replace a file's row and its symbol nodes + edges atomically: the file's prior
    /// nodes are deleted first (cascading their edges), so re-indexing is idempotent.
    pub fn replace_lattice_file(
        &self,
        file: &LatticeFileRow,
        nodes: &[LatticeNodeRow],
        edges: &[LatticeEdgeRow],
        refs: &[LatticeRefRow],
    ) -> Result<()> {
        let mut conn = self.lock()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT INTO lattice_file (id, repo_root, rel_path, lang, content_hash, parse_status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET
                content_hash = excluded.content_hash,
                lang = excluded.lang,
                parse_status = excluded.parse_status,
                indexed_at = strftime('%s','now')",
            (
                &file.id,
                &file.repo_root,
                &file.rel_path,
                &file.lang,
                &file.content_hash,
                &file.parse_status,
            ),
        )?;
        // Replace the file's symbols (FK ON DELETE CASCADE clears their edges too).
        tx.execute("DELETE FROM lattice_node WHERE file_id = ?1", (&file.id,))?;
        for n in nodes {
            tx.execute(
                "INSERT INTO lattice_node
                   (id, file_id, kind, name, qualname, signature, span_start, span_end, line_start, pagerank)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0.0)",
                rusqlite::params![
                    n.id,
                    n.file_id,
                    n.kind,
                    n.name,
                    n.qualname,
                    n.signature,
                    n.span_start,
                    n.span_end,
                    n.line_start,
                ],
            )?;
        }
        for e in edges {
            tx.execute(
                "INSERT INTO lattice_edge (id, src_id, dst_id, kind, unresolved_name)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![e.id, e.src_id, e.dst_id, e.kind, e.unresolved_name],
            )?;
        }
        for r in refs {
            tx.execute(
                "INSERT INTO lattice_ref (id, src_id, name, kind, line)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![r.id, r.src_id, r.name, r.kind, r.line],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Distinct definitions that reference `name` — the direct callers/dependents of a symbol
    /// (one hop of `impact`). Resolves the name-keyed `lattice_ref` rows back to their src nodes.
    pub fn lattice_callers_by_name(
        &self,
        repo_root: &str,
        name: &str,
        limit: usize,
    ) -> Result<Vec<LatticeNodeRow>> {
        let conn = self.lock()?;
        // Scoped to `repo_root`: the store is global (one DB across every project + bench clone), so
        // an unscoped name match returns cross-repo collisions (a `Command` in a vendored django/ or
        // another crate). The caller's Lattice is bound to one repo_root; only its rows are relevant.
        let mut stmt = conn.prepare(
            "SELECT DISTINCT n.id, n.file_id, n.kind, n.name, n.qualname, n.signature,
                    n.span_start, n.span_end, n.line_start, n.pagerank
             FROM lattice_ref r
             JOIN lattice_node n ON n.id = r.src_id
             JOIN lattice_file f ON f.id = n.file_id
             WHERE r.name = ?1 AND n.name <> ?1 AND f.repo_root = ?2
             ORDER BY n.name
             LIMIT ?3",
        )?;
        let rows = stmt
            .query_map(
                rusqlite::params![name, repo_root, limit as i64],
                lattice_node_from_row,
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Distinct identifier names referenced *by* definitions named `name` — one forward hop for
    /// `path` BFS (what the symbol calls/uses).
    pub fn lattice_callees_of_name(&self, repo_root: &str, name: &str) -> Result<Vec<String>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT DISTINCT r.name
             FROM lattice_ref r
             JOIN lattice_node n ON n.id = r.src_id
             JOIN lattice_file f ON f.id = n.file_id
             WHERE n.name = ?1 AND r.name <> ?1 AND f.repo_root = ?2",
        )?;
        let rows = stmt
            .query_map(rusqlite::params![name, repo_root], |r| {
                r.get::<_, String>(0)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Reference rows belonging to `repo_root` — completes the `status` summary. Scoped like the
    /// structural queries: the store is global (one DB across every project and bench clone), so an
    /// unscoped `COUNT(*)` reports another project's rows as if they were this one's.
    pub fn lattice_ref_count(&self, repo_root: &str) -> Result<i64> {
        let conn = self.lock()?;
        Ok(conn.query_row(
            "SELECT COUNT(*) FROM lattice_ref r
             JOIN lattice_node n ON n.id = r.src_id
             JOIN lattice_file f ON f.id = n.file_id
             WHERE f.repo_root = ?1",
            [repo_root],
            |r| r.get(0),
        )?)
    }

    /// Symbols whose name contains `query` (case-insensitive), best-first: exact name, then
    /// prefix, then substring; capped at `limit`.
    pub fn lattice_nodes_by_name(
        &self,
        repo_root: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<LatticeNodeRow>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT n.id, n.file_id, n.kind, n.name, n.qualname, n.signature,
                    n.span_start, n.span_end, n.line_start, n.pagerank,
                    CASE
                        WHEN lower(n.name) = lower(?1) THEN 0
                        WHEN lower(n.name) LIKE lower(?1) || '%' THEN 1
                        ELSE 2
                    END AS rank
             FROM lattice_node n
             JOIN lattice_file f ON f.id = n.file_id
             WHERE lower(n.name) LIKE '%' || lower(?1) || '%' AND f.repo_root = ?3
             ORDER BY rank, length(n.name), n.name
             LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(rusqlite::params![query, limit as i64, repo_root], |r| {
                Ok(LatticeNodeRow {
                    id: r.get(0)?,
                    file_id: r.get(1)?,
                    kind: r.get(2)?,
                    name: r.get(3)?,
                    qualname: r.get(4)?,
                    signature: r.get(5)?,
                    span_start: r.get(6)?,
                    span_end: r.get(7)?,
                    line_start: r.get(8)?,
                    pagerank: r.get(9)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// A single node row by id — used to resolve embedding-ranked node ids back to nodes.
    pub fn lattice_node_by_id(&self, id: &str) -> Result<Option<LatticeNodeRow>> {
        let conn = self.lock()?;
        match conn.query_row(
            "SELECT id, file_id, kind, name, qualname, signature, span_start, span_end, line_start, pagerank
             FROM lattice_node WHERE id = ?1",
            [id],
            lattice_node_from_row,
        ) {
            Ok(r) => Ok(Some(r)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Delete every indexed file under `repo_root` whose `rel_path` is NOT in `keep` — and, via
    /// `ON DELETE CASCADE`, all of its symbols/edges/refs. Called after a full `update` walk to
    /// purge files that were removed or are now skipped (deleted files, nested git repos / vendored
    /// trees), so stale symbols don't linger in queries or bloat the store. Returns the count pruned.
    pub fn prune_lattice_files_except(
        &self,
        repo_root: &str,
        keep: &std::collections::HashSet<String>,
    ) -> Result<usize> {
        let mut conn = self.lock()?;
        // IMMEDIATE: SELECTs then DELETEs — a DEFERRED read snapshot could fail to upgrade with
        // SQLITE_BUSY_SNAPSHOT if the indexer committed concurrently.
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let stale: Vec<String> = {
            let mut stmt =
                tx.prepare("SELECT id, rel_path FROM lattice_file WHERE repo_root = ?1")?;
            let rows = stmt.query_map([repo_root], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?;
            rows.filter_map(|r| r.ok())
                .filter(|(_, rel)| !keep.contains(rel))
                .map(|(id, _)| id)
                .collect()
        };
        for id in &stale {
            tx.execute("DELETE FROM lattice_file WHERE id = ?1", (id,))?;
        }
        tx.commit()?;
        Ok(stale.len())
    }

    /// Every distinct `repo_root` with indexed files. The store is global (shared across projects
    /// and bench clones), so this surfaces orphan roots — e.g. a deleted `/tmp/swe-*/django` scratch
    /// checkout — that `update` can prune.
    pub fn lattice_repo_roots(&self) -> Result<Vec<String>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare("SELECT DISTINCT repo_root FROM lattice_file")?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Delete every indexed file under `repo_root` (cascading to its symbols/edges/refs). Used to
    /// drop an orphan root whose directory no longer exists on disk. Returns the count removed.
    pub fn prune_lattice_repo(&self, repo_root: &str) -> Result<usize> {
        let conn = self.lock()?;
        Ok(conn.execute("DELETE FROM lattice_file WHERE repo_root = ?1", [repo_root])?)
    }

    /// Delete a single indexed file's row (cascading to its symbols/edges/refs). Called by the file
    /// watcher when a source file is removed on disk, so its nodes don't linger as phantom symbols
    /// in `query`/`impact`. Returns 1 if a row was removed, 0 if it wasn't indexed.
    pub fn delete_lattice_file(&self, repo_root: &str, rel_path: &str) -> Result<usize> {
        Ok(self.lock()?.execute(
            "DELETE FROM lattice_file WHERE repo_root = ?1 AND rel_path = ?2",
            (repo_root, rel_path),
        )?)
    }

    /// The `rel_path` of an indexed file by its id (for rendering a node's location).
    pub fn lattice_file_path(&self, file_id: &str) -> Result<Option<String>> {
        let conn = self.lock()?;
        Ok(conn
            .query_row(
                "SELECT rel_path FROM lattice_file WHERE id = ?1",
                (file_id,),
                |r| r.get::<_, String>(0),
            )
            .ok())
    }

    /// `(files, nodes, edges)` row counts for `repo_root` — the `forge lattice status` summary.
    /// Scoped, because the store is global: unscoped `COUNT(*)`s made `status` report every indexed
    /// project's rows as this project's, which is how a multi-GB stray index (a deleted `/tmp` bench
    /// clone) could sit in the database for a month without anyone noticing. Use
    /// [`Store::lattice_repo_roots`] for the deliberate all-roots view.
    pub fn lattice_counts(&self, repo_root: &str) -> Result<(i64, i64, i64)> {
        let conn = self.lock()?;
        let files = conn.query_row(
            "SELECT COUNT(*) FROM lattice_file WHERE repo_root = ?1",
            [repo_root],
            |r| r.get(0),
        )?;
        let nodes = conn.query_row(
            "SELECT COUNT(*) FROM lattice_node n
             JOIN lattice_file f ON f.id = n.file_id
             WHERE f.repo_root = ?1",
            [repo_root],
            |r| r.get(0),
        )?;
        // Scoped by the edge's SOURCE node, matching `lattice_ref_edges`: the indexer writes an
        // edge together with the file that produced it, so its src always lives in that repo.
        let edges = conn.query_row(
            "SELECT COUNT(*) FROM lattice_edge e
             JOIN lattice_node n ON n.id = e.src_id
             JOIN lattice_file f ON f.id = n.file_id
             WHERE f.repo_root = ?1",
            [repo_root],
            |r| r.get(0),
        )?;
        Ok((files, nodes, edges))
    }

    /// Upsert a node's embedding vector (semantic retrieval, code-intelligence.md §5.6). `vec` is
    /// stored as little-endian f32 components.
    pub fn put_lattice_embedding(&self, node_id: &str, vec: &[f32]) -> Result<()> {
        let mut bytes = Vec::with_capacity(vec.len() * 4);
        for f in vec {
            bytes.extend_from_slice(&f.to_le_bytes());
        }
        self.lock()?.execute(
            "INSERT INTO lattice_embedding (node_id, dim, vec) VALUES (?1, ?2, ?3)
             ON CONFLICT(node_id) DO UPDATE SET dim = excluded.dim, vec = excluded.vec",
            rusqlite::params![node_id, vec.len() as i64, bytes],
        )?;
        Ok(())
    }

    /// Nodes under `repo_root` that don't yet have an embedding — the work list for incremental
    /// `embed_pending`. Scoped: unscoped, `embed_pending` spends this project's embedding API calls
    /// (and quota) on every OTHER project indexed in the shared store.
    pub fn lattice_nodes_without_embedding(
        &self,
        repo_root: &str,
        limit: usize,
    ) -> Result<Vec<LatticeNodeRow>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT n.id, n.file_id, n.kind, n.name, n.qualname, n.signature,
                    n.span_start, n.span_end, n.line_start, n.pagerank
             FROM lattice_node n
             JOIN lattice_file f ON f.id = n.file_id
             LEFT JOIN lattice_embedding e ON e.node_id = n.id
             WHERE e.node_id IS NULL AND f.repo_root = ?1
             LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(
                rusqlite::params![repo_root, limit as i64],
                lattice_node_from_row,
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// The `(node_id, vector)` embeddings under `repo_root` — loaded once to cosine-rank a query
    /// vector. Scoped for the same reason as the structural queries: unscoped, semantic retrieval
    /// ranks every indexed project's symbols against this project's prompt and can inject another
    /// repo's code as context.
    pub fn lattice_embeddings(&self, repo_root: &str) -> Result<Vec<(String, Vec<f32>)>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT e.node_id, e.vec FROM lattice_embedding e
             JOIN lattice_node n ON n.id = e.node_id
             JOIN lattice_file f ON f.id = n.file_id
             WHERE f.repo_root = ?1",
        )?;
        let rows = stmt.query_map([repo_root], |r| {
            let id: String = r.get(0)?;
            let blob: Vec<u8> = r.get(1)?;
            let vec = blob
                .chunks_exact(4)
                .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                .collect();
            Ok((id, vec))
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    /// How many of `repo_root`'s nodes currently have an embedding (`forge lattice status`:
    /// "N embedded"). Also the gate `retrieve_hybrid` uses to decide whether semantic retrieval is
    /// available for THIS repo, so it must not be satisfied by another project's vectors.
    pub fn lattice_embedding_count(&self, repo_root: &str) -> Result<i64> {
        Ok(self.lock()?.query_row(
            "SELECT COUNT(*) FROM lattice_embedding e
             JOIN lattice_node n ON n.id = e.node_id
             JOIN lattice_file f ON f.id = n.file_id
             WHERE f.repo_root = ?1",
            [repo_root],
            |r| r.get(0),
        )?)
    }

    /// All (src_id, dst_name) pairs from lattice_ref — the directed reference graph for PageRank.
    /// `src_id` is the referencing node's id; `dst_name` is the referenced identifier (resolved to
    /// node ids by name-join at call time). Returns (src_node_id, referenced_name) pairs.
    /// Scoped to `repo_root` — the store is global (one DB across every project), so an unscoped
    /// scan would mix another project's refs into THIS repo's PageRank (cross-repo contamination).
    pub fn lattice_ref_edges(&self, repo_root: &str) -> Result<Vec<(String, String)>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT r.src_id, r.name FROM lattice_ref r
             JOIN lattice_node n ON n.id = r.src_id
             JOIN lattice_file f ON f.id = n.file_id
             WHERE f.repo_root = ?1",
        )?;
        let rows = stmt
            .query_map([repo_root], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// All nodes ordered by pagerank descending, capped at `limit` — the repo-map selection query.
    /// Returns the top-N most important symbols across all files in the index; the caller applies
    /// a token-budget cutoff. Use `usize::MAX` to retrieve every node (for small repos).
    pub fn lattice_nodes_ranked(
        &self,
        repo_root: &str,
        limit: usize,
    ) -> Result<Vec<LatticeNodeRow>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT n.id, n.file_id, n.kind, n.name, n.qualname, n.signature,
                    n.span_start, n.span_end, n.line_start, n.pagerank
             FROM lattice_node n
             JOIN lattice_file f ON f.id = n.file_id
             WHERE f.repo_root = ?1
             ORDER BY n.pagerank DESC
             LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(
                rusqlite::params![repo_root, limit as i64],
                lattice_node_from_row,
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// All (node_id, node_name) pairs for `repo_root` — needed to resolve reference names to node ids
    /// for PageRank. Scoped so a sibling project's nodes don't absorb this repo's reference rank.
    pub fn lattice_node_ids_and_names(&self, repo_root: &str) -> Result<Vec<(String, String)>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT n.id, n.name FROM lattice_node n
             JOIN lattice_file f ON f.id = n.file_id
             WHERE f.repo_root = ?1",
        )?;
        let rows = stmt
            .query_map([repo_root], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Batch-update pagerank scores: for each `(node_id, score)` pair, set `pagerank = score`.
    /// Committed in chunks (each its own IMMEDIATE transaction) rather than one giant write-txn, so
    /// the single WAL writer lock is released between batches — a full-table update used to hold it
    /// long enough to starve a concurrent critical write (transcript/usage) past `busy_timeout`.
    pub fn set_lattice_pageranks(&self, scores: &[(String, f64)]) -> Result<()> {
        if scores.is_empty() {
            return Ok(());
        }
        const CHUNK: usize = 500;
        for chunk in scores.chunks(CHUNK) {
            with_busy_retry(|| {
                let mut conn = self.lock()?;
                let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
                {
                    let mut stmt =
                        tx.prepare("UPDATE lattice_node SET pagerank = ?2 WHERE id = ?1")?;
                    for (id, score) in chunk {
                        stmt.execute(rusqlite::params![id, score])?;
                    }
                }
                tx.commit()?;
                Ok(())
            })?;
        }
        Ok(())
    }
}
