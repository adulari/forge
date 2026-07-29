# Code archaeology: Core direct completeness policy

The direct completeness audit was introduced after plausible identifier
migrations passed focused tests while leaving sibling production consumers on
the deprecated path. It is deliberately bounded: only identifier/API/config
migration prompts qualify, searches are literal and production-scoped, returned
unedited paths require inspection, and the retries address only missing/empty
search evidence or unhandled paths. It must not become a general self-review
loop.

The redrive answer guard is equally load-bearing: a guard-halted re-drive often
has empty final text, so it may not erase the primary completion. Failure
tracking classifies repeated tool errors by tool/category to stop unproductive
loops without treating an unrelated succeeding tool as the same failure.

Core completeness, failure-loop, and end-to-end turn tests characterize these
invariants; the extraction keeps the policy private and deterministic.
