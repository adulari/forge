# Handoff — mesh-dogfood UI/OAuth batch (2026-07-14 ~22:40 CEST)

Continue in a new session. Nothing is lost — all work is committed to branches on origin.

## origin/main state
`e237301c` — includes **#760** (init banner below tabs) and **#761** (mesh classifier: code tasks never route trivial + classify with capable free models). Both MERGED.

## The batch (6 user-requested items + 1 mesh bug)

| # | Item | Status |
|---|------|--------|
| 1 | Headless OAuth (device + paste-back, auto-select) | session `ebf04c2c` finished on `claude-cli::fable`; **NOT yet on a handoff branch** — see "OAuth" below |
| 2 | Init-banner below tabs | ✅ MERGED #760 |
| 3 | Native header two-tone | branch `handoff/header` — needs rebase+verify+PR; **re-test on native** |
| 4 | Live Activity clipping/progress | branch `handoff/liveactivity` — needs rebase+verify+PR; **re-test on native** |
| 5 | Command autocomplete + skills | branch `handoff/autocomplete` — needs rebase+verify+PR |
| 6 | EAS Update OTA | branch `handoff/eas` — needs rebase+verify+PR; **user must set `EXPO_TOKEN` repo secret** |
| — | Mesh classifier bug (found dogfooding) | ✅ MERGED #761 |
| — | Mesh parallel-collision on free models | OPEN — needs a fix + **user decision on approach** |

## Handoff branches (pushed to origin, based on old `0d6aa977` = v2.6.2)
- `handoff/autocomplete` @ `dfeaeedb` — files: `mobile/src/components/chat/Composer.tsx`, `mobile/src/components/overlay/CommandPalette.tsx`, `mobile/src/lib/commands.ts` (new; single source of truth, `useSkillCommands()` fetches `/api/skills`). Clean, on-task.
- `handoff/header` @ `b0373bc6` — files: `mobile/src/app/session/[id]/_layout.tsx`, `mobile/src/components/ds/Segmented.tsx` (adds `flush` prop: bg2 track + bg3 thumb; explicit bg2 header wrapper). **`_layout.tsx` is ENTANGLED with #760's banner move — rebase onto origin/main WILL conflict on `_layout.tsx`; resolve by keeping #760's banner-below-tabs position AND header's bg2 wrapper.** (Stray dup live-activity files were already stripped from this branch.)
- `handoff/liveactivity` @ `75edf493` — files: `crates/forge-cli/src/apns.rs`, `crates/forge-cli/src/cli/commands/run/driver.rs`, `mobile/modules/live-activity/index.ts`, `mobile/modules/live-activity/ios/LiveActivityModule.swift`, `mobile/src/lib/queries.ts`, `mobile/targets/widget/ForgeSessionActivity.swift`, `mobile/targets/widget/ForgeSessionActivityAttributes.swift`. Self-consistent: `updateLiveActivity`/`start` now take a 6th `contextLimit` arg (index.ts def + queries.ts call must stay in sync — this was the tsc gotcha). Adds horizontal padding to the lock-screen view + real context-limit progress divisor.
- `handoff/eas` @ `696429dd` — files: `.github/workflows/eas-update.yml`, `mobile/EAS_UPDATE.md`, `mobile/app.config.ts` (runtimeVersion fingerprint + updates block, projectId `e1d145b5-344e-4147-ba35-5f0b993b4c8c`), `mobile/package.json` (+expo-updates), `mobile/package-lock.json`, `mobile/src/app/_layout.tsx` (mount hook — ROOT layout, not session), `mobile/src/lib/useOtaUpdates.ts` (new). Needs `EXPO_TOKEN` secret before the workflow can publish. Verify tsc needs `expo install` (expo-updates not installed in node_modules yet).

## Clean integration procedure (per branch)
Do NOT cherry-pick files across bases — it caused signature drift. Instead:
1. `git fetch origin`; for each: `git checkout -b pr/<name> origin/handoff/<name> && git rebase origin/main` (resolve `_layout.tsx` for header).
2. Verify: mobile → `cd mobile && npx tsc --noEmit | grep ^src/ | grep -v src-tauri | grep -viE 'updater.ts|voice.ts|plugin-'` must be empty (eas: run `npx expo install expo-updates` first). Rust (liveactivity) → `cargo build --release -p forge-agent` (use `scripts/rebuild.sh`, never bare cargo).
3. `gh pr create` + `gh pr merge <n> --squash --auto`. Standing automerge auth on this repo.
Order: autocomplete, eas, liveactivity first (independent); header last (needs the `_layout.tsx` merge).

## OAuth (item 1) — WIP, INCOMPLETE, on `handoff/oauth-wip`
Session `ebf04c2c` (worktree `.forge/worktrees/2b760d0e-1de`, `claude-cli::fable`) barely progressed: it created a new `crates/forge-cli/src/cli/commands/oauth_flow.rs` scaffold but did NOT wire it into `local.rs`/`codex_oauth.rs`/`mcp.rs`/`mod.rs`. Committed as-is to `handoff/oauth-wip` so it's not lost. **Treat as a fresh start** — re-dispatch the full brief (`/tmp/forge-scratch/brief_headless_oauth.txt`): device-flow where the provider supports RFC 8628 (xai_oauth.rs is the reference) + universal paste-back fallback + auto-detection, in `local.rs` (codex, ~line 397) and `mcp.rs` (~line 347). Security-sensitive (preserve PKCE + state/CSRF). Pin a capable model.

## Open mesh issue — parallel collision (needs your decision)
Dogfooding N parallel sessions routes them all to the SAME top free model → self-inflicted rate-limit → ~60s waits before failover. Failover works; it's an efficiency gap. **Ask the user how to fix**: e.g. (a) round-robin/stagger parallel sessions across the ranked free models, (b) per-model in-flight concurrency cap that demotes a model already serving N sessions, (c) leave as-is. Code: `crates/forge-mesh/src/lib.rs` `ordered_usable_for_tier` / `candidates_for_tier`.

## Also pending
- **Daemon** (pid `3202498`, tunnel `anne-matt-aruba-games`) runs v2.6.2 — does NOT have #761 classifier fix. After integration, `scripts/rebuild.sh` + kill pid 3202498 + restart `forge serve --anywhere` so live routing uses the fix. New tunnel URL each restart → repoint desktop via `/tmp/forge-scratch/repoint_server.py`.
- **No releases** until the user says so (their instruction).
- Native re-test needed for header (#3) + live activity (#4) on next TestFlight build.
- Dogfood sessions still exist on the daemon (autocomplete `7fe421f5`, header `01a36ab5`, liveactivity `d65af88b`, oauth `ebf04c2c`) — discard after integration via `POST /api/sessions/<id>/discard`.

## Safety net
- Stash `stash@{0}` = `handoff-mixed-integrate-state-a806972a` = main repo's mixed leftovers (autocomplete + live-activity dups). Redundant with the branches; drop once integration confirmed (`git stash drop`).
- serve token `45e35475f429a097b91498b5265b0ebf`; `serve_drive.py` + briefs in `/tmp/forge-scratch/`.
