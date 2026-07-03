# Demo recordings

Reproducible sources for the README GIFs (`docs/assets/forge-demo.gif` + `docs/assets/demo-*.gif`).

```bash
scripts/demo/record.sh            # re-record everything
scripts/demo/record.sh hero tui   # just these tapes
```

Requirements: [vhs](https://github.com/charmbracelet/vhs) (which needs `ttyd`, `ffmpeg`, and a
headless Chrome/Chromium) and a "JetBrainsMono Nerd Font" install.

How it works:

- `setup.sh` stages `scripts/demo/.stage/` (gitignored): the repo's release `forge` binary on
  `PATH`, a scratch `acme-api` git project with a saved `.forge/workflows/audit.js`, an isolated
  `FORGE_DB` (the real store is never touched), one seeded session for the provenance tape, and
  one drained queue task for the digest shot. It is re-run before every tape so each recording
  starts from identical state.
- `tapes/*.tape` are vhs scripts; `@SID@` is replaced with the seeded session id at stage time.
- Everything runs offline against `--mock`, paced by `FORGE_MOCK_STREAM_DELAY_MS` (set in
  `.stage/env.sh`) so streaming and the workflow view animate like a real model. The one
  exception is `mesh.tape` — `forge mesh` explains routing over the recording machine's live
  provider catalog.
- `record.sh` finishes with an ffmpeg palette pass to keep the GIFs small.
