# Serve project discovery ownership phase

## History and boundary

The project picker, recent-workspace projection, and safe filesystem browser landed as one control-surface capability in `f2378dea`. Its policy is cohesive: configured roots are expanded and canonicalized once; passive directory discovery is constrained to those roots; generated worktrees are excluded from recent projects; live and durable sessions are merged by activity and canonical path.

This phase moves the complete project catalog/browser policy and its containment tests to `serve/serve_projects.rs`. Serve retains route registration and daemon state construction. A real-router characterization remains in the composition root to pin token scoping, default-root selection, response shape, and directory projection. The old response models, root resolution, browsing, MRU projection, and handlers are deleted from `serve.rs`.

## Review and security

Independent review confirmed component-aware canonical containment and symlink-escape rejection. It requested route-level characterization and explicit relative-path semantics. The route characterization is included, and browse queries now reject relative paths instead of resolving them against the daemon process working directory; omission still selects the first configured canonical root. Descriptor-relative traversal is not introduced because this authenticated local control surface is not a hostile concurrent-filesystem boundary, but every returned child is canonicalized and checked before disclosure.

## Measured result

- Serve root: 3,113 to 2,872 implementation lines.
- New project owner: below 500 implementation lines.
- Repository distribution: 218/297 (73.4%) at or below 500 and 276/297 (92.9%) at or below 800.
- Eight owners remain above 2,000; none exceeds 5,000.

This is an intermediate deep-domain phase. It does not waive or claim the 90%/95% terminal gates, regenerate the baseline, or enable auto-merge.
