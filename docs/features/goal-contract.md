# Feature: `/goal --contract`, a validation contract and an independent validator

> **Status: shipped (slice S1 of Missions).** Off by default. `/goal --contract <objective>`
> turns it on for one goal; `[goal] contract = true` turns it on for every goal.

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
- `crates/forge-cli/src/remote/goal.rs` - `SnapContract`; `mobile/src/lib/goalState.ts` and
  `GoalBanner.tsx` - the table.

## 7. Not in this slice

Fresh worker sessions per milestone and milestone sequencing. A dedicated `/contract` overlay
(the table is in `/goal status` and the goal panel), and the "contract" toggle in the goal start
sheet on the phone: the phone starts a contract goal by sending `/goal --contract <objective>`.
