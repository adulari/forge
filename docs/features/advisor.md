# Expert on call (`[harness.expert]`, and the DONE review `[harness.advisor]`)

A cheap model does the work. At the three moments that decide a long run, Forge asks a stronger
expert model one question, gets a short diagnosis and a next step back, and the cheap model carries
on. The expert has no tools and sees the turn's transcript, so it costs one call, not a second
agent. Default off.

| Moment | Trigger | What the expert is asked | What the cheap model gets |
| --- | --- | --- | --- |
| The progress supervisor is about to nudge or stop a stalled turn | `stall` | Why is this going nowhere, and what is the one next step | The diagnosis and next step inside the supervisor's nudge. If it gave a next step, the stop is deferred once |
| The turn is about to finish having changed something | `done` | The advisor review: up to 4 blocker/concern notes and one check | A user-role nudge, once (the existing advisor behaviour) |
| The model calls `consult_expert {question}` | `tool` | The model's own question, with the transcript | The answer as the tool result |

```toml
[harness.expert]
model = "codex-oauth::gpt-5.6-sol"   # "" = no expert
budget = "codex:5%w"                 # required for a subscription model; "$2" for an API model
max_consults_per_turn = 2            # across all three triggers
timeout_secs = 120

[harness.expert.triggers]            # all true once a model is set
stall = true
done = true
tool = true
```

## Which model, and the subscription opt-in

The expert may be an API model (`anthropic::`, `bedrock::`, ...) or, only here, a subscription model:
`codex-oauth::*`. A CLI bridge (`claude-cli::*`, `codex-cli::*`, `agy-cli::*`) is refused: it
runs as a tool-using agent, so it could edit or run commands unseen. A subscription model is refused unless
`budget` is a slice of that family's window (`codex:5%w`, `claude:10%5h`, same spelling as a goal
envelope). Other subscriptions (Kimi, OpenCode Go, ...) have no usage meter and cannot be the
expert. A configuration that cannot be used is ignored with one `expert unavailable: <why>`
warning per session; it never stops a turn.

An expert that is the same model as the one driving the turn is skipped for the stall and tool
triggers (it would add nothing), and a benched or privacy-denied expert is skipped everywhere.

## Budget

The budget is metered like a `/goal` envelope: the window reading when the first consult is checked
is the baseline, and the consult is refused once the window has risen by the slice. A window that
rolls over mid-session carries what was used. A `$` budget counts the expert's own spend. A refused
consult prints `expert not consulted: the expert budget (codex:5%w) is spent` (the tool path hands
the same sentence to the model so it carries on alone) and costs nothing.

Without a fresh reading of the window (a reading older than ten minutes, or none) the slice cannot
be verified. The consult still goes ahead, bounded by `max_consults_per_turn`, and the status line
says `usage unread`. When the provider reports its window with the reply (`codex-oauth` does), that reading is stored and the next check uses it.

## DONE: the advisor, migrated

The DONE trigger is the old advisor review, unchanged in what it asks and returns. `[harness.advisor]`
keeps working exactly as before when no expert is configured (default off, the turn's own model or
`advisor.model`, never a subscription). With `[harness.expert] model` set and `triggers.done`, the
expert is the reviewer even when `advisor.enabled` is false, and the model, timeout and budget come
from `[harness.expert]`. One review per turn, one re-drive.

Replay evidence for the review itself (13 baseline Terminal-Bench trials: 11 that finished wrong, 2
that passed; 3 runs each, same rendered input, no tools):

| Reviewer | Unparsable or truncated | Passing trials flagged `blocker` | Names the verifier's failing requirement |
| --- | --- | --- | --- |
| Claude Sonnet | 0 of 39 | 0 of 6 (concern only) | 5 of 11 trials |
| Claude Haiku | 1 of 39 | blockers on both passing tasks | no discrimination |
| deepseek-v4.1-flash | 5 of 13 | not measurable | misses the clearest case |

A small same-model reviewer spends its whole 8k output budget on hidden reasoning for large
transcripts, which is the case for a stronger expert. The replay does not prove the review raises
pass rates; that needs the A/B below.

## Behaviour

- Stall: the expert is asked at the nudge (unless the harness profile is `minimal`, which sends no
  nudge) and again before an enforced stop. The first reply rides in the same `LoopGuard` note as
  the supervisor's nudge. At the stop, a reply with a next step defers the stop once: the model
  gets the advice as a user-role nudge and `MIN_STEPS_AFTER_NUDGE` more steps; if it stalls again
  the stop goes ahead without a second consult. `progress_guard = warn` or `off` never consults at
  the stop.
- Strict JSON, `{"diagnosis": ..., "next_step": ...}`. An unparsable, truncated, timed-out or
  failed reply is recorded (`expert/<trigger>:unparsed|truncated|timeout|error`) and the turn goes
  on as it would have. It never benches a model.
- The cap is per turn. Not consulted: past a turn deadline, at a spend cap, or when the cap or
  budget is spent.
- `consult_expert` is advertised only when an expert is configured, usable and `triggers.tool` is
  on. It is a core tool on the lean surface. On a CLI bridge (`claude-cli`, `codex-cli`) it is served by
  `forge mcp-serve`, which answers from the parent session's stored transcript with the same cap
  and budget; the parent counts those consults into the turn when it ends.

## Where it shows

- Usage: one side-call row per consult, labelled `expert/stall:advice`, `expert/tool:advice`,
  `advisor/done:<outcome>` (and `...:unparsed` and so on for failures).
- Turn record: the turn row's `detail_json` gains `expert_consults`, `expert_model`,
  `expert_triggers`, `expert_status`. `forge turns` lists "turns that consulted the expert"
  (consults, expert and triggers); `forge turns --json` has `expert_turns`; the daemon's turn views
  carry `expert_consults` and `expert`.
- TUI: a scrollback line per consult (`expert: <model> · 1 consult · 2.1% of weekly window (stall):
  <diagnosis> Next: <next step>`) and an `EXPERT` row in `/status`. `forge run` prints the same line
  (`-p` too, on stderr with the trace). A refused consult, whatever the trigger, prints
  `expert not consulted: <why>`.
- Remote snapshot: an additive `expert` string, shown above the composer docks on the phone and
  desktop session screen.
- Settings: `harness.expert.*` appear in the TUI and app settings (search `expert`). A saved
  setting that cannot work yet (a subscription model without its budget, a typo'd provider, a
  window budget on an API model) is flagged right after the save: a `⚠` line in the TUI overlay and
  a warning under the model and budget fields in the app. The same reason is shown once at the
  first turn of a session, with privacy mode, a benched model and a keyless endpoint that has not
  answered discovery.

## Offline replay

`scripts/expert_replay.py` replays the stall and DONE triggers on the gate-0 baseline transcripts
with no model call. The stall side is an approximation of `progress_ledger.rs` (failure, futile and
cycle lanes with the same thresholds, reading tool results as text); the DONE side answers with the
recorded Sonnet review of each trial, standing in for a scripted expert. `--all` scans every
baseline trial for stalls.

| trial | kind | steps | stall asks (step, before, lane) | DONE ask | recorded Sonnet review | re-drive |
| --- | --- | --- | --- | --- | --- | --- |
| bun-sourcemap-leak__a6vAFF5 | failing | 30 | none | step 30 | concern (concern/ok/concern) | yes |
| bun-sourcemap-leak__jorsfpX | failing | 57 | none | step 57 | concern (concern/ok/concern) | yes |
| cargo-flight-dispatch__Sz7jQ9h | failing | 30 | none | step 30 | blocker (concern/blocker/blocker) | yes |
| cargo-flight-dispatch__m9ygT8h | failing | 48 | none | step 48 | blocker (blocker/concern/blocker) | yes |
| foodstuff-beta-activity__TkeKWvT | failing | 13 | none | step 13 | concern (concern/concern/ok) | yes |
| foodstuff-beta-activity__neTWCxk | failing | 21 | none | step 21 | blocker (blocker/concern/blocker) | yes |
| intrastat-meldung__N87j9AW | failing | 81 | none | step 81 | ok (concern/ok/ok) | no |
| intrastat-meldung__mztkPMf | failing | 97 | none | step 97 | concern (concern/ok/concern) | yes |
| kv-live-surgery__zfQy9xY | failing | 34 | none | step 34 | ok (concern/ok/ok) | no |
| mvcc-lsm-compaction__ANJFvzC | failing | 13 | none | step 13 | ok (ok/ok/ok) | no |
| roy-polymorph-cn__fFP6wBe | failing | 26 | none | step 26 | concern (concern/concern/concern) | yes |
| mvcc-lsm-compaction__UebHWvv | passing | 35 | none | step 35 | concern (concern/concern/concern) | yes |
| roy-polymorph-cn__baHpkz7 | passing | 24 | none | step 24 | concern (concern/concern/ok) | yes |

13 trials: stall trigger fires on 0, DONE trigger on 13; 13 expert consults in total (cap 2 per turn).

What it says: the gate-0 failures are wrong-finished, not stalled. None of the 13 would have
reached the stall trigger, and `--all` finds it firing on only 3 of 72 baseline trials
(`cargo-flight-dispatch__wvBtmfm` futile at step 18, `legacy-utility-triage__FdwtpPG` cycle at
step 38, `pretrain-shard-corruption__CFderNi` failure at step 39). For runs like these the lever is
DONE; the stall trigger is for the long agentic runs where a cheap model circles for dozens of
steps. The DONE consult would have re-driven 8 of 11 wrong-finished trials and both passing trials, the
passing ones on concern-level notes only (no blocker on a passing trial), the same numbers as the
advisor replay. A re-drive on a passing trial costs a step or two, not the pass.

## Live A/B (needs the owner's OK)

Not run. Every command below spends ChatGPT plan quota (the expert) and model-gateway credit (the
cheap model), so each needs the owner's explicit go-ahead. Setup is the frontier harness kit in
`~/.cache/forge-ops/harness-frontier` (Harbor subset `subset-signal.txt`, 25 non-GPU tasks, 3
trials, 900 s).

```bash
cd ~/.cache/forge-ops/harness-frontier
export PROVIDER=modelgateway          # cheap model: deepseek-v4.1-flash
S=subset-signal.txt; D=$PWD/ds2/tb2cap; BIN=$PWD/bins   # BIN/forge-expert = this branch's release build
NL=$'\n'
arm() { ./run_round.sh -l ab-$1 -a forge -b $BIN/forge-$2 -C "$3" -k 3 -t 900 -n 3 -s $S -d $D > results-ab-$1.log 2>&1; }
arm base   expert ""
arm expert expert "[harness.expert]${NL}model = \"codex-oauth::gpt-5.6-sol\"${NL}budget = \"codex:5%w\"${NL}max_consults_per_turn = 2"
arm done   expert "[harness.expert]${NL}model = \"codex-oauth::gpt-5.6-sol\"${NL}budget = \"codex:3%w\"${NL}[harness.expert.triggers]${NL}stall = false${NL}tool = false"
```

- Arms: `base` (cheap model alone), `expert` (all triggers), `done` (DONE only). Run `expert` and
  `done` one at a time and read the window between runs (the usage page), because both draw on the
  same weekly ChatGPT window.
- Budget: `codex:5%w` for `expert` and `codex:3%w` for `done`, so at most about 8% of the weekly
  ChatGPT window across the study; the meter refuses further consults once an arm's slice is spent,
  and a refused consult is itself a result worth reading. Worst case by the cap: 25 tasks x 3
  trials x 2 consults = 150 consults per arm.
- Read: pass rate, cost per passed task (expert tokens are separate `expert/...` and
  `advisor/done:...` usage rows), consults per trial (`forge turns --json` `expert_turns`), and the
  share of consults that changed the next action. Stop the study if `done` flags a blocker on a task
  the baseline passed more than once in 6.
- Prerequisite: `[harness.expert]` with a `codex-oauth` model needs a live ChatGPT login on the
  machine running the study.
