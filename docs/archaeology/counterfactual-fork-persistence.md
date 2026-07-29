# Code archaeology: counterfactual fork persistence

## Summary

Counterfactual forks copy a stable transcript prefix into a new top-level session and retain ancestry for tree display. This transactional lifecycle is now isolated from the store root without changing sequence boundaries, copied metadata, or child-session filtering.

## History and invariants

- `d96ca7bd` introduced counterfactual forks for model comparison and linked the new session to its source and boundary sequence.
- Fork creation deliberately copies only active messages with `seq < at_seq`; the triggering turn is rerun in the new session.
- Fork trees exclude subagent child sessions so user-visible conversation branches are not polluted by worker fan-out.

## Boundary

`fork_store.rs` owns atomic fork creation and ancestry projection. General session creation, transcript writes, catalog/resume filtering, and Anywhere handoff remain with their lifecycle owners.

## Interface as test surface

`fork_copies_the_prefix_and_links_back` characterizes the strict prefix, new identity, ancestry fields, source preservation, and tree projection. `fork_journals_the_session_and_copied_prefix_atomically` characterizes the Anywhere outbox boundary. Session-catalog tests retain the top-level/child distinction consumed by the tree.

## Leave alone

- Fork creation is one immediate transaction.
- Only active rows strictly before `at_seq` are copied.
- Copied rows retain role, content, model, tool linkage, visibility, and sequence.
- The fork remains top-level while recording `forked_from` and `forked_at_seq`.
- Fork session and copied-message snapshots enter the Anywhere sync journal in the same transaction.
- Fork-tree rows exclude subagent children and are ordered by creation timestamp then id; consumers construct the hierarchy.
