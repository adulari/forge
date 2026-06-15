# Feature: `/config` — in-chat configuration wizard

Status: built. Lets the user (re)configure Forge from inside a chat session — set provider &
search API keys and bridge subscription plans — via the same beautiful animated wizard as
`forge init`, without restarting.

## Problem (JTBD)

> When I'm in a chat session and realise a key is missing (e.g. `web_search` needs a search
> key, or I want to add an Anthropic key), I want to configure it **right there** and have the
> running session pick it up — not quit, run `forge init`, and start over.

`forge init` only ran at first launch / from the shell. There was no way to change keys or
plans mid-session.

## What it does

- New slash command **`/config`** (aliases `/cfg`, `/settings`) in the chat TUI.
- Launches the existing animated `init_wizard` **full-screen** (alt-screen takeover), then
  returns to the inline chat — scrollback preserved. The inline viewport is rebuilt on return
  (`Tui::run_fullscreen`), so the chat loop resumes cleanly (raw mode re-enabled, cursor sane).
- The wizard now has a dedicated **Search** section alongside Providers and Subscription
  bridges, so search-API keys (`brave`, and any future `SearchBackend`) are set the same way.
- On finish: keys → OS keyring (ADR-0007), plans → user config, then **injected into the
  running process** (`inject_provider_keys` + `inject_search_keys`) so the current session's
  tools use them immediately — no restart.
- Gated while a turn is in flight (it mutates config / takes the screen): "finish or Esc the
  current turn first".

## Design notes

- **Reuses, doesn't duplicate.** `forge init` and `/config` share `wizard_input()` (builds
  providers + search + bridges) and `apply_wizard_outcome()` (store + write + inject) in
  forge-cli. The wizard itself is unchanged in structure — providers and search are both just
  "id + label + masked key field", unified behind `Row::Provider`/`Row::Search` + a single
  `key_field_mut()` editing path.
- **Full-screen, not inline.** The inline chat viewport is a fixed small height and can't host
  masked key entry well; the wizard is already the polished animated surface, so `/config`
  takes over the screen and restores — "in chat" = reachable without leaving the session.

## Verification

- Pure wizard `State` transitions unit-tested (incl. typing a search key → outcome).
- `parse_command` tests for `/config`/`/cfg`/`/settings`.
- PTY e2e (`tui_config_opens_wizard_fullscreen_and_returns_to_chat`): `/config` opens the
  wizard, Esc cancels, the chat loop resumes, `/quit` exits cleanly — proving the takeover +
  restore doesn't wedge the terminal.
