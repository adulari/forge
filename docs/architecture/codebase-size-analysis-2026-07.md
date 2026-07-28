# Forge codebase size analysis

- **Observed:** 2026-07-28
- **Forge scope:** active `perf/cache-aware-session-affinity-20260728` worktree
- **External scope:** live GitHub default branches on the observation date

## Executive summary

Forge is large in absolute terms, but it is not unusually large for its product
surface. The initial gross count of **290,449 physical source lines** included
**21,154 lines of vendored `genai` code**. The fair Forge-owned count is
therefore approximately **269,295 physical lines of code (LOC)**.

That owned source includes:

- the Rust agent runtime, CLI, TUI, Model Mesh, providers, Store, tools,
  Lattice, and MCP support;
- a mobile, web, and desktop client;
- substantial benchmark, manual-E2E, CI, demo, and release infrastructure;
- roughly 60,000 lines of inline Rust tests, plus external test files.

After excluding vendored code, tests, benchmark harnesses, and non-runtime
utilities, the shipping implementation is approximately **180,000–205,000
physical LOC**. This is an estimate, not a strict language-aware logical SLOC
count.

The evidence supports five high-level conclusions:

1. Forge is not obviously bloated at the repository level.
2. Forge is unusually feature-dense relative to similarly sized open-source AI
   coding tools.
3. Similar benchmark performance with much less code than Codex is plausible
   because model intelligence and a relatively small critical agent loop
   dominate the measured workloads.
4. Codex's additional size primarily buys operational depth, platform
   compatibility, protocol stability, migration support, and large-scale
   hardening rather than proportionally more benchmark intelligence.
5. Forge's main size-related risk is concentration in several very large
   hotspot files, not its total repository LOC.

## Measurement methodology

### Forge

Forge was measured in the active worktree using physical newline counts over
recognized source extensions, including Rust, Python, shell, TypeScript,
JavaScript, Go, SQL, JSON, YAML, and TOML. Markdown, generated build output,
Git metadata, and other prose or data formats were not counted as source.

This is a reproducible physical-line count. It is not equivalent to logical
SLOC from a language-aware tool: it includes blank lines and comments in the
selected source files.

The original 290,449 figure included the checked-in
`vendor/genai-0.6.5` source. This document reports both gross and Forge-owned
values so the distinction remains explicit.

### External projects

External project values are estimates derived from live GitHub Linguist
source-byte totals. Markdown, MDX, and Jupyter Notebook bytes were excluded.
Per-language bytes were divided by the observed bytes-per-line ratio in Forge
for matching languages, with conservative defaults for languages not
represented locally.

External values should be treated as approximately **±15–25%**, especially for
repositories containing generated protocol clients, bundled UI source,
unusual formatting, or language-specific line-length patterns. They are
suitable for a scale comparison, not exact LOC accounting.

The comparison also has a scope limitation: repository LOC measures the public
implementation, not feature completeness, operational maturity, or proprietary
systems behind it.

## Forge source composition

### Gross versus owned source

| Scope | Physical LOC |
|---|---:|
| Gross recognized source | 290,449 |
| Vendored `genai` source | 21,154 |
| **Forge-owned source** | **269,295** |

Forge-owned source occupies approximately 11.24 MB. Vendored recognized source
contributes another 0.66 MB.

### Owned source by top-level area

| Area | Physical LOC | Share of owned source | Role |
|---|---:|---:|---|
| `crates/` | 190,285 | 70.7% | Rust runtime, CLI, TUI, Mesh, providers, Store, tools, indexing |
| `mobile/` | 54,939 | 20.4% | TypeScript UI, Tauri, and mobile platform integrations |
| `scripts/` | 20,025 | 7.4% | Benchmarks, manual E2E, CI, demos, and release tooling |
| `docs/` source and root utilities | 4,046 | 1.5% | Interactive documentation/demo code and root utilities |

The mobile client is mostly concentrated in `mobile/src`:

| Mobile area | Physical LOC |
|---|---:|
| `mobile/src` | 51,698 |
| `mobile/src-tauri` | 1,379 |
| `mobile/targets` | 1,025 |
| Remaining mobile/platform files | 837 |

The scripts directory is predominantly evidence and verification
infrastructure:

| Script area | Physical LOC |
|---|---:|
| `scripts/manual-e2e` | 8,359 |
| `scripts/harness-bench` | 6,590 |
| `scripts/promo` | 2,464 |
| Other script areas | 1,402 |
| `scripts/ci` | 1,025 |
| `scripts/demo` | 185 |

`manual-e2e` and `harness-bench` alone account for **14,949 LOC**, or roughly
75% of the scripts directory.

### Rust workspace by crate

| Crate | Physical LOC | Source files |
|---|---:|---:|
| `forge-cli` | 60,785 | 74 |
| `forge-core` | 35,990 | 24 |
| `forge-tui` | 18,674 | 16 |
| `forge-provider` | 16,448 | 14 |
| `forge-store` | 14,394 | 4 |
| `forge-mesh` | 10,643 | 7 |
| `forge-config` | 8,696 | 6 |
| `forge-tools` | 5,721 | 6 |
| `forge-index` | 4,419 | 7 |
| `forge-mcp` | 2,794 | 5 |
| `forge-skills` | 2,605 | 6 |
| `forge-anywhere-protocol` | 2,117 | 11 |
| `forge-types` | 1,906 | 2 |
| `forge-voice` | 1,586 | 5 |
| `forge-relay` | 1,377 | 4 |
| `forge-lsp` | 1,125 | 5 |
| `forge-workflow` | 561 | 1 |
| `xtasks` | 444 | 5 |

`forge-cli` is large because it is both the composition root and the home of
substantial remote-access, server, authentication, benchmark, update, and
lifecycle behavior. Its size is spread across many files, unlike the more
concentrated Core, Store, TUI, and Mesh hotspots.

## Test and verification weight

Forge's total Rust count is 211,726 physical LOC when vendored Rust source is
included. Approximately:

- **60,393 LOC** are in inline `#[cfg(test)] mod tests` sections;
- **3,516 LOC** are in external Rust test files;
- **6,982 LOC** across all languages are in externally identifiable test paths
  or files;
- **14,949 LOC** are in the manual-E2E and harness-benchmark script suites.

The inline-test estimate locates the first conventional
`#[cfg(test)] mod tests` block and counts to the end of the Rust file. It is
intentionally approximate, but it shows that around 30% of the Rust footprint
is tests even before counting manual E2E and benchmark harnesses.

Approximate per-crate inline-test weight:

| Crate | Total Rust LOC | Approx. inline-test LOC | Approx. non-inline-test LOC |
|---|---:|---:|---:|
| `forge-cli` | 59,095 | 11,695 | 47,400 |
| `forge-core` | 35,990 | 16,346 | 19,644 |
| `forge-tui` | 18,674 | 4,957 | 13,717 |
| `forge-provider` | 16,448 | 6,404 | 10,044 |
| `forge-store` | 14,394 | 4,141 | 10,253 |
| `forge-mesh` | 10,643 | 4,980 | 5,663 |
| `forge-config` | 8,696 | 2,335 | 6,361 |
| `forge-tools` | 5,721 | 1,861 | 3,860 |
| `forge-index` | 4,419 | 1,808 | 2,611 |

This verification share is a positive explanation for repository growth: a
significant part of Forge's size is executable evidence, rather than additional
product branching.

## Comparison with open-source AI coding tools

The table below uses the external estimation method described above and the
corrected Forge-owned count.

| Project | Approx. source LOC | Relative to Forge | Notes |
|---|---:|---:|---|
| [OpenAI Codex](https://github.com/openai/codex) | ~1.14M | 4.2× larger | Large Rust monorepo with multiple servers, protocols, sandboxes, and clients |
| [OpenCode](https://github.com/anomalyco/opencode) | ~551k | 2.0× larger | TypeScript monorepo with CLI/TUI, server, web, and desktop surfaces |
| [Gemini CLI](https://github.com/google-gemini/gemini-cli) | ~495k | 1.8× larger | TypeScript CLI agent platform |
| [Cline](https://github.com/cline/cline) | ~466k | 1.7× larger | VS Code agent, UI, providers, and integrations |
| [Goose](https://github.com/block/goose) | ~285k | 1.1× larger | Rust agent runtime with desktop and extension surfaces |
| **Forge owned source** | **269k** | **1.0×** | Full Rust harness plus mobile/web/desktop and verification infrastructure |
| [Roo Code](https://github.com/RooCodeInc/Roo-Code) | ~266k | Approximately equal | VS Code agent and UI; observed repository was archived |
| [Continue](https://github.com/continuedev/continue) | ~257k | Approximately equal | IDE extensions, agent runtime, and model integrations |
| [OpenHands](https://github.com/OpenHands/OpenHands) | ~177k | 0.66× | Agent platform and web application |
| [Plandex](https://github.com/plandex-ai/plandex) | ~77k | 0.29× | Primarily Go-based terminal coding agent |
| [Aider](https://github.com/Aider-AI/aider) | ~44k | 0.16× | Focused Python terminal coding assistant |

Claude Code and Copilot CLI are not meaningfully comparable from their public
repositories:

- `anthropics/claude-code` exposed only about 12,000 estimated source lines at
  observation time and does not contain the complete Claude Code
  implementation.
- `github/copilot-cli` contained essentially installation and support material
  rather than the proprietary agent core.

The corrected Forge count places it in the same broad footprint band as Goose,
Roo Code, and Continue. It remains substantially smaller than Codex, OpenCode,
Gemini CLI, and Cline.

These estimates do not establish that one project is more efficient purely
because it has fewer lines. They establish that Forge's size is credible for
its scope and that it is not an outlier among full product-style coding
agents.

## Why Codex is much larger

Codex's `codex-rs` workspace contains approximately **44.4 MB** of recognized
source, versus approximately **11.24 MB** for all Forge-owned source. That is
roughly a 4:1 source-byte ratio, consistent with the estimated LOC comparison.

Representative large Codex areas at the observation date included:

| Codex area | Approx. source bytes | Approx. source files |
|---|---:|---:|
| Core | 10.22 MB | 539 |
| TUI | 8.50 MB | 403 |
| App server | 4.36 MB | 205 |
| App-server protocol | 1.43 MB | 675 |
| Exec server | 1.36 MB | 104 |
| Core plugins | 1.21 MB | 58 |
| Extension support | 0.83 MB | 132 |
| Protocol | 0.80 MB | 39 |
| Configuration | 0.72 MB | 59 |
| Windows sandbox | 0.71 MB | 60 |
| MCP client | 0.67 MB | 55 |
| Thread store | 0.67 MB | 35 |
| State | 0.65 MB | 29 |
| Network proxy | 0.59 MB | 34 |
| App-server transport | 0.59 MB | 24 |

For scale:

- all of Forge Core is about 1.50 MB, including tests;
- Forge TUI is about 0.73 MB, including tests;
- Forge CLI is about 2.47 MB; and
- every Forge Rust crate combined is about 7.74 MB.

At least **15.25 MB** of Codex's 44.4 MB Rust-workspace source was located in
externally identifiable test or fixture paths, about 34%, before counting
inline tests and non-source snapshot data. Codex and Forge both invest heavily
in tests, but Codex supports a much wider and deeper operational envelope.

The additional Codex source appears to support:

- multiple app-server, execution-server, and transport protocols;
- long-lived protocol and client compatibility;
- Windows-specific sandboxing;
- network proxying and policy enforcement;
- rollout and analytics infrastructure;
- durable thread state and migrations;
- many protocol consumers and generated or adapted clients;
- a much larger fixture and integration-test matrix;
- production hardening for a large, heterogeneous user base.

These capabilities matter greatly for product reliability, but they do not
necessarily improve a narrow, history-isolated six-turn coding benchmark.

## Why Forge can perform similarly with less code

The intelligence of an AI coding harness does not scale linearly with its
repository LOC. Forge and Codex ultimately call frontier models; harness
performance depends disproportionately on a much smaller critical path:

1. task framing and instruction quality;
2. relevant context selection and fitting;
3. model selection and routing;
4. tool schema and result representation;
5. session continuity and cache behavior;
6. completion verification and enforcement;
7. retry, recovery, and failover behavior;
8. latency and token overhead around provider calls.

Forge has concentrated substantial engineering on that path through Model Mesh
routing, continuation affinity, context fitting, stable provider-native
transports, tool recovery, and completion guards. Strong benchmark performance
with less total code is therefore plausible.

Codex's extra source is more likely to show its value on Windows, unreliable
networks, enterprise policies, migrations, unusual terminals, very large
histories, backward compatibility, and large-scale operational reliability
than on a narrow benchmark cell.

Important limitations remain:

- A six-turn benchmark measures the exercised agent path, not total platform
  maturity.
- Feature count is not feature depth.
- A public feature may require substantially more compatibility work in Codex
  than in Forge.
- Strong retained benchmark results do not prove equal reliability across
  every repository, task class, operating system, provider outage, or
  long-session shape.
- LOC cannot measure code quality, correctness, maintainability, or user
  experience by itself.

## Architecture hotspots

The overall repository size is defensible. Concentration inside several
central files is the more important warning signal:

| File | Physical LOC | Main test module begins |
|---|---:|---:|
| [`crates/forge-core/src/lib.rs`](../../crates/forge-core/src/lib.rs) | 21,184 | Approximately line 10,726 |
| [`crates/forge-store/src/lib.rs`](../../crates/forge-store/src/lib.rs) | 12,281 | Approximately line 8,300 |
| [`crates/forge-tui/src/app.rs`](../../crates/forge-tui/src/app.rs) | 10,643 | Approximately line 7,210 |
| [`crates/forge-mesh/src/lib.rs`](../../crates/forge-mesh/src/lib.rs) | 5,534 | Approximately line 2,945 |

Large test sections explain part of these totals, but the production sections
remain large. The resulting risks include:

- high merge-conflict probability;
- too many responsibilities in one compilation unit;
- harder review navigation;
- a greater chance of unrelated behavior interacting;
- larger context requirements for coding agents;
- crate-level seams that are stronger than internal module seams.

This does not imply that these files should be split mechanically by line
count. Future growth should prefer extracting deep modules with clear
ownership, stable interfaces, and independent tests rather than appending more
responsibilities to these files.

## Comparative architecture study

This section turns the size observations into an improvement workflow. It uses
two external reference snapshots:

- OpenAI Codex at
  [`bb1af235ea28`](https://github.com/openai/codex/commit/bb1af235ea2822d7a40f75ef52e4d6a2cde84da2),
  observed 2026-07-28.
- The unofficial `yasasbanukaofficial/claude-code` mirror at
  [`a371abbe75ff`](https://github.com/yasasbanukaofficial/claude-code/commit/a371abbe75ffa0d0a3c92290e2bbf56a7ef54367),
  observed 2026-07-28.

These references have different evidentiary value. Codex is an official,
actively maintained Apache-2.0 repository with public contribution rules,
tests, history, and CI. The Claude Code mirror explicitly says it contains
proprietary source recovered from a published sourcemap and is not an official
Anthropic product. It can reveal a release snapshot's high-level structure,
but not Anthropic's current source, review rules, test suite, development
history, or intended architecture. No source from either project should be
copied into Forge; only general engineering patterns are considered here.

### What Codex does with large Rust modules

Codex does not avoid large files perfectly. In the inspected Core, TUI, State,
Thread Store, and model-management areas, 688 non-test-path Rust files had this
physical-size distribution:

| File size | Files | Share |
|---|---:|---:|
| Up to 500 LOC | 513 | 74.6% |
| 501–800 LOC | 76 | 11.0% |
| 801–2,000 LOC | 72 | 10.5% |
| 2,001–5,000 LOC | 24 | 3.5% |
| More than 5,000 LOC | 3 | 0.4% |

This is indicative rather than repository-wide: it excludes obvious test
paths but may still include inline tests. Current Codex exceptions include
approximately 4,590 implementation LOC in Core configuration, 4,150 in its
session module, 4,815 in the TUI chat composer before its main inline test
module, and 3,228 in the resume picker before tests. Codex therefore provides
useful practices, not a flawless target state.

The important difference is that Codex makes module size an explicit
engineering policy. Its
[`AGENTS.md`](https://github.com/openai/codex/blob/bb1af235ea2822d7a40f75ef52e4d6a2cde84da2/AGENTS.md)
states:

- target Rust modules below 500 implementation LOC;
- at roughly 800 LOC, put new behavior in another module unless a strong
  reason is documented;
- keep modules private and export the crate interface explicitly;
- move related tests and type documentation with an extraction;
- resist adding new concepts to `codex-core`; use an existing owner or create a
  focused crate when the concept is independently reusable;
- keep crate interface surfaces small;
- build model-visible context incrementally, keep reusable context stable, and
  hard-cap every injected item.

The current repository also shows how Codex applies those rules:

1. **Thin crate roots and explicit exports.** `codex-core/src/lib.rs` is about
   202 lines and declares or re-exports focused private modules.
   `thread-store/src/lib.rs` is about 68 lines. The roots present an interface
   rather than containing most implementation.
2. **Private submodules around a state owner.** A large `App` or session can
   remain the state owner while its behavior is implemented in topical files.
   The TUI `app/` directory separates event dispatch, thread routing, session
   lifecycle, configuration persistence, background requests, startup,
   history, and platform actions.
3. **Tests are large but visible as tests.** Codex frequently uses sibling
   `tests.rs`, `*_tests.rs`, and `tests/` modules. This does not reduce total
   code, but it makes implementation size and ownership honest.
4. **Mechanical extraction precedes behavior changes.** The project commonly
   moves code and tests without changing behavior, verifies the focused crate,
   and then lands functional work separately.
5. **New crates require a real seam.** Codex moves behavior out of Core when it
   is cohesive, has its own dependencies and tests, and benefits multiple
   callers. It does not create a crate merely to make a file smaller.
6. **Protocol and persistence concerns have dedicated owners.** Protocol
   types, app-server protocol versions, state runtime, and thread storage are
   not embedded in the central agent loop.

Representative public refactors make this process concrete:

- [Refactor TUI app module into submodules](https://github.com/openai/codex/commit/2af4f154797ab382d79a56c6eb616bcfe3cba1d4)
  kept `App` state and run-loop wiring at the top, moved topical behavior into
  private submodules, preserved runtime behavior, and initially retained the
  existing test surface.
- [Split Codex session modules](https://github.com/openai/codex/commit/91e8eebd03aa13c7b59b9c4aa96bab1cf69da04)
  separated session construction, MCP behavior, turn context, and review
  spawning.
- [Refactor ChatWidget in five phases](https://github.com/openai/codex/commit/3c3e18c2227888d941acc4dffa60abaf5a164ff3)
  followed earlier state, input, protocol, and settings extractions rather than
  attempting one rewrite.
- [Split command parsing and safety out of Core](https://github.com/openai/codex/commit/d8f9bb65e21589c84ba1d56f83a610676fcc78a3)
  moved a cohesive domain and 134 tests into a focused crate, retained
  compatibility through re-exports, and measured a 6–13% improvement in
  relevant clean compile and test-build timings.
- [Extract executable tool contracts](https://github.com/openai/codex/commit/95bfea847d9672de9e94f27db51ab52efd76b346)
  established a small reusable interface before migrating tool families.
- [Split the MCP connection manager](https://github.com/openai/codex/commit/2d85e6d3a616dc1fac258a5320c7a00a5e5bceb2)
  separated startup requirements and tool-catalog behavior while preserving
  the existing interface and behavior.

The lesson is not “use as many crates as Codex.” Codex is much larger and
serves a broader platform. The transferable lesson is to keep the central
runtime as an orchestrator, establish topical private modules first, promote a
module to a crate only when a real reuse or dependency seam exists, and measure
whether the change improves iteration cost.

### What the Claude Code mirror shows

The mirror contains approximately **512,685 physical TypeScript/TSX/JavaScript
LOC across 1,902 source files**:

| File size | Files | Share |
|---|---:|---:|
| Up to 500 LOC | 1,652 | 86.9% |
| 501–800 LOC | 124 | 6.5% |
| 801–2,000 LOC | 102 | 5.4% |
| 2,001–5,000 LOC | 19 | 1.0% |
| More than 5,000 LOC | 5 | 0.3% |

Its largest broad areas are `utils/` at roughly 180,000 LOC,
`components/` at 82,000 LOC, `services/` at 54,000 LOC, `tools/` at
51,000 LOC, and `bridge/` at 13,000 LOC. Only three files have test-like
names, which is likely a release-sourcemap limitation; no conclusion about
Anthropic's real test coverage is defensible.

Useful high-level patterns include:

- a central typed tool contract with one directory per substantial tool;
- vertical tool slices that can own execution, prompt text, result
  presentation, permission checks, and validation;
- shell execution separated from command parsing, read-only validation, path
  validation, permission policy, and security classification;
- model transport, MCP, plugins, remote bridge, session storage, and terminal
  UI represented as recognizable feature areas;
- many small UI modules rather than one terminal renderer containing every
  view;
- optional feature loading kept out of some startup paths.

The mirror also demonstrates patterns Forge should avoid:

- `utils/` has become a 180,000-LOC catch-all with weak domain ownership;
- entry and coordination files such as `main.tsx`, `REPL.tsx`, session
  storage, messages, hooks, and print handling remain 4,700–5,600 LOC;
- feature flags and dynamic imports can obscure dependency and startup
  behavior when not wrapped in a clear owning module;
- the snapshot provides no trustworthy architecture policy, review workflow,
  CI standard, or evolution history.

The strongest transferable Claude Code lesson is the vertical tool slice:
security-sensitive tools should own their validation and presentation details
without forcing the session loop to know them. Forge should combine that with
its load-bearing central permission broker from ADR-0008. Tool-local
validation is defense in depth; it must not become an alternative permission
path.

### Forge versus the reference structures

Inline tests explain a large fraction of Forge's four headline files, but the
implementation sections are still more concentrated than the closest Codex
owners:

| Forge hotspot | Snapshot total | Approx. implementation before main tests | Primary concentration |
|---|---:|---:|---|
| `forge-core/src/lib.rs` | 21,184 | 10,725 | Session state, turn loop, routing, compaction, recovery, built-in tool dispatch |
| `forge-store/src/lib.rs` | 12,281 | 8,299 | Connections, migrations, sessions, usage, routing, sync, handoff, push, workflows |
| `forge-tui/src/app.rs` | 10,643 | 7,209 | App state plus event, session, command, remote, and UI coordination |
| `forge-mesh/src/lib.rs` | 5,534 | 2,944 | Classification, ranking, health/quota policy, affinity, and route construction |

Exact totals are snapshot values and will move as the active branch changes.
The implementation/test split is the important signal. Extracting tests alone
would improve navigation but would not resolve implementation concentration.

Forge's accepted ADRs already point toward the correct shape:

- ADR-0002 requires a modular monolith with explicit crate seams.
- ADR-0004 requires the session core to own interactions while renderers remain
  adapters.
- ADR-0005 requires SQLite access to remain encapsulated in Store.
- ADR-0006 requires deterministic, explainable, pluggable Model Mesh routing.
- ADR-0008 requires all side effects to cross the central permission broker.

Refactoring should deepen those decisions, not replace them.

## Target architecture for Forge hotspots

The targets below name ownership slices, not predetermined Rust traits or
public interfaces. Exact interfaces should be designed only after history,
callers, invariants, and tests have been examined for each extraction.

### Core session runtime

**Recommendation strength: Strong**

Keep `Session` as the state-owning module and turn orchestrator, but stop using
the crate root as its implementation file. Candidate private modules are:

- session construction, resume, reset, and workspace transitions;
- turn orchestration and the provider/tool continuation loop;
- context assembly, fitting, compaction, recap, and cache-stable preambles;
- route admission, failover, model degradation, and recovery;
- lifecycle and terminal interaction emission;
- built-in session tool specifications and dispatch;
- plan, task, workflow, assay, and subagent coordination.

Some of these concepts already have modules (`completion`,
`context_pipeline`, `llm_router`, `permission`, `subagent`, and
`turn_contract`). The first question is whether remaining code belongs behind
those modules rather than whether Forge needs more crates.

The crate root should eventually become a private-module map plus deliberate
re-exports. Core-specific orchestration may remain larger than 800 LOC when
state locality genuinely requires it, but every exception should name the
invariant that would become harder to preserve if split.

### Store

**Recommendation strength: Strong**

Preserve one public Store seam and ADR-0005's SQLite encapsulation. Split the
implementation internally by owned data and transaction invariants:

- connection pool, busy retry, and transaction helpers;
- schema and migrations;
- sessions, messages, tool calls, checkpoints, and replay;
- usage, budgets, quota observations, and model reservations;
- routing outcomes and calibration;
- sync journal, portable records, conflicts, and file synchronization;
- handoff/import/export;
- push subscriptions and live activities;
- workflows, tasks, and other durable product records.

The current `schema.rs` and `memory.rs` are evidence that internal extraction
fits the existing decision. New crates are justified only if another crate
needs a storage-independent interface or a second adapter appears. Splitting
SQL into many pass-through files without concentrating transactions and
invariants would create shallow modules and should be rejected.

### TUI App

**Recommendation strength: Strong**

Follow the proven Codex TUI pattern while preserving Forge's interaction seam:

- keep `App` state and the main event/run loop together;
- move event dispatch, session lifecycle, command handling, remote/fleet
  coordination, overlays/dialogs, persistence requests, and background work
  into private `app/` modules;
- keep rendering in the renderer adapter rather than moving it back into
  session core;
- move topical tests with each extracted implementation after the mechanical
  split is stable.

This is a structural pattern, not a request to copy Codex names or code.

### Model Mesh

**Recommendation strength: Strong**

Keep one deterministic Router interface and one decision object used by both
execution and explanation. Give these policies explicit internal owners:

- task and continuation classification;
- tier policy and candidate construction;
- capability, context-window, and availability filtering;
- quota, health, price, and latency ranking;
- session affinity and cold-cache switching policy;
- failover and degradation;
- rationale construction and route inspection;
- catalog and calibration data.

`catalog.rs`, `capability.rs`, `pricing.rs`, and `explain.rs` already provide
natural homes. The goal is not more indirection: it is that changing affinity
should not require navigating classification implementation, and changing
classification should not risk silently diverging route explanation from
execution order.

### Tool ownership

**Recommendation strength: Worth exploring after the four hotspots**

Use a private vertical module per substantial or security-sensitive tool:

- definition and schema;
- execution;
- permission metadata and local validation;
- compact model-facing result;
- surface-specific presentation where applicable;
- focused tests.

The central permission broker remains the sole authorization seam. This
structure should reduce `forge-tools/src/lib.rs` concentration and keep
security rules close to the tool behavior they constrain.

## Architecture improvement workflow

Each extraction should follow the same evidence-first sequence.

### 1. Freeze and record the comparison base

Record:

- Forge commit and dirty-state patch;
- file and implementation-only LOC;
- relevant ADRs;
- dependency graph;
- focused test list and current results;
- compile/check timing;
- affected runtime benchmark cells.

Never compare a post-refactor tree with a moving or differently configured
baseline.

### 2. Reconstruct why the code is together

For the selected hotspot:

- inspect `git log --follow`, blame, linked PRs, and regression fixes;
- identify responsibilities that repeatedly change together;
- identify load-bearing ordering, transaction, concurrency, cache, permission,
  and recovery invariants;
- map existing tests to those responsibilities;
- classify code as load-bearing, previously load-bearing, accidental
  accretion, or genuinely misplaced.

A file being large is not sufficient evidence that two behaviors should be
separated.

### 3. Choose a deep module, not a line-count slice

A good candidate:

- owns a domain concept and its invariants;
- hides meaningful behavior behind a smaller interface;
- improves locality for callers and tests;
- has either multiple callers, multiple adapters, distinct dependencies, or a
  coherent independent test surface;
- passes the deletion test: removing it would spread its complexity back
  across callers.

Reject pass-through modules that only rename calls or create a second place to
look. One adapter is a hypothetical seam; do not add a public trait merely in
case variation appears later.

### 4. Characterize behavior before moving it

Add or identify tests for externally observable behavior, especially:

- session event ordering and persistence;
- context, compaction, and cache invariants;
- route selection, rationale, quota, affinity, and failover;
- transaction atomicity, migrations, and crash recovery;
- permission precedence and tool safety;
- TUI event routing and replay.

Prefer tests through the same interface callers use. Keep focused unit tests
for policy-heavy pure logic, but use integration and replay tests for agent
loop behavior.

### 5. Make a mechanical extraction

- move one responsibility and its tests;
- keep behavior and public paths stable where practical;
- keep the new module private by default;
- re-export only what real callers require;
- do not combine the move with a policy rewrite, optimization, or feature;
- preserve comments that explain invariants, not comments that narrate syntax.

If the extraction requires widespread new parameters or exposes internal
state, the proposed seam is probably wrong.

### 6. Verify locally and economically

Run, in order:

1. formatting;
2. the focused crate's lint and tests;
3. affected integration and endurance tests;
4. workspace dependency and public-interface checks;
5. broader workspace tests only when shared behavior changed;
6. relevant agent benchmarks only when runtime behavior or model-visible
   context could have changed.

Architecture-only changes should not consume provider quota unless a model
request, prompt, context, tool schema, ordering, or persistence behavior
actually changed.

### 7. Measure the result

Compare against the recorded base:

- implementation LOC distribution, separately from test LOC;
- crate-root size and public exports;
- dependency fan-in/fan-out and cycles;
- files and modules touched by representative changes;
- clean and incremental check/test timing;
- binary size where a new crate or dependency was added;
- focused and integration test results;
- benchmark quality, wall time, raw tokens, cache-adjusted tokens, and routing
  integrity when applicable.

Do not call an extraction successful merely because the original file became
smaller.

### 8. Land in reviewable phases

Prefer the Codex pattern:

1. characterization;
2. mechanical extraction;
3. caller migration or compatibility cleanup;
4. behavior improvement;
5. deletion of obsolete compatibility paths.

Each phase should be independently testable and revertible.

## Architecture evaluation scorecard

Forge should use absolute health criteria and reference comparisons. Beating
Codex on one proxy such as file count is not sufficient.

### Module size and depth

Long-term targets for implementation files, excluding tests and generated
code:

- at least **90% at or below 500 LOC**;
- at least **95% at or below 800 LOC**;
- no new file above 800 LOC without a documented exception;
- no implementation file above 5,000 LOC;
- drive existing files above 2,000 LOC toward a justified exception or a deep
  extraction;
- crate roots should normally stay below 500 implementation LOC and primarily
  declare modules and exports.

These targets are stricter than the observed Codex subset and Claude mirror.
They must be paired with depth and locality review so the metric does not
reward hundreds of shallow files.

### Dependency and interface health

- zero dependency cycles;
- private modules by default;
- no new public trait until a real second adapter or caller variation exists;
- no generic utility added to Core when a narrower owner exists;
- public exports reviewed as a deliberate compatibility surface;
- protocol and persistence types do not leak provider, terminal, or database
  implementation details;
- execution and route inspection consume the same routing decision.

### Change locality

Track from Git history:

- frequency with which each hotspot changes;
- unrelated domains changed in the same commit;
- files that repeatedly co-change with a hotspot;
- median implementation files needed for representative changes;
- defect-fix concentration after extractions.

Success means a Store change does not routinely edit the Core session or TUI,
an affinity change remains inside Mesh plus focused callers, and a new TUI
surface does not alter agent-loop policy.

### Verification quality

- test LOC remains visible and is never counted as implementation improvement;
- extracted tests move toward the owning module;
- policy logic has focused deterministic tests;
- agent behavior has public-interface integration and replay coverage;
- persistence changes include migration and transaction-failure coverage;
- security-sensitive tools include permission and adversarial validation
  coverage;
- Linux, macOS, and Windows behavior remains represented where relevant.

### Runtime and product performance

An architecture improvement must not regress:

- solve quality and completion integrity;
- wall time and startup latency;
- raw, cache-adjusted, and cache-zero-credit token measures;
- model-visible context stability;
- failover and recovery;
- persistence correctness;
- binary footprint without a documented reason.

Compile and test iteration time should improve or remain neutral. When a new
crate is proposed, record the same kind of before/after clean and incremental
timings Codex used for its command-safety extraction.

### AI navigability

Because Forge is maintained with coding agents, evaluate architecture with a
small paired maintenance suite:

- identify the owning module for a requested change;
- make a focused policy change;
- diagnose a seeded regression;
- add a provider or tool capability;
- update persistence and replay behavior.

For the same model, instructions, base commit, and tools, record:

- source files opened before the first correct edit;
- source tokens read;
- unrelated modules edited;
- first-pass focused-test result;
- final test result and wall time;
- whether the agent correctly named the load-bearing invariant.

The goal is not to optimize for one model's search habits. It is to verify that
domain ownership is discoverable and the context required for a safe change is
declining.

## Recommended execution order

1. **Install measurement and no-growth gates first.** Prevent new debt while
   preserving existing behavior.
2. **Model Mesh pilot.** Its approximately 2,944 implementation LOC contains
   clear policy families and strong deterministic tests, making it a useful
   extraction template. Perform this only after active routing work is merged
   and stable.
3. **Store internal split.** Preserve the Store interface and extract owned
   transaction domains without changing the database contract.
4. **TUI App split.** Keep App state/run-loop ownership and move topical
   behavior into private modules in reviewable phases.
5. **Core session program.** Characterize the highest-risk invariants first,
   then extract stable policy and lifecycle slices incrementally. Avoid a
   rewrite.
6. **Tool vertical slices and remaining repository hotspots.**

The ordering is risk-based, not importance-based. Core is the highest-priority
maintainability problem, but it is also the most dangerous place to learn the
extraction process.

## Definition of “better than Codex”

Forge can defensibly claim a better architectural standing only when all of
the following are true:

- it meets the module-size targets above without shallow-module inflation;
- none of the four current hotspot files exceeds 5,000 implementation LOC;
- Core and crate roots are thin, with deliberate public exports;
- dependency cycles remain zero and feature ownership is obvious;
- representative changes touch fewer unrelated modules and require less
  context;
- focused and integration coverage remains at least as strong;
- compile/test iteration time is neutral or better;
- retained agent benchmarks show no quality, speed, token, cache, safety, or
  resilience regression;
- the metrics are generated at a recorded commit and reviewed continuously,
  rather than asserted from a one-time document.

Until those conditions are measured, the accurate claim is that Forge is
smaller and feature-dense, but more internally concentrated than Codex in its
largest runtime files.

## Conclusions

1. **Forge is large but not anomalously large.** The fair owned-source figure
   is about 269,000 physical LOC, not 290,000.
2. **Much of the size is justified.** Mobile/web/desktop surfaces, tests,
   benchmark harnesses, and provider/platform integration account for
   substantial, understandable portions.
3. **Forge is feature-dense.** It offers a broad product surface while
   remaining in the same size band as Goose, Roo Code, and Continue.
4. **The benchmark efficiency is consistent with the architecture.** The
   critical agent path can be excellent without reproducing Codex's entire
   operational compatibility envelope.
5. **Codex's approximately fourfold source footprint should not be interpreted
   as fourfold intelligence.** Much of it buys hardening, portability,
   compatibility, and scale.
6. **Forge's main maintainability concern is internal concentration.** Core,
   Store, TUI, Mesh, and the CLI composition root require disciplined seams as
   the project grows.
7. **LOC should not be minimized for its own sake.** More useful metrics
   include owned production LOC, test/benchmark LOC, vendored/generated LOC,
   duplication, dependency direction, churn, compile cost, defect density,
   and the context needed to modify a subsystem safely.

## Recommended size guardrails

Track these categories separately over time:

- owned runtime and product source;
- unit and integration test source;
- benchmark and manual-E2E source;
- generated source;
- vendored source;
- documentation examples;
- mobile/web/desktop source versus Rust runtime source.

Also monitor:

- files crossing 2,000, 5,000, and 10,000 physical LOC;
- crate growth without corresponding responsibility changes;
- cyclic or high-fan-in module dependencies;
- duplicate implementations across CLI, remote, mobile, and desktop surfaces;
- frequently co-changing files and other high-churn hotspots;
- compile-time contribution per crate;
- benchmark regressions in quality, speed, and token efficiency relative to
  implementation growth.

The practical goal should not be a smaller repository at any cost. It should
be to preserve Forge's current feature density while ensuring each new
capability has a clear owning module, with proportional tests and without
duplicating behavior across surfaces.
