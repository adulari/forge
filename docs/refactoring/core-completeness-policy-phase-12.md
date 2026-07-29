# Core direct-completeness policy split

This phase isolates direct-provider identifier-migration completeness policy in
private `completeness.rs`: bounded audit prompts, repository-search evidence
classification, unedited production-path reconciliation, redrive text adoption,
and the shared tool-failure tracker used by the turn loop.

The strict behavior is unchanged: an identifier migration receives at most one
bounded completeness sweep plus narrowly defined retries; evidence must cover
real production siblings rather than only tests or narration; blank re-drive
text never replaces a primary answer; repeated tool failures retain their
category/count across the turn.

Core root implementation lines reduce from 6,093 to 5,535; the extracted owner
is 569 lines. Core Clippy, 507 Core tests, three long-session endurance tests,
formatting, and the architecture guard passed.
