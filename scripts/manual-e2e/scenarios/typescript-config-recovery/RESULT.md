# TypeScript config recovery reference

The fixture has an invalid TypeScript configuration, broken package entry point, and unsafe shallow
merge. The live run recovered from a rejected atomic multi-file edit. The reference passes strict
compilation and all secure deep-merge tests:

```bash
cd reference
npm test
npm run lint
```

## Verified unpinned-mesh result (2026-07-23)

A fresh live TUI run used the Codex subscription for the parent and independently completed two
inspection agents through Codex and OpenRouter. After its first `npm test` exposed an incomplete
fix, Forge surfaced the failure, launched a visible auxiliary diagnosis, repaired the build config,
safe recursive merge, package entry point, and offline lint command, then passed all 4 security and
aliasing tests plus a fresh strict TypeScript build. All 38 persisted tool envelopes/execution
records were structurally valid; three non-OK outcomes were expected checks or edits that the turn
subsequently recovered. The retained run ID is
`typescript-config-recovery-20260723T032551Z-1879768`.
