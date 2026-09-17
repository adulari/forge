# Feature: repetition loops — the harness stops manufacturing them

> Status: **SHIPPED** (2026-09-18). Code: `crates/forge-core/src/narration_dedupe.rs`,
> the sampling ladder in `crates/forge-core/src/lib.rs` (`repetition_temperature` and the two
> penalty helpers), the pressure signal in `crates/forge-core/src/stall_guard.rs`, the wire
> gating in `crates/forge-provider/src/wire_params.rs`, and `CONTINUATION_MAX` in
> `crates/forge-core/src/lib.rs`.

## 1. Problem

A session on a long turn starts restating itself: a handful of near-identical sentences, step
after step, while the tool calls keep changing. The turn never ends on its own, and no guard
stops it. Measured live on 2026-09-17 (metadata only; the transcript is private):

- of the last 40 assistant rows, 15 distinct texts; one 58-character text appeared **7 times
  across 750 s**, another **5 times across 862 s**;
- ordered oldest→newest, the distinct-text pattern was
  `A B A C A D E F F E D G C H A I A C C B A D I I G J D C A K B G B B L M M N E O` — about five
  phrasings cycling, only 5 of 39 adjacent pairs identical;
- in the same 30 minutes: 62 tool calls, **every one with different arguments**, 63 provider
  calls, 6.8 M input tokens, 212 k output tokens.

Every guard Forge had was blind to it. The doom-loop guard needs identical tool arguments. The
failure-loop guard needs failures. The narration-stall guard counted *consecutive* repeats and
reset on the first different sentence, so a cycling loop never reached its bar. The task-staleness
escalation and the empty-diff nudge did the opposite of helping: they re-drove the model.

## 2. Why it happened here and not elsewhere

Three Forge decisions combined into a machine that produces repetition:

1. **The request was near-deterministic.** Every step of every coding turn was sent at
   `CODING_TEMPERATURE` = 0.1, with no frequency, presence or repetition penalty on any provider.
   A low temperature is right for patches, but it also means an unchanged context yields an
   unchanged continuation.
2. **The context carried the model's own repeats.** `to_llm` deduplicated repeated *tool results*
   but never repeated *assistant prose*, so a turn that said the same sentence five times sent the
   model five verbatim copies of it. A model conditions on its context: copies of a sentence are
   the strongest possible evidence that emitting that sentence is what happens here.
3. **The harness added copies of its own.** The empty-diff gate could re-drive a turn **six**
   times, appending the byte-identical `EMPTY_DIFF_NUDGE` each time and replaying the whole model
   loop. So the request grew copies of both the instruction and the narration — and each re-drive
   made the next repeat more likely, not less.

The failure was not the model being weak. It was the harness assembling the exact pattern that
makes any model repeat, then detecting the result and asking again in the same way.

## 3. What ships

**A repeated statement reaches the model once.** `narration_dedupe::collapse_repeated_narration`
runs inside `to_llm`, next to the existing tool-result dedupe. An assistant or user message whose
text repeats verbatim (whitespace- and case-insensitive, ≥ 40 characters) keeps its **first**
occurrence; later ones are replaced by a marker that names the relationship and says not to
restate it. Tool calls are untouched, so the call/result round-trip a provider validates is
unchanged. Keeping the *first* copy is deliberate: rewriting an older message would change the
prefix that providers prompt-cache, which would turn loop prevention into a cache-miss generator.
Matching is exact, not fuzzy — collapsing two messages that merely open alike would hide real
content from the model.

**A repeating turn gets a different request, not a louder instruction.** `NarrationTracker` now
reports `pressure()` (0–2) from **verbatim** repetition counts, which is the early signal, long
before the nudge/halt bar; `narration_stalled` carries it onto the session and
`model_request` turns it into sampling: temperature 0.1 → 0.5 → 0.9, plus `frequency_penalty`
0.4 → 0.8 and `presence_penalty` 0.2 → 0.5. Penalties travel in genai's `extra_body` and only to
providers verified to accept them (`PENALTY_CAPABLE_PROVIDERS`); Anthropic has no such parameter
and rejects unknown body fields, so it is deliberately absent. An ordinary step sends exactly what
it sent before — no penalties, temperature 0.1 — so nothing changes for turns that are fine.
Pressure is driven by exact repetition precisely so a healthy turn is never quietly run hotter.

**The push-back budget is one, not six.** `CONTINUATION_MAX` = 1. Telling a model once that it
described a fix instead of making it is worth it; telling it a second time has never been observed
to rescue a turn the first did not, and it is how a stuck turn became a 40-minute spiral. After
one push-back the turn ends and says plainly what did not happen. The diminishing-returns
bookkeeping that existed to survive six re-drives is gone with them.

**The detectors stay as the backstop.** The frequency-window narration guard (nudge at 4
recurrences in 20 steps, halt after 2 more) and the identical-call doom-loop guard are unchanged.
They now sit behind three layers that stop the pattern from forming, rather than being the only
thing between a model and an unbounded loop.

## 4. Numbers

| | before | after |
|---|---|---|
| verbatim copies of one sentence in a request | unbounded (7 observed) | 1 |
| copies of the empty-diff nudge in a request | up to 6 | 1 |
| model-loop replays per turn from the empty-diff gate | up to 6 | 1 |
| sampling when the model is repeating | identical to a healthy step (0.1, no penalties) | 0.5–0.9 + penalties where supported |
| what ends a cycling loop | nothing structural; a human interrupt | the pattern never forms; halt remains as backstop |

## 5. Behaviour at the seams

- **Prompt caching.** Only later duplicates are rewritten, so the cached prefix is stable.
- **Providers without penalties.** Anthropic and the CLI bridges get the temperature ladder only.
- **Reasoning models that reject a custom temperature.** The existing `temperature_refused` gate
  still wins, so those models get penalties (where supported) and no temperature.
- **Short acknowledgements.** "Done." twice is not a loop; below 40 characters nothing collapses.
- **A model that rephrases instead of repeating.** Exact matching will not collapse it, and
  pressure stays at 0; the fuzzy frequency guard is what covers that case.
- **Cost.** One hash per message per request, and a small map per turn.

## 6. Tests

- `narration_dedupe` / `context_pipeline::tests` — a five-step turn whose narration repeats
  verbatim while every tool call differs reaches the model with exactly one copy and four markers,
  with the call/result pairing intact; the earliest copy is the one kept; a repeated injected
  instruction collapses too; short acknowledgements are left alone.
- `stall_guard::tests::pressure_rises_on_the_first_restatement_and_stays_flat_for_healthy_work` —
  pressure is 0 across eight genuinely different narrations, 1 on the first verbatim restatement,
  2 when it persists.
- `tests::repetition_pressure_changes_the_request_not_just_the_instruction` — the ladder is
  monotonic, stays within a coding-usable range, and sends nothing at pressure 0.
- `wire_params::repetition_penalty_tests` — only penalty-capable providers are sent the fields,
  nothing is sent for an ordinary step, and Anthropic gets nothing even under pressure.
- `tests::continuation_stops_once_the_single_push_back_is_spent` — one push-back, then an honest
  stop; progress or a verified goal is accepted at any count.
- `stall_guard::tests::the_interleaved_live_pattern_eventually_nudges_then_halts` and
  `tests::stall_guard::a_model_cycling_among_a_few_phrasings_is_nudged_then_stopped` (from the
  previous change) still pass: the backstop is intact.
