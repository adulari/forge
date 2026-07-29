//! Walking the indexed graph: centrality, blast radius, connection paths, and provenance.
//!
//! The index stores definitions plus name-keyed reference edges. This module owns the questions
//! that require *traversing* them — how central a node is (PageRank), what breaks if a symbol
//! changes (the reverse-dependency closure), how two symbols connect, and which commit last
//! touched a definition. Extraction, storage, and retrieval stay with the index itself.
//!
//! One scoping caveat runs through all of it: tree-sitter references are keyed by name with no
//! cross-crate binding, so an unscoped walk on a name that exists in several crates mixes their
//! results. That is why the scoped variants exist and why the callers reach for them.

use std::collections::{HashSet, VecDeque};

use crate::{BlastRadius, Lattice, LatticeError, NodeHit, Provenance};

impl Lattice {
    /// Compute PageRank over the lattice reference graph and persist scores for every node.
    ///
    /// Algorithm: iterative power method, damping factor 0.85, up to 20 iterations or until
    /// the L1 norm of the update is < 1e-6 (convergence). The graph is built from `lattice_ref`
    /// (name-keyed edges) by joining to `lattice_node` to resolve names to node ids; nodes with
    /// no outgoing edges (dangling nodes) distribute their rank uniformly. Scores are normalized
    /// to sum to 1.0 before persisting so they're comparable across index sizes.
    pub fn recompute_pagerank(&self) -> Result<(), LatticeError> {
        use std::collections::HashMap;

        // Load all nodes and build an id→index map.
        let node_pairs = self.store.lattice_node_ids_and_names(&self.repo_root)?;
        if node_pairs.is_empty() {
            return Ok(());
        }
        let n = node_pairs.len();
        let id_to_idx: HashMap<&str, usize> = node_pairs
            .iter()
            .enumerate()
            .map(|(i, (id, _))| (id.as_str(), i))
            .collect();
        // name → list of node indices (multiple nodes can share a name across files).
        let mut name_to_idxs: HashMap<&str, Vec<usize>> = HashMap::new();
        for (i, (_, name)) in node_pairs.iter().enumerate() {
            name_to_idxs.entry(name.as_str()).or_default().push(i);
        }

        // Build adjacency: out_edges[src_idx] = list of dst_idx (resolved from lattice_ref).
        let ref_edges = self.store.lattice_ref_edges(&self.repo_root)?;
        let mut out_edges: Vec<Vec<usize>> = vec![vec![]; n];
        for (src_id, dst_name) in &ref_edges {
            let Some(&src_idx) = id_to_idx.get(src_id.as_str()) else {
                continue;
            };
            if let Some(targets) = name_to_idxs.get(dst_name.as_str()) {
                for &dst_idx in targets {
                    if dst_idx != src_idx {
                        out_edges[src_idx].push(dst_idx);
                    }
                }
            }
        }

        const DAMPING: f64 = 0.85;
        const MAX_ITER: usize = 20;
        const CONVERGENCE: f64 = 1e-6;

        let uniform = 1.0 / n as f64;
        let mut rank = vec![uniform; n];
        let mut next = vec![0.0f64; n];

        for _ in 0..MAX_ITER {
            // Dangling rank: nodes with no out-edges contribute uniformly.
            let dangling: f64 = rank
                .iter()
                .enumerate()
                .filter(|(i, _)| out_edges[*i].is_empty())
                .map(|(_, r)| r)
                .sum::<f64>();

            for v in next.iter_mut() {
                *v = (1.0 - DAMPING) * uniform + DAMPING * dangling * uniform;
            }
            for (src, targets) in out_edges.iter().enumerate() {
                if targets.is_empty() {
                    continue;
                }
                let share = DAMPING * rank[src] / targets.len() as f64;
                for &dst in targets {
                    next[dst] += share;
                }
            }

            // Check convergence (L1 norm of delta).
            let delta: f64 = rank.iter().zip(&next).map(|(a, b)| (a - b).abs()).sum();
            rank.copy_from_slice(&next);
            for v in next.iter_mut() {
                *v = 0.0;
            }
            if delta < CONVERGENCE {
                break;
            }
        }

        // Normalize so scores sum to 1.0 (keeps values comparable across index sizes).
        let total: f64 = rank.iter().sum();
        if total > 0.0 {
            for r in rank.iter_mut() {
                *r /= total;
            }
        }

        let scores: Vec<(String, f64)> = node_pairs
            .iter()
            .enumerate()
            .map(|(i, (id, _))| (id.clone(), rank[i]))
            .collect();
        self.store.set_lattice_pageranks(&scores)?;
        Ok(())
    }

    /// Reverse-dependency closure: who references `symbol`, transitively, up to `max_depth` hops.
    pub fn impact(&self, symbol: &str, max_depth: usize) -> Result<BlastRadius, LatticeError> {
        self.impact_in_scope(symbol, max_depth, None)
    }

    /// Like [`impact`](Self::impact), but confine the roots, the dependents, and the whole walk to
    /// nodes whose repo-relative path starts with `scope` (e.g. `crates/forge-core`). tree-sitter
    /// refs are keyed by *name* with no cross-crate binding, so an unscoped `impact` on a symbol
    /// that exists in several crates mixes their blast radii together. `scope` confines it to one
    /// crate/dir so the result is unambiguous for a within-crate refactor check.
    pub fn impact_in_scope(
        &self,
        symbol: &str,
        max_depth: usize,
        scope: Option<&str>,
    ) -> Result<BlastRadius, LatticeError> {
        // A naive prefix check would let `--scope crates/forge-cli` also match a sibling like
        // `crates/forge-cli-extra`; require the match to land on a path-component boundary.
        let in_scope = |h: &NodeHit| {
            scope.is_none_or(|s| {
                let s = s.trim_end_matches('/');
                h.rel_path == s
                    || h.rel_path
                        .strip_prefix(s)
                        .is_some_and(|rest| rest.starts_with('/'))
            })
        };
        let roots = self.rows_to_hits(self.store.lattice_nodes_by_name(
            &self.repo_root,
            symbol,
            32,
        )?)?;
        let roots: Vec<NodeHit> = roots
            .into_iter()
            .filter(|h| h.name == symbol && in_scope(h))
            .collect();

        let mut seen: HashSet<String> = HashSet::from([symbol.to_string()]);
        let mut frontier = vec![symbol.to_string()];
        let mut dependents: Vec<NodeHit> = Vec::new();
        let mut files: HashSet<String> = roots.iter().map(|h| h.rel_path.clone()).collect();

        for _ in 0..max_depth.max(1) {
            let mut next = Vec::new();
            for name in &frontier {
                for hit in self.rows_to_hits(self.store.lattice_callers_by_name(
                    &self.repo_root,
                    name,
                    200,
                )?)? {
                    // A scoped walk only follows + reports dependents inside the scope, so the
                    // closure can't wander into a same-named symbol in another crate.
                    if !in_scope(&hit) {
                        continue;
                    }
                    if seen.insert(hit.name.clone()) {
                        next.push(hit.name.clone());
                    }
                    files.insert(hit.rel_path.clone());
                    dependents.push(hit);
                }
            }
            if next.is_empty() {
                break;
            }
            frontier = next;
        }

        // De-dup dependents by (name, rel_path, line) — a symbol can be reached via many paths.
        dependents.sort_by(|a, b| {
            (a.rel_path.as_str(), a.line, a.name.as_str()).cmp(&(
                b.rel_path.as_str(),
                b.line,
                b.name.as_str(),
            ))
        });
        dependents.dedup();
        let total_sites = dependents.len();
        let mut files: Vec<String> = files.into_iter().collect();
        files.sort();
        Ok(BlastRadius {
            roots,
            dependents,
            files,
            total_sites,
        })
    }

    /// A shortest call/reference chain of symbol *names* from `a` to `b` (BFS over forward
    /// references), or `None` if `b` isn't reachable from `a` within `max_depth` hops.
    pub fn path(
        &self,
        a: &str,
        b: &str,
        max_depth: usize,
    ) -> Result<Option<Vec<String>>, LatticeError> {
        if a == b {
            return Ok(Some(vec![a.to_string()]));
        }
        let mut seen: HashSet<String> = HashSet::from([a.to_string()]);
        let mut queue: VecDeque<Vec<String>> = VecDeque::from([vec![a.to_string()]]);
        while let Some(chain) = queue.pop_front() {
            if chain.len() > max_depth.max(1) {
                continue;
            }
            let last = chain.last().unwrap();
            for callee in self.store.lattice_callees_of_name(&self.repo_root, last)? {
                if callee == b {
                    let mut found = chain.clone();
                    found.push(callee);
                    return Ok(Some(found));
                }
                if seen.insert(callee.clone()) {
                    let mut next = chain.clone();
                    next.push(callee);
                    queue.push_back(next);
                }
            }
        }
        Ok(None)
    }

    /// Git provenance for a symbol: resolve its definition's file+line, `git blame` that line for
    /// the last commit that touched it, and report author/date/commit/subject. `Ok(None)` when the
    /// symbol isn't indexed, the tree isn't under git, or git is unavailable (never errors the turn).
    pub fn why(&self, symbol: &str) -> Result<Option<Provenance>, LatticeError> {
        let Some(hit) = self
            .query(symbol, 8)?
            .into_iter()
            .find(|h| h.name == symbol)
        else {
            return Ok(None);
        };
        let sha = match git_blame_sha(&self.repo_root, &hit.rel_path, hit.line) {
            Some(s) => s,
            None => return Ok(None),
        };
        let Some(meta) = git_show_meta(&self.repo_root, &sha) else {
            return Ok(None);
        };
        Ok(Some(Provenance {
            name: hit.name,
            rel_path: hit.rel_path,
            line: hit.line,
            author: meta.0,
            date: meta.1,
            commit: meta.2,
            subject: meta.3,
        }))
    }
}

/// The commit sha that last touched `line` of `rel_path`, via `git blame --porcelain`. `None` if
/// git fails (not a repo, git missing, path untracked).
fn git_blame_sha(repo_root: &str, rel_path: &str, line: i64) -> Option<String> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(["blame", "-L"])
        .arg(format!("{line},{line}"))
        .args(["--porcelain", "--"])
        .arg(rel_path)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    parse_blame_sha(&String::from_utf8_lossy(&out.stdout))
}

/// The first token of `git blame --porcelain` output is the commit sha.
pub(crate) fn parse_blame_sha(porcelain: &str) -> Option<String> {
    let sha = porcelain.split_whitespace().next()?;
    (sha.len() >= 7 && sha.chars().all(|c| c.is_ascii_hexdigit())).then(|| sha.to_string())
}

/// `(author, date, short-sha, subject)` for a commit via `git show`. `None` on git failure.
fn git_show_meta(repo_root: &str, sha: &str) -> Option<(String, String, String, String)> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args([
            "show",
            "-s",
            "--date=short",
            "--format=%an%x09%ad%x09%h%x09%s",
            sha,
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    parse_show_meta(&String::from_utf8_lossy(&out.stdout))
}

/// Parse the tab-separated `git show` line into `(author, date, short-sha, subject)`.
pub(crate) fn parse_show_meta(line: &str) -> Option<(String, String, String, String)> {
    let line = line.trim();
    let mut parts = line.splitn(4, '\t');
    let author = parts.next()?.to_string();
    let date = parts.next()?.to_string();
    let commit = parts.next()?.to_string();
    let subject = parts.next().unwrap_or("").to_string();
    Some((author, date, commit, subject))
}
