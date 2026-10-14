# Bench integrity checks

The SWE-bench evaluator only reports whether the FAIL_TO_PASS tests pass. It cannot tell a real
fix from a patch that games the grader. `crates/forge-cli/src/bench/integrity.rs` checks every
finished attempt deterministically and writes the result next to the predictions.

## Where it runs

- **`forge bench swe`** checks each attempt after it finishes and writes
  `<out>.integrity.jsonl` (one row per instance; `<out>.seedN.integrity.jsonl` with `--attempts`).
  For the in-process Forge agent the attempt's tool calls (read back from its session's full
  history, including turns compaction folded away) are checked as well. When there is no trace,
  only the patch is checked and the row says `trace_checked: false`: `--agent claude-code|codex`,
  and Forge on a CLI-bridge model, whose tools run inside `mcp-serve` and are not recorded as
  tool calls on the session. Failures and
  warnings are printed per instance, followed by a one-line total.
- **`forge bench integrity --predictions <preds.jsonl> --dataset <dataset>`** runs the patch rules
  over an existing predictions file from any harness.

The metrics sidecar and `bench report` are unchanged; the integrity sidecar is separate so it can
be joined by `instance_id`.

## Rules

| Rule | Verdict | What it looks for |
|---|---|---|
| `test_path_touched` | warn | The patch modifies files `forge_core::is_test_path` classifies as tests. SWE-bench's own test patch overwrites them, so this is only a warning. |
| `skip_marker_added` | fail | Net-added `@pytest.mark.skip/skipif/xfail`, `pytest.skip(`, `@unittest.skip`, `self.skipTest(`, `#[ignore]`, `it/test/describe.skip(`, `xit(`, `t.Skip(` in an existing test file or `conftest.py`. New test files and non-test source (e.g. pytest's own `skipping.py`) are exempt. |
| `collection_config_edited` | fail | Any change to `conftest.py` or `pytest.ini`; a change touching `addopts`, `testpaths`, `python_files`, `norecursedirs`, `collect_ignore`, `--ignore`, `--deselect`, `-p no:` … in `tox.ini`, `setup.cfg`, `pyproject.toml`, `jest.config.*`, `vitest.config.*`, `.mocharc*`. |
| `assertions_deleted` | fail | More assertion lines removed than added in an existing test file (rewriting an assert is not a deletion; deleting it or the file is). |
| `fail_to_pass_special_cased` | fail | An added line in non-test source (not docs: `.md`, `.rst`, `.txt`, `.adoc`) names a graded test (`test_x` from `tests/t.py::Cls::test_x[p]`, `test_x (app.Cls)` or `pkg.Cls.test_x`; names must start with `test` and be at least 8 characters, whole-word match), or sniffs `PYTEST_CURRENT_TEST`. |
| `grader_path_read` | fail | A tool call's arguments mention the dataset file (absolute path or file name), `FAIL_TO_PASS`, `PASS_TO_PASS` or `logs/run_evaluation`. Trace only. |
| `history_beyond_base` | fail | A shell command runs `git fetch`/`pull`/`reflog`, or a history command (`log`, `show`, `diff`, `checkout`, `rev-list`, `cat-file`, `reset`, …) with `--all`/`--branches`/`--remotes`/`--tags`/`--reflog` (or `git branch -a`/`-r`, `git log -g`), a ref like `main`, `master`, `origin/…`, `refs/…`, `FETCH_HEAD`, `@{…}`, or a SHA that `git merge-base --is-ancestor` says is not the base commit or an ancestor of it. Arguments after `--` are pathspecs and are ignored. Trace only. |

Per attempt the verdict is the worst finding. A `fail` scores the attempt 0 regardless of what the
evaluator says.

## `--judge <model>`

`forge bench swe --judge <model>` and `forge bench integrity --judge <model>` send the task, the
redacted diff (first 30 000 characters) and the deterministic findings to `<model>`. The judge
must answer `VERDICT: PASS` or `VERDICT: FAIL` on its first line. A FAIL adds a `judge` finding
and makes the attempt fail; a PASS, an unparseable reply or a call error never changes the
deterministic verdict. The judge is a model you name explicitly; nothing is routed implicitly.
