# Provider outage drill

Runs `forge run` against a fake OpenAI-compatible gateway that answers `503 no_capacity` for a set
time after its first request, then a healthy reply. It shows the turn waiting through the outage
(`mesh.outage_wait_secs`) instead of failing, and prints when each request reached the gateway.

    scripts/drills/provider-outage/run.sh [DOWN_SECS=180]    # HOT_SECS / LONG_SECS tune the waits

Everything lives in a temp root (HOME, XDG, FORGE_DB, workspace); your real daemon, store and config
are never touched. No live model is called. `fake_gateway.py PORT DOWN_SECS LOG [retry_after]` can
also be run alone. The in-process version is `outage_e2e.rs` in forge-cli.
