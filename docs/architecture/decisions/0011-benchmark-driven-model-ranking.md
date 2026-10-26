# ADR-0011: Benchmark-driven model ranking

**Date:** 2026-06-18 (amended 2026-10-26: keyless mirror and bundled snapshot)
**Status:** Accepted

## Context

The mesh ranks models for a task tier with `capability_score`, whose "quality" term comes
from `capability::quality_class` — a family-name heuristic (substring match: `opus`/`gpt-5`/
`sonnet`/`-70b` → quality 3, `mini`/`haiku`/`flash` → 1, etc.). This is coarse and brittle:

- It can't tell two "frontier" models apart (opus vs sonnet vs gpt-5.5 all collapse to quality 3;
  only the lightest-on-tie + version tiebreaks separate them).
- It has no notion of *actual* performance — a model is "good" because its name matches a pattern,
  not because it scores well. A new family name is unknown until the substring list is edited.
- It can't distinguish general reasoning from coding ability.

The mesh should rank on **real, trustworthy, dynamic performance data** so it makes an informed
choice, not a name-pattern guess.

## Decision

Add an optional **benchmark score layer** sourced from the **Artificial Analysis Data API**
(`https://artificialanalysis.ai/api/v2/data/llms/models`, `x-api-key`). Per model it exposes
`artificial_analysis_intelligence_index` (composite of MMLU-Pro, GPQA, LiveCodeBench, SciCode,
HLE, … — a widely-cited 0–100 index) and `artificial_analysis_coding_index`. It covers both
closed frontier and open models and is refreshed continuously.

- **Sources, in precedence order** (`crates/forge-cli/src/benchmarks.rs`):
  1. **Your own key** — `ARTIFICIALANALYSIS_API_KEY` or the `artificialanalysis` keyring entry
     (`forge auth artificialanalysis`). Fetched directly; cached with `source: own_key`.
  2. **The Forge mirror** (no key needed) — `https://forge.adulari.dev/data/benchmarks.json`, then
     `https://github.com/adulari/forge/releases/download/benchmarks-data/benchmarks.json`. A
     scheduled workflow (`.github/workflows/benchmarks-mirror.yml`, every 3 hours) fetches the feed
     with a repository secret, transforms it to Forge's own schema (`forge benchmarks
     --export-mirror`: coding indices Artificial Analysis has not published yet are estimated and
     flagged `coding_estimated`), validates it (non-empty, 0–100, not shrunk by more than half), and
     publishes it as an asset of the rolling `benchmarks-data` prerelease. The document carries the
     attribution block. Requests send `If-None-Match`; the TTL is 6 hours, and a catalog model with
     no rating triggers a refetch sooner, but never more than once an hour. 10 s timeout.
  3. **The bundled snapshot** — `crates/forge-cli/data/benchmarks-snapshot.json`, same schema,
     compiled in with `include_bytes!` and refreshed by the release checklist
     (`scripts/refresh-benchmarks-snapshot.sh`). Used offline or when the mirror is down; seeded
     into the cache so a failed fetch is not retried on every run.
  4. **The family-name heuristic**, when none of the above yields data.

  One cache file, `~/.local/share/forge/benchmarks.json` (schema 2), records the `source`, the
  mirror ETag, when the data was read from Artificial Analysis (`data_as_of`) and when we last asked
  (`checked_at`). Adding a key later replaces a mirror or snapshot cache on the next run. Best-effort
  and never blocks a turn on failure; `forge benchmarks --refresh` forces a fetch.
- **Mapping:** Artificial Analysis slugs (e.g. `gpt-5-2`, `claude-4-5-sonnet`) → Forge
  `provider::model` ids via a normalizer (lowercase, drop provider prefix, collapse separators,
  extract family + version) plus a small alias map for the CLI bridges
  (`claude-cli::opus` → the AA Claude-Opus row). Match is by canonical key; unmatched models simply
  fall back.
- **Scoring:** `capability_score` gains an optional `&BenchmarkScores`. When a model has a score,
  the quality term is `index / 20` (so a ~60 index ≈ quality 3.0, on the same scale the old
  heuristic produced, so cost/conserve terms are unchanged). Complex uses the intelligence index;
  **code-heavy** complex/standard use the **coding index**; trivial keeps the speed-favouring blend.
  When a model has no score (or the layer is disabled / no key), it falls back to `quality_class` —
  so behaviour degrades gracefully and existing tests (no benchmark data) are unchanged.
- **Carrier:** `ModelCatalog` holds an optional `BenchmarkScores` (attached at discovery like the
  model list), so `route_score`/`ranked_rows` read it with no new threading through call sites.
- **Config:** `mesh.benchmark_ranking` (default `true`; `false` disables the layer and makes no
  request). `mesh.benchmark_mirror` (default `true`; or `FORGE_BENCHMARK_MIRROR=0`) stops the mirror
  fetch for privacy-minded users: ranking then uses your own key if set, else the bundled snapshot.
  With no data from any source it's a no-op (heuristic).
- **Attribution:** every surface that shows the scores (`forge benchmarks`, the `/mesh` overlay, the
  Models screen, the README, the published document itself) credits "Benchmark data: Artificial
  Analysis (artificialanalysis.ai)".

## Rationale

- Artificial Analysis is the one source that is **dynamic, programmatic (real API, not scraping),
  trustworthy (composite of standard benchmarks), and covers closed + open** in a single feed with
  a free tier — and it already separates a **coding index** from general intelligence, which the
  mesh needs (it routes code-heavy tasks differently).
- Keeping it an *optional layer over* the heuristic means zero regression risk: the mesh works
  exactly as before without a key, and gets sharper rankings with one.

## Alternatives considered

- **LMArena / Chatbot Arena Elo** (HF dataset): trustworthy human-preference signal, but coding is
  split across separate arenas, model-name mapping is messier, and it's a dataset to track rather
  than a clean per-model API. Good *secondary* signal; not the primary.
- **OpenRouter models API:** has pricing/context metadata but **no benchmark/performance scores** —
  doesn't answer "which is more capable".
- **Aider polyglot leaderboard:** excellent coding signal but coding-only and a smaller model set;
  subsumed by the AA coding index.
- **HF Open LLM Leaderboard:** open models only — misses the closed frontier the mesh routes to.
- **Keep the family heuristic:** rejected — it's the problem being solved.

## Consequences

**Positive:** rankings reflect measured performance; new models are ranked the moment AA lists them
(no code edit); coding tasks rank on coding ability.
**Negative:** the mirror is a service the maintainers run (mitigated by the bundled snapshot and
the heuristic); the published copy can be up to ~9 hours behind Artificial Analysis (3 h publish +
6 h client TTL); an external dependency (mitigated by caching + heuristic fallback); slug→id mapping needs maintenance for odd names (mitigated by `forge benchmarks`
to inspect coverage).
**Neutral:** the heuristic `quality_class` stays as the fallback, not removed.
</content>
