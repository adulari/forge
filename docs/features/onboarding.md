# Onboarding

The first time Forge starts in a terminal, it walks you through setting itself up. Detection fills
in everything the machine already has, so most people press Enter a few times.

## When it opens

- **Bare `forge`, `forge chat`, `forge run`, `forge nl`** on a terminal, once per release flow: a
  machine that has never run onboarding (including one that already works) sees it the first time
  it launches a version that has it. After that it stays closed.
- **`forge setup`** (alias `forge init`) always opens the full flow.
- **Never** in a pipe, under CI (`CI`, `GITHUB_ACTIONS`, ...), with `TERM=dumb`, in `forge run`
  scripts, `forge mcp-serve` or a daemon. `FORGE_NO_SETUP=1` keeps it closed everywhere except
  `forge setup`; the installers set it when you decline the post-install setup.

Required steps decide whether it reopens: a machine with **no working model** gets onboarding at
every launch until one is connected. A machine that already works can press `q` ("Not now")
twice; after that Forge stops opening it and prints one line, `Run forge setup to finish setting up
Forge.` A later release that adds steps shows only the new recommended ones, once.

## The steps

| Step | What it does | Required |
| --- | --- | --- |
| Welcome | What was found on this machine, and **Set up everything recommended** | no |
| Models | Cards for Claude Code, Codex, Antigravity, ChatGPT/SuperGrok plans, API keys (free tiers first) and a local Ollama model, each with a live check | **yes** |
| Safety and cost | Permission mode (temper) and credit mode, one example each | no |
| Web search | Works with no key; optionally a local SearXNG container or a Brave key | no |
| Benchmarks | Shared benchmark data, no key needed; your own Artificial Analysis key is optional | no |
| Bring your setup | Commands, skills and agents from Claude Code, Codex, Cursor; MCP servers; Claude Code key bindings | no |
| Background service | `forge service install`, this computer only by default | desktop only |
| Forge Anywhere | Phone access, end to end encrypted | no |
| Other places | Install the desktop app | no |
| Notifications and theme | Two switches | no |
| Error reports | Automatic (default), ask each time, or off; `w` shows exactly what is sent | no |
| Ready | What works now | no |

A step with nothing to offer on your machine is left out.

### One action per sign-in

Claude Code, Codex and xAI sign in without leaving the screen. Forge runs the CLI's own login in a
pseudo-terminal, shows the sign-in page URL (and a QR code, `v`, so you can finish on a phone over
SSH), and carries on when you are done. For Claude Code it captures the token `claude setup-token`
issues, stores it in the secret store as `claude_oauth_token`, and checks it with a one-line bridge
round trip. The token is never printed or logged. If it cannot be read from the output, a masked
paste field appears instead. If the page shows a code to paste back, a masked field takes it.

An existing install whose Claude Code has its own login but no Forge token (the bridge no longer
shares your main login) is told so: the card reads "Reconnect Claude Code, one click, keeps your
main Claude login safe". If the once-per-release run is already behind you, the same offer appears
as a small card when a session starts; `Enter` connects, `l` is later, `n` never asks again.

### Set up everything recommended

`Enter` on the welcome runs every action that needs no decision, in order, with a live checklist:
reconnect Claude Code, keep the safe defaults, import commands and skills, Claude Code key bindings,
install the background service (this computer only), and automatic error reports. It pauses on a
browser sign-in until you finish it, skips the MCP picker (a real choice), and stops at the model
step only when no model is connected. A failed item does not stop the others; the run ends on the
failed step with the reason.

## Keys

`Enter` does the primary action (its label is in the footer), `↑↓` choose, `Space` set up or check
the focused card, `s` skip a step, `b` back, `q` not now. Secrets are typed into a masked field and
go straight to the secret store.

## State

Progress is `onboarding.json` in the data directory (beside `FORGE_DB` when that is set). It records
each step as done, skipped or pending, the last live check of every model source (never a
credential), how many times the run was deferred, and the flow version. The terminal and the
desktop app share it, so whichever opens first, the other resumes.

## For other front ends

The `forge-onboarding` crate is the engine; nothing in it draws. A front end calls
`forge_onboarding::start` to decide, reads the `Machine` and `plan`, and runs `Action`s as `Jobs`
that stream serializable `JobEvent`s: output lines, `Prompt` events (open a URL, enter a code),
and a final result. `Jobs::send_input` answers a code prompt and `Jobs::cancel` stops a flow. The
sign-in flows run inside the process that owns the `Jobs`, so a daemon can run them for the desktop
app and the token never leaves it.
