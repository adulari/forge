# Feature: `/goal --contract`, a validation contract and an independent validator

> **Status: shipped (slices S1 to S3 of Missions).** Off by default. `/goal --contract <objective>`
> turns it on for one goal; `[goal] contract = true` turns it on for every goal. Sections 7 and 8
> add the fresh-worker relay and missions (`/goal --mission`).

## 1. Problem (JTBD)
> When I hand Forge a big goal and go to bed, I want what "done" means written down before the
> work starts, and something other than the worker proving it at the end — not a cheap model
> reading the git log and saying "achieved".

The [goal evaluator](goal-evaluator.md) is a trivial-tier side call with no tools. It can judge
progress, but it cannot run anything, so it can be talked into "achieved" by a confident commit
message. `--gate` commands are checked, but only the ones the person remembered to pass.

## 2. Behaviour

1. **Draft.** `/goal --contract <objective>` first makes one side call (the evaluator's model and
   safety rules) that writes 3 to 12 numbered assertions. Each has a statement and a concrete
   check: a read-only shell command, an optional expected outcome, and an optional text the
   output must contain. A behaviour or file statement is written as a command
   (`test -f path`, `grep -q 'text' path`, `git log --oneline | grep -q 'subject'`). Assertions
   without a command are dropped; commands that are not provably read-only trigger one redraft
   and are then dropped. With fewer than three left the goal does not start.
2. **Approve.** The contract is shown as a card (the plan card, with the check under each
   assertion) and the person answers Approve, Cancel, or types changes, which redraft it (three
   revisions at most). It is the same question path as `present_plan`, so the TUI question form,
   the phone and the desktop all answer it. Nobody to ask (a headless run, a daemon goal with no
   client attached) or Bypass mode: the card is shown and the run goes on. Cancel ends the goal
   before any work (`🎯 goal not started — the contract was declined`).
3. **Work.** The approved assertions are primed on the session as guidance, so the worker builds
   toward the checks. Everything else is the normal `/goal` loop, `--gate`, budgets and envelope
   included.
4. **Validate.** After each worker turn, in place of the evaluator, the validator runs. It makes
   no model call: it executes every assertion's command itself through the session's `shell`
   tool. Each line first goes through the permission broker in plan mode, which lets a command
   pass only when it is provably read-only and no deny rule matches; anything else fails that
   assertion with `refused`. A check has `[goal] contract_check_timeout_secs` (default 60); one
   that outruns it is killed and fails with `timed out after Ns`. A check passes when it exits 0
   and, if the assertion has `contains`, its output contains that text. The result per assertion
   is pass or fail, the reason, and the tail of the output (600 characters) as proof.
5. **Decide.** The goal is achieved only when every assertion passes (the `--gate` commands then
   run as usual). Anything else goes back to the worker as the next prompt:

   ```
   Goal: ship the parser
   The independent validator ran the goal contract: 2 of 3 assertions pass. Fix what fails, then stop; ...
   assertion 2 failed: cat b.txt -> cat: b.txt: No such file or directory
     (b.txt says ready)
   ```

   The worker's own "done", a finished task plan, or an evaluator verdict cannot end a contract
   goal. The same failing set on three validations in a row stops it as wedged
   (`🎯 goal wedged — assertions 2, 3 still fail after 3 validations`); the iteration ceiling,
   `--max-tokens`, `--max-minutes` and the spend envelope apply unchanged.

The validator only reads, so a contract cannot express "the tests pass" unless the test run is
read-only. Put builds and test suites in `--gate`; use the contract for the facts the gates do
not cover.

## 3. Where it shows

- **TUI:** `/goal status` prints the table under the goal (mark, number, statement, `check:`,
  `expect:`, `why:`, `proof:`); the composer chip adds `contract 2/3 pass`; each validator run
  prints `📜 contract 2/3 pass · failing: 2`.
- **CLI:** `forge goal show [SESSION] [--json]` prints the goal state and the contract table of
  the newest goal on record (or the named session). It reads the store, so it works the morning
  after, with no daemon running. Goals run by the daemon (`forge serve`, the phone, the desktop)
  are recorded; a goal in a plain `forge chat` is not.
- **Snapshot:** `Snapshot.goal.contract` (`headline`, `approved`, `rounds`, `passed`, `total`,
  `checked_at`, and per assertion `id`, `text`, `command`, `expect`, `state`
  `pass|fail|pending`, `reason`, `proof`), additive within the protocol.
- **Phone and desktop:** the goal panel draws the contract as a table under the goal: a mark per
  assertion, its text, the check, and the failure reason with the last line of proof.

## 4. Persistence

The contract and its latest results live in the `session.goal_state` JSON (migration 53), next to
the envelope record, written by the daemon driver. A goal with a contract keeps its record when
it ends, so a restart restores the finished goal's table; a running or paused goal comes back
paused and `/goal resume` continues against the same contract. No new table.

## 5. Config

```toml
[goal]
contract = false                  # true: every /goal drafts a contract (same as --contract)
contract_check_timeout_secs = 60  # per check, clamped 1..600
relay_after_compactions = 4       # hand the goal on once the session was compacted this often; 0 = never for wear
max_relays = 3                    # fresh-worker hand-offs one goal may make; 0 = relay off
mission_max_milestones = 6        # most milestones a /goal --mission may have, clamped 2..12
```

## 6. Implementation

- `crates/forge-types/src/goal_contract.rs` - `GoalContract`, `parse_contract` (tolerant JSON
  like the evaluator), the card steps, the worker guidance and the failure prompt.
- `crates/forge-core/src/goal_contract.rs` - `Session::request_goal_contract`, drafting and the
  approval question (run at the start of the first turn, from `hooks_turn.rs`), and
  `Session::validate_goal_contract`.
- `crates/forge-cli/src/cli/commands/run/goal_progress.rs` - `GoalNext::Validate`,
  `contract_decision`, the wedge rule; `goal_headless.rs` does the same without an `App`.
- `crates/forge-cli/src/cli/commands/run/goal_envelope.rs` (`GoalRecord`) and `goal_show.rs`.
- `crates/forge-store/src/relay_store.rs` and migration 58 - the lineage, the compaction count.
- `crates/forge-core/src/goal_relay.rs` - the brief and `Session::hand_off_goal`;
  `recall.rs` reads the lineage. `crates/forge-cli/.../run/goal_relay.rs` - `perform`,
  `successor_outcome`; `serve_relay.rs` hosts the successor.
- `crates/forge-cli/src/remote/goal.rs` - `SnapContract`; `mobile/src/lib/goalState.ts` and
  `GoalBanner.tsx` - the table.

## 7. Fresh-worker relay

> Slice S2 of Missions. On by default (`max_relays = 3`); `max_relays = 0` turns it off.

A long run used to live and die in one session: a goal that wedged simply stopped, and the owner
recovered it by hand (a new session on the same worktree, the task re-sent, a note on what the dead
one had produced). Now Forge does that itself.

**When.** The goal hands itself to a fresh worker session when it

- wedges: the evaluator saw no movement for three verdicts, the agent claimed done against the
  evaluator's "not achieved" too often, or the contract validator saw the same failing set three
  times in a row (`wedged`);
- stalls: six turns without task progress (`stalled`; the expert on call has already had its
  say by then);
- or the session is worn out: it was compacted `relay_after_compactions` times (`worn`). The
  count is stored with the compaction (migration 58), so a daemon restart does not reset it.

A spent envelope, a budget cap, the iteration ceiling, a declined contract and an interrupt still
stop or pause the goal as before: a new worker would only hit the same wall. After `max_relays`
hand-offs the goal stops with its original wedge or stall message. The headless local
`forge run "/goal ..."` has no driver to host a second session and stops as before.

**What happens.**

1. A new session is created on the same cwd and worktree, with the same permission mode and
   model/effort pin, titled `<title> (worker N)`. It is linked to the old one in `session_relay`.
2. The old session's last turn ends `handed_off` (`forge turns` shows "goal handed to a fresh
   worker"; the reason it had before is kept in the turn detail as `was`). Its goal reads
   `handed off`.
3. The successor's goal record carries the goal, the contract with its latest results, the spend
   envelope, and the iteration and token counters, so the budget is one budget across workers.
4. The successor's first prompt is a deterministic brief, read from git, the contract, the task
   list, the compaction index and the stored turns (no model call):
   commits since the goal started and uncommitted files; the contract's passing assertions and the
   failing ones with the check, the reason and the proof tail; open tasks; the old session's
   episode index (the seq ranges its compactions folded); how its last turn ended; the expert's
   diagnosis when one was given; the envelope; and a pointer to `recall`.
5. The successor's `recall` searches the old session's whole history and reads it with
   `session=<id prefix>`, and so on up the chain. Nothing else is reachable: an unrelated
   session is refused.

**Where it shows.**

- TUI: `/goal status` prints `S1 handed off (wedged) -> S2 running`. In a terminal chat the
  screen switches to the successor and carries on; under `forge serve` the daemon hosts the
  successor as a new session in the fleet.
- `forge turns` and `/turns`: the `handed_off` ending. `forge goal show` prints the chain.
- Snapshot: `Snapshot.goal.relay` (`session_id`, `state`, `reason`) and state `handed_off`.
- Phone and desktop: the goal panel shows the chain; a handed-off goal has no controls.
- Push: "goal handed to a fresh worker (wedged)".
- `/contract` (TUI): the contract as a table (pass or fail per assertion, the last line of
  proof); Enter prints the check and the full proof.

**Restart.** The successor is flagged as a fleet session when it is created, and it has no
messages until its first turn. A daemon that dies between the hand-off and that first turn brings
the successor back, finds its goal record and its stored brief, and starts the goal from it; after
the first turn it is an ordinary session and comes back paused like any other goal. A predecessor
whose own record was not yet written is shown as handed off because the relation exists.

## 8. Missions (milestones)

> Slice S3 of Missions. `/goal --mission <objective>` (and the Mission switch on the phone's goal
> start sheet, which sends the same line). Off unless asked for.

A big goal as one flat list of assertions makes one session carry the whole job. A mission splits
it into milestones that run in order, each by a fresh worker with a narrow context.

**Draft and approve.** One side call (the contract drafter's model and safety rules) writes
2 to `mission_max_milestones` (default 6) milestones, each with a title and 2 to 6 assertions
under the same rules as section 2: a concrete read-only check, unrunnable or write checks dropped
(one redraft first). A milestone left with fewer than two assertions is dropped, and fewer than two
milestones is no mission (the goal does not start). The whole mission is shown on one approval
card, a step per milestone listing its checks; Approve, Cancel or type changes (three revisions)
work as for a contract, and an unattended run (headless, no client, Bypass) accepts the card as it
does for a contract.

**Run.** A mission is one `GoalContract` whose `mission` field says which assertions belong to
which milestone (assertions are numbered once across the mission) and which milestone is current.
Milestone 1 runs in the current session. After each worker turn the validator runs the assertions
of the current milestone and of every earlier one; it makes no model call, as before.

- Anything failing goes back as the next prompt. A broken assertion of an earlier milestone is a
  regression: the prompt says so, lists it first and tells the worker to fix it before finishing
  the milestone. Because earlier milestones are re-checked at every validation, the regression
  check always happens at the boundary too.
- When the current milestone and all earlier ones pass, the mission moves to the next milestone,
  which starts in a fresh worker session through the relay of section 7 (reason `milestone`,
  detail `milestone N passed`). The brief is the same deterministic brief, plus the mission line
  (`you are on milestone 2/4 - <title>`, the later milestones' titles) and only the active
  assertions. The goal record handed to the successor already names the new current milestone.
- The mission is achieved when the last milestone is current and every assertion of every
  milestone passes; the `--gate` commands then run as usual.
- A milestone hand-off is not a failure: it does not count against `max_relays`, and a wedge still
  relays up to that cap. With `max_relays = 0`, or in a headless `forge run "/goal --mission ..."`
  that has no driver to host a second session, the next milestone starts in the same session with
  a prompt naming what passed and what the next milestone must prove.
- The wedge rule, budgets, the envelope and the iteration ceiling apply across the whole mission.

**Where it shows.**

- Progress line `milestone 2/4: 3/5 pass` (the current milestone's tally): the composer chip,
  `/goal status`, the `/contract` overlay title, `forge goal show`, the snapshot, the phone and
  desktop goal panel.
- The table groups assertions under `milestone N/M - <title> [done 2/2 | current 1/3 pass |
  upcoming, 3 assertions]`; the `/contract` overlay prefixes each row `M2/4`.
- Snapshot: `Snapshot.goal.contract.mission` (`current`, `total`, `progress`, and per milestone
  `index`, `title`, `state` `done|active|upcoming`, `passed`, `total`, `assertion_ids`); absent for
  a flat contract. The phone panel draws a `Mission` line and a header per milestone.
- Push at each boundary: "mission: milestone 1/4 passed - the next one starts in a fresh worker".

**Persistence.** The mission lives inside the contract in `session.goal_state` (no new table, no
migration): the milestone list, which assertions belong to each, the current milestone and the
latest results. A daemon restart restores the goal paused on the same milestone, and a successor
created at a boundary but not yet started comes back from its stored brief and goal record as in
section 7.

**Code.** `crates/forge-types/src/goal_contract/mission.rs` (model, tolerant parser, prompts, the
milestone-aware table), `crates/forge-core/src/goal_mission.rs` (draft and approval),
`crates/forge-cli/src/cli/commands/run/goal_mission.rs` (the `--mission` flag, the step to the
next milestone; hooks in `goal_progress.rs`), `RelayReason::Milestone` in `goal_relay.rs`.

## 9. Not in this slice

Relaying a goal that has no driver (headless `forge run`; its missions run in one session). Editing
a running mission's milestones. The relay settings are config only.

## 10. Behaviour assertions (QA)

> **Experimental.** The QA step is new and has had one scripted drill and no field use. Its
> contract and defaults may change.

A second assertion kind for goals about something that runs ("the server answers /health", "the
CLI prints X"). The read-only validator cannot check those, and the worker must not grade itself,
so a QA runner does it.

```json
{"assertion": "/health says ok v2",
 "behaviour": {"run": "python3 server.py $FORGE_QA_PORT",
               "probe": "GET http://127.0.0.1:$FORGE_QA_PORT/health",
               "expect": "ok v2", "cleanup": "optional", "ready": "optional: port | line:<text>"}}
```

- **When.** After every read-only check passes. Until then a behaviour assertion fails with
  `not run: the read-only checks must pass first`.
- **Run.** `run` starts in the session workspace in its own process group with `FORGE_QA_PORT` (a
  free port) and `FORGE_QA_DIR` (a scratch directory, removed afterwards). Output goes to a log.
  The permission safety floor and deny rules still apply (`refused` otherwise); the approved card
  showed the commands. Timeout: `contract_check_timeout_secs`.
- **Probe.** `GET <url>` (built-in HTTP client), `screenshot <url>` (the `browser` tool, headless;
  the image is filed in the session evidence folder), or any shell command (exit 0 = answered).
  Without `ready` the probe is retried until it answers; with `ready` it runs once the port opens
  or the log line appears. A run that exits non-zero fails with its log as proof; one that exits 0
  is a setup step.
- **Proof.** The probe output tail (and screenshot path) is stored in the result and fed back as
  the failure prompt. Cleanup always runs, then the whole process group is killed.
- **Unverified.** A probe that cannot run (a `screenshot` probe with no browser tool) is not a
  pass. The result carries reason `unverified: <reason>` and proof `skipped: <reason>`; it does not
  block the goal (a missing tool cannot wedge it) but is listed apart everywhere: mark `?` and
  `UNVERIFIED` in the table, `/contract` and `forge goal show`, `unverified` as the snapshot state,
  `(N unverified)` in the headline, and `· unverified (probe could not run): N` on the validator's
  progress line.
- **Shows.** `/contract` (full proof, `screenshot:` path), `/goal status`, `forge goal show`,
  `Snapshot.goal.contract.assertions[].artifact|behaviour`, and the phone/desktop goal panel (proof
  line plus the screenshot through the evidence image route).
- **Drafting.** The drafter is told to propose behaviour assertions for runnable goals.
- **Code.** `forge-types/src/goal_contract/behaviour.rs`, `forge-core/src/goal_qa.rs`.
