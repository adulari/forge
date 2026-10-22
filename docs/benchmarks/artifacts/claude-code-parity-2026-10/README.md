# Claude Code parity study: data and tools

Data behind [claude-code-parity-2026-10.md](../../claude-code-parity-2026-10.md). Standard-library Python only.

| Path | What it is |
|---|---|
| `runs.json` | Manifest of run sets. The first entry is the baseline; each later entry is a measured Forge build. |
| `fixes.json` | The fix ledger rendered into the report (PR number, title, state, finding, evidence). Edit it when a PR changes state. |
| `baseline/suites-abd.json` | Suites a, b, d: per arm and repeat, per task: solved, agent wall, tokens, tool calls, model calls, plus arm totals and medians. |
| `baseline/suite-c.json` | Suite c in the same shape. FD `c06` is recorded as failed (`no_result`, "panic after completion"). |
| `baseline/multiturn-*.json` | Multi-turn summaries, copied unchanged from the bench kit. |
| `baseline/swe/<arm>/` | Official `swebench==4.1.0` report, `predictions.jsonl`, `predictions.metrics.jsonl`, `predictions.integrity.jsonl`, `summary.tsv`, `run-meta.json`, `eval.log`. |
| `baseline/tasks.json` | Every task: id, suite, language, type, one-line summary and the SHA-256 commitment of its withheld directory. |
| `baseline/environment.json` | Versions, model ids, host, dates, concurrency, commit provenance, private archive hash. |
| `baseline/isolation-check.json` | Which of the operator's instruction files reached the model in the isolated Claude Code and bridge arms (counts and sizes only). |
| `baseline/PARITY.md` | Static feature audit (280 features, 25 ranked gaps). |
| `render.py` | Rewrites the tables of the report between `<!-- BEGIN:name -->` and `<!-- END:name -->` markers. Deterministic. |
| `ingest.py` | Converts bench-kit output (`aggregate.py` summaries, task directories) into the JSON above. |
| `audit_raw.py` | Independent check of the report against the kit's raw files: recomputes every table cell, the headline rows, the figures quoted in the text and the task commitments, sharing no code with `render.py`. Needs the kit. |

## Render

```
python3 render.py            # rewrite the report's tables
python3 render.py --check    # exit 1 if the report is out of date
python3 render.py --verify   # recompute arm totals from per-task rows and compare with the stored totals
python3 audit_raw.py --kit ~/.cache/forge-ops/ccbench --commitments   # recompute from the kit's raw files (needs the kit)
```

`render.py --verify` only proves that the stored totals match the stored rows. `audit_raw.py` is the check against the raw output; run it
after every `render.py` and after adding a run set (extend it for the new run set first: it covers the baseline).

## Regenerate the baseline files

`ingest.py` regenerates the derived baseline files from the kit byte for byte (checked with `cmp` during review; the multi-turn files and the
SWE-bench files are plain copies of the kit's own):

```
K=~/.cache/forge-ops/ccbench
python3 ingest.py abd $K/results/all-now/summary.json baseline/suites-abd.json --default-run sonnet-s1   # the Sonnet arms carry no @run suffix
python3 ingest.py c $K/results/c-agg/summary.json baseline/suite-c.json --runs-dir $K/results/runs/c \
    --failed "FD-deepseek:c06-rename-config-provider-of:panic after completion"
python3 ingest.py tasks $K/suite $K/multiturn/sessions $K/swe/instances.jsonl $K/results/swe/FD-deepseek/run-meta.json baseline/tasks.json
```

## Add an after-fixes run set

1. Run the arms on the new Forge build with the bench kit, using the same suites and the same arm ids.
2. `python3 ingest.py abd <aggregate summary.json> after-1/suites-abd.json --default-run s1` (and `c`, with `--runs-dir`
   and `--failed ARM:TASK:REASON` for runs that produced no result row). Copy multi-turn and SWE-bench files in the same
   shapes under `after-1/`, and write a new `environment.json`.
3. Append an entry to `runs.json` with its `id`, `label`, Forge version and commit, and the `files` map used by the baseline.
4. `python3 render.py`. The `after-*` blocks compare the newest run set with the baseline for the Forge arms it contains.
   Claude Code arms missing from the new run set are taken from the baseline.

## Not in this directory

Hidden graders, hidden tests, reference solutions and the suite archives are not committed. `baseline/tasks.json` holds
their SHA-256 commitments so they can be disclosed and checked later. The private raw archive (transcripts, Forge stores,
the pinned binary, the full task kit) is named in the report's Reproduction section. The InferHub API key is not stored here.
