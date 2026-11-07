# PR review gate

Nothing merges unreviewed. When PR Watch (merge mode: a ship, the merge queue's head, an issue
drain) is about to merge a pull request, the daemon first sends the PR's diff to an independent
reviewer. Confirmed defects hold the merge and go back to the PR's own session as a fix turn;
the review runs again on the head the session pushes; a clean review lets the merge go ahead.

## What happens

1. PR Watch reaches its merge decision (checks green, no unresolved comments, not held, no
   conflicts). Before it merges, `serve_pr_review::gate` runs. The gate sits after that decision
   for every method, so GitHub auto-merge (`method = auto`) is gated too: `gh pr merge --auto`
   is not called until the head is clean.
2. The reviewer gets `gh pr diff` for the PR (capped at 150 KB), once per head SHA. A review
   that already exists for the current head is reused, never repeated.
3. Findings are capped (`review_max_findings`, worst first). Those at or above `review_severity`
   are *blocking*; the rest are kept as advisory and shown, not sent.
4. Blocking findings produce one fix turn listing each as `[severity] file:line: title`, why it
   is a problem and the suggested fix. It is sent when the session is free; a busy session is
   waited for. After the push the next poll sees a new head and reviews again.
5. After `review_max_rounds` fix rounds (default 3) with blocking findings still present the
   watch is blocked with `review findings unresolved after N fix rounds`, and the owner gets a
   high-priority push like any other blocked PR.
6. A clean review (or none at or above the threshold) lets the merge proceed.

The reviewer is Assay's critic crew restricted to the correctness and unsafe lenses, with each
candidate checked by an independent adversarial verifier; only survivors count. Fix mode never
reviews (nothing is about to merge).

## Reviewer failure

A reviewer that errors, times out, has no model, or cannot read the diff is not a clean review.
By default (`review_on_error = "block"`) the PR is not merged, the reason is shown, and the review
is retried no sooner than 5 minutes later. `review_on_error = "merge"` lets the merge through and
records `review failed (...); merging anyway` as the last action.

## Config

```toml
[pr_watch]
review = "auto"            # or "off": merge exactly as before
review_model = ""          # empty: [harness.expert].model when set, else the mesh's configured tiers
review_budget = ""         # "$2" caps one review's estimated cost; subscription models require it
review_max_findings = 5
review_severity = "high"   # low | medium | high | critical
review_on_error = "block"  # or "merge"
review_timeout_secs = 600
review_max_rounds = 3
```

A subscription reviewer (`claude-cli::`, `codex-cli::`, `codex-oauth::`) is refused without a
budget. Only the dollar form (`$2`) is enforced per review today (against Assay's pre-run cost
estimate); a window slice (`codex:5%w`) is accepted as the required opt-in but is not metered
yet, unlike the expert's envelope.

## Where it shows

- PR Watch rows and the phone/desktop PR panel: `review: 2 confirmed, fixing`, `review clean`,
  `review failed, will retry`, plus the blocking findings (file:line, title) in the panel.
- The merge queue (`GET /api/pr/queue` rows carry `review`): the mobile queue screen, `forge prs
  queue`, TUI `/queue` and the board's queue section show it beside the row's reason.
- `forge prs review <pr|session>` prints the verdict, model and every finding with file:line and
  why; `GET /api/pr/review?pr=|session=` serves the same.
- A push (`PR #7 blocked by review: 2 confirmed defect(s)`), once per head, when findings hold a
  merge.

## Where it lives

The verdict is stored on the watch record in `pr-watch.json` (`review`), not in the database: no
migration. Re-arming a watch clears it. The review runs inline in the daemon's PR Watch tick, so
a slow reviewer delays other watches' polls by up to `review_timeout_secs`.
