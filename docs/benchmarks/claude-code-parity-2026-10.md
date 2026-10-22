# Forge vs Claude Code: parity benchmark, baseline (October 2026)

**Baseline, recorded 2026-10-22.** Forge 2.21.0 against Claude Code 2.1.308 on the same models,
before any fix from this study is measured. Later runs append to the [After fixes](#after-fixes)
section of this report; the baseline numbers stay as they are. The samples are small (26, 11, 3 and 10
tasks, one or two repeats), so read every difference together with its confidence column.

## Summary

**Errata (added 2026-10-22, after publication):** the Forge native (FD) arm read the operator's personal
Forge configuration, and the suite c grader failed five correct answers. The numbers below stay as
recorded; see [Errata](#errata) for the corrections and what they change.

Nothing here shows Forge ahead of Claude Code with confidence, and nothing shows it far behind on
the main task suite. On quality, Forge and Claude Code are level within noise on the 26 tasks of
suites a+b+d for all three models: with DeepSeek V4.1 Flash Forge solved 18/26 and 17/26 against
17/26 and 17/26; with Haiku 5.5 through the Forge bridge 16/26 and 16/26 against 17/26 and 16/26;
with Sonnet 5.5 through the bridge 18/26 against 21/26 (three tasks only Claude Code solved, sign
test p=0.25, one repeat). On SWE-bench Verified Forge resolved 10 of 10 instances and Claude Code 9
of 10 (one instance, within noise), and both completed all three multi-turn sessions. Forge is
behind on one suite, the large-codebase suite c: 7/11 against 11/11. Four tasks are within noise on
their own (sign test p=0.125), but the four failures have visible causes: c08 ran as a read-only turn
(the suite keeps that prompt wording on purpose, as a canary for a Forge defect known beforehand),
c06 ended in a Forge panic after the work was done, and c02 and c04 returned wrong answers in runs
with four automatic compactions each, though c10 also had four and passed, so a link to compaction
is not established (F1, F2, F6). Seven of the eleven suite c tasks use a snapshot of Forge's own source, and the
suites were written by the Forge team.

Claude Code is faster and lighter in most measured comparisons. Forge's median agent time is
x1.06 to x1.49 Claude Code's on suites a+b+d, x1.96 on suite c, and x2.31 in total on SWE-bench.
With the same DeepSeek model, Forge's own loop writes x1.2 to x5.5 the output tokens, makes x1.6 to
x2.2 the tool calls, and reads less of its prompt from cache (84-85% against 91% on a+b+d, 67%
against 94% on c, 65% against 97% on SWE-bench), so the input it pays full price for is x1.3 to
x14 larger, even where its total input tokens are equal or lower (a+b+d, c and multi-turn). Counting only input not
read from cache, Claude Code is lower in every DeepSeek comparison, significantly so in suite c and SWE-bench. The Claude Code
bridge adds 4% to 36% input tokens and 18% to 51% model calls over plain Claude Code on the same model. Forge's own wins
are small and none is solid: b09 was solved only by the Forge DeepSeek arm (both repeats; each of the other eight arms failed one documentation check, F8), the SWE-bench result leans towards Forge (within noise),
and Forge's total input tokens are lower on the DeepSeek pair (significant in one of the two a+b+d repeats, p=0.04,
uncorrected for multiplicity), which is mostly cheaper cached input.

Nine fix PRs were opened from these findings and the static feature audit (six merged and three
open when this was written; the [ledger](#fix-ledger) has each state). None has been re-measured, so every
number in this report is the pre-fix baseline, and no claim that a fix works rests on it. One isolation
gap matters for reading the numbers: the Claude Code and bridge arms of suites a to d received the operator's
global instruction files through their working directory, and the Forge native arm probably did not (F9); if
anything that handicaps Claude Code in the DeepSeek comparison. TUI and UX, trivial-prompt latency,
the Opus subset and the Haiku and Sonnet runs on suite c, multi-turn and SWE-bench are not
measured yet ([Not yet measured](#not-yet-measured)).

<!-- BEGIN:headline -->
| Dimension | Arms and sample | Claude Code | Forge | Leads | Confidence (lowest level over repeats; sign test) |
|---|---|---:|---:|---|---|
| Quality, suites a+b+d | DeepSeek V4.1 Flash, FD vs CC, repeats 1, 2, 26 tasks | 17/26, 17/26 | 18/26, 17/26 | Within noise | Low (p=1, p=1) |
| Quality, suites a+b+d | Haiku 5.5, FB vs CC, repeats 1, 2, 26 tasks | 17/26, 16/26 | 16/26, 16/26 | Within noise | Low (p=1, p=1) |
| Quality, suites a+b+d | Sonnet 5.5, FB vs CC, 1 repeat, 26 tasks | 21/26 | 18/26 | Within noise, Claude Code ahead | Low, p=0.25 |
| Quality, suite c | DeepSeek V4.1 Flash, FD vs CC, 1 repeat, 11 tasks | 11/11 | 7/11 | Within noise, Claude Code ahead | Low, p=0.125 |
| Quality, multi-turn | DeepSeek V4.1 Flash, FD vs CC, 1 repeat, 3 sessions | 3/3 graded pass | 3/3 graded pass | Equal | n/a, identical |
| Quality, SWE-bench Verified | DeepSeek V4.1 Flash, FD vs CC, 1 attempt, 10 instances | 9/10 | 10/10 | Within noise, Forge ahead | Low, p=1 |
| Speed, suites a+b+d, median agent wall | DeepSeek V4.1 Flash, FD vs CC, repeats 1, 2, 26 tasks | 57.1 s, 61.9 s | 66.1 s, 92.2 s | Claude Code (both repeats) | Moderate (p=0.029, p=0.00249) |
| Speed, suites a+b+d, median agent wall | Haiku 5.5, FB vs CC, repeats 1, 2, 26 tasks | 54.1 s, 52.1 s | 61.8 s, 55.1 s | Claude Code in repeat 1, within noise in repeat 2 | Low (p=0.029, p=0.0755) |
| Speed, suites a+b+d, median agent wall | Sonnet 5.5, FB vs CC, 1 repeat, 26 tasks | 25.4 s | 28.0 s | Within noise, Claude Code ahead | Low, p=0.169 |
| Speed, suite c, median agent wall | DeepSeek V4.1 Flash, FD vs CC, 1 repeat, 11 tasks | 81.9 s | 160.5 s | Claude Code | Moderate, p=0.0215 |
| Speed, multi-turn, total wall | DeepSeek V4.1 Flash, FD vs CC, 3 sessions | 783 s | 816 s | Within noise, Claude Code ahead | Low, p=0.25 |
| Speed, SWE-bench Verified, total wall | DeepSeek V4.1 Flash, FD vs CC, 10 instances | 3,012 s | 6,957 s | Claude Code | High, p=0.00195 |
| Efficiency, suites a+b+d, input tokens (cache reads included) | DeepSeek V4.1 Flash, FD vs CC, repeats 1, 2, 26 tasks | 6.73M, 8.42M | 6.52M, 6.81M | Forge in repeat 1, within noise in repeat 2 | Low (p=0.0433, p=1) |
| Efficiency, suites a+b+d, input tokens (cache reads included) | Haiku 5.5, FB vs CC, repeats 1, 2, 26 tasks | 7.83M, 6.51M | 8.15M, 7.54M | Within noise, Claude Code ahead in both repeats | Low (p=0.0755, p=0.169) |
| Efficiency, suites a+b+d, input tokens (cache reads included) | Sonnet 5.5, FB vs CC, 1 repeat, 26 tasks | 2.83M | 3.85M | Claude Code | High, p<0.001 |
| Efficiency, suite c, input tokens (cache reads included) | DeepSeek V4.1 Flash, FD vs CC, 1 repeat, 10 tasks | 5.45M | 4.48M | Within noise, Forge ahead | Low, p=1 |
| Efficiency, multi-turn, input tokens (cache reads included) | DeepSeek V4.1 Flash, FD vs CC, 3 sessions | 5.01M | 3.94M | Within noise, Forge ahead | Low, p=0.25 |
| Efficiency, SWE-bench Verified, input tokens (cache reads included) | DeepSeek V4.1 Flash, FD vs CC, 10 instances | 12.09M | 14.00M | Within noise, Claude Code ahead | Low, p=0.109 |
| Efficiency, suites a+b+d, input tokens not read from cache | DeepSeek V4.1 Flash, FD vs CC, repeats 1, 2, 26 tasks | 608k, 782k | 1.05M, 1.05M | Within noise, Claude Code ahead in both repeats | Low (p=0.23, p=0.69) |
| Efficiency, suites a+b+d, input tokens not read from cache | Haiku 5.5, FB vs CC, repeats 1, 2, 26 tasks | 674k, 678k | 698k, 664k | Within noise, direction differs by repeat | Low (p=0.0755, p=0.845) |
| Efficiency, suites a+b+d, input tokens not read from cache | Sonnet 5.5, FB vs CC, 1 repeat, 26 tasks | 398k | 434k | Claude Code | High, p=0.00936 |
| Efficiency, suite c, input tokens not read from cache | DeepSeek V4.1 Flash, FD vs CC, 1 repeat, 10 tasks | 307k | 1.48M | Claude Code | High, p=0.00195 |
| Efficiency, multi-turn, input tokens not read from cache | DeepSeek V4.1 Flash, FD vs CC, 3 sessions | 190k | 603k | Within noise, Claude Code ahead | Low, p=0.25 |
| Efficiency, SWE-bench Verified, input tokens not read from cache | DeepSeek V4.1 Flash, FD vs CC, 10 instances | 346k | 4.84M | Claude Code | High, p=0.00195 |
| Efficiency, suites a+b+d, output tokens | DeepSeek V4.1 Flash, FD vs CC, repeats 1, 2, 26 tasks | 224k, 292k | 309k, 342k | Claude Code (both repeats) | High (p<0.001, p<0.001) |
| Efficiency, suites a+b+d, output tokens | Haiku 5.5, FB vs CC, repeats 1, 2, 26 tasks | 250k, 261k | 266k, 247k | Claude Code in repeat 1, within noise in repeat 2 | Low (p=0.00936, p=0.327) |
| Efficiency, suites a+b+d, output tokens | Sonnet 5.5, FB vs CC, 1 repeat, 26 tasks | 74k | 77k | Within noise, Claude Code ahead | Low, p=0.327 |
| Efficiency, suite c, output tokens | DeepSeek V4.1 Flash, FD vs CC, 1 repeat, 10 tasks | 67k | 248k | Claude Code | High, p=0.00195 |
| Efficiency, multi-turn, output tokens | DeepSeek V4.1 Flash, FD vs CC, 3 sessions | 88k | 91k | Within noise, Claude Code ahead | Low, p=0.25 |
| Efficiency, SWE-bench Verified, output tokens | DeepSeek V4.1 Flash, FD vs CC, 10 instances | 153k | 842k | Claude Code | High, p=0.00195 |
<!-- END:headline -->

"Leads" names a side only when the exact sign test on paired tasks reaches p<0.05; otherwise it
reads "Within noise" and names the side the point estimate favours. With more than one repeat the
columns list the repeats in order; a label that differs between repeats is spelled out by repeat. Confidence is the
lowest level over the repeats: High p<0.01, Moderate p<0.05, Low otherwise. About two dozen
comparisons are tested here with no correction for multiplicity, so a single Moderate result is weak
evidence. Token rows use only task pairs where both sides reported complete metrics. The first input-token rows
count cache reads in full; the "not read from cache" rows are the closer proxy for what a request costs.

## Method

### Arms

Four arm types, each run with the same model:

- **CC** is Claude Code's own loop and native tools (`claude -p`, stream-json output).
- **FB** is the Forge claude-cli bridge: Claude Code's loop with Forge's tools (through MCP), preamble
  and permission layer. It exists to isolate what Forge adds around an unchanged inner loop.
- **FD** is Forge's own loop on `inferhub::cb/deepseek-v4.1-flash`.
- **CC-deepseek** is Claude Code pointed at InferHub's Anthropic-compatible `/v1/messages` endpoint
  with model `cb/deepseek-v4.1-flash`. Together with FD it is the one true same-model harness A/B,
  with the caveat that the two harnesses reach the model through different API surfaces.

<!-- BEGIN:arms -->
| Arm | Harness | Requested model | Model id seen | Repeats (suites a+b+d) |
|---|---|---|---|---|
| `CC-deepseek` | Claude Code native loop | `cb/deepseek-v4.1-flash` | `cb/deepseek-v4.1-flash` | 1, 2 |
| `FD-deepseek` | Forge native loop | `inferhub::cb/deepseek-v4.1-flash` | `inferhub::cb/deepseek-v4.1-flash` | 1, 2 |
| `CC-haiku` | Claude Code native loop | `haiku` | `claude-haiku-5-5` | 1, 2 |
| `FB-haiku` | Forge claude-cli bridge (Claude Code loop + Forge MCP tools and preamble) | `claude-cli::haiku` | `claude-haiku-5-5` | 1, 2 |
| `CC-sonnet` | Claude Code native loop | `sonnet` | `claude-sonnet-5-5` | 1 |
| `FB-sonnet` | Forge claude-cli bridge (Claude Code loop + Forge MCP tools and preamble) | `claude-cli::sonnet` | `claude-sonnet-5-5` | 1 |
<!-- END:arms -->

### Isolation

- Claude Code arms in suites a, b, c and d run with `--setting-sources project --strict-mcp-config
  --no-session-persistence`; the init event reports `mcp_servers: []`. The SWE-bench Claude Code arm
  runs with `--safe-mode`.
- Forge arms get a neutral `.forge/AGENTS.md` in every workspace so that Forge does not inject the
  operator's global `~/.claude/CLAUDE.md` as project instructions.
- Known gap (F9): the workspaces of suites a, b, c and d lie under `$HOME`, and Claude Code loads the `CLAUDE.md`
  files of ancestor directories as project instructions whatever `--setting-sources` says. The bridged Claude Code
  process did (78 of 78 transcripts), a local stub-API check shows that the clean Claude Code flag set does too, so
  the Claude Code and bridge arms received the operator's global `~/.claude/CLAUDE.md` and `/home/floris/CLAUDE.md`
  (about 8.5k characters per request). The Forge native arm probably did not (not captured). Multi-turn and SWE-bench
  workspaces are outside `$HOME`.
- No effort override was passed in suites a to d, so each harness used its default reasoning
  effort. In SWE-bench neither DeepSeek arm was given an effort setting either (`run.sh` sets `SWE_CLAUDE_EFFORT` empty for
  CC-deepseek and the kit's `claude` shim then omits `--effort`; the `claude_flags` string in `run-meta.json` is a constant that
  says "high" for every Claude Code arm). Every arm used the same 1,200 s timeout per phase (setup, agent turn, grading) in suites
  a to d and 1,500 s per instance in SWE-bench.

### Environment

<!-- BEGIN:env -->
| Item | Value |
|---|---|
| Forge | 2.21.0, commit `8b18cad8` (built by the kit's forge-dev-update from this commit: its log reads `building 8b18cad8 fix(push): keep generic iOS alerts until replaced; warn on failed APNs sends (#1601)` and was written at 2026-10-21 13:00:43 +0200, within a second of the binary's modification time, and the pinned copy has the same SHA-256 as the binary that build produced. The binary itself embeds no commit hash) |
| Forge binary | `ccbench/bin/forge (pinned copy of the release build used for every Forge arm)`, SHA-256 `9fd1225f2995fdadde8af7e05dc9361e6dbb6e15fa179747bd12451918f07b3b` |
| Claude Code | 2.1.308 (Claude Code) |
| Models actually used | `cb/deepseek-v4.1-flash`; `claude-haiku-5-5`; `claude-sonnet-5-5`; `inferhub::cb/deepseek-v4.1-flash (Forge FD arm, provider id)` |
| Provider for DeepSeek | InferHub (https://api.inferhub.dev): Forge arm through its inferhub provider, Claude Code arm through InferHub's Anthropic-compatible /v1/messages endpoint |
| SWE-bench evaluator | swebench 4.1.0 (official Docker harness), Docker 29.8.0 |
| Python | 3.14.7 (host); 3.14.7 (SWE-bench venv) |
| CPU | 13th Gen Intel(R) Core(TM) i9-13950HX, 32 threads |
| RAM | 31 GiB |
| OS | Arch Linux, kernel Linux 7.2.6-arch2-1 |
| Run window (UTC) | 2026-10-22, 12:21Z to 14:27Z |
| Concurrency | The eight Haiku and DeepSeek arms of suites a+b+d were started within a millisecond of each other (12:21Z) and ran side by side; the Haiku arms ended by 12:56Z and the DeepSeek arms between 13:05Z and 13:15Z. The Sonnet pair started at 13:11Z, while the DeepSeek arms were still finishing, and ended at 13:28Z and 13:29Z. Suite c (one chain per harness), the multi-turn sessions (both harnesses) and SWE-bench (both harnesses, two instances at a time each) all started together at 13:25Z, so they overlapped the last minutes of the Sonnet pair. The Claude Code chain of suite c ended at 13:43Z and the Forge chain at 13:56Z; multi-turn ended at 13:39Z (CC) and 13:40Z (FD), SWE-bench at 13:51Z (CC) and 14:27Z (FD). The machine also hosted other agents' work, and load was not recorded, so absolute wall times carry noise. |
<!-- END:env -->

### Suites

Four task suites, one multi-turn set and one public benchmark. The hidden graders are not
published. Instead each task directory (instruction, setup, hidden grader, reference solution,
negative controls) has a SHA-256 commitment, so the graders can be disclosed later and checked
against these hashes. The commitment is computed as:

```
cd <suite root>/<a|b|c|d> && tar --sort=name --mtime=@0 --owner=0 --group=0 --numeric-owner -cf - <task dir> | sha256sum
```

The table shows the first 12 hex digits; [tasks.json](artifacts/claude-code-parity-2026-10/baseline/tasks.json)
holds the full hashes, the task directory names, and hashes for the shared inputs (`_tools`,
`_src`, and the source snapshots used by suite c).

- **a (9 tasks)**: small feature, bugfix, refactor and reading tasks in Python, JavaScript and TypeScript.
- **b (9 tasks)**: longer or trickier work in Rust, Go and shell: multi-step requests, concurrency, merge repair, history search.
- **c (11 tasks)**: large offline codebases (a ~390k-line Rust snapshot of Forge, Twisted, dulwich, npm); answers need search, call-chain following or context management. Seven of the tasks use a
  snapshot of Forge's own source, and c08 keeps a phrase that triggers a known Forge defect on purpose (F1).
- **d (8 tasks)**: harness robustness: interactive prompts, huge output, flaky tests, long builds, background processes, read-only files, binary neighbours, portability.

<!-- BEGIN:tasks -->
| Task | Suite | Language | Type | What is asked | Commitment |
|---|---|---|---|---|---|
| `a01-fix-back-to-back-conflicts` | a | Python | bugfix | A room scheduler reports back-to-back bookings as conflicts; fix it to match the README without editing existing tests. | `6d37af256130` |
| `a02-sliding-window-limiter` | a | Python | feature | Implement a sliding-window rate limiter from an approved spec, following the layout of the existing TokenBucket. | `27bc87268066` |
| `a03-refactor-order-pricing` | a | JavaScript | refactor | Split a 170-line order-total function into discount, tax and shipping modules behind a short orchestrator. | `bb04dcb738d4` |
| `a04-debug-nightly-export-crash` | a | Python | debugging | Find and fix the root cause of a nightly export crash on a finance CSV; no skipped rows, input untouched. | `285e403ad302` |
| `a05-mailer-options-object` | a | TypeScript | refactor | Replace sendMail's six positional parameters with one options object and update every call site. | `cd387626fafe` |
| `a06-answer-retry-policy` | a | JavaScript | code reading (no edits) | Read a sync agent and answer questions about its production retry policy in ANSWER.md; change no code. | `4c4982e1cfe4` |
| `a07-speed-up-log-report` | a | Python | performance | Make an access-log report fast on ~100k lines with byte-identical output, standard library only. | `4e337e2800fa` |
| `a08-signup-validation-constraints` | a | JavaScript | feature | Add input validation to a signup handler (name, email, password rules; case-insensitive duplicate check). | `f1a91a8616d9` |
| `a09-layered-config-cli` | a | Python | feature | Layer a CLI's settings (flags, environment, TOML file, defaults) and add a command to show the resolved values. | `986c9bc215c3` |
| `b01-stockctl-longhorizon` | b | Rust | multi-step feature | Five ordered requests to a Rust CLI: --min-qty filter, per-category summary, export command, format error, README and tests. | `4c0c5f5f0b9e` |
| `b02-ttlcache-expiry-trap` | b | Go | bugfix | Make Cache.Get never return an expired entry, keeping existing tests green. | `365e6daaf149` |
| `b03-rename-account-wire-frozen` | b | Go | cross-cutting refactor | Rename the User concept to Account across a billing service while it still builds and tests pass. | `67bbce1f5d37` |
| `b04-batch-run-concurrency` | b | Go | bugfix (concurrency) | Make batch.Run preserve order, pass the race detector and stop hanging after a failed call, without changing its signature. | `5a7db22c1dc8` |
| `b05-minimd-broken-merge` | b | Rust | merge repair | Repair a broken merge so the project builds and tests pass offline, with blockquotes behaving as the README says. | `08cb81edb543` |
| `b06-semver-regression-archaeology` | b | Go | git archaeology + bugfix | Find the commit that broke release-candidate ordering, fix it without reverting newer work, add a test and a changelog line. | `d65d7e474d04` |
| `b07-ledger-rollup-awk` | b | Bash/awk | scripting | Write a plain bash and POSIX-tools script that rolls bank-export CSVs up into monthly category totals. | `85c1a4b8f383` |
| `b08-scaffold-service-noninteractive` | b | Go + shell | tooling | Create a new service through the repo's scaffolding script (next free port, owners) and make the registry check pass. | `5f9e8d6e56f8` |
| `b09-fsweep-docs-drift` | b | Rust | documentation sync | Bring README, usage docs and --help text in line with what the tool actually does, without changing behaviour. | `4f3f01cd5826` |
| `c01-pinned-429-wait-budget` | c | Rust (Forge snapshot) | code reading + computation | Answer questions about the retry and wait budget for rate-limited pinned requests across three crates (ANSWER.md). | `5e2e61f9dd04` |
| `c02-compaction-trigger-budget` | c | Rust (Forge snapshot) | code reading + computation | Trace the compaction trigger from its config default to the token budgets and compute them for a given window (ANSWER.md). | `5f7367e4a5ff` |
| `c03-provider-implementors` | c | Rust (Forge snapshot) | exhaustive search + counts | Enumerate the production implementors of the Provider trait and answer count questions (ANSWER.md). | `bf0a8078b14c` |
| `c04-claude-bridge-spawn-path` | c | Rust (Forge snapshot) | data-flow tracing | Trace how a model id becomes the arguments and environment of the spawned claude process (ANSWER.md). | `b170a97aeaf9` |
| `c05-effort-ceiling-value-rung` | c | Rust (Forge snapshot) | bugfix (silent tests) | Find and fix an off-by-one in an effort-ceiling selection function that the existing tests do not catch. | `04eeca5f5949` |
| `c06-rename-config-provider-of` | c | Rust (Forge snapshot) | cross-crate rename | Rename one config function across crates without touching same-named functions elsewhere; the crate must still compile. | `03d47a76b590` |
| `c07-http-conditional-leap-march` | c | Python (Twisted) | bugfix (deep call chain) | Fix wrong HTTP conditional-request behaviour reported only as a symptom. | `9f5c28693997` |
| `c08-gitignore-negation-parent-dir` | c | Python (dulwich) | bugfix (deep call chain) | Fix a .gitignore negation behaviour reported only as a symptom; the prompt says not to edit existing files under dulwich/tests. | `365133ff474d` |
| `c09-npm-config-hash-value` | c | JavaScript (npm) | bugfix (deep call chain) | Fix an npm config round trip that breaks on a value containing a hash character. | `eec473797289` |
| `c10-dulwich-commit-to-loose-object` | c | Python (dulwich) | data-flow tracing | Trace the path from creating a commit to the loose object file on disk (ANSWER.md). | `012895e532c7` |
| `c11-tool-output-truncation` | c | Rust (Forge snapshot) | code reading + computation | Compare two tool-output truncation functions and compute how each counts lines (ANSWER.md). | `bf1ff0f4a3f1` |
| `d01-interactive-test-runner` | d | Python + shell | robustness (interactive prompt) | Fix a duration parser so the project's own test runner passes; the runner prompts interactively unless told not to. | `6b6cf640ce36` |
| `d02-huge-output-needle` | d | shell + Python | robustness (huge output) | Find one failing batch id in a tens-of-megabytes trace and write it to answer.txt. | `4e772aa08c9d` |
| `d03-flaky-loading-cache` | d | Python | robustness (flaky test, race) | A test fails about one run in three; fix the real race in the library, not the test. | `916478c57e26` |
| `d04-long-build-await` | d | shell | robustness (long-running command) | Cut a release: set version and channel, run the multi-minute build and leave a complete dist/. | `f3d7e0474d10` |
| `d05-background-server-roundtrip` | d | Python | robustness (process lifecycle) | Start a local service, fetch a token from its API, save it, and shut the service down cleanly. | `de86baa0b675` |
| `d06-readonly-config-files` | d | shell | robustness (file permissions) | Raise a limit in a config file that is mode 0444 and make the checker print OK. | `a781b87ac5b9` |
| `d07-binary-neighbor-schema` | d | text + binary data | robustness (large binary neighbour) | Update a stale schema document to match a large binary data file's header, touching nothing else. | `a397c32ab95e` |
| `d08-bsd-sed-make-target` | d | Make + YAML | robustness (portability) | Set every service to debug logging through a make target that was written for BSD sed and date. | `dcba03563e93` |
<!-- END:tasks -->

Multi-turn sessions (7 scripted prompts each, one persistent conversation, hidden grader plus a
context-retention probe) and the SWE-bench Verified instances:

<!-- BEGIN:tasks-other -->
| Id | Dimension | Language | Notes | Commitment |
|---|---|---|---|---|
| `S1-tasklog` | multi-turn | Python | Seven prompts building a task tracker CLI feature by feature under a standing constraint set in prompt 2. | `5f956fe46101` |
| `S2-fxbatch` | multi-turn | Python | Seven prompts on a batch currency converter; prompt 5 reverses an earlier design decision. | `df4970c2d8d5` |
| `S3-invoicer` | multi-turn | Python | Seven prompts debugging three reported bugs, with later prompts referring back to earlier findings. | `4e77e7936cb9` |
| `django__django-13821` | SWE-bench Verified | Python | django/django 3.2, difficulty "<15 min fix" | public |
| `sympy__sympy-13877` | SWE-bench Verified | Python | sympy/sympy 1.1, difficulty "15 min - 1 hour" | public |
| `astropy__astropy-13033` | SWE-bench Verified | Python | astropy/astropy 4.3, difficulty "15 min - 1 hour" | public |
| `pytest-dev__pytest-8399` | SWE-bench Verified | Python | pytest-dev/pytest 6.3, difficulty "15 min - 1 hour" | public |
| `psf__requests-1921` | SWE-bench Verified | Python | psf/requests 2.3, difficulty "<15 min fix" | public |
| `sphinx-doc__sphinx-8638` | SWE-bench Verified | Python | sphinx-doc/sphinx 4.0, difficulty "15 min - 1 hour" | public |
| `scikit-learn__scikit-learn-25973` | SWE-bench Verified | Python | scikit-learn/scikit-learn 1.3, difficulty "<15 min fix" | public |
| `matplotlib__matplotlib-22865` | SWE-bench Verified | Python | matplotlib/matplotlib 3.5, difficulty "15 min - 1 hour" | public |
| `pydata__xarray-6461` | SWE-bench Verified | Python | pydata/xarray 2022.03, difficulty "<15 min fix" | public |
| `django__django-13343` | SWE-bench Verified | Python | django/django 3.2, difficulty "15 min - 1 hour" | public |
<!-- END:tasks-other -->

### Attempts and repeats

| Dimension | Arms | Attempts |
|---|---|---|
| Suites a+b+d | CC and FD with DeepSeek; CC and FB with Haiku | 2 repeats of the 26 tasks |
| Suites a+b+d | CC and FB with Sonnet | 1 repeat |
| Suite c | CC and FD with DeepSeek | 1 attempt per task |
| Multi-turn | CC and FD with DeepSeek | 1 repeat of 3 sessions |
| SWE-bench Verified | CC and FD with DeepSeek | 1 attempt per instance |

A repeat ("seed" in the data files) is an independent run of the same tasks. Repeats of one arm
differ by a few tasks on their own (see [Limitations](#limitations)).
Suites a+b+d are scored by `forge bench tasks`: each attempt gets a fresh workspace, the agent
turn runs under a timeout, and the hidden `test.sh` decides solved or not. An attempt that ends in a
timeout is scored by whatever state the workspace is in.

### Metrics

- **Solved**: the hidden grader exits 0. For the SWE-bench instances, solved means resolved by the
  official `swebench==4.1.0` evaluator; a patch or a model claim alone never counts.
- **Agent wall**: seconds from the end of `setup.sh` to the start of `test.sh`, stamped by shims around
  the agent turn. It includes harness startup and excludes grading.
- **Input tokens**: uncached input plus cache reads plus cache writes, as the harness reports them.
  **Input not read from cache** is input minus cache reads. **Cache hit** is cache reads over input.
  **Output tokens** are as the harness reports them.
- **Tool calls**: counted from each harness's own record (Claude Code's stream or transcript, Forge's store). The two use
  different tool names and granularity (`Read`/`Edit` against `read_file`/`edit_file`), so only the ratios are
  indicative. **Tool errors** follow the same sources.
- **Pass rate interval**: Wilson 95%. **Sign test**: exact two-sided binomial test on the tasks only one
  side solved (quality) or on the tasks where one side was strictly faster or cheaper (speed, tokens).
- Recorded costs are kept in the arm totals of `suites-abd.json` but not compared: Claude Code's cost for a
  non-Anthropic model is not a price, and the bridge arms record zero cost.

### Integrity

- Hidden graders and hidden tests run outside the agent's workspace after the turn. Several graders
  also fail a run that touches protected files (for example `a01` lists `tests/**`, `README.md`,
  `pyproject.toml` and two source files in `protected.txt`; `d01` checks that `tests/` is untouched).
- Each suite ships a `validate.sh` that needs no model: setup, grader fails, reference solution, grader
  passes twice, every negative control fails again. The suite c README records that all eleven tasks
  pass it; the validation output is not part of this artifact set.
- SWE-bench: single-commit synthetic base per instance, no remotes, an integrity preamble forbidding
  network and history access, and the patch re-captured against the base. The integrity file
  `predictions.integrity.jsonl` has 0 `fail` verdicts for both arms. It has 2 `warn` verdicts for Forge and 9 for Claude
  Code, all `test_path_touched` (the patch modifies a test file). Forge's traces were audited
  for history and network access (10 of 10 instances); `trace_checked` is false for all ten Claude Code instances,
  so their integrity rests on the patch-level checks and the isolation above.
- Official SWE-bench reports and evaluator logs are in the artifacts directory, with the predictions
  and metrics they were produced from.

## Results

### Suites a+b+d: DeepSeek, Haiku and Sonnet

Per arm, quality and speed:

<!-- BEGIN:abd-arm-qs -->
| Arm | Solved | Pass rate | 95% CI (Wilson) | Timeouts | Agent wall median (s) | Agent wall total (s) |
|---|---:|---:|---:|---:|---:|---:|
| `CC-deepseek/seed1` | 17/26 | 65% | 46-81% | 1 | 57.1 | 3,186.2 |
| `CC-deepseek/seed2` | 17/26 | 65% | 46-81% | 0 | 61.9 | 2,617.1 |
| `FD-deepseek/seed1` | 18/26 | 69% | 50-83% | 0 | 66.1 | 2,758.5 |
| `FD-deepseek/seed2` | 17/26 | 65% | 46-81% | 0 | 92.2 | 3,033.2 |
| `CC-haiku/seed1` | 17/26 | 65% | 46-81% | 0 | 54.1 | 2,063.7 |
| `CC-haiku/seed2` | 16/26 | 62% | 43-78% | 0 | 52.1 | 2,026.8 |
| `FB-haiku/seed1` | 16/26 | 62% | 43-78% | 0 | 61.8 | 2,021.5 |
| `FB-haiku/seed2` | 16/26 | 62% | 43-78% | 0 | 55.1 | 1,958.2 |
| `CC-sonnet/seed1` | 21/26 | 81% | 62-91% | 0 | 25.4 | 966.9 |
| `FB-sonnet/seed1` | 18/26 | 69% | 50-83% | 0 | 28.0 | 1,069.4 |
<!-- END:abd-arm-qs -->

Per arm, efficiency:

<!-- BEGIN:abd-arm-eff -->
| Arm | Input tokens | Input not read from cache | Output tokens | Cache hit | Tool calls | Tool errors | Model calls | Rows with incomplete metrics |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| `CC-deepseek/seed1` | 6,734,386 | 608,178 | 223,861 | 91% | 498 | 12 | 315 | 1 |
| `CC-deepseek/seed2` | 8,744,721 | 811,537 | 299,308 | 91% | 458 | 11 | 306 | 0 |
| `FD-deepseek/seed1` | 6,591,020 | 1,065,260 | 316,385 | 84% | 672 | 18 | 371 | 0 |
| `FD-deepseek/seed2` | 6,808,528 | 1,048,784 | 342,294 | 85% | 779 | 26 | 377 | 1 |
| `CC-haiku/seed1` | 7,834,466 | 674,422 | 249,615 | 91% | 292 | 5 | 234 | 0 |
| `CC-haiku/seed2` | 6,508,629 | 678,141 | 260,643 | 90% | 290 | 5 | 203 | 0 |
| `FB-haiku/seed1` | 8,153,342 | 698,389 | 265,532 | 91% | 328 | 10 | 276 | 0 |
| `FB-haiku/seed2` | 7,539,085 | 664,132 | 246,791 | 91% | 296 | 9 | 260 | 0 |
| `CC-sonnet/seed1` | 2,828,346 | 398,283 | 74,457 | 86% | 107 | 5 | 117 | 0 |
| `FB-sonnet/seed1` | 3,850,096 | 434,398 | 76,570 | 89% | 147 | 2 | 177 | 0 |
<!-- END:abd-arm-eff -->

Rows with incomplete metrics (their token fields are zero or missing and are left out of the paired
token comparisons):

<!-- BEGIN:abd-notes -->
- `CC-deepseek/seed1` `a07-speed-up-log-report`: timed out at the 1200 s limit; Claude Code reports tokens only when a run ends, so token fields are 0 (tool and model-call counts come from the stream)
- `FD-deepseek/seed2` `a05-mailer-options-object`: no usage row: the Forge session failed to start with 'database is locked'; the concurrent Forge arms shared one store; counted as unsolved; its 47 s agent wall (a failed start) is still part of the speed comparison
<!-- END:abd-notes -->

Forge against Claude Code on the same model and repeat, quality:

<!-- BEGIN:abd-quality -->
| Model | Pair | Repeat | Claude Code solved | Forge solved | Forge minus CC | Solved only by CC / only by Forge | Leads | Confidence (sign test on the discordant tasks) |
|---|---|---:|---:|---:|---:|---:|---|---|
| DeepSeek V4.1 Flash | FD vs CC | 1 | 17/26 | 18/26 | +4 pp | 4 / 5 | Within noise, Forge ahead | Low, p=1 |
| DeepSeek V4.1 Flash | FD vs CC | 2 | 17/26 | 17/26 | +0 pp | 3 / 3 | Equal | n/a, identical |
| Haiku 5.5 | FB vs CC | 1 | 17/26 | 16/26 | -4 pp | 2 / 1 | Within noise, Claude Code ahead | Low, p=1 |
| Haiku 5.5 | FB vs CC | 2 | 16/26 | 16/26 | +0 pp | 1 / 1 | Equal | n/a, identical |
| Sonnet 5.5 | FB vs CC | 1 | 21/26 | 18/26 | -12 pp | 3 / 0 | Within noise, Claude Code ahead | Low, p=0.25 |
<!-- END:abd-quality -->

Speed:

<!-- BEGIN:abd-speed -->
| Model | Pair | Repeat | CC median (s) | Forge median (s) | Ratio of medians | Median paired ratio | Total wall CC / Forge (s) | Tasks where Forge was faster | Leads | Confidence (sign test over tasks) |
|---|---|---:|---:|---:|---:|---:|---:|---:|---|---|
| DeepSeek V4.1 Flash | FD vs CC | 1 | 57.1 | 66.1 | x1.16 | x1.21 | 3,186.2 / 2,758.5 | 7 of 26 | Claude Code | Moderate, p=0.029 |
| DeepSeek V4.1 Flash | FD vs CC | 2 | 61.9 | 92.2 | x1.49 | x1.38 | 2,617.1 / 3,033.2 | 5 of 26 | Claude Code | High, p=0.00249 |
| Haiku 5.5 | FB vs CC | 1 | 54.1 | 61.8 | x1.14 | x1.19 | 2,063.7 / 2,021.5 | 7 of 26 | Claude Code | Moderate, p=0.029 |
| Haiku 5.5 | FB vs CC | 2 | 52.1 | 55.1 | x1.06 | x1.09 | 2,026.8 / 1,958.2 | 8 of 26 | Within noise, Claude Code ahead | Low, p=0.0755 |
| Sonnet 5.5 | FB vs CC | 1 | 25.4 | 28.0 | x1.10 | x1.14 | 966.9 / 1,069.4 | 9 of 26 | Within noise, Claude Code ahead | Low, p=0.169 |
<!-- END:abd-speed -->

Efficiency (ratios are Forge over Claude Code):

<!-- BEGIN:abd-eff -->
| Model | Pair | Repeat | Paired tasks with complete metrics | Input tokens (Forge / CC) | Output tokens (Forge / CC) | Tool calls (Forge / CC) | Model calls (Forge / CC) | Cache hit CC / Forge | Input not read from cache (Forge / CC, whole arm) |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|
| DeepSeek V4.1 Flash | FD vs CC | 1 | 25 | x0.97 | x1.38 | x1.58 | x1.38 | 91% / 84% | x1.75 |
| DeepSeek V4.1 Flash | FD vs CC | 2 | 25 | x0.81 | x1.17 | x1.83 | x1.29 | 91% / 85% | x1.29 |
| Haiku 5.5 | FB vs CC | 1 | 26 | x1.04 | x1.06 | x1.12 | x1.18 | 91% / 91% | x1.04 |
| Haiku 5.5 | FB vs CC | 2 | 26 | x1.16 | x0.95 | x1.02 | x1.28 | 90% / 91% | x0.98 |
| Sonnet 5.5 | FB vs CC | 1 | 26 | x1.36 | x1.03 | x1.37 | x1.51 | 86% / 89% | x1.09 |
<!-- END:abd-eff -->

Reading:

- Quality is within noise for every pair; the largest gap is Sonnet (21/26 against 18/26, p=0.25).
  The 95% intervals of all ten arms overlap, and between two repeats of the same arm 2 to 5 of the 26 tasks
  change outcome (CC-deepseek 4, FD-deepseek 5, CC-haiku 3, FB-haiku 2) even where the solved count barely moves.
- Four tasks (a04, a07, a08, a09) were solved by no arm and twelve by all ten arms, so only ten of the 26 tasks
  can separate the arms. `a01` is the one task Claude Code solved in all five of its arms and Forge in none (F1).
  `b09` is the one task Forge solved (both FD repeats) and no Claude Code arm did.
- Claude Code has the lower median agent time in all five pairs. The gap is significant in 3 of the 5
  pairs (DeepSeek both repeats, Haiku repeat 1) and not in the others. Totals are dominated by a few long tasks
  and by one 1,200 s timeout in CC-deepseek repeat 1, which is why the medians lead.
- Forge's DeepSeek loop uses no more input tokens in total (x0.97 and x0.81 of Claude Code's) but x1.2 to x1.4 the output tokens
  and x1.6 to x1.8 the tool calls, with a lower cache hit and so x1.3 to x1.8 the input not read from cache (F5, F6). The bridge costs more input and model calls than plain Claude Code (F4).

### Suite c: large codebases

Per arm; the Forge row excludes c06 from token and time totals because that run produced no result row:

<!-- BEGIN:c-arm-qs -->
| Arm | Solved | Pass rate | 95% CI (Wilson) | Timeouts | Agent wall median (s) | Agent wall total (s) |
|---|---:|---:|---:|---:|---:|---:|
| `CC-deepseek/seed1` | 11/11 | 100% | 74-100% | 0 | 81.9 | 934.7 |
| `FD-deepseek/seed1` | 7/11 | 64% | 35-85% | 0 | 160.5 | 1,634.0 |
<!-- END:c-arm-qs -->

<!-- BEGIN:c-arm-eff -->
| Arm | Input tokens | Input not read from cache | Output tokens | Cache hit | Tool calls | Tool errors | Model calls | Rows with incomplete metrics |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| `CC-deepseek/seed1` | 5,826,028 | 335,084 | 72,945 | 94% | 259 | 7 | 194 | 0 |
| `FD-deepseek/seed1` | 4,476,782 | 1,477,486 | 248,087 | 67% | 516 | 13 | 251 | 1 |
<!-- END:c-arm-eff -->

<!-- BEGIN:c-notes -->
- `FD-deepseek/seed1` `c06-rename-config-provider-of`: no result row: panic after completion
<!-- END:c-notes -->

Forge against Claude Code, quality, speed and efficiency:

<!-- BEGIN:c-pairs -->
| Model | Pair | Repeat | Claude Code solved | Forge solved | Forge minus CC | Solved only by CC / only by Forge | Leads | Confidence (sign test on the discordant tasks) |
|---|---|---:|---:|---:|---:|---:|---|---|
| DeepSeek V4.1 Flash | FD vs CC | 1 | 11/11 | 7/11 | -36 pp | 4 / 0 | Within noise, Claude Code ahead | Low, p=0.125 |

| Model | Pair | Repeat | CC median (s) | Forge median (s) | Ratio of medians | Median paired ratio | Total wall CC / Forge (s) | Tasks where Forge was faster | Leads | Confidence (sign test over tasks) |
|---|---|---:|---:|---:|---:|---:|---:|---:|---|---|
| DeepSeek V4.1 Flash | FD vs CC | 1 | 81.9 | 160.5 | x1.96 | x2.30 | 934.7 / 1,634.0 | 1 of 10 | Claude Code | Moderate, p=0.0215 |

| Model | Pair | Repeat | Paired tasks with complete metrics | Input tokens (Forge / CC) | Output tokens (Forge / CC) | Tool calls (Forge / CC) | Model calls (Forge / CC) | Cache hit CC / Forge | Input not read from cache (Forge / CC, whole arm) |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|
| DeepSeek V4.1 Flash | FD vs CC | 1 | 10 | x0.82 | x3.68 | x2.21 | x1.40 | 94% / 67% | x4.41 |
<!-- END:c-pairs -->

Per task (P solved, F failed; "answer items" is the partial credit printed by the graders of the
reading tasks):

<!-- BEGIN:c-tasks -->
| Task | CC | FD | CC wall (s) | FD wall (s) | CC input | FD input | CC output | FD output | CC tools | FD tools | CC answer items | FD answer items |
|---|:---:|:---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| `c01-pinned-429-wait-budget` | P | P | 112.5 | 126.1 | 1,427,646 | 386,620 | 9,297 | 22,875 | 36 | 46 | 4/4 | 4/4 |
| `c02-compaction-trigger-budget` | P | F | 71.9 | 177.3 | 357,900 | 449,580 | 5,500 | 36,229 | 19 | 60 | 4/4 | 2/4 |
| `c03-provider-implementors` | P | P | 76.7 | 64.1 | 432,720 | 212,369 | 7,233 | 9,558 | 24 | 27 | 3/3 | 3/3 |
| `c04-claude-bridge-spawn-path` | P | F | 81.9 | 175.1 | 672,971 | 514,663 | 8,847 | 31,719 | 30 | 51 | 4/4 | 3/4 |
| `c05-effort-ceiling-value-rung` | P | P | 109.0 | 286.6 | 346,162 | 666,166 | 3,468 | 27,528 | 16 | 74 | - | - |
| `c06-rename-config-provider-of` | P | F (no result) | 140.5 | - | 379,634 | - | 5,571 | - | 26 | - | - | - |
| `c07-http-conditional-leap-march` | P | P | 44.1 | 189.2 | 213,051 | 420,410 | 3,318 | 23,440 | 12 | 46 | - | - |
| `c08-gitignore-negation-parent-dir` | P | F | 98.9 | 127.8 | 616,924 | 315,365 | 10,415 | 22,664 | 29 | 39 | - | - |
| `c09-npm-config-hash-value` | P | P | 49.4 | 145.9 | 194,363 | 331,670 | 3,302 | 17,561 | 16 | 44 | - | - |
| `c10-dulwich-commit-to-loose-object` | P | P | 122.9 | 224.5 | 1,061,868 | 870,164 | 13,179 | 35,171 | 44 | 98 | 4/4 | 4/4 |
| `c11-tool-output-truncation` | P | P | 26.8 | 117.4 | 122,789 | 309,775 | 2,815 | 21,342 | 7 | 31 | 4/4 | 4/4 |
<!-- END:c-tasks -->

Reading: Claude Code solved all 11; Forge solved 7. Forge's four failures: c02 (2 of 4 answer items) and
c04 (3 of 4) are wrong answers on reading tasks, c08 is the read-only misclassification (F1; the suite keeps that prompt wording
on purpose as a canary), and c06 is the panic (F2). Forge was slower on 9 of 10 comparable tasks (ratio of medians x1.96), wrote x3.68 the output
tokens and made x2.21 the tool calls. Forge ran with a fresh store per task, and its logs show 30
automatic compactions across the 11 tasks; Claude Code's streams show none (F6).

### Multi-turn

Three scripted sessions of seven prompts, each one persistent conversation per arm, with a hidden
grader and a probe of whether a standing rule or an earlier finding is still honoured.

<!-- BEGIN:mt -->
| Arm | Sessions completed | Hidden grader pass | Probe score (mean) | Wall total (s) | Input tokens | Input not read from cache | Output tokens | Tool calls | Tool errors |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| FD-deepseek | 3/3 | 100% | 1.00 | 816.1 | 3,941,905 | 603,025 | 91,496 | 269 | 3 |
| CC-deepseek | 3/3 | 100% | 1.00 | 783.2 | 5,006,885 | 189,733 | 87,765 | 204 | 3 |

| Session | Arm | Prompts | Hidden tests | Checks | Probe | Wall (s) | Input tokens | Input not read from cache | Output tokens | Tool calls | Tool errors |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| S1-tasklog | FD-deepseek | 7/7 | 29/29 | 4/4 | 1.00 | 210.2 | 857,962 | 167,530 | 27,796 | 72 | 0 |
| S1-tasklog | CC-deepseek | 7/7 | 29/29 | 4/4 | 1.00 | 203.7 | 926,925 | 59,469 | 27,079 | 70 | 1 |
| S2-fxbatch | FD-deepseek | 7/7 | 14/14 | 5/5 | 1.00 | 441.5 | 2,540,090 | 299,834 | 49,790 | 149 | 0 |
| S2-fxbatch | CC-deepseek | 7/7 | 14/14 | 5/5 | 1.00 | 438.9 | 3,432,577 | 94,209 | 49,243 | 95 | 2 |
| S3-invoicer | FD-deepseek | 7/7 | 16/16 | 4/4 | 1.00 | 164.5 | 543,853 | 135,661 | 13,910 | 48 | 3 |
| S3-invoicer | CC-deepseek | 7/7 | 16/16 | 4/4 | 1.00 | 140.6 | 647,383 | 36,055 | 11,443 | 39 | 0 |
<!-- END:mt -->

Reading: both arms completed all 21 prompts, passed every hidden test and scored 1.00 on the probe, so
this dimension is at its ceiling and says nothing about quality. Total prompt time differs by about 4%.
Forge reads less of its prompt from cache (603,025 of 3,941,905 input tokens not from cache against 189,733 of
5,006,885) and made x1.32 the tool calls. "Wall" is the sum of the seven prompt times of a session; the whole session, with
workspace setup, the hidden grader and the probe, took 884 s for Forge and 821 s for Claude Code (8% apart). Claude Code was faster in all three
sessions. One repeat only.

### SWE-bench Verified

Ten instances (`instances_sha256` `7a808cff...`), scored by the official evaluator.

<!-- BEGIN:swe -->
| Arm | Resolved (official) | Wall total (s) | Wall median (s) | Input tokens | Input not read from cache | Output tokens | Integrity fail / warn | Traces audited | Repos left with refs/stash |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| FD-deepseek | 10/10 | 6,956.8 | 731.1 | 13,998,608 | 4,836,112 | 841,616 | 0 / 2 | 10/10 | 8 |
| CC-deepseek | 9/10 | 3,011.9 | 255.5 | 12,089,374 | 345,630 | 152,667 | 0 / 9 | 0/10 | 0 |

Paired over 10 instances: resolved only by FD-deepseek 1, only by CC-deepseek 0 (sign test p=1); FD-deepseek faster on 0, CC-deepseek faster on 10 (sign test p=0.00195).
<!-- END:swe -->

<!-- BEGIN:swe-instances -->
| Instance | FD-deepseek official | FD-deepseek wall (s) | FD-deepseek output tokens | FD-deepseek test files touched | CC-deepseek official | CC-deepseek wall (s) | CC-deepseek output tokens | CC-deepseek test files touched |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| `astropy__astropy-13033` | resolved | 920.7 | 122,819 | no | unresolved | 205.8 | 24,605 | yes |
| `django__django-13343` | resolved | 413.9 | 28,866 | yes | resolved | 57.7 | 4,009 | yes |
| `django__django-13821` | resolved | 176.0 | 27,869 | yes | resolved | 118.8 | 10,554 | yes |
| `matplotlib__matplotlib-22865` | resolved | 1,154.7 | 196,438 | no | resolved | 507.3 | 23,321 | yes |
| `psf__requests-1921` | resolved | 256.7 | 30,134 | no | resolved | 36.9 | 2,769 | no |
| `pydata__xarray-6461` | resolved | 714.4 | 64,951 | no | resolved | 305.3 | 20,846 | yes |
| `pytest-dev__pytest-8399` | resolved | 424.1 | 52,623 | no | resolved | 107.1 | 4,649 | yes |
| `scikit-learn__scikit-learn-25973` | resolved | 747.8 | 39,357 | no | resolved | 400.5 | 9,697 | yes |
| `sphinx-doc__sphinx-8638` | resolved | 1,232.5 | 183,451 | no | resolved | 724.3 | 33,545 | yes |
| `sympy__sympy-13877` | resolved | 916.1 | 95,108 | no | resolved | 548.2 | 18,672 | yes |
<!-- END:swe-instances -->

Reading: Forge resolved all ten and Claude Code nine (`astropy__astropy-13033` unresolved), a one-instance
difference that is inside the noise. Forge took x2.31 the total time (summed over instances; each arm ran two instances at a time, so elapsed time was about half) and was slower on every instance (10 of 10), and it wrote
x5.51 the output tokens, 50% of them reported as reasoning. Its cache hit was 65% against 97%. Forge left
`refs/stash` in 8 of the 10 repositories (F3).

### Per-task matrix

Suites a+b+d (P solved, F failed; number is agent wall in seconds):

<!-- BEGIN:matrix-abd -->
| Task | CC-deepseek r1 | CC-deepseek r2 | FD-deepseek r1 | FD-deepseek r2 | CC-haiku r1 | CC-haiku r2 | FB-haiku r1 | FB-haiku r2 | CC-sonnet r1 | FB-sonnet r1 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| `a01-fix-back-to-back-conflicts` | P 21 | P 22 | F 31 | F 25 | P 16 | P 22 | F 18 | F 12 | P 14 | F 11 |
| `a02-sliding-window-limiter` | P 50 | F 60 | F 101 | F 134 | F 54 | F 77 | F 66 | F 58 | P 42 | P 37 |
| `a03-refactor-order-pricing` | P 95 | P 109 | P 98 | P 112 | P 86 | P 119 | P 126 | P 106 | P 42 | P 37 |
| `a04-debug-nightly-export-crash` | F 73 | F 169 | F 154 | F 273 | F 54 | F 47 | F 57 | F 182 | F 29 | F 19 |
| `a05-mailer-options-object` | P 65 | P 65 | P 221 | F 47 | F 136 | P 91 | P 85 | P 114 | P 38 | P 40 |
| `a06-answer-retry-policy` | F 33 | F 35 | P 58 | P 50 | P 61 | F 53 | F 69 | P 123 | P 31 | F 69 |
| `a07-speed-up-log-report` | F 1200 | F 626 | F 70 | F 198 | F 290 | F 410 | F 173 | F 97 | F 66 | F 21 |
| `a08-signup-validation-constraints` | F 55 | F 81 | F 57 | F 97 | F 50 | F 46 | F 71 | F 67 | F 26 | F 26 |
| `a09-layered-config-cli` | F 133 | F 146 | F 218 | F 216 | F 169 | F 164 | F 153 | F 140 | F 70 | F 67 |
| `b01-stockctl-longhorizon` | F 88 | P 95 | P 224 | P 199 | P 91 | F 97 | P 101 | F 111 | P 50 | P 68 |
| `b02-ttlcache-expiry-trap` | F 33 | F 29 | P 38 | F 40 | F 30 | F 31 | F 37 | F 30 | P 20 | P 21 |
| `b03-rename-account-wire-frozen` | P 95 | P 74 | F 144 | P 163 | P 78 | P 77 | P 91 | P 85 | P 32 | P 40 |
| `b04-batch-run-concurrency` | P 212 | F 124 | F 130 | P 345 | F 231 | F 59 | F 55 | F 68 | P 25 | P 45 |
| `b05-minimd-broken-merge` | F 59 | P 42 | P 54 | F 87 | P 47 | P 56 | P 46 | P 43 | P 33 | F 25 |
| `b06-semver-regression-archaeology` | P 51 | P 64 | P 62 | P 54 | P 53 | P 51 | P 45 | P 52 | P 23 | P 32 |
| `b07-ledger-rollup-awk` | P 362 | P 397 | P 433 | P 255 | P 163 | P 184 | P 216 | P 186 | P 90 | P 79 |
| `b08-scaffold-service-noninteractive` | P 16 | P 18 | P 43 | P 25 | P 19 | P 17 | P 33 | P 29 | P 9 | P 26 |
| `b09-fsweep-docs-drift` | F 35 | F 34 | P 89 | P 97 | F 61 | F 47 | F 81 | F 50 | F 23 | F 46 |
| `d01-interactive-test-runner` | P 38 | P 26 | P 27 | P 36 | P 38 | P 29 | P 50 | P 37 | P 16 | P 17 |
| `d02-huge-output-needle` | P 88 | P 30 | P 60 | P 55 | P 15 | P 20 | P 32 | P 21 | P 14 | P 20 |
| `d03-flaky-loading-cache` | P 65 | P 72 | P 57 | P 188 | P 54 | P 65 | P 120 | P 49 | P 17 | P 28 |
| `d04-long-build-await` | P 200 | P 199 | P 228 | P 206 | P 199 | P 201 | P 196 | P 206 | P 195 | P 199 |
| `d05-background-server-roundtrip` | P 26 | P 25 | P 28 | P 26 | P 18 | P 22 | P 23 | P 26 | P 18 | P 22 |
| `d06-readonly-config-files` | P 31 | P 24 | P 25 | P 33 | P 11 | P 8 | P 24 | P 25 | P 18 | P 28 |
| `d07-binary-neighbor-schema` | P 15 | P 20 | P 21 | P 19 | P 10 | P 13 | P 22 | P 15 | P 12 | P 21 |
| `d08-bsd-sed-make-target` | P 46 | P 32 | P 86 | P 54 | P 27 | P 22 | P 31 | P 28 | P 13 | P 26 |
| **solved** | 17 | 17 | 18 | 17 | 17 | 16 | 16 | 16 | 21 | 18 |
<!-- END:matrix-abd -->

Suite c:

<!-- BEGIN:matrix-c -->
| Task | CC-deepseek r1 | FD-deepseek r1 |
|---|---:|---:|
| `c01-pinned-429-wait-budget` | P 113 | P 126 |
| `c02-compaction-trigger-budget` | P 72 | F 177 |
| `c03-provider-implementors` | P 77 | P 64 |
| `c04-claude-bridge-spawn-path` | P 82 | F 175 |
| `c05-effort-ceiling-value-rung` | P 109 | P 287 |
| `c06-rename-config-provider-of` | P 141 | F (no result) |
| `c07-http-conditional-leap-march` | P 44 | P 189 |
| `c08-gitignore-negation-parent-dir` | P 99 | F 128 |
| `c09-npm-config-hash-value` | P 49 | P 146 |
| `c10-dulwich-commit-to-loose-object` | P 123 | P 225 |
| `c11-tool-output-truncation` | P 27 | P 117 |
| **solved** | 11 | 7 |
<!-- END:matrix-c -->

## Notable findings

Each finding says what was observed, where, and how sure we are of the cause.

**F1. A scoped "don't edit X" turned fix requests into read-only turns.**
Observed: in both FD repeats the run log shows `permission denied by task scope <id>: shell is
unavailable for Rea…` (the log line is cut off; the agent's own text names `ReadOnlyReview`) on a01, a04 and a07 and on no other
task. The agent then printed a diagnosis and a patch and said it could not apply it. The three prompts each contain a clause of the form "don't
edit the existing tests" / "don't edit the incoming CSV". In all three FB arms the bridged model also
says the turn is read-only on the same three tasks. Suite c shows the same denial on c08, whose prompt keeps
"do not edit existing files under `dulwich/tests`" on purpose as a canary. Effect on score: a04 and a07
were solved by no arm, so within suites a+b+d it shows on a01 only (Claude Code 5 of 5 arms, Forge 0 of 5); it cost c08.
PR #1603 narrows the rule so a protected object no longer makes the whole turn read-only (its description says
Claude Code fixed all three tasks; in this baseline it fixed a01 only). That PR has unit tests only; the tasks have not
been re-run, so that a01 and c08 now pass is a hypothesis.

**F2. Forge panicked after finishing c06.**
Observed: the FD run for c06 exited with code 101 after 165 s with `thread 'main' panicked at
crates/forge-core/src/lib.rs:3862:50: range start index 17 out of range for slice of length 10`. The log shows the
agent had finished its edits (its final summary lists 28 files changed) but the process died before grading, so there is no
result row. It is counted as failed; whether the work would have passed is unknown. The suite c
README records the same panic on c06 and c10 during calibration. PR #1607 attributes it to the completeness audit slicing a
transcript that automatic compaction had already shortened (2 of 8 compaction-heavy runs in its measurements).

**F3. Forge leaves `refs/stash` in the user's repository.**
Observed: 8 of 10 FD SWE-bench repositories ended with a `refs/stash`; none of the 10 Claude Code ones
did (`summary.tsv`, column `history`). All ten FD logs contain Forge's warning that the turn modified existing test
files and that it stashed the test edits and pushed back once. That is the existing-tests-are-spec guard in
`crates/forge-core/src/lib.rs` (it runs `git stash push -- <test files>`). The two repositories without a stash
(django-13343, django-13821) are the two whose final patch still touches test files: both logs show the model
running `git stash pop`. The patch is captured from the working tree, so stashed test edits are not in it; the official
evaluator resets the test files to the base commit before applying its own test patch (`git checkout <base> <test files>` in
`swebench/harness/test_spec/python.py`), so test-file edits in a patch do not change the verdict either way. The side effect is visible to a user: an unexpected stash entry holding the agent's
rejected test edits. The guard fired in 10 of 10 FD instances, so DeepSeek edited existing tests every time;
9 of 10 Claude Code patches also touch test files.

**F4. The Claude Code bridge costs more than Claude Code itself.**
Observed on the same model, paired tasks: input tokens x1.04 and x1.16 (Haiku repeats 1 and 2) and x1.36 (Sonnet),
model calls x1.18, x1.28 and x1.51, median agent time x1.14, x1.06 and x1.10. The recorded argv shows an extra
3,025-character `--append-system-prompt` and Forge's tools exposed through MCP. PR #1606 itemises per-request overhead
(an 11,970-character system prompt cut to 9,616, a 2,354-character auto-memory section) and notes that Forge's tool
schemas (18.8k characters) are smaller than Claude Code's own (29.5k); those figures come from the PR's own
measurements on a debug build, not from this baseline, and this baseline did not isolate the contributors.
F9 does not explain it: the bridge and Claude Code arms both received the global instruction files.

**F5. Forge's loop writes more and acts more than Claude Code's on the same model.**
Observed with DeepSeek: output tokens x1.38 and x1.17 (a+b+d), x3.68 (c), x5.51 (SWE-bench); tool calls
x1.58 and x1.83 (a+b+d), x2.21 (c). Forge reports 52% of its a+b+d output (164,936 of 316,385 and 179,458 of 342,294)
and 50% of its SWE-bench output (418,482 of 841,616) as reasoning tokens. Caveat on the counters: in every FD row of suites a+b+d reasoning is
30% to 72% of the reported output, so Forge's output figure includes it. Claude Code's streams in repeat 1 hold 261 thinking blocks
across 498 tool calls, and its 223,861 reported output tokens are well above the roughly 56k that the visible text and tool inputs
would come to at four characters per token, and above the roughly 189k with the thinking text included, which indicates its counter
includes thinking as well. That is an estimate, not a tokenizer count, and how InferHub counts reasoning on the two API surfaces
was not checked. Wall time and tool calls point the same way and are not subject to this caveat.

**F6. Low cache hit and heavy compaction on large tasks.**
Observed: Forge's cache hit is 84-85% (a+b+d), 67% (c) and 65% (SWE) against 91%, 94% and 97% for Claude Code.
Suite c logs show 30 automatic compactions in Forge's 11 tasks (1 to 4 each; c02 and c04, the two wrong answers, have 4 each, as does c10, which was solved)
and no compaction event in Claude Code's streams; suites a+b+d show 11 and 14 over 26 tasks. The model's window is 1,048,576 tokens (per PR #1607). PR #1607 attributes this to a
fresh store not knowing the model's context window and falling back to a 32,000-token floor; suite c used a fresh store per
task, suites a+b+d one shared store. Our logs confirm the compactions, not the cause. PR #1607 reports c01 compactions
3 to 0 and c02 unsolved to solved in one live run each, which is not a re-run of the suite.

**F7. A Forge session can fail to start on store contention.**
Observed: FD repeat 2 lost a05 with `building session: starting session: database is locked` after 47 s, with no usage row.
That log has 8 such lines (the other FD repeat none). The two FD and two FB-Haiku arms ran at the same time on one shared
SQLite store (`forge-bench.db`), so part of the contention is the benchmark's own doing; a session start should
still not fail on it. The two FB-Haiku arms logged one and two lattice warnings with the same error but lost no session. One attempt of 52 was lost. It is counted
as unsolved. PR #1607 changes the busy handling.

**F8. Forge solved one task nobody else did.**
Observed: `b09-fsweep-docs-drift` was solved by both FD repeats and by none of the eight other arms. The grader stops at the
first failing check: five of the eight other arms failed on "--help output does not describe the dry-run output (would remove)" and three
(CC-Haiku in both repeats, FB-Haiku in repeat 2) on "docs/usage.md does not mention --verbose". So each missed one documentation item. Why
Forge's loop caught them was not examined. It is a single task, so it shows that the two harnesses fail differently, not that one is better.

**F9. Isolation was not perfect.**
Observed: the suite a, b, c and d workspaces are created under `$HOME` (`~/.cache/forge-ops/ccbench/tmp/`), and Claude Code loads the
`CLAUDE.md` and `.claude/CLAUDE.md` of every ancestor directory as project instructions. The `instructions` attachment of all 78 bridged
transcripts (26 tasks in each of three FB arms) lists `/home/floris/CLAUDE.md` (1,434 characters) and the operator's global
`/home/floris/.claude/CLAUDE.md` (7,049), both typed as project instructions. The Claude Code arms' stream-json does not record memory, so
we checked the same flag set (`--setting-sources project --strict-mcp-config --no-session-persistence`) against a local fake API after the
run, with no model call ([isolation-check.json](artifacts/claude-code-parity-2026-10/baseline/isolation-check.json)): from a directory under
`$HOME` the first user message is 10,328 characters and contains text of both files; from `/tmp` it is
1,275 characters and contains neither. So the Claude Code and bridge arms received about 8.5k characters of the operator's private instructions
on every request (the files hold behavioural rules, not only facts), and the request body of that check was 9.1k bytes larger. That is a bias of unknown size on quality, and it is symmetric between CC and FB. The Forge native arm had a neutral `.forge/AGENTS.md`, and
the static audit records that Forge loads only the first instruction file and does not walk parent directories (PARITY.md, G3), so FD probably
received neither; no FD request was captured. If so, the DeepSeek A/B handicaps Claude Code slightly, which does not explain Forge's losses on time and
output tokens. Multi-turn and SWE-bench workspaces are outside `$HOME` (`/tmp`, `/var/tmp`), and the SWE-bench Claude Code arm also used `--safe-mode`.
Before the next run, move the suite a to d workspaces outside `$HOME` (or use `--safe-mode`) and capture the first request of every arm, FD included.

## Fix ledger

Fix PRs opened from these findings and the [feature audit](artifacts/claude-code-parity-2026-10/baseline/PARITY.md)
(280 compared features: Forge better on 40, equal on 110, worse on 71, no equivalent on 59; 25 ranked gaps; the Claude Code
side of that audit is read from its documentation and `--help` output, not executed).
Findings F1 to F9 are listed above; the audit's gaps are G1 to G25.

<!-- BEGIN:ledger -->
| PR | Title | State (2026-10-22 15:43Z, from gh pr view) | Finding or gap it addresses | Evidence it works |
|---|---|---|---|---|
| #1602 | feat(cli): forge run -p is a drop-in for claude -p | merged | Parity gaps G1 (print mode), G2 (stream-json shapes), headless half of G7 (flags); PARITY.md | Before/after on the mock provider in the PR. No benchmark dimension measures it yet. |
| #1603 | fix(core): a scoped "don't edit X" no longer makes a change request read-only | merged | F1: a01, a04, a07 (Forge arms) and c08 were run as read-only turns because the prompt says not to edit tests or a directory | Unit tests of the phrase rule in the PR. The affected tasks have not been re-run, so the effect on pass rate is a hypothesis. |
| #1604 | feat(tui): fold tool runs into one summary row; nav pills only when scrolled up | merged | UI/UX parity with Claude Code's folded tool groups; a navigation pill that covered the newest rows | Not measured: the scripted TUI comparison is pending. |
| #1605 | fix(cli,tui): bounded stdin wait, per-directory --continue, auth --list on stderr, notices never split an answer | merged | Headless and TUI robustness defects found while driving both tools (a run that hung on an unclosed stdin pipe, --continue taking another project's session) | Unit tests and before/after runs in the PR. Robustness is not a benchmarked dimension yet. |
| #1606 | perf(bridge): cut claude-cli bridge startup, prompt and post-turn overhead | open | F4 (bridge token and latency overhead). Its first item (the global CLAUDE.md sent twice when a project has no instruction file) does not apply to the baseline FB arms, which had a neutral project instruction file, and nothing in it changes F9 | Local fake-API measurements in the PR (for example mcp-serve handshake 935 ms to 101 ms, system prompt 11,970 to 9,616 characters). The FB arms have not been re-run. |
| #1607 | fix(core): bench robustness — compaction-safe audit, store lock starvation, pinned windows and side calls | merged | F2 (c06 panic), F6 (compaction churn and low cache hit on large codebases), F7 ('database is locked' at session start), side calls that ignored the pin | Two live single runs in the PR (c01, c02): compactions 3 and 4 before, 0 after; c02 unsolved before, solved after. One sample each, not a re-run of the suite. |
| #1608 | perf(core): nothing optional runs before a turn's answer or its first token | merged | Latency before the first token and after the answer (trivial-prompt latency, not yet benchmarked) | Release-build timings in the PR. The latency benchmark is pending. |
| #1609 | feat(bench): task selection, failure reasons, Claude Code metrics, bridged-CLI tool counts | open | Measurement gaps behind the tool-call comparability caveat and the missing failure reasons in this baseline | Tooling change; it makes the next run set more comparable. |
| #1610 | feat(cli): bare forge starts chat, claude-style mcp add, /clear = new, Ctrl+L redraw, 9 Claude Code slash commands | open | Parity gaps G6 (entry points), G10 (slash commands), G11 (/clear and Ctrl+L) | Not measured: feature parity, no benchmark dimension. |
<!-- END:ledger -->

## Not yet measured

- The scripted TUI and UX comparison (launch, permission prompt, diff, interrupt, resume, narrow terminal, paste) with a blinded rubric. The harness is in the kit; there are no results.
- Trivial-prompt and first-token latency, TUI startup and keystroke latency.
- The Opus subset.
- Haiku and Sonnet on suite c, multi-turn and SWE-bench. Sonnet suite c was still being collected when this baseline was cut and is not part of it.
- Robustness (interrupt and resume correctness, tool-failure recovery) and headless ergonomics beyond the static audit.
- Claude quota consumed per arm (the probe was off) and a dollar comparison.
- Re-runs of any task after a fix.

## Limitations

- **Small samples.** 26 tasks per repeat, 11 in suite c, 3 sessions, 10 instances. Wilson intervals on the 26-task pass rates are 29 to 35 points wide and overlap for every pair.
  Between repeats of one arm 2 to 5 tasks change outcome, comparable to most harness differences.
- **Only ten of the 26 a+b+d tasks discriminate** (4 solved by nobody, 12 by everybody). The multi-turn set is at its ceiling.
- **One repeat for Sonnet and for everything in suite c, multi-turn and SWE-bench.**
- **Shared machine.** Paired arms ran side by side on one machine that also hosted other agents' work (see Environment). Load was not recorded, so wall times carry noise, though pairs share it. The Sonnet pair and the suite c, multi-turn and SWE-bench runs overlapped in time with other arms, so wall times of different run sets are not comparable with each other.
- **Different paths to the model.** The DeepSeek A/B uses an Anthropic-compatible endpoint for Claude Code and Forge's provider for FD. Caching, reasoning accounting and latency may differ by path, apart from the harness.
- **Token and tool accounting differ** between harnesses (cache writes, reasoning tokens, tool vocabulary). Ratios are indicative.
- **Isolation and effort.** See F9: the Claude Code and bridge arms of suites a to d saw the operator's global instruction files and the Forge native arm probably did not. Suites a to d used each harness's default effort. The SWE-bench `run-meta.json` for the Claude Code arm says `--effort high`, but that string is a constant written by `run.sh` for every Claude Code arm; the arm's environment sets `SWE_CLAUDE_EFFORT` empty, which the shim turns into no `--effort` flag, so both SWE-bench arms ran at their default effort (read from the scripts; the Claude Code argv was not recorded for that run).
- **Authored by the Forge team.** The suites were written for this study; c01-c06 and c11 use a snapshot of Forge's own source, and c08 keeps a prompt phrase that triggers a Forge defect known beforehand. Suite c was calibrated against Forge's own loop before this run: its README records c01, c02, c03, c04, c05, c07, c09 and c11 solved in that run, so c02 and c04 passed once and failed in the baseline, and a single attempt per task is noisy. Hidden graders are withheld, so outsiders cannot re-run the scoring until they are disclosed against the commitments.
- **FD c06 is scored as failed** because the grader never ran. That favours Claude Code by at most one task in suite c (7/11 would become 8/11 if the work passed).
- **Forge commit provenance.** The binary embeds no commit hash. Commit `8b18cad8` is traced through the kit's build log (see Environment) rather than through the binary itself.
- **One Claude Code timeout.** One Claude Code attempt (a07, repeat 1) timed out; its tokens are recorded as 0 and the arm's token totals are slightly low. Paired token comparisons exclude it.

## Reproduction

Everything below ran from the bench kit at `~/.cache/forge-ops/ccbench/` (not in this repository).
Kit layout: `PLAN.md`, `runner/` (`arm.sh`, `aggregate.py`), `suite/{a,b,c,d,all}/` (task directories with
hidden graders), `multiturn/` (scripted sessions, `run.sh`), `swe/` (`run.sh`, `eval.sh`, instances), `parity/` (the audit), `tui/` and
`latency/` (harnesses without results yet), `results/` (raw output).

```
# suites a+b+d: one arm.sh per arm, all started together; the pinned Forge binary is passed in CCBENCH_FORGE
CCBENCH_FORGE=bin/forge CCBENCH_USAGE=0 runner/arm.sh <arm> suite/all 1 results/runs/<run>
python3 runner/aggregate.py results/runs/<run> ... --results-dir results/all-now

# suite c: one task per arm.sh call, one Forge store per task
FORGE_DB=c-db/<arm>-<task>.db CCBENCH_TASKS=<task> runner/arm.sh <arm> suite/c 1 results/runs/c/<task>

# multi-turn and SWE-bench Verified
multiturn/run.sh <arm> results/mt 1
JOBS=2 TIMEOUT=1500 swe/run.sh <arm> results/swe/<arm>   &&   swe/eval.sh results/swe/<arm>/predictions.jsonl
```

Arm ids: `CC-<model>`, `FB-<model>` (`forge --model claude-cli::<model>`), `FD-deepseek`, `CC-deepseek`. The
InferHub key is read from a mode-0600 file by `arm.sh` and exported only into the child process; it
is not in this repository.

Private raw archive (transcripts, Forge stores, the pinned binary, the full task kit including
hidden tests): `~/Documents/forge-benchmarks/cc-parity-2026-10/baseline-20261022T1457Z.tar.zst`,
SHA-256 `4449b48707e10132a8c0ef7cdee67c843e4931075d3b6b19c6cb5aa46d75951a`.

In this repository:

- `docs/benchmarks/artifacts/claude-code-parity-2026-10/runs.json` lists the run sets and their files.
- `baseline/` holds `suites-abd.json`, `suite-c.json`, `multiturn-*.json`, `swe/` (official reports,
  predictions, metrics, integrity, `summary.tsv`, `run-meta.json`, `eval.log` per arm), `tasks.json`,
  `environment.json`, `isolation-check.json` and `PARITY.md`.
- `python3 docs/benchmarks/artifacts/claude-code-parity-2026-10/render.py` rewrites every table in this report from
  those files (`--check` verifies, `--verify` recomputes arm totals from the per-task rows).
  `ingest.py` converts a new kit output into the same file shapes.
- `python3 docs/benchmarks/artifacts/claude-code-parity-2026-10/audit_raw.py --kit ~/.cache/forge-ops/ccbench --commitments` is an
  independent check. It shares no code with `render.py`, reads none of the derived JSON, and recomputes every table cell, every
  headline row, the figures quoted in the text and the task commitments from the kit's own files. It needs the kit, so until the kit
  or its archive is shared only the Forge team can run it.
- Hidden tests, graders, reference solutions and the suite archives are not committed. The commitments in
  `tasks.json` and the table above are the way to check them later.

## Errata

Problems with the baseline that came to light after it was published. The tables above are left as
recorded. The after-fix runs use a corrected kit (kit v2) and re-run every arm, Claude Code included,
so before/after comparisons are made inside kit v2 only, never against these baseline numbers.

1. **The Forge native arm used the operator's personal Forge configuration.** Runner v1 gave Forge arms
   their own store (`FORGE_DB`) but not their own config directory. Every FD-deepseek run in suites a to d,
   suite c, multi-turn and SWE-bench therefore read the operator's live `config.toml`. For the InferHub
   provider, which FD used, that file sets a custom system-prompt override and a reasoning prefill, and it
   also carries user MCP servers, a compaction cap and mesh settings. The FD numbers describe Forge with a
   personal prompt, not Forge's shipped default prompt, so they say less about the harness than intended.
   Kit v2 gives every Forge seed a private config, store and instruction file. One visible effect: task
   b09, which only FD solved in the baseline (2 of 2), is solved 0 of 5 times by the same baseline binary
   under kit v2, and 1 of 5 times with the override put back, so the baseline b09 result is not
   reproducible and should not be read as a Forge advantage.
2. **The suite c grader failed five correct answers.** It merged every `A<n>:` line of an answer and let
   the last `key=value` win, so an explanatory note that repeated a label (the instruction allows notes
   after the four answer lines) overwrote a correct answer; one exact match also rejected a qualified
   name (`CliProvider::complete_oneshot` for `complete_oneshot`). Re-grading the recorded answers with a
   grader that reads only the first block and accepts a qualified name changes 5 of 35 attempts from fail
   to pass and none from pass to fail: FD c02 and c04, FB-sonnet c02, CC-haiku c04 and FB-haiku c04. All
   negative examples still fail. Corrected suite c: DeepSeek, Claude Code 11/11 and Forge 9/11; Sonnet,
   Claude Code 11/11 and Forge bridge 10/11; Haiku, both 11/11. The remaining Forge failures are Forge
   defects: c06 (a panic after the work was done, fixed in #1607) and c08 (an edit request misread as a
   read-only turn, fixed in #1603 and widened in #1623). The summary's remark about c02 and c04 and
   compaction is superseded: both were grading errors.
3. **The bridge arm's TUI and latency runs saw the operator's Claude Code settings.** Not part of the
   published numbers, but recorded here because it explains a gap reported elsewhere: in the scripted TUI
   harness the Claude Code arm ran with `--safe-mode` and an explicit low effort, while the bridged Claude Code
   process loaded the operator's own settings (global instructions, high reasoning effort). Kit v2 gives every Claude-based
   arm, Claude Code and the bridge alike, the same empty configuration directory, and the remaining
   differences are listed in the kit's `CHANGES.md` (section 2.5).

## After fixes

This section holds the later run sets. To add one: run the arms with the same kit on the new Forge
build, convert the output with `ingest.py` (`abd` and `c` read the kit's `aggregate.py` summary), copy the
multi-turn and SWE files in the same shapes, append an entry to `runs.json` with the Forge version and commit,
and run `render.py`. The tables below compare the newest run set against the baseline for the Forge arms it
contains, taking Claude Code reference arms from the baseline when the new run set omits them. A kit v2 run set
must not omit them (see [Errata](#errata)).

### Suites a+b+d

<!-- BEGIN:after-abd -->
_No after-fixes run set has been recorded yet. Append one to `runs.json` and run `render.py`._
<!-- END:after-abd -->

### Suite c

<!-- BEGIN:after-c -->
_No after-fixes run set has been recorded yet. Append one to `runs.json` and run `render.py`._
<!-- END:after-c -->

### Multi-turn

<!-- BEGIN:after-mt -->
_No after-fixes run set has been recorded yet. Append one to `runs.json` and run `render.py`._
<!-- END:after-mt -->

### SWE-bench Verified

<!-- BEGIN:after-swe -->
_No after-fixes run set has been recorded yet. Append one to `runs.json` and run `render.py`._
<!-- END:after-swe -->
