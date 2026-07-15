# Forge — unfinished work (session of 2026-07-11)

Everything that was still open when the overnight wave stopped. Standing pre-session
deferrals (Fly relay, helm install, deferred-features note) are intentionally excluded.

Context: 20 PRs shipped (#588–607). The two adversarial-audit Workflows were the highest-yield
move — a 47-agent deep audit confirmed 31 real bugs. HIGH (3) shipped in #606; 6 medium subsystems
in #607. What remains is below, most-actionable first.

---

## 1. Deep-audit bugs found but NOT fixed

### 1a. Mediums dropped from #607 (verified real; ready to implement)

- **mesh — conservation penalty should be provider-scoped.** `crates/forge-mesh/src/catalog.rs:857`.
  Conservation decides on the most-pressured subscription but applies `CONSERVE_PENALTY` to EVERY
  subscription, so a fresh/high-headroom sub is abandoned whenever any sibling is pressured.
  FIX: compute the spread decision per subscription provider (its own `effective_fraction_for`/plan)
  and only penalize models whose OWN provider's roll fired.
  ⚠ The Workflow's attempt at this + the two below broke `a_pace_projecting_near_exhaustion_ramps_conservation_like_a_full_window`; redo carefully and keep that test green.
- **mesh — conservation is a no-op at High/XHigh/WhiteHot effort.** `catalog.rs:891`. The ranked
  sort's PRIMARY key is `bench_band`; `CONSERVE_PENALTY` only moves the secondary `route_score`.
  FIX: fold the conserve penalty into the `bench_band` comparison, or drop fired subscriptions from
  the candidate set before the sort. Decide + document whether high effort overrides conservation.
- **mesh — `gemini::gemini-2.5-flash` misclassified paid, not free.** `catalog.rs:81`. `DEFAULT_RATES`
  prices it, so `is_free`'s `cost > EPSILON` early-return preempts the gemini free-tier branch.
  FIX: move the standing-free-tier determination (ollama/groq/gemini-non-pro/custom.free) ABOVE the
  `cost > EPSILON` gate so a bundled list price can't override a known free tier.
- **serve — `stop_and_join` skips the wait under concurrent Arc holders.** `crates/forge-cli/src/serve.rs:859`
  (+ `archive_session`). The wind-down wait only runs when `Arc::try_unwrap` succeeds; a concurrent
  `/api/sessions` poll holding a clone silently skips it, so git can run on a still-live worktree.
  FIX: store the driver `JoinHandle` behind a `Mutex<Option<JoinHandle>>` (or a completion `Notify`
  the driver fires on exit) so ANY Arc holder can await real task exit; call `shutdown()` then await
  it unconditionally, bounded by `ARCHIVE_JOIN_TIMEOUT`.
- **serve — orphaned worktree+branch leak on spawn failure.** `serve.rs:527`. `WorktreeGuard` is
  `mem::forget`-ed BEFORE `spawn_session_driver`, so a spawn failure leaves the worktree/branch.
  FIX: defer `std::mem::forget(guard)` until AFTER the spawn returns Ok (keep the guard live so an
  early Err drops it and RAII-cleans up).
- **remote — N clients re-serialize the identical Snapshot.** `crates/forge-cli/src/remote.rs:1687`.
  Each WS client serializes the same Snapshot to JSON independently (N clients = N serializations/frame).
  FIX: serialize once in `RemoteControl::broadcast` into a shared `Arc<str>` carried on the watch value;
  each forward task sends the shared buffer. Keep the per-client `resync` marking correct (conflicts
  with the #602 forward-loop change — rebase onto it).
- **tui perf (whole batch — the agent that owned these failed on the output cap):**
  - `crates/forge-tui/src/app.rs:2842` — in-flight reply is markdown-parsed + re-wrapped over its
    ENTIRE length on every delta (O(reply²)); the cache key is `len` (misses same-length edits).
    FIX: throttle the full re-parse (re-render markdown every N ms/N bytes, cheap plain-text tail
    between) and switch the cache key to a monotonic streaming revision counter.
  - `crates/forge-tui/src/render.rs:157` — `diff_to_lines` builds a new syntect `HighlightLines`
    and does a full syntax-set lookup per diff line. FIX: resolve the syntax once, reuse it.
  - `crates/forge-tui/src/transcript.rs:62` — standalone transcript viewer wraps the whole selected
    view twice per poll frame, no cache. FIX: cache wrapped rows keyed on (selected, width, content-rev).
  - `crates/forge-tui/src/app.rs:4380` — `render_input` always scrolls to the bottom rows, ignoring
    the cursor, so the cursor is invisible when editing the top of a tall input. FIX: cursor-aware
    scroll offset (keep the cursor's wrapped-row within [scroll, scroll+visible_rows)).

### 1b. LOW-severity confirmed (none fixed)

- `oauth_responses.rs:364` — a final SSE event lacking the trailing blank-line terminator is never
  parsed (a completion with no `\n\n` suffix is dropped). FIX: flush the remainder after the loop.
- `serve.rs:917` — on a merge CONFLICT, `merge_session` still removes + archives the session, so the
  deliberately-preserved worktree/branch becomes non-resumable. FIX: on `Conflicts`, leave the driver
  running (or re-insert the handle).
- `store/lib.rs:1809` — `record_quota` writes the `subscription_usage` upsert and `quota_history`
  insert as two autocommit statements. FIX: wrap both in one `transaction_with_behavior(Immediate)`.
- `store/lib.rs:1131` — seq-collision recovery assumes any UNIQUE/PK violation is the (session_id, seq)
  index; a `PRIMARY KEY(message.id)` violation would wrongly re-allocate seq forever. FIX: constrain
  the recovery arm to the seq index + a bounded attempt counter.
- `mobile/src/lib/queries.ts:209` — `useTurnCompleted` starts an iOS Live Activity with no unmount
  cleanup, orphaning the activity (+ its push subscription) when you leave a session mid-turn.
  FIX: return a cleanup effect that `endLiveActivity` if `activityIdRef.current` is set.
- `mobile/src/components/chat/Markdown.tsx:247` — inline spans re-parsed every render (unlike memoized
  blocks). FIX: fold inline parsing into the same `useMemo` as blocks.
- `forge-tui/src/app.rs:2862` — `streaming_edge` clones every wrapped row each frame though callers
  render only `body_h`. FIX: pass a (start,len) window and clone just those rows.
- `catalog.rs:301` — `subscription_burn_penalty` inverts sign for a config burn-weight in (0,1) (guard
  is `weight <= 0.0` but `ln(weight)` is negative for 0<weight<1). FIX: guard `weight <= 1.0` (or clamp
  to a ≥1.0 floor before `ln`).
- `catalog.rs:437` — `conserve_probability` contradicts its "Trivial always spreads" contract (the
  Trivial base 1.0 is multiplied by plan_factor 0.8–0.85). FIX: exempt Trivial from plan_factor, or
  fix the doc.
- `forge-core/src/lib.rs:3575` — `empty_nudges` accumulates across the whole turn, reset only on model
  switch, never after a productive step. FIX: reset `empty_nudges = 0` after any non-empty / tool-call
  response so only a run of empties-making-no-progress trips the cap.
- `forge-core/src/lib.rs:3832` — when the context-overflow compact-retry budget is spent and the error
  is a non-retryable `Request`, the turn hard-fails instead of failing over. FIX: after the budget is
  spent, route a still-overflowing error into bench+failover.

---

## 2. Design-plan items designed but NOT built

From the Fable design Workflow's ranked 12-item plan. Shipped: #1 WS resync (#602), #2 statusline
(#603), #3 gauge correctness (#605), #10 partial (#604 did thermal-pulse + friendly cwd + tappable
NEEDS YOU). **Not built:**

- **#4 [sol] Session management from `/sessions`** — archive (Del) + reveal-archived (Tab) +
  saved-title display. Needs a `store::unarchive_session` + `show_archived` sourcing in the picker.
- **#5 [sol] Unified `/help` + F1 reference viewer** — one screen with commands + keybinds (via
  `keybinds::combo_display`) + fixed nav keys. New `crates/forge-tui/src/help.rs`.
- **#6 [terra] Client live-session resilience** — half-open socket watchdog, incoming-frame
  validation, guarded control inputs. (Builds on the non-churning socket from #602.)
- **#7 [luna] Global foreground fleet-awareness watcher** — cross-session waiting/done alerts while
  the app is foregrounded (not just push).
- **#8 [terra] Offline decision queue for permission/question ANSWERS** — with a `prompt_seq`
  staleness guard. (We did offline queueing for prompts; ANSWERS still drop offline.)
- **#9 [luna] Fleet server switcher** — live per-server reachability + waiting counts.
- **#10 [luna] "Bed of Coals"** — full living Emberline command deck for the Fleet home (only a
  partial pass shipped in #604).
- **#11 [terra] "Heat Rail & Ember Spine"** — session screen as a living forge bench (gauge
  truncation already fixed in #601; the full overhaul is unbuilt).
- **#12 [luna] "The Floor" (capstone)** — a live multi-session control room, new tab in
  `mobile/src/app/(tabs)/`, every burning agent streaming on one wall. Zero server changes (protocol
  v7 as-is).

The three "bold overhaul" designs (`The Floor`, `Bed of Coals`, `Heat Rail & Ember Spine`) have full
concrete-change specs in the design Workflow journal
(`wf_9469c1c3-9ce/journal.jsonl`) if a fuller brief is wanted.

---

## 3. The UX ambition (biggest gap vs the stated goal)

The goal was "insanely good, beautiful, animated, stunning, super easy to use." We shipped real
bug-fixes + motion/fleet polish (#597, #599, #604) and the reliability spine — but the **big
delight/overhaul pass was only DESIGNED, never built** (items #10/#11/#12 above). That is the single
largest piece of the original intent still outstanding.

---

## 4. Dispatched Forge work that failed mid-flight

- **Session-screen "turn-failure banner" fix (from your screen recording)** — luna's wave failed
  mid-edit and was discarded. **A hard turn-failure still renders as a heavy persistent bottom
  banner that reappears on every session reopen**, instead of a dim in-transcript row. Unfixed.
- **Command palette unreachable on iOS/Android** (first-audit rank 9) — `PaletteHost` is only
  reachable via a web/desktop keybind; mobile has no entry point. Never built.

---

## 5. Reserved final step (not triggered)

- **Xcode Cloud mobile build** — reserved for right-before-deadline, once. The deadline passed while
  the medium-fix apply was stuck, so it was **never triggered**. Kick off manually when ready
  (EAS/Xcode Cloud, per `mobile/BUILD_PLAN.md` §9a — unsigned CI path needs no Apple secrets).
