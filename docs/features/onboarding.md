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

## The desktop app

The desktop app runs the same flow in `/onboarding`, over the daemon. It opens by itself when the
app has no server yet (a first launch) and, for an existing install, once on the first launch of
a version that has it (the daemon's `GET /api/onboarding?brief=1` says whether it should open). The
required steps on the desktop are a working model and a running background service.

### Phase 0: no Forge yet

With no `forge` command on the computer, the app installs one before anything else, with a progress
row for each stage:

1. **Install Forge** (`install_forge_cli`, a Tauri command): resolves the latest release
   (`FORGE_RELEASE_BASE_URL` overrides the base URL for tests), downloads the platform archive,
   verifies it against the release `checksums.txt` (`SHA256SUMS` is accepted too) and fails closed
   on a missing manifest or a mismatch, then places the binary atomically in `FORGE_INSTALL_DIR`,
   else `~/.local/bin` (`%LOCALAPPDATA%\Programs\forge` on Windows).
2. **Start Forge in the background** (`install_forge_service`): `forge service install --local`,
   `forge service start`, then waits for the daemon's `serve-state.json`.
3. **Connect this app** to that daemon, then the rest of the flow runs against it.

A failed row shows the one-line reason and a "Try again"; "Connect to an existing server" opens
the manual pairing screen.

### The daemon API

All routes sit under the daemon token, answer only requests that reach the daemon from the machine
it runs on (loopback `Host`, no proxy headers; anything else gets `403`), and are not part of the
Forge Anywhere allowlist.

| Route | |
| --- | --- |
| `GET /api/onboarding` | plan, shared state, detected machine, model sources, the recommended checklist; `?brief=1` is only "should it open" and "does Claude need reconnecting"; `?force=1` is the full flow |
| `POST /api/onboarding/steps/{id}` | `{"op":"done"\|"skip"\|"reset"}` for a step; `{"op":"postpone"\|"complete"}` on `flow` |
| `POST /api/onboarding/actions` | `{"action":{...},"step":"models"}` starts a job (`Action` as in `forge-onboarding`); `{"recommended":true}` returns the checklist |
| `GET /api/onboarding/jobs/{id}?cursor=&wait=` | events since `cursor`, held open up to `wait` ms when there are none |
| `POST /api/onboarding/jobs/{id}/input` | `{"text":"..."}` a code the page showed |
| `DELETE /api/onboarding/jobs/{id}` | cancel |

Sign-ins that need a terminal (`claude setup-token`, the Codex and xAI OAuth flows) run in the
daemon, in a pseudo-terminal, and their tokens are stored by the daemon. A secret the app sends
(a pasted key) deserializes into a masked type that serializes as `***`, job output is scrubbed
of every secret the action carries, and no response contains a credential. A finished job is
folded into `onboarding.json` by the daemon, so the terminal and the desktop never disagree.

### What the screens do

Wide window: a step rail on the left (done, current, skipped; required steps marked), the step in
a card on the right. Narrow window: a progress bar on top and one column. Each step opens with what
was found on this computer, then one primary button; alternatives and explanations are under
"Details". The sign-in panel shows the page to open (the system browser through the opener
plugin), the device code when there is one, and a QR code to finish on another device. The
**Reconnect Claude Code** banner appears outside setup when Claude Code is installed and signed in
but Forge has no token for it, and opens setup on the model step with the connection already
running.

### Screenshots

Captured off-screen against an isolated daemon, with fake `claude` and release servers (no real
accounts): [every step, wide, dark](onboarding-desktop/wide-dark.jpg),
[wide, light](onboarding-desktop/wide-light.jpg),
[phone width, dark](onboarding-desktop/narrow-dark.jpg),
[phone width, light](onboarding-desktop/narrow-light.jpg), and
[phase 0, the failed install, the existing-install welcome and the Reconnect Claude Code banner](onboarding-desktop/flow.jpg).
