# Advisor review (`[harness.advisor]`)

When a direct-API turn that changed something is about to finish, a second model reads the turn's
transcript and the candidate answer once. It returns at most 4 blocker/concern notes and one
suggested check. If it raises anything, the turn goes back to the agent once, through a user-role
`Nudge`. If it raises nothing, the turn finishes as it would have. Default off.

```toml
[harness.advisor]
enabled = true
model = ""          # "" = the model driving the turn
timeout_secs = 120
```

## Which model

Use a Claude Sonnet-class model through the API (`anthropic::` or `bedrock::`), not a CLI bridge:
bridges and subscription plans (ChatGPT, Kimi, OpenCode Go) never review, and a configured model
that is a bridge, a subscription, privacy-denied or benched falls back to the turn's model. If the
turn's model is itself a bridge or subscription, the review is skipped.

Replay evidence (13 baseline Terminal-Bench trials: 11 that finished wrong, 2 that passed; 3 runs
each, same rendered input as this feature, no tools):

| Reviewer | Unparsable or truncated | Passing trials flagged `blocker` | Names the verifier's failing requirement |
| --- | --- | --- | --- |
| Claude Sonnet | 0 of 39 | 0 of 6 (concern only) | 5 of 11 trials |
| Claude Haiku | 1 of 39 | blockers on both passing tasks | no discrimination |
| deepseek-v4.1-flash | 5 of 13 | not measurable | misses the clearest case |

A small same-model reviewer spends its whole 8k output budget on hidden reasoning for large
transcripts, so the default (the turn's own model) is only as good as that model. Raw outputs and the
method are in the PR description. This replay does not prove the advisor raises pass rates; that
needs the A/B on the target tasks.

## Behaviour

- One checkpoint (DONE), one call, one possible re-drive per turn. The re-drive's final reply is not
  reviewed. Medium effort, 8192 output tokens, `timeout_secs`.
- Not reviewed: read-only turns (no edit or mutation this turn), turns with unfinished tasks,
  turns past a deadline or forced stop, and turns at a spend cap.
- A provider error, timeout, unparsable or truncated reply publishes the original answer, writes
  one `advisor/done:<outcome>` usage row and one warning. It never benches a model.
- In a non-git directory the original answer is always kept after a re-drive.
- Workflow, daemon-dispatched and goal sessions run the same loop, so with the flag on they get one
  review per turn. Subagents never reach the hook.
- Independent of `[mesh] harness_profile`: an enabled flag wins over `minimal`.

## Surfaces

No new UI. The review shows as an `AuxiliaryRequest` ("advisor review") in the TUI, headless output
and remote and app clients; a re-drive and any failure show as `Warning`; a clean pass is a local
`Progress` line. The critique itself is a harness nudge in the transcript. The three keys appear in
TUI and app settings.
