# Changelog

All notable changes to Forge are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and Forge adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- `forge doctor` — diagnose your setup (providers/keys, CLI bridges, Ollama, MCP, config, git,
  terminal) with actionable fixes.
- Startup update check — a throttled notice when a newer release is available (disable with
  `[update] check = false` or `FORGE_NO_UPDATE_CHECK=1`).
- Community infrastructure: `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`, `SECURITY.md`, issue
  templates, and this changelog.

## [0.2.0] - 2026-06-23

### Added
- Full-screen (alternate-screen) TUI by default — scrollable transcript, pinned panels, mouse-wheel
  scroll; `--inline` (or `[tui] fullscreen = false`) keeps the classic inline-scrollback mode.
- Unified in-loop activity viewer (Ctrl+O): main chat + subagents + assay critics in one navigable
  full-screen view.
- Dynamic `/config` editor — grouped sections, friendly labels, type-appropriate controls (toggle /
  cycle / typed input), API-key management (keyring), per-setting help, default/modified/source,
  reset-to-default, fuzzy search, user/project scope.
- `forge local` — local LLMs via Ollama: hardware detection, live model discovery, ranking by
  Artificial Analysis benchmark scores (multi-family catalog offline), install/start/status, an
  animated picker, and opt-in autostart.
- `forge setup` — guided first-run wizard (providers/plans + optional local-LLM step); auto-runs on
  first launch. `forge init` aliases it.
- Per-turn AI recap; persisted TUI view state restored on resume.

### Changed
- `forge models --probe` rechecks only benched models by default (cheap); `--all` re-pings every
  model. Capability-exclusion window shortened to 24h.

### Fixed
- Activity-viewer crash and scrollback corruption in full-screen mode; assay per-critic model/cost
  detail; macOS release checksum.

## [0.1.0] - 2026-06-18

Initial public release: Model Mesh routing, multi-provider support, cost/budget caps, the
inline TUI, session persistence + checkpoints, permission broker, subagents, Assay analysis,
Lattice code intelligence, MCP client, web tools, hooks, skills/commands, and more.

[Unreleased]: https://github.com/florisvoskamp/forge/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/florisvoskamp/forge/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/florisvoskamp/forge/releases/tag/v0.1.0
