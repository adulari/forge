# All Open Issues Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Resolve every currently open Forge issue (#832–#845, #850, and #861) in one production-ready pull request, with issue-specific automated regressions and real acceptance-path evidence before closure.

**Architecture:** Keep the persisted audit transcript lossless while deriving bounded provider and user-facing views; make session lifecycle decisions from explicit state (prior permission, matching question, meaningful tool progress, and check outcomes), not inferred strings or UI timing. Add narrow platform adapters for desktop version/host identity/Linux recording, and make release workflows publish only complete, verifiable artifacts.

**Tech Stack:** Rust workspace (Tokio, Axum, SQLite), React Native/Expo Router/TypeScript, Tauri, Bash/GitHub Actions, EAS Update, GitHub Releases.

## Global Constraints

- One aggregate branch and one pull request from `fix/all-open-issues`.
- Write and observe each regression test failing before its production change.
- Keep direct/LAN operation available; never require Forge Anywhere for local use.
- Do not expose credentials, recovery data, host identifiers, or account data in logs or URLs.
- Do not close an issue until its real acceptance path has fresh evidence.
- Native mobile behavior must be verified on a physical installed build/OTA, not browser emulation.
- Preserve the dirty primary checkout; work only in `/tmp/forge-all-open-issues`.

---

### Task 1: OTA-safe change classifier (#861)

**Files:**
- Modify: `.github/workflows/eas-update.yml`
- Create: `scripts/ci/eas-ota-safety.sh`
- Create: `scripts/ci/test-eas-ota-safety.sh`

**Interfaces:**
- Produces workflow outputs `safe=true|false` and `ota_changed=true|false`.
- Accepts `EVENT_NAME`, `BASE_SHA`, and `HEAD_SHA`; positional paths are the deterministic test seam.

- [ ] Cherry-pick commit `8d65d2ca`, then run `bash scripts/ci/test-eas-ota-safety.sh` and require `EAS OTA safety classification passed`.
- [ ] Inspect the classifier against these cases: source/asset publishes; docs/tests/root metadata neutral; native/config/dependency paths block; unknown `mobile/*` blocks; all-zero/missing base blocks; diff failure exits non-zero; manual dispatch publishes only through the runtime pin.
- [ ] Amend only if one of those cases lacks a failing regression.
- [ ] Live-verify with a docs-only workflow run (neutral outcome) and an OTA source change (published update group), then comment/close #861.

### Task 2: Plan approval restores exact permissions and commits tasks only after approval (#832)

**Files:**
- Modify: `crates/forge-core/src/lib.rs`
- Test: `crates/forge-core/src/lib.rs` test module

**Interfaces:**
- Add `pre_plan_temper: Option<Temper>` to `Session`.
- `set_temper(Plan)` captures the previous non-Plan mode once.
- `resolve_plan_approval` returns Build/Revise/Cancel behavior without discarding the captured mode prematurely.

- [ ] Add failing tests proving Build and Cancel restore `AcceptEdits`, `Full`, and any other prior non-Plan mode; fallback is `AcceptEdits` only when no prior mode exists.
- [ ] Add a failing test proving `ingest_plan` does not seed `Session.tasks` or persistent tasks before Build.
- [ ] Implement captured-mode restoration; Revise remains in Plan; seed tasks only in the Build branch.
- [ ] Run the targeted core tests, then a scripted session that enters Plan from Full, approves Build, executes a tool, and finishes without another plan prompt.
- [ ] Live-verify the same flow through one remote UI before closing #832.

### Task 3: Bind Plan controls to the matching live question (#833)

**Files:**
- Create: `mobile/src/lib/planDecision.ts`
- Create: `mobile/src/lib/__tests__/planDecision.test.ts`
- Modify: `mobile/src/components/review/PlanCard.tsx`
- Modify: `mobile/src/app/session/[id]/review.tsx`
- Modify: `mobile/src/app/(tabs)/plans.tsx`

**Interfaces:**
- Export `resolvePlanDecision(planTitle, question, promptSeq): { build: string; cancel: string; promptSeq: number } | null`.
- Return `null` unless `promptSeq > 0`, the question is live and corresponds to the shown plan, and explicit Build and Cancel options exist.

- [ ] Add failing tests for missing question, stale/zero sequence, unrelated question, missing Build/Cancel options, and a matching decision.
- [ ] Implement the pure resolver without synthesizing option numbers.
- [ ] Render a non-interactive “Waiting for approval request…” state until the resolver returns a binding; enable automatically when the matching question arrives.
- [ ] Run the focused mobile test and screen test; exercise delayed WebSocket question arrival in a live session before closing #833.

### Task 4: Bound provider context and retire stale contracts (#834, #841)

**Files:**
- Modify: `crates/forge-core/src/context_pipeline.rs`
- Modify: `crates/forge-core/src/context_pack.rs`
- Modify: `crates/forge-core/src/turn_contract.rs`
- Modify: `crates/forge-core/src/lib.rs`
- Test: corresponding Rust test modules

**Interfaces:**
- Add a provider-view normalization pass that retains the newest `Turn contract:` system message and one copy of exact repeated standing guidance.
- Do not delete or rewrite rows in the persisted audit transcript.

- [ ] Add failing deterministic tests with many turns proving the provider sees only the latest contract and one exact guidance copy while `load_all_messages` remains complete.
- [ ] Add a failing size test proving repeated turns converge to bounded provider-message count/token estimate.
- [ ] Implement normalization at the provider-view boundary, after persisted replay is loaded and before provider submission.
- [ ] Run context/contract/core tests and a long scripted session; inspect the actual outbound provider message roles/content before closing #834 and #841.

### Task 5: Remove nonexistent skill-list instruction (#835)

**Files:**
- Modify: `crates/forge-skills/src/lib.rs`
- Test: `crates/forge-skills/src/lib.rs` test module

**Interfaces:**
- Orchestrate guidance tells the model that available skills are already enumerated in the `use_skill` tool description and only valid skill names may be passed.

- [ ] Add failing tests asserting neither builtin command nor system guidance contains `use_skill list`.
- [ ] Replace both invalid instructions with the real discovery contract.
- [ ] Run `cargo test -p forge-skills` and execute `/orchestrate` once with tool tracing, confirming no call named `list`, before closing #835.

### Task 6: Verification evidence cannot be cleared by unrelated reads (#836)

**Files:**
- Modify: `crates/forge-core/src/completion.rs`
- Modify: `crates/forge-core/src/lib.rs`
- Test: both modules' Rust tests

**Interfaces:**
- Track verification command families and latest result separately from generic inspection.
- `cat`, `ls`, file reads, and `git diff` never resolve a failed build/typecheck/lint/test family.
- A later successful matching family may resolve it; exhausted budget ends explicitly unverified.

- [ ] Add failing tests for failed `tsc`, lint, test, and build followed by `cat`/`git diff`.
- [ ] Add failing tests for failure followed by a successful matching check and for bounded explicit-unverified termination.
- [ ] Implement the evidence ledger and update the verification nudge to request the unresolved check, not generic inspection.
- [ ] Run focused completion/core tests and a scripted failure→read→completion scenario before closing #836.

### Task 7: Retry recoverable Codex OAuth request failures (#845)

**Files:**
- Modify: `crates/forge-provider/src/oauth_responses.rs`
- Modify if required: `crates/forge-provider/src/codex_oauth.rs`
- Test: provider OAuth response tests and core retry tests

**Interfaces:**
- `classify_stream_error("provider request failed")` returns the transient/unavailable class.
- Existing core bounded backoff/failover handles the classification; no completed direct tool call is replayed.

- [ ] Add a failing classification test for the observed phrase and variants with provider prefixes/casing.
- [ ] Add a failing scripted-provider test proving one transient failure retries and resumes, and exhaustion emits one safe action.
- [ ] Implement the narrow classification and preserve bounded backoff/account hopping.
- [ ] Run provider/core focused tests; live-verify by injecting one recoverable continuation fault against a disposable local proxy or deterministic provider seam, then confirm automatic recovery before closing #845.

### Task 8: Tool-only bridge progress is not an empty reply and cannot idle (#842, #843)

**Files:**
- Modify: `crates/forge-core/src/lib.rs`
- Modify if required: `crates/forge-store/src/lib.rs`
- Test: core scripted-provider tests and store history tests

**Interfaces:**
- Classify a completion as `ToolProgress` when tools started during that provider invocation even if returned assistant text is empty.
- Empty-response nudges apply only when there is no text, structured tool call, or observed tool action.
- User history/replay contains tool actions/results but no blank Assistant item.

- [ ] Add a failing lifecycle test: bridge tool start/result → empty provider completion → unfinished stored task; assert automatic continuation and terminal `Done`.
- [ ] Add a failing replay/history test proving no empty assistant item and no “last response was empty” nudge for tool progress.
- [ ] Implement per-invocation tool deltas and route `ToolProgress` through the unfinished-task continuation gate.
- [ ] Preserve lossless provider linkage only where required; keep blank carrier rows out of all user-facing projections.
- [ ] Run focused tests and a real `codex-cli` bridge session with a tool-only intermediate step, verifying continuous progress and replay before closing #842/#843.

### Task 9: Publish exactly one terminal assistant answer (#844)

**Files:**
- Modify: `crates/forge-core/src/lib.rs`
- Modify if required: presenter event types/remote reducer tests
- Test: core scripted-provider and history tests

**Interfaces:**
- Completion text is provisional while verification requests another provider turn.
- Only the accepted terminal text emits as the final Assistant/Done answer; tool continuity remains in the internal transcript.

- [ ] Add a failing scripted test with “done” → verification nudge → inspection → repeated “done”; assert one user-facing terminal assistant and one Done event.
- [ ] Implement provisional-answer handling without breaking provider message ordering or tool linkage.
- [ ] Run focused tests and a live verification-reminder session; inspect remote history for one authoritative answer before closing #844.

### Task 10: Desktop connection health and platform version (#838, #839)

**Files:**
- Create: `mobile/src/lib/appVersion.ts`
- Create: `mobile/src/lib/__tests__/appVersion.test.ts`
- Modify: `mobile/src/app/(tabs)/settings.tsx`
- Modify: `mobile/src/components/fleet/DesktopDrillDown.tsx`
- Modify/add: component tests for Desktop drill-down

**Interfaces:**
- `resolveAppVersion({ isTauri, tauriVersion, expoVersion }): string` uses the Tauri bundle version on desktop and Expo version elsewhere.
- Desktop Fleet status derives from the sessions query state, never a hardcoded success token/string.

- [ ] Add failing version resolver tests for Tauri, Expo, and fallback paths.
- [ ] Add failing Fleet component tests for loading, connected, unreachable/error, and empty-success states.
- [ ] Implement dynamic Tauri `getVersion()` resolution with graceful fallback and replace both incorrect Settings references.
- [ ] Drive dot, label, summary, and safe retry action from query state.
- [ ] Run mobile checks; launch installed desktop against a deliberately stopped endpoint and confirm red/offline, then restart and confirm green/online; compare displayed version with the installed bundle before closing #838/#839.

### Task 11: Stable host identity and rename UI (#840)

**Files:**
- Modify: `crates/forge-cli/src/serve.rs`
- Modify: `mobile/src/lib/api.ts`
- Modify: `mobile/src/lib/auth.tsx`
- Modify: `mobile/src/lib/connectUrl.ts`
- Modify: `mobile/src/lib/serverTargets.ts`
- Modify: `mobile/src/app/(tabs)/settings.tsx`
- Add focused Rust and mobile tests

**Interfaces:**
- Add authenticated `GET /api/identity` returning `{ hostname: string }` from the system-hostname helper.
- Keep transport endpoint in `StoredServer.host`; set display `name` from identity unless user-renamed.
- Add `renameServer(id: string, name: string): Promise<void>` to the auth provider and persist/reconcile it without changing the endpoint.

- [ ] Add failing daemon auth/identity tests and mobile pairing tests proving tunnel domain is not the default display name.
- [ ] Add failing rename/reconciliation tests proving a custom name survives reconnect, endpoint change, and managed/direct deduplication.
- [ ] Implement endpoint, pairing identity fetch, rename method, and an accessible Settings rename control.
- [ ] Run focused Rust/mobile tests; pair through a real quick tunnel, confirm system hostname, rename it, reconnect, and confirm persistence before closing #840.

### Task 12: Complete Desktop release publication and repair Latest (#837)

**Files:**
- Modify: `.github/workflows/release.yml`
- Modify: `.github/workflows/app-desktop.yml`
- Create/modify: release workflow contract test under `scripts/ci/`
- Modify if required: installer checksum lookup script

**Interfaces:**
- CLI release creation does not set Latest while Desktop assets are incomplete.
- Desktop workflow uploads signed bundles, `latest.json`, and `desktop-checksums.txt`, verifies their public downloads/hashes, then marks the release Latest.

- [ ] Add a failing workflow contract test proving `release.yml` cannot make Latest and `app-desktop.yml` requires/checks all assets before Latest.
- [ ] Implement publication ordering and public verification.
- [ ] Run YAML/actionlint/shell contract checks.
- [ ] Repair `v2.7.0`: download public Desktop artifacts, generate/upload `desktop-checksums.txt`, download it publicly, verify every listed hash, and run the real desktop installer path in a disposable prefix before closing #837.

### Task 13: Portable Linux TUI Voice and cross-client Voice correctness (#850)

**Files:**
- Modify: `crates/forge-voice/src/record.rs`
- Modify: `crates/forge-voice/src/lib.rs`
- Modify if required: `crates/forge-cli/src/cli/commands/run.rs`
- Verify/modify: `mobile/src/lib/voice/voice.ts`
- Verify/modify: `mobile/src/lib/voice/voice.web.ts`
- Verify/modify: `mobile/src/components/chat/VoiceRecordingPill.tsx`
- Add Rust and mobile tests

**Interfaces:**
- On portable Linux builds without CPAL, `Recorder` uses a runtime `pw-record` backend, then `arecord` fallback, and returns a WAV path/samples through the existing transcription pipeline.
- Cancel/stop terminates and reaps the child; failures name the safe package/device action.
- Every client appends transcript to the existing draft and never auto-sends.

- [ ] Add failing backend-selection/process-lifecycle tests with fake `pw-record`/`arecord` executables, including missing tools, cancellation, stop, and non-zero exit.
- [ ] Implement the Linux process recorder without linking portable binaries to ALSA.
- [ ] Re-run native/web Voice unit tests for recorder lifetime, multipart bytes, WAV format, draft preservation, and no auto-send; add a failing regression first for any uncovered behavior.
- [ ] Build the portable CLI and perform a real short microphone capture on this Arch host through `pw-record`, transcribe it, and confirm insertion into the TUI draft.
- [ ] Publish an OTA only if mobile runtime code changes; physically verify capture→transcribe→draft on iPhone, and verify desktop/web with real microphone permissions before closing #850.

### Task 14: Aggregate validation, PR, deployment, and issue closure

**Files:**
- Modify: this plan checkboxes/evidence notes as execution proceeds
- No product file may be changed solely to satisfy a test.

- [ ] Run `cargo fmt --all -- --check`.
- [ ] Run `cargo clippy --workspace --all-targets --all-features -- -D warnings` (or the repository's exact all-feature compatibility matrix if mutually exclusive platform features prevent this command).
- [ ] Run `cargo test --workspace --all-targets`.
- [ ] Run `npm run check` in `mobile/`.
- [ ] Build the Tauri desktop bundle and launch it.
- [ ] Run all workflow/shell contract tests and validate changed workflow YAML.
- [ ] Review `git diff --check`, secret scan, changed-file inventory, and the issue-to-evidence matrix.
- [ ] Commit coherent issue groups on `fix/all-open-issues`, push, and open one non-draft PR listing all 16 issues and their verification evidence.
- [ ] Resolve review and CI failures at root cause; merge only after required checks pass.
- [ ] Deploy service/desktop/OTA changes required by the merged diff and verify production endpoints/artifacts/update groups.
- [ ] Comment exact automated and live evidence on each issue; close only verified issues, leaving any externally blocked acceptance path open with the precise blocker.

## Live Evidence Matrix

| Issue | Required real evidence before closure |
|---|---|
| #832 | Remote Plan approval restores prior mode and executes without another approval loop |
| #833 | Delayed matching question enables the correct plan card; unrelated/stale question does not |
| #834 | Long session outbound provider view contains only newest permission contract |
| #835 | `/orchestrate` trace never attempts a skill named `list` |
| #836 | Failed check followed by read remains unverified; matching successful rerun resolves it |
| #837 | Public Latest checksum manifest and real installer checksum path succeed |
| #838 | Installed desktop shows bundle version, not Expo mobile version |
| #839 | Installed desktop is offline/red with stopped host and online/green after restart |
| #840 | Quick-tunnel pair shows system hostname; custom rename survives reconnect |
| #841 | Long session provider payload remains bounded while audit history stays complete |
| #842 | Real bridge tool-only intermediate automatically continues to terminal Done |
| #843 | Same replay shows tool action/result and no blank assistant/empty-response nudge |
| #844 | Verification-reminder turn yields one terminal answer in remote history |
| #845 | Injected recoverable Codex OAuth continuation fault retries and resumes automatically |
| #850 | Arch TUI, iPhone native, desktop, and web each capture/transcribe/append without auto-send |
| #861 | Neutral workflow visibly skips; OTA runtime change publishes a real update group |
