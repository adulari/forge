# Code archaeology: edit matching and notebook editing

## Boundaries

Two owners split out of `core_tools.rs`, both along a "judgement vs. I/O" seam:

- `core_tools/edits.rs` — where an edit lands and whether it should land at all. The
  whitespace-insensitive fallback, the block-anchor fallback, the truncation guards
  (quote balance, bracket balance, char-literal scanning), `apply_edit`, the `multi_edit`
  argument extraction, and the all-or-nothing fold.
- `core_tools/notebook.rs` — cell-level `.ipynb` editing: `NotebookEditTool`, nbformat's
  line-list `source` shape, and the rewrite that clears a replaced code cell's stale outputs.

`core_tools.rs` keeps path confinement, the read/write/append/delete/patch tools, and the read
caps.

## Why these are real owners, not line moves

`edits.rs` exists because two failure modes of model-authored edits are decided together: an
`old` string that is *nearly* right (indentation drift) must still find its unique target, and a
`new` string that was *cut off* must be refused rather than silently truncating a block. Both
rules are uniqueness- and safety-preserving, and neither touches the filesystem — the tools do the
I/O. `notebook.rs` exists because an `.ipynb` is JSON: the generic text tools would corrupt it, so
the safe path is a separate owner rather than a special case inside the text tools.

## Interfaces

`edits.rs` exposes `apply_edit`, `apply_edits`, `multi_edit_pairs` (plus the matchers its own
tests characterize) as `pub(super)`. `notebook.rs` re-exports `NotebookEditTool` unchanged for
`ToolRegistry`, and reuses the parent's `confine` and `check_readable_size` rather than
duplicating the confinement rule.

## Characterization

Every test moved with the code it characterizes: the flexible/anchor matcher uniqueness rules, the
truncation and bracket-balance heuristics (including comment, string, and lifetime handling), the
all-or-nothing multi-edit fold, and the notebook replace/insert/delete cases with their
outputs-clearing assertion.
