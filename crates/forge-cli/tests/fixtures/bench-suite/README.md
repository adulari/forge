Example `forge bench tasks` suite used by `tests/bench_suite_e2e.rs`. With `--mock`,
`write-note` is solved (the mock provider writes `mock-note.txt` when the prompt says
`mock:write`) and `needs-fix` is not, so a run scores 1/2.
