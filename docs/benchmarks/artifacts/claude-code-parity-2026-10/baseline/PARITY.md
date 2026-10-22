# Claude Code 2.1.308 vs Forge 2.21.0: static feature and UX parity audit

Date: 2026-10-22. Part of the ccbench program (`~/.cache/forge-ops/ccbench/PLAN.md`, dimension 7). Author: parity-audit agent.

## How this was produced

- Claude Code side: `claude --version` (2.1.308) and `claude --help` / `claude mcp ...` output, plus the live docs at https://code.claude.com/docs/en/*.md (commands, interactive-mode, keybindings, hooks, skills, sub-agents, memory, permission-modes, checkpointing, headless, tools-reference, mcp, statusline, output-styles, settings and others; raw copies in `ccdocs/`). Behaviour is taken from those pages; no `claude` model call was made, so CC rows are documentation-grade, not executed.
- Forge side: installed binary `forge 2.21.0` (`/home/floris/.local/bin/forge`) and the repo at `fa753ba1` (main, two small commits after the 2.21.0 prep; none touch the areas below). Every Forge claim marked `live` was reproduced with the binary under a throw-away `HOME` and `XDG_*`, `FORGE_DB` pointing at `parity/sandbox/forge-parity.db`, the `--mock` provider, and a private tmux socket (`tmux -L ccbench-parity`). Claims marked with a file and line were read in the code; the rest cite the feature docs under `docs/features/`.
- Model calls: none. (`forge doctor` in the sandbox attempted bridge liveness turns; all three failed at authentication because HOME was redirected, so no quota was spent.) Raw evidence is in `evidence/`; help dumps in `raw/`; the table generator is `build_parity*.py`.
- Status vocabulary: **better** = Forge has it and it is stronger or has a capability CC lacks; **equal** = same capability, possibly different keys or names (the cell says so); **worse** = Forge has a weaker or differently behaving version; **missing** = Forge has nothing comparable.

## Scorecard

| Area | better | equal | worse | missing | rows |
| --- | ---: | ---: | ---: | ---: | ---: |
| 1. Launch modes and CLI flags | 2 | 8 | 14 | 9 | 33 |
| 2. Subcommands | 6 | 1 | 3 | 4 | 14 |
| 3. Slash commands | 7 | 12 | 9 | 7 | 35 |
| 4. Keybindings and input editing | 2 | 11 | 9 | 8 | 30 |
| 5. Interactive UX features | 2 | 11 | 6 | 2 | 21 |
| 6. Permissions, modes and safety | 3 | 5 | 2 | 2 | 12 |
| 7. Plan mode | 1 | 3 | 1 | 0 | 5 |
| 8. Project instructions and memory | 0 | 3 | 3 | 5 | 11 |
| 9. Settings, configuration and environment | 1 | 5 | 5 | 1 | 12 |
| 10. Hooks | 1 | 9 | 2 | 7 | 19 |
| 11. Skills, custom commands and plugins | 2 | 4 | 4 | 5 | 15 |
| 12. Subagents, parallel work and worktrees | 4 | 4 | 3 | 1 | 12 |
| 13. MCP | 1 | 6 | 3 | 2 | 12 |
| 14. Context, compaction, sessions and checkpoints | 4 | 5 | 2 | 2 | 13 |
| 15. Built-in model tools | 1 | 15 | 1 | 2 | 19 |
| 16. Rendering, status line and accessibility | 0 | 5 | 1 | 1 | 7 |
| 17. Headless, SDK and integrations | 3 | 3 | 3 | 1 | 10 |
| **Total** | **40** | **110** | **71** | **59** | **280** |

Of 280 compared features, Forge is better on 40, equal on 110, worse on 71 and has nothing comparable on 59 (130 gap rows rolled up into 25 ranked gaps below).

Reading the result: Forge's gaps cluster in three places: (1) drop-in compatibility surface (flags, hooks, skills, instruction files, MCP CLI shape, stream-json), (2) CC's terminal-UX conveniences (`!` shell mode, readline/vim editing, `/context`, `/diff`, `/permissions`, transcript search, rewind variants), and (3) enterprise and IDE reach. Forge's advantages are the routing mesh, the daemon/fleet model, safety and trust handling, cost transparency and the autonomy tooling.

## Top findings (read these first)

1. **G1 `forge run -p` text output is not scriptable** (impact high, effort S). `forge run -p` prints the session header, routing line, tool trace, recap and cost footer on stdout; `claude -p` prints only the answer.
1. **G2 stream-json is only CC-shaped on the surface** (impact high, effort M). stream-json emits word-sized `assistant` events, a bare `system/init`, and a final `result` without usage or cost.
1. **G3 Instruction files: first match wins, no hierarchy, no imports** (impact high, effort M). Only the first non-empty instruction file loads: a repo AGENTS.md hides your global ~/.claude/CLAUDE.md; no parent walk, imports, local file or rules dir.
1. **G4 No `!` shell mode in the composer** (impact high, effort S). Typing `!cmd` sends it to the model instead of running it.
1. **G5 Unbound Ctrl chords type the letter; readline set incomplete** (impact high, effort S). Unbound Ctrl chords insert their letter (Ctrl+D, Ctrl+G, Ctrl+B, Ctrl+X, Ctrl+Y, Ctrl+P all verified live).
1. **G6 Entry-point ergonomics differ from `claude`** (impact high, effort S). Bare `forge` does not start a session, `forge chat "prompt"` is rejected, and `claude mcp add`/`list` syntax fails.
1. **G7 `claude -p` drop-in flags missing; `chat` has almost none** (impact high, effort M). More than twenty `claude -p` flags are rejected (--add-dir, --system-prompt, --max-budget-usd, --mcp-config, --settings, --effort, --worktree, ...); `chat` has almost none.
1. **G8 Hook coverage: 11 of 33 CC events, 3 of 5 handler types** (impact high, effort M). Forge fires 11 of CC's 33 hook events, has no http handler, no `if`/`async`, and `forge run` skips SessionStart/UserPromptSubmit.

## 1. Launch modes and CLI flags

`claude --help` (2.1.308) versus `forge run --help` and `forge chat --help` (2.21.0). Verified by running every flag against `forge run --mock` (unknown flags exit 2 with `unexpected argument`).

| Claude Code feature | Forge equivalent | Status | Evidence |
| --- | --- | --- | --- |
| `claude` (bare) starts an interactive session | `forge chat`. Bare `forge` prints a summary screen and exits | **worse** | main.rs:249-255 and first_run.rs:38 print `Common commands` and return; `forge chat` is the only interactive entry |
| `claude "prompt"` starts interactive with an initial prompt | `forge run --tui "prompt"`. `forge chat "prompt"` is rejected | **worse** | live: `forge chat --mock "hello"` -> `error: unexpected argument 'hello there' found`; usage is `forge chat [OPTIONS]` with no PROMPT |
| `-p/--print` non-interactive | `forge run [-p]` (`-p` is an accepted no-op) | **equal** | claude_compat.rs; but see section 17: stdout is polluted with a trace in text mode |
| `--output-format text/json/stream-json` | `forge run --output-format text/json/stream-json` | **worse** | json is CC-shaped; stream-json differs materially (section 17, gaps G1/G2) |
| `--input-format stream-json` | rejected: `invalid value 'stream-json' ... [possible values: text]` | **missing** | live run |
| `--include-partial-messages`, `--include-hook-events`, `--replay-user-messages`, `--forward-subagent-text` | none accepted | **missing** | live: each `unexpected argument` |
| `--json-schema` structured output | none | **missing** | live: `unexpected argument '--json-schema'` |
| `--max-budget-usd` | flag rejected. Budget caps exist in config and `/goal --max-tokens N --max-minutes N`; `forge queue` is budget-capped | **worse** | live: rejected; fix-budget-cap.md; autonomous-gates.md |
| `--max-turns` (print mode; documented in cli-reference, absent from `claude --help`) | `forge run --max-turns N` (maps to `mesh.max_steps`) | **equal** | forge run --help |
| `--model <alias or id>` | `--model provider::model` or a comma-separated pool ranked by the mesh | **better** | forge run --help; mesh-routing.md |
| `--fallback-model` | no flag needed: automatic cross-provider failover down the whole ranked catalog | **better** | mesh-routing.md; claude_compat.rs lists the flag as intentionally unsupported |
| `--effort low..max` | no CLI flag. `/effort`, `Alt+E`, config `[model_effort]` | **worse** | live: `unexpected argument '--effort'` |
| `--permission-mode default/acceptEdits/plan/auto/dontAsk/bypassPermissions` | `--permission-mode default/acceptEdits/bypassPermissions/plan` and `--mode default/accept-edits/bypass/plan/auto` (run). `chat` has only `--mode`. No `dontAsk` | **equal** | forge run --help; forge chat --help |
| `--dangerously-skip-permissions` | `forge run --dangerously-skip-permissions`; on `chat` use `--mode bypass` | **equal** | forge run --help |
| `--allowedTools` / `--disallowedTools` | `forge run --allowedTools/--disallowedTools` (CC rule syntax). Not on `chat` | **equal** | forge run --help; chat --help has neither |
| `--tools` (restrict the built-in tool set) | none | **missing** | live: `unexpected argument '--tools'` |
| `--add-dir` | flag rejected. Only config `[tools] extra_roots` | **worse** | live; setup.rs:76 resolve_extra_tool_roots |
| `--system-prompt` | rejected (`Forge's own prompt is not replaceable`); config `system_prompt_overrides` exists | **worse** | claude-code-parity.md; live: rejected |
| `--append-system-prompt` | `forge run --system TEXT` (alias `--append-system-prompt`). Not on `chat` | **equal** | forge run --help |
| `-c/--continue`, `-r/--resume [id or name]` | `--continue`, `--resume [id prefix]` with picker (chat, and run with `--tui`). No session names | **equal** | forge chat --help |
| `--fork-session`, `--session-id <uuid>` | both rejected. Separate `forge fork <id> --turn N [--model]` counterfactual | **worse** | live; forge fork --help |
| `-n/--name` session names | none (titles only set by daemon or app) | **missing** | live; store has `set_session_title` for daemon/app only |
| `--no-session-persistence` | none. Closest: zero-retention config | **missing** | live; zero-retention.md |
| `-w/--worktree [name]`, `--tmux` | no flag. `worktree` model tool, daemon sessions in worktrees, `forge worktree`, `/duel` | **worse** | live; session-worktrees.md |
| `--mcp-config`, `--strict-mcp-config` | no flag. `.forge/mcp.toml`, `forge mcp add/import` | **worse** | live: rejected |
| `--settings`, `--setting-sources` | no flag. Layers: user config, project `.forge/config.toml`, `.forge/settings.json`; project-trust gate | **worse** | live: rejected; project-trust.md |
| `--agents <json>`, `--agent` | no flag. `.forge/agents/*.md` (and `.claude/agents`) | **worse** | live; agents.rs |
| `--plugin-dir`, `--plugin-url` | none. `forge plugin install owner/repo` (skill packs) | **worse** | forge plugin --help |
| `--bare`, `--safe-mode`, `--restricted` | none. Nearest: untrusted-project sanitizer + `forge trust` | **missing** | live; project-trust.md |
| `--debug [filter]`, `--debug-file`, `--verbose` | `RUST_LOG=...`; `--verbose` accepted as no-op | **worse** | main.rs:329-361 hints at RUST_LOG |
| `--chrome/--ide/--desktop/--teleport/--cloud/--remote-control/--from-pr/--file/--brief/--betas` | `/remote`, `forge serve`, `forge switch`, `forge browser attach`; the rest absent | **missing** | forge --help |
| `--ax-screen-reader` | none (only `reduce_motion`, `NO_COLOR`, ASCII glyph tier) | **missing** | tui-theme.md; no screen-reader code in forge-tui |
| `--permission-prompts host/none`, `--permission-prompt-tool` | headless asks are denied; `PermissionRequest` hook can answer even in headless | **equal** | hooks.md (PermissionRequest row) |

## 2. Subcommands

`claude` command list versus `forge --help`.

| Claude Code feature | Forge equivalent | Status | Evidence |
| --- | --- | --- | --- |
| `claude agents / attach / logs / stop / respawn / rm`, `--bg` (background sessions, agent view) | `forge serve` (multi-session daemon), `forge attach`, `forge board`, `forge sessions`, `forge send`, `forge service` | **better** | forge --help; remote-control.md; project-board.md. Daemon sessions survive client disconnects and are reachable from phone and desktop apps |
| `claude auth`, `claude setup-token` | `forge auth`, `forge setup`, `forge anywhere` | **equal** | forge --help |
| `claude doctor` | `forge doctor` (also probes live bridge turns, model discovery, store) | **better** | live: doctor output shows Bridge liveness / model health sections |
| `claude install/update/upgrade` (native auto-update) | `forge update [--check]`; update notice only, no background auto-update | **worse** | diagnostics-and-updates.md |
| `claude mcp add/add-json/add-from-claude-desktop/get/list/remove/serve/reset-project-choices` | `forge mcp add/remove/get/agent/tools/import/login/logout`; bare `forge mcp` lists. No `list`, `add-json`, desktop import; URL must use `--url` | **worse** | live: `forge mcp list` -> unrecognized subcommand; `forge mcp add --transport http n https://u` -> unexpected argument |
| `claude plugin` (install/enable/disable/marketplaces) | `forge plugin install/list/remove/update/marketplace`; skill packs only, no enable/disable | **worse** | forge plugin --help |
| `claude import codex/gemini/cursor` | `forge import claude/codex/cursor/aider/claude-session`. No gemini | **better** | forge import --help; import.rs also transfers permissions, hooks (--hooks) and statusLine although help text says only commands and skills |
| `claude auto-mode` (inspect classifier) | none; `forge mesh` explains routing, `permissions.auto_classifier` config | **missing** | forge --help |
| `claude purge` (delete all state for a project) | none: `forge sessions --archived`, `forge memory clear`, no project purge | **missing** | forge sessions --help |
| `claude ultrareview`, `/code-review ultra` | none cloud-hosted. Local multi-agent: `forge assay`, `/assay` | **missing** | forge assay --help |
| `claude gateway`, enterprise gateways | none | **missing** | forge --help |
| (no CC equivalent) fleet and daemon: `serve`, `anywhere`, `board`, `dispatch`, `send`, `service`, `queue`, `schedule` | present | **better** | forge --help |
| (no CC equivalent) `replay`, `fork`, `tree`, `blame`, `scoreboard`, `stats cache`, `bench` | present | **better** | forge --help |
| (no CC equivalent) `models`, `mesh`, `benchmarks`, `local` (Ollama), `api` (OpenAI-compatible endpoint), `switch`, `migrate`, `voice`, `lattice`, `nl` | present | **better** | forge --help |

## 3. Slash commands

CC: the commands table (about 120 rows incl. bundled skills, aliases and removed entries; https://code.claude.com/docs/en/commands.md). Forge: `COMMANDS` registry has 53 built-ins (crates/forge-tui/src/commands/registry.rs) plus 3 built-in command/skill entries (`/orchestrate`, `/rust`, `/rust-best-practices`); the palette shows 56. Unknown names print `unknown command: /x -- try /help` (live). Project and user commands and skills are added dynamically (`forge commands`).

| Claude Code feature | Forge equivalent | Status | Evidence |
| --- | --- | --- | --- |
| `/help`, `/keybindings` | `/help` (`/h`, `/?`), `/keys` (`/shortcuts`), `F1`, `?`, `Ctrl+,` configurator | **equal** | registry.rs; tui-help-overlay.txt |
| `/clear` (new conversation, empty context) and `/reset`, `/new` | `/new` (`/n`, `Ctrl+N`) starts a fresh session. `/clear` only clears the screen and keeps the session | **worse** | registry.rs:221 `clear: clear the screen (keep the session)`; commands.rs:401. Same word, different effect from CC |
| `/compact [instructions]` | `/compact [what to keep]`, `Ctrl+L`; plus `/uncompact` to restore | **better** | registry.rs; context-compaction.md |
| `/context` (usage grid, suggestions) | none; the context gauge in the status line and `/status` | **worse** | live: `unknown command: /context` |
| `/cost`, `/usage`, `/stats` | `/usage` (spend and tokens across providers), `/status` (quotas), `forge stats` | **better** | registry.rs; mesh quotas in /status |
| `/model`, `/effort`, `/fast`, `/advisor` | `/model` (`/models`), `/effort` (adds whitehot and `exact`), `Alt+M`, `Alt+E`. No fast mode or advisor | **equal** | registry.rs; whitehot-effort.md |
| `/permissions`, `/allowed-tools` | none. Rules live in `[[permissions.rules]]` in config; `Shift+Tab` and `/mode` switch temper | **missing** | live: unknown command |
| `/hooks` | none (hooks are config only) | **missing** | live: unknown command |
| `/memory` | `/memories`, `/remember <text>`, `forge memory`; no AGENTS.md editor | **worse** | registry.rs; memory.rs |
| `/init` | `/init` writes `.forge/AGENTS.md` | **equal** | registry.rs |
| `/mcp` (list, reconnect, enable, disable, OAuth) | `/mcp [server]` read-only listing; auth via `forge mcp login` | **worse** | registry.rs `/mcp`; mcp --help |
| `/plan [desc]` | `/plan <task>` then `/execute` (`/approve`, `/go`) | **equal** | registry.rs |
| `/rewind` (`/checkpoint`, `/undo`) | `/rewind` (`/undo`, `Esc Esc`), `/checkpoint [name]`, `/checkpoints` | **better** | rewind.md; one picker restores chat, context and files |
| `/resume`, `/continue`, `/branch`, `/fork`, `/rename` | `/resume`, `/sessions`; no in-session `/branch`, `/fork`, `/rename` (CLI has `forge fork`) | **worse** | parse_command in commands.rs lists no such names |
| `/export` | `/export [path]` writes one HTML file (CC writes plain text) | **equal** | registry.rs |
| `/copy [N]` | `/copy [N]` (`/yank`), `Ctrl+Shift+C` | **equal** | registry.rs; CC adds a block picker |
| `/btw` | `/btw` (`/side`), works while a turn runs | **equal** | side-questions.md |
| `/diff` | none; diffs only inside tool cards and permission prompts | **missing** | live: unknown command |
| `/review`, `/code-review`, `/security-review`, `/simplify` | none built in. `/assay`, `/commit`, `/pr`. CC skills can be imported with `forge import claude` | **missing** | registry.rs; forge commands lists only /orchestrate /rust /rust-best-practices |
| `/verify`, `/run`, `/debug`, `/batch`, `/loop`, `/schedule`, `/deep-research` | `/loop` (gated retry loop), `/goal`, `/heartbeat`, `/dispatch`, `/workflow`, `/duel`; `forge schedule`. No `/verify` `/run` `/debug` | **equal** | registry.rs |
| `/goal` | `/goal <objective> [--gate cmd] [--max-tokens N] [--max-minutes N]` | **better** | autonomous-gates.md; goal-evaluator.md |
| `/add-dir`, `/cd` | none (config `extra_roots`) | **missing** | live: unknown command |
| `/doctor` | CLI `forge doctor` only | **worse** | registry.rs |
| `/config`, `/settings` | `/config` (`/cfg`, `/settings`) wizard for keys, plans, search | **equal** | config-command.md |
| `/theme`, `/color`, `/output-style`, `/vim`, `/terminal-setup`, `/tui` | none in-session. Theme via `[tui]` config and `FORGE_THEME`; `--inline` / `--fullscreen` flags | **missing** | tui-theme.md |
| `/statusline` | `/statusline [layout, toggle, reset, edit]` plus CC-style `[statusline] command` | **equal** | claude-code-parity.md |
| `/login`, `/logout` | `forge auth`, `/config` wizard | **equal** | forge auth --help |
| `/tasks`, `/bashes`, `/background`, `/stop` | task panel (`Ctrl+O`), model tool `shell_job`; no user command to list or kill jobs | **worse** | tui-help-overlay.txt (Ctrl+O); shell-tool.md |
| `/skills`, `/reload-skills`, `/reload-plugins`, `/plugin` | palette `skill` tag and `Ctrl+K` Skills tab; `Alt+R` reloads keybinds/catalog; plugins CLI only | **worse** | tui-interaction.md |
| `/remote-control`, `/mobile`, `/desktop`, `/teleport` | `/remote [--lan, --local, --anywhere]`, `/anywhere`, `/claude` (continue in Claude Code) | **better** | registry.rs; remote-control.md |
| `/recap` | automatic `recap` line after each turn (`[recap] enabled`); no on-demand command | **equal** | RecapConfig lib.rs:1194 |
| `/feedback`, `/bug`, `/release-notes`, `/insights`, `/powerup` | none (`forge tour` is outside the session) | **missing** | forge --help |
| `/sandbox` | config `[shell] sandbox = true` | **worse** | shell-sandbox.md |
| `/voice` | `/voice` (`Ctrl+V`), local whisper.cpp, never auto-sends | **better** | voice.md; works offline |
| (no CC equivalent) `/mesh`, `/models`, `/assay`, `/duel`, `/workflow`, `/dispatch`, `/board`, `/lattice`, `/refine`, `/reground`, `/subagents`, `/self-mcp`, `/replay`, `/commit`, `/pr`, `/image`, `/output`, `/thinking` | present | **better** | registry.rs |

## 4. Keybindings and input editing

CC: interactive-mode.md and keybindings.md. Forge: `crates/forge-tui/src/driver.rs` (poll_event key map), keybinds.md and the live F1 overlay (evidence/tui-help-overlay.txt). Rows marked `collision` are chords both tools bind to different actions.

| Claude Code feature | Forge equivalent | Status | Evidence |
| --- | --- | --- | --- |
| `Ctrl+C` interrupt; idle clears draft; twice quits | same; draft saved to history, `Up` recalls it | **equal** | keybinds.md row `interrupt` |
| `Esc` interrupts keeping work; `Esc Esc` clears draft or opens rewind | `Esc` interrupts; idle `Esc Esc` opens rewind picker; never quits | **equal** | rewind.md Keys |
| `Shift+Tab` cycles permission modes | `Shift+Tab` cycles Ask, Auto-edit, Auto, Read-only (bypass excluded). Default temper at launch is Auto-edit | **equal** | live: BTab x4 cycled Auto-edit to Auto to Read-only to Ask to Auto-edit |
| `Alt+P` model, `Alt+T` thinking, `Alt+O` fast | collision: `Alt+M` model, `Alt+E` effort, `Alt+T` cycles temper | **worse** | keybinds.md; Alt+T means different things |
| `Ctrl+R` reverse history search | collision: `Ctrl+R` toggles reasoning display. No history search; `Up/Down` recall only | **missing** | live: Ctrl+R printed `reasoning display on`; driver.rs:489 |
| `Ctrl+G` / `Ctrl+X Ctrl+E` edit prompt in `$EDITOR` | none. Live: `Ctrl+G` types a literal `g` | **missing** | evidence/tui-unhandled-ctrl-chords.txt |
| `Ctrl+S` stash prompt | collision: `Ctrl+S` saves a checkpoint | **missing** | keybinds.md `checkpoint` |
| `Ctrl+L` redraw | collision: `Ctrl+L` runs compaction (a model call) | **worse** | keybinds.md `compact`; accidental press spends tokens |
| `Ctrl+V` paste image | `Ctrl+V` reads clipboard image, falls back to `/voice`; paths pasted or dragged attach as images or `@path` | **better** | image-input.md |
| `Ctrl+B` background a running command | none. Live: `Ctrl+B` types `b` | **missing** | evidence/tui-unhandled-ctrl-chords.txt |
| `Ctrl+T` task checklist | collision: `Ctrl+T` expands the latest tool card; tasks and agents are `Ctrl+O` | **equal** | help overlay |
| `Ctrl+O` transcript viewer with timestamps and model | collision: toggles task and agent details. `/output` opens full tool output with search | **worse** | help overlay; output_view.rs |
| `Ctrl+D` exit (first press hint) or delete char | none. Live: `Ctrl+D` types `d` | **worse** | evidence/tui-unhandled-ctrl-chords.txt |
| `Ctrl+Z` suspend | collision: `Ctrl+Z` undoes the last file write | **worse** | keybinds.md `undo` |
| `Ctrl+N` / `Ctrl+P` history | collision: `Ctrl+N` starts a new session; `Ctrl+P` types `p` | **worse** | keybinds.md `new_session`; live |
| `Ctrl+K` delete to end of line, `Ctrl+Y` yank, `Alt+Y` cycle, `Ctrl+_` undo | collision: `Ctrl+K` opens command center (`Alt+K` kills line). `Ctrl+Y` types `y`. No yank ring, no input undo | **worse** | keybinds.md (`command_center` Ctrl-K, `kill_line_forward` Alt-K); live |
| `Ctrl+A/E/W/U`, `Alt+B/F/D`, word moves | `Ctrl+A/E/W/U`, `Alt+B/F`, `Ctrl+Left/Right`, `Alt+Backspace`. No `Alt+D` | **equal** | driver.rs:474-496 |
| Multiline: `\ Enter`, `Shift+Enter`, `Ctrl+J`, `Option+Enter` | `Ctrl+J`, `Shift+Enter`, `Alt+Enter`. No backslash-Enter | **equal** | driver.rs Enter arm; help overlay; live: `abc\` then Enter submitted the message |
| `Ctrl+Enter` / `Ctrl+X Ctrl+S` send queued now; `Up` takes queued back | none: Enter queues, queued prompts are absorbed at the next tool boundary (steering); `Up` is history only | **worse** | steering.md |
| `Ctrl+X Ctrl+K` stop all background subagents | none; `Esc` interrupts the turn | **missing** | - |
| `!` prefix shell mode | none. Live: `!echo hello` is sent to the model as a prompt | **missing** | evidence/tui-bang-shell-mode.txt |
| `@` file mention with autocomplete | `@path` picker (fuzzy, git-modified marks, directory descent) | **equal** | live: `@RE` opened `@ files` popup with README.md |
| `/` command palette incl. mid-prompt completion | `/` palette, recent-first, usage line; tokens recognised anywhere in the line; `Ctrl+K` command center with tabs | **equal** | tui-interaction.md; commands.rs slash_token_at |
| `:` emoji shortcodes, spell check | none | **missing** | - |
| Vim editor mode | none | **missing** | no vim handling in forge-tui |
| `?` on empty prompt toggles shortcut help | `?` or `F1` opens Help takeover (Commands, Keys, Mouse) | **better** | tui-interaction.md Help; searchable |
| Customisable keybindings (`keybindings.json`, chords, contexts) | `[keybinds.binds]` per action with in-TUI editor (`Ctrl+,`); single combos only, no chords | **equal** | keybinds.md |
| Bracketed paste collapses long text to a chip | `[pasted text (30 lines)]` chip | **equal** | live: 30-line paste |
| Voice: hold Space to dictate | `Ctrl+V` overlay, tap to record; local whisper | **equal** | voice.md |
| Unbound Ctrl chord is ignored | unbound Ctrl+letter inserts the plain letter into the draft | **worse** | driver.rs:498-499 `KeyCode::Char(c) => KeyKind::Char(c)` with no CONTROL check; live `Ctrl+X,Y,P,Q,G,D,B` produced `xypqgdb` |

## 5. Interactive UX features

Behaviour of the running session. CC: interactive-mode.md, fullscreen.md. Forge: tui-*.md docs plus live captures.

| Claude Code feature | Forge equivalent | Status | Evidence |
| --- | --- | --- | --- |
| Queue messages while Claude works; steer at next tool boundary | same: prompt queued and absorbed at the turn's next boundary as `you steer`; slash commands wait | **equal** | steering.md; composer placeholder `queue a follow-up…` |
| Prompt suggestions (grey next-prompt, Tab/Right accepts) | `try: <text>` italic in composer, Tab accepts (`[suggest] enabled`) | **equal** | live composer showed `try: Let me inspect…` |
| Session recap line | `※ recap` line after turns | **equal** | live headless output |
| Task checklist panel | tasks and agents band (`update_tasks`), `Ctrl+O` details | **equal** | tui-panels.md |
| Fullscreen renderer with mouse, scrollback and `/tui` switch | fullscreen by default with mouse; `--inline` renders in native scrollback; no in-session switch | **equal** | forge chat --help |
| Conversation search in transcript mode (`/`, `n/N`), `[` dump to scrollback, `v` open in editor | search only inside `/output` tool viewer. `--inline` leaves search to the terminal | **worse** | output_view.rs:174; tui-inline-scrollback.md:86 |
| `/focus` view | none | **missing** | - |
| Diff panel and `/diff` (per-turn views, ask-about-file) | inline diffs in tool cards; `Ctrl+T` expands; no working-tree diff view | **worse** | tui-rich-rendering.md; live unknown command |
| Syntax highlighting, markdown tables, code blocks with copy | yes (render/code.rs, table.rs), `/copy` in code header | **equal** | tui-rich-rendering.md |
| Permission prompt with diff preview and 3 choices | decision block: Allow once, Always allow (scoped rule), Deny; `Ctrl+T` expands diff; digits and y/a/n | **equal** | tui-interaction.md; interaction_ui.rs:75 |
| Permission prompt: add a comment to Yes/No, Esc declines | no comment/feedback path; Esc or n denies | **worse** | interaction_ui.rs:75-110 returns only Allow, AlwaysAllow, Deny |
| Permission prompt: allow for this session without persisting | none for built-in tools; `a` persists a scoped rule to `.forge/config.toml` (MCP tools get a session-only rule) | **worse** | run.rs:3038-3050; tool_dispatch.rs:553-575 |
| Rewind menu: restore code and conversation / conversation only / code only / summarize from here | one action: rewind chat, context and files to a chosen message; message returns to the composer | **worse** | live rewind picker: footer `Enter rewind to this point` |
| Checkpoint every prompt; not tracking bash edits | same (shadow snapshots of tool-written files; bash edits not tracked) | **equal** | snapshot.rs:1-8 |
| Image paste, drag-drop, `@image` | yes, plus Wayland/X11/macOS/Windows clipboard backends | **better** | image-input.md |
| PDF reading via Read (`pages`) | PDF paths recognised but not attachable | **worse** | image-input.md ('PDF paths are recognised but not yet attachable') |
| Desktop notifications / terminal bell on turn end and prompts | `[notifications]` desktop and bell, `min_turn_secs`, focus aware | **equal** | claude-code-parity.md |
| Auto-continue at usage limit | not needed: mesh fails over to another provider or model | **better** | mesh-routing.md |
| PR/MR status badge in footer, OSC-8 hyperlinks for issue refs | branch and worktree marker only; no hyperlinks | **missing** | tui-header-statusline.md; no OSC 8 in forge-tui |
| `--resume` picker with search; background sessions marked | `/sessions`, `forge sessions`, picker; daemon sessions | **equal** | forge sessions --help |
| Cleared-draft recovery (last 10 drafts, `Up` brings them back) | `Ctrl+C` saves the one cleared draft to history, `Up` recalls it; no multi-draft buffer | **equal** | rewind.md Keys |

## 6. Permissions, modes and safety

CC: permission-modes.md, permissions.md, sandboxing.md. Forge: temper-modes.md, fix-permission-rules.md, project-trust.md, shell-sandbox.md.

| Claude Code feature | Forge equivalent | Status | Evidence |
| --- | --- | --- | --- |
| Modes: default (Manual), acceptEdits, plan, auto, dontAsk, bypassPermissions | Ask, Auto-edit, Auto, Read-only (=plan), Full (=bypass). No dontAsk | **equal** | temper-modes.md table |
| Default mode (Manual) asks before edits and non-read-only commands | default temper is Auto-edit: workspace edits and ordinary shell run without a prompt, risky shell and writes outside the workspace still ask | **better** | live TUI border `Auto-edit`; fewer interruptions, but a looser default than CC (trade-off) |
| Auto mode with classifier | `auto`: builtin Safe/Risky lists plus a bounded cheap-model classifier for Unknown commands | **equal** | temper-modes.md; auto_classifier.rs |
| Rule syntax `Bash(cmd:*)`, `Read(path)`, `WebFetch(domain:)`, MCP rules, allow/ask/deny | `[[permissions.rules]]` with glob patterns per tool, deny beats allow, specificity; `forge import claude` converts CC lists; WebFetch rules skipped | **equal** | fix-permission-rules.md 5.6; import.rs transfer_permissions |
| Protected and critical paths, catastrophic deletes | unoverridable safety floor: catastrophic deletes of home or system dirs, privilege escalation (`sudo`, `su`, `pkexec`) and secret denylist in every temper including Full | **better** | temper-modes.md Safety floor |
| Workspace trust dialog | granular untrusted-project sanitizer: keeps only restrictive and cosmetic settings until `forge trust`; always-allow answers stored outside the repo | **better** | project-trust.md |
| OS sandbox (Seatbelt, bubblewrap) with fs and network allowlists | opt-in Landlock write confinement on Linux only; PTY path unconfined; no network control; no macOS/Windows | **worse** | shell-sandbox.md Scope + limits |
| Managed (enterprise) settings and org policy, `allowManagedPermissionRulesOnly` | none | **missing** | no managed or policy layer in forge-config |
| Local gitignored settings (`settings.local.json`) | none: project config is committed `.forge/config.toml` | **missing** | forge-config lib.rs load order |
| `additionalDirectories`, `/add-dir` | `[tools] extra_roots`; writes outside workspace ask even in Auto-edit | **equal** | temper-modes.md File tools |
| `PermissionDenied`, auto-mode denial review in `/permissions` | warnings with the reason; no review list | **worse** | temper-modes.md (warning text) |
| Headless: asks denied unless allowed or hook answers | same, plus `PermissionRequest` hook works headless and on the bridge | **equal** | hooks.md |

## 7. Plan mode

CC: permission-modes.md plan section. Forge: temper-modes.md, plan-and-dispatch.md, `/plan`.

| Claude Code feature | Forge equivalent | Status | Evidence |
| --- | --- | --- | --- |
| Enter via `Shift+Tab`, `/plan [desc]`, `--permission-mode plan` | Read-only temper via `Shift+Tab` or `--mode plan`; `/plan <task>` | **equal** | registry.rs; forge run --help |
| Plan approval dialog (accept edits, manual, keep planning) | plan card then `/execute` (switches to Auto-edit); `present_plan` tool | **equal** | registry.rs `/execute` |
| Edit the plan in `$EDITOR` (`Ctrl+G`) and pick the follow-on mode on approval, or keep planning | plan card; `/execute` always switches to Auto-edit; no editor round-trip; to keep planning just keep chatting | **worse** | registry.rs `/execute: switches to Auto-edit`; CC permission-modes.md 'Review and approve a plan' |
| Parallel decomposition after approval (`/batch`) | `/dispatch` proposes a split, approval gate, one worktree session per item; `forge board` tracks them | **better** | plan-and-dispatch.md; forge dispatch --help |
| `opusplan` (plan with one model, execute with another) | mesh tiering per turn and `/subagents`; not a dedicated setting | **equal** | mesh-routing.md |

## 8. Project instructions and memory

CC: memory.md, large-codebases.md. Forge: `agents_md_candidates` / `read_project_agents_md` in crates/forge-core/src/lib.rs:1957-1990, lean_context.rs, memory docs.

| Claude Code feature | Forge equivalent | Status | Evidence |
| --- | --- | --- | --- |
| `CLAUDE.md` loaded from the working dir and every parent dir, concatenated root-down | ONE file: the first non-empty of `.forge/AGENTS.md`, `AGENTS.md`, `CLAUDE.md`, `.claude/CLAUDE.md`, `<config>/AGENTS.md`, `~/.forge/AGENTS.md`, `~/.claude/CLAUDE.md`. No concatenation, no parent walk | **worse** | lib.rs:1957-1990 `read_project_agents_md` returns the first non-empty body. A project `AGENTS.md` therefore hides your global `~/.claude/CLAUDE.md` |
| User-level `~/.claude/CLAUDE.md` always loads alongside project files | only used if no project-level file exists | **worse** | same; CC memory.md 'How CLAUDE.md files load' and the AGENTS.md table |
| `CLAUDE.local.md` (personal, gitignored) | none | **missing** | agents_md_candidates has no local variant |
| Subdirectory `CLAUDE.md` lazy-loaded when a file there is read | none | **missing** | live: sandbox `sub/CLAUDE.md` never referenced; code has no lazy loader |
| `@path` imports inside CLAUDE.md (recursive, external approval) | only the exact line `@AGENTS.md` is recognised, and only to avoid double-injecting on the claude bridge | **missing** | lean_context.rs `claude_md_covers` |
| `.claude/rules/*.md` with `paths:` globs, `claudeMdExcludes` | none | **missing** | no rules loader in forge-core |
| `AGENTS.md` fallback when no CLAUDE.md | supported (and ranks above CLAUDE.md) | **equal** | lib.rs:1957-1966 |
| HTML comments stripped from CLAUDE.md | not stripped | **missing** | - |
| Auto memory (MEMORY.md written by Claude, `/memory` toggles) | auto memory capture into a per-project store: `forge memory`, `/remember`, `/memories`; injected as context | **equal** | forge memory --help; memory_scope.rs |
| `/init` generates CLAUDE.md | `/init` generates `.forge/AGENTS.md` (also `forge import claude` copies CLAUDE.md into it) | **equal** | registry.rs; import.rs:127-153 |
| Project config/commands/skills found from a subdirectory of the repo | not found: project `.forge/` is read from the cwd only | **worse** | live: command defined in `proj/.forge/commands` listed from `proj`, absent from `proj/sub/deep`; lib.rs test `parent_project_config_is_not_inherited` (deliberate for config.toml, but it also hides skills/commands/AGENTS) |

## 9. Settings, configuration and environment

CC: settings.md, settings-reference.md, env-vars.md. Forge: config-command.md, project-trust.md, `docs/config-schema.json`.

| Claude Code feature | Forge equivalent | Status | Evidence |
| --- | --- | --- | --- |
| `settings.json` layers: managed, user, project, local | TOML: user `config.toml`, project `.forge/config.toml`, CC-shaped `settings.json` for hooks/statusLine; env `FORGE_*` beats config | **worse** | project-trust.md; no managed or local layer |
| `/config` settings UI, `/config key=value` | `/config` wizard (keys, plans, search, permission mode, conservation); `forge` writes with comment-preserving toml_edit | **equal** | config-command.md |
| JSON schema, validation messages with file location | typed load errors with `file:line:col`, unknown-key warnings; `docs/config-schema.json` | **equal** | config-command.md 'How config files are written' |
| Env var block in settings (`env`) | no `env` table; `FORGE_*` variables only | **worse** | - |
| Language/locale setting, `apiKeyHelper`, credential helpers | keys in OS keyring or encrypted file (`forge auth`); reply language follows the user's message | **equal** | claude-code-parity.md |
| Provider breadth: Anthropic, Bedrock, Vertex, Foundry, gateways | 17+ providers, custom OpenAI-compatible endpoints, subscription bridges (claude, codex, agy), Ollama | **better** | forge provider --help; provider-integrations.md |
| Statusline command with JSON on stdin | `[statusline] command` (reads CC scripts unchanged; `forge import claude` copies it) plus built-in widgets and layout editor | **equal** | claude-code-parity.md; StatuslineConfig.command |
| Output styles (`/output-style`, custom `.claude/output-styles`) | none | **missing** | no output-style code in forge-config or forge-tui |
| Themes: auto, light, dark, daltonized, ANSI, custom `~/.claude/themes` | `[tui] theme = auto/dark/light`, colour tier (truecolor/256/16/none), glyph tier, `reduce_motion`; no named or custom themes | **worse** | tui-theme.md table |
| Telemetry via OpenTelemetry export | anonymous usage counts only (opt-out); local `forge stats` | **worse** | main.rs telemetry notice; telemetry.md |
| Auto-update channel settings | `forge update`; update notice | **worse** | diagnostics-and-updates.md |
| Cost and token transparency (`/usage`, statusline cost fields) | per-turn tokens and cost in the status line, `/usage` across providers, quota views in `/status`, per-turn cost in `forge replay` | **equal** | forge replay --help; CC costs.md and statusline.md |

## 10. Hooks

CC: hooks.md (lifecycle table lists 33 events; 5 handler types). Forge: `HookEvent` enum in crates/forge-config/src/lib.rs:661 (11 events), hooks.md, hooks_cc.rs, hooks_llm.rs.

| Claude Code feature | Forge equivalent | Status | Evidence |
| --- | --- | --- | --- |
| `PreToolUse` (block, rewrite, ask/allow/deny) | yes. `updatedInput` supported; `permissionDecision allow` does NOT skip Forge's permission gate; `ask`/`defer` ignored | **equal** | hooks.md CC table, PreToolUse row |
| `PostToolUse` | yes (context injection, block reason fed back) | **equal** | hooks.md |
| `UserPromptSubmit` | yes; context appended visibly to the prompt text; `sessionTitle`, `suppressOriginalPrompt` ignored; NOT fired by one-shot `forge run` | **equal** | hooks.md line 89 and 125 |
| `SessionStart` / `SessionEnd` | yes; `source` is always `startup` (no resume/clear/compact); not fired by `forge run` | **equal** | hooks.md line 88 |
| `Stop` with block and `stop_hook_active` | yes, capped by `stop_hook_max_blocks` (default 8) | **equal** | hooks.md line 86 |
| `SubagentStop` | yes; block appends reason to the `spawn_agents` result | **equal** | hooks.md line 87 |
| `PermissionRequest` | yes; also fires headless and on the CLI bridge | **better** | hooks.md line 92 |
| `Notification`, `PreCompact`, `PostCompact` | yes | **equal** | lib.rs:661-696 |
| `PostToolUseFailure`, `PostToolBatch`, `PermissionDenied` | none | **missing** | not in HookEvent |
| `SubagentStart`, `TaskCreated`, `TaskCompleted`, `TeammateIdle` | none | **missing** | not in HookEvent |
| `StopFailure`, `Setup`, `UserPromptExpansion`, `MessageDisplay` | none | **missing** | not in HookEvent |
| `InstructionsLoaded`, `ConfigChange`, `CwdChanged`, `FileChanged`, `DirectoryAdded`, `WorktreeCreate/Remove` | none | **missing** | not in HookEvent |
| `PreModelSwitch`, `PostModelSwitch`, `Elicitation`, `ElicitationResult` | none | **missing** | not in HookEvent |
| Handler types: command, http, mcp_tool, prompt, agent | command, prompt, agent. `http` is skipped at load | **worse** | hooks.md 'Not implemented' |
| Handler options: `if` filter, `async`, `once`, `statusMessage`, parallel execution | none of these; matched hooks run one after another | **worse** | hooks.md 'Not implemented'; hooks.rs:158-168 sequential loop |
| Matchers with regex and MCP names | regex matchers tested against Forge name and CC alias; `server__tool` for MCP | **equal** | hooks.md line 81 |
| Hooks defined in skill and agent frontmatter | none (skills have no hooks field) | **missing** | frontmatter.rs consumers |
| Import existing hooks | `forge import claude --hooks` (opt-in) | **equal** | forge import claude --help |
| `/hooks` viewer | none | **missing** | section 3 |

## 11. Skills, custom commands and plugins

CC: skills.md frontmatter table. Forge: command-skill-system.md, crates/forge-skills (frontmatter.rs, template.rs), forge plugin.

| Claude Code feature | Forge equivalent | Status | Evidence |
| --- | --- | --- | --- |
| Skill = `SKILL.md` directory with resources; model-invocable | same layout; `use_skill` tool lists them for the model; project and user scopes; builtins `/orchestrate`, `/rust` | **equal** | command-skill-system.md 'use_skill' |
| Custom commands `.claude/commands/*.md` with `$ARGUMENTS` | same files read via `forge import claude` or `.forge/commands`; `$ARGUMENTS`, `$1..$9`, `$NAME` | **equal** | template.rs |
| Argument indexing `$ARGUMENTS[N]`, `$N` is 0-based | `$1` is the first argument; `$0` and `$ARGUMENTS[N]` left literal | **worse** | template.rs `n >= 1`; CC skills.md substitutions table |
| Frontmatter `allowed-tools` / `disallowed-tools` | read leniently and ignored | **missing** | tests.rs:539-544 (parses, never consumed); frontmatter.rs |
| Frontmatter `disable-model-invocation`, `user-invocable` | none | **missing** | forge-skills Command/SkillMeta fields: name, description, args, tier, resources |
| Frontmatter `argument-hint`, `arguments` | `args: [a, b?]` names the arguments and the palette derives a hint from them; the CC `argument-hint` string is not read | **equal** | command-skill-system.md Frontmatter schema; forge-skills lib.rs Command::arg_hint |
| Frontmatter `model`, `effort` | `tier:` routing hint (trivial/standard/complex); CC `model` read for commands, `effort` ignored | **equal** | command-skill-system.md |
| Frontmatter `context: fork` + `agent` (run in subagent) | none | **missing** | - |
| Frontmatter `hooks`, `paths` (auto-activate by file glob), `shell` | none | **missing** | - |
| Dynamic context injection ``!`cmd` `` and `${CLAUDE_SKILL_DIR}` / `${CLAUDE_SESSION_ID}` | none: values substitute as-is, no shell parsing | **missing** | template.rs header comment |
| Chain several skills in one message (`/a /b task`) | only the first `/name` is dispatched; the rest is its argument text | **worse** | forge-skills lib.rs Catalog::resolve:511-530 |
| Bundled skills: `/batch /loop /debug /verify /run /simplify /code-review /doctor /claude-api /dataviz ...` | `/orchestrate`, `/rust`, `/rust-best-practices` only | **worse** | live `forge commands` |
| Skill marketplace and plugin bundles (skills + agents + hooks + MCP + LSP + styles + themes) | `forge plugin` installs skill packs from GitHub or marketplaces with a lockfile and `update`; hooks, agents, MCP not bundled; no enable/disable | **worse** | forge plugin --help; marketplace.rs:309 ignores hooks/agents/commands/.mcp.json inside packs |
| Export/import of one's library | `forge skill export/import`, `forge migrate` | **better** | forge skill --help |
| Distil a session into a skill | `forge skill from-session` | **better** | forge skill --help |

## 12. Subagents, parallel work and worktrees

CC: sub-agents.md, agents.md, worktrees.md, agent-teams.md. Forge: subagent-orchestration.md, agents.rs, session-worktrees.md, plan-and-dispatch.md.

| Claude Code feature | Forge equivalent | Status | Evidence |
| --- | --- | --- | --- |
| Agent tool spawns subagents with own context | `spawn_agents` virtual tool; mesh routes each child; concurrency, per-provider and depth caps; cost rolls up | **better** | subagent-orchestration.md |
| Built-in subagent types (Explore, Plan, general-purpose) | none named; default read-only tool set for children; `.forge/agents/*.md` custom types | **worse** | agents.rs:32-42 |
| Agent file frontmatter: name, description, tools, disallowedTools, model, permissionMode, maxTurns, skills, mcpServers, hooks, memory, background, effort, isolation, color | name, description, tools, tier, model (+ body prompt) | **worse** | agents.rs AgentDef:32-42 and parse_agent:158-180 |
| Load `.claude/agents` and `~/.claude/agents` | merged in layers, `haiku/sonnet/opus` mapped to tiers | **equal** | claude-code-parity.md; agents.rs:44-57 |
| `--agents` JSON, `/agents` UI | none | **missing** | section 1 and 3 |
| Foreground/background subagent control, `Ctrl+B`, fork (`/subtask`, `/fork`) | subagents run in parallel and report back; detached subagents exist; no user-facing fork-with-context or background toggle | **worse** | detached_subagents.rs; no /subtask |
| Resume a subagent / SendMessage to it | `send_to_agent`, `list_subagents`, `cancel_subagent`, `message_session` tools | **equal** | subagent/requests.rs consts |
| Agent teams (experimental) | `/duel` (race models), `/dispatch`, `/workflow` (JS orchestration); fleet messaging | **equal** | duel.md; forge-workflow.md |
| Worktree isolation (`-w`, `isolation: worktree`, `EnterWorktree`) | `worktree` tool (create/enter/list/exit/remove), `[mesh.subagents] worktree_isolation`, dispatched items default to their own worktree, `forge worktree reclaim` | **equal** | session-worktrees.md; forge worktree --help |
| Parallel background sessions (`claude --bg`, agent view) | `forge serve` + `forge board` (Needs you / Working / Ready / Done) + `forge attach`; phone and desktop apps | **better** | project-board.md; remote-control.md |
| Overnight unattended runs | `forge queue` (worktree + branch per task, budget-capped, digest) + `forge schedule` | **better** | queue-autopilot.md |
| Model arena / counterfactual | `/duel`, `forge fork --turn N --model`, `forge scoreboard` | **better** | duel.md; fork-counterfactual.md |

## 13. MCP

CC: mcp.md. Forge: mcp-client.md, crates/forge-mcp (handler.rs, transport.rs, oauth.rs), forge-config/src/mcp.rs.

| Claude Code feature | Forge equivalent | Status | Evidence |
| --- | --- | --- | --- |
| stdio, SSE, streamable HTTP transports | stdio, SSE, streamable HTTP via rmcp | **equal** | transport.rs:15,160,184 |
| WebSocket transport | none | **missing** | transport.rs |
| OAuth login for remote servers | `forge mcp login/logout`, token store, per-account logout | **equal** | oauth.rs; forge mcp --help |
| Scopes local/project/user; project `.mcp.json` shared with approval prompt | scopes local/project/user into `.forge/mcp.toml` and user `mcp.toml`; `.mcp.json` is import-only, not auto-loaded; untrusted project servers are not started | **worse** | mcp --help; project-trust.md |
| Import from other tools | `forge mcp import` auto-scans Claude Code, Codex, Cursor, Claude Desktop, Windsurf, VS Code and lets you pick; secrets never copied to TOML | **better** | mcp-client.md 5.3; mcp.rs:122 |
| Tool search / deferred loading for large servers | `mcp_search_tools` + `mcp_call`, `max_eager_tools` | **equal** | mcp-client.md 5.4; deferred-tools.md |
| Resources and prompts | `mcp_list_resources`, `mcp_read_resource`, `mcp_get_prompt` tools; no `@server:resource` mentions; prompts are not slash commands | **worse** | lib.rs:78 MCP_GET_PROMPT; no palette hook |
| Dynamic `list_changed` updates, reconnect | handled (`tools/list_changed`), bounded auto-reconnect, `connect_timeout_secs`; no in-session `/mcp reconnect` | **equal** | handler.rs:17 |
| Elicitation (and roots, sampling) | client advertises and answers sampling, roots and elicitation | **equal** | handler.rs:9-17; CC exposes elicitation through hooks |
| Expose the agent as an MCP server | `forge mcp agent` (stdio `forge_chat`, `forge_status`, `forge_set_mode`), `/self-mcp` | **equal** | forge mcp --help |
| Plugin-provided and managed MCP servers | none | **missing** | marketplace.rs:309 |
| `claude mcp add` CLI shape: `add -t http name URL -H ...`, `list`, `add-json` | `add name --transport http --url URL --header K=V`; no positional URL, no `list`, no `add-json` | **worse** | live (both rejected); forge mcp add --help claims compatibility |

## 14. Context, compaction, sessions and checkpoints

CC: context-window.md, checkpointing.md, sessions.md. Forge: context-compaction.md, rewind.md, session-*.md.

| Claude Code feature | Forge equivalent | Status | Evidence |
| --- | --- | --- | --- |
| Automatic compaction, `/compact [instr]`, `/autocompact` window | auto-compact with policy and budget; `/compact [keep]`; `[compact_cap]` per model; `/uncompact` restores full transcript | **better** | context-compaction.md; registry.rs |
| Context breakdown view (`/context`) | status-line gauge (`ctx 12% of 32k`), `/status` context section; no per-category breakdown | **worse** | live: unknown command /context |
| Per-model real context windows and trim | windows fetched per model and used to trim and auto-compact; consent prompt on failover to a smaller window (needed because Forge is multi-model) | **equal** | crates/forge-cli/src/context_windows.rs |
| Prompt-cache hit visibility (statusline `prompt_cache` fields, `/usage` line, miss causes) | `forge stats cache [--session [ID]]`: hit rate per model and the cause of each miss; cache accounting also in the status line tokens | **equal** | forge stats --help; CC statusline.md 'Prompt cache fields' |
| Resume by id/name, picker, `--continue` | ids and prefixes, picker, `--continue`; names absent | **equal** | forge chat --help |
| `/branch`, `/fork` a conversation | `forge fork <id> --turn N [--model] [--rerun]` (conversation only, files not rewound), `forge tree` lineage | **equal** | forge fork --help |
| Rewind code and/or conversation to a checkpoint | `/rewind` restores chat + context + files together; handles CLI-bridge epoch, compaction and latches | **equal** | rewind.md |
| Rewind granularity and 'summarize from here' | single combined action | **worse** | live rewind picker |
| Session export | `/export` HTML, `forge replay --html/--json`, secrets redacted by default | **better** | forge replay --help |
| Continue the same conversation in the other tool | `forge switch` (Forge to and from Claude Code), `forge import claude-session`, `/claude` | **better** | forge switch --help |
| Move session between machines | `forge migrate`, host-move guard | **better** | session-host-move.md |
| Session naming, `--name`, `/rename` | none in terminal | **missing** | section 1 and 3 |
| Cloud/teleport sessions | none (daemon sessions are local-first; Anywhere relays to phone) | **missing** | forge-anywhere.md |

## 15. Built-in model tools

CC: tools-reference.md. Forge: `impl Tool for` sites in forge-tools plus virtual tools in forge-core (names from consts).

| Claude Code feature | Forge equivalent | Status | Evidence |
| --- | --- | --- | --- |
| Read / Write / Edit / MultiEdit / NotebookEdit | `read_file`, `write_file`, `append_file`, `edit_file`, `multi_edit`, `apply_patch`, `delete_file`, `notebook_edit` | **equal** | forge-tools/src/core_tools*.rs |
| Glob / Grep | `glob`, `search`, `list_dir` | **equal** | discovery_tools.rs |
| Bash with background jobs | `shell` (pty option, sandbox, error interceptor), `shell_job` (background; the model is woken when a job exits) | **equal** | shell-tool.md; hooks.md job wake-ups |
| Monitor (stream each output line or WebSocket message back to the model) | none: background jobs wake the model on exit only, not per line | **missing** | hooks.md 'Background job wake-ups' |
| WebFetch / WebSearch | `web_fetch` (paged Markdown), `web_search` (local SearXNG or Brave) | **equal** | web-tools.md |
| Agent, SendMessage, ListAgents, TaskStop | `spawn_agents`, `send_to_agent`, `list_subagents`, `cancel_subagent`, `message_session`, `list_sessions` | **equal** | const names in forge-core |
| TaskCreate/Update/List (checklist) | `update_tasks` | **equal** | task-tracking.md |
| AskUserQuestion | `ask_user` form (single/multi-select, Other, notes) | **equal** | ask-user-question.md |
| EnterPlanMode / ExitPlanMode | `present_plan` plus Read-only temper | **equal** | lib.rs PRESENT_PLAN_TOOL |
| EnterWorktree / ExitWorktree | `worktree` tool | **equal** | session-worktrees.md |
| Skill | `use_skill` | **equal** | lib.rs USE_SKILL_TOOL |
| CronCreate / ScheduleWakeup / RemoteTrigger | `manage_heartbeats`, `schedule_wakeup`; OS timers via `forge schedule`; no cloud triggers | **equal** | heartbeat.rs; wakeup.rs |
| PushNotification | `push_notification` (desktop, phone, ntfy-style channels, whatever is configured) | **equal** | push-notification-tool.md |
| SendUserFile (send a generated file to the user's device) | none | **missing** | no such tool among forge-core tool consts |
| LSP tool (definition, references, diagnostics) | post-edit LSP diagnostics as deltas; symbol navigation by Lattice (tree-sitter, 18 grammars: query, impact, path); no model-callable LSP definition/references | **worse** | lsp-diagnostics.md; lattice_tool.rs; forge-index Cargo.toml |
| ToolSearch / deferred MCP | deferred tool catalogue with `tool_call` | **equal** | deferred-tools.md |
| Workflow tool (dynamic workflows) | `run_workflow` / `/workflow` sandboxed JS with mesh-routed `agent()` calls; workflow TUI view | **equal** | forge-workflow.md; tui-panels.md |
| Chrome integration, computer use | `browser` tool (CDP attach to logged-in browser), `device` / `device_logs`, `proxy` | **equal** | browser.md; device.md |
| (no CC equivalent) `lattice`, `run_code` (codemode), `open_pr`, `remember`, `dispatch_sessions` | present | **better** | tool consts |

## 16. Rendering, status line and accessibility

CC: statusline.md, fullscreen.md, accessibility.md, terminal-config.md. Forge: tui-*.md.

| Claude Code feature | Forge equivalent | Status | Evidence |
| --- | --- | --- | --- |
| Status line data (model, cost, context %, rate limits, git) | status line: model and tier, cost, turn tokens, throughput, context gauge, mesh effort, quotas in `/status` | **equal** | tui-header-statusline.md |
| Heat/progress indicator and token counter | heat rail while working, live token counter and throughput | **equal** | tui-token-counter.md |
| Screen reader mode, spellcheck | none | **missing** | - |
| Colour-blind themes, high contrast | `NO_COLOR`, ASCII glyph tier, reduced motion; no daltonized palette | **worse** | tui-theme.md Detection |
| Mouse: click, drag-select, wheel, copy | yes (copy_mark.rs, driver mouse capture) | **equal** | driver.rs mouse arm |
| Narrow terminal handling | widgets drop in priority order; row budget documented for 80x24 | **equal** | tui-header-statusline.md:153 |
| Welcome/onboarding state | welcome panel with examples and `/init` nudge | **equal** | live welcome capture |

## 17. Headless, SDK and integrations

CC: headless.md and the Agent SDK. Forge: bench-tasks, `forge run`, stream_json.rs, json_result.rs, headless.rs.

| Claude Code feature | Forge equivalent | Status | Evidence |
| --- | --- | --- | --- |
| `claude -p` prints only the final answer on stdout | default text mode prints `● session <id>`, `⚒ mesh → ...` routing line, tool trace lines, `※ recap` and `$ session total ...` to STDOUT (stderr stays empty) | **worse** | evidence/forge-run-p-stdout.txt; headless.rs:93 and :222 |
| `--output-format json` single result object | CC-shaped `result` object with usage, cost, duration, num_turns | **equal** | evidence/forge-json-result-sample.json |
| stream-json: whole `assistant` messages, `stream_event` deltas only with `--include-partial-messages`, rich `system/init` (model, tools, cwd, mcp, plugins), `result` with usage and cost | one `assistant` event per streamed chunk (word-sized); `system/init` carries only session_id; separate `system/routing` and `system/usage` events; final `result` has no usage, cost, duration or num_turns | **worse** | evidence/forge-stream-json-sample.ndjson; stream_json.rs:57-66, :144 |
| Piped stdin appended to prompt | supported (stdin appended after the prompt) | **equal** | live: replay shows `summarize this` + stdin text |
| Exit codes | 0 on success, 2 on usage error, 1 on a failed turn (text, json and stream-json) | **equal** | live: bad flag exit=2; unknown model exit=1 in all three formats |
| `--input-format stream-json` bidirectional session; Agent SDK (TS/Python) | none. Alternatives: `forge api` (OpenAI-compatible HTTP), `forge serve` HTTP/WS control API, `forge mcp agent` | **missing** | forge-help; embed-ai-backend.md |
| IDE extensions (VS Code, JetBrains), desktop app, Slack, GitHub Actions, GitLab CI | desktop app (Tauri) and mobile/web PWA from `mobile/`, `open_pr` tool; no IDE extension, ACP server, Slack app or CI action | **worse** | repo tree: no vscode/jetbrains/acp directories; mobile/src-tauri is the desktop shell |
| Remote control from phone/web | `/remote`, `forge serve`, Forge Anywhere with push notifications | **better** | remote-control.md; forge-anywhere.md |
| Benchmarking and evaluation harness | `forge bench swe/tasks/integrity/recall`, bench export | **better** | forge bench --help |
| Embed as an OpenAI-compatible backend | `forge api` with mesh routing, failover and cost tracking | **better** | embed-ai-backend.md |

## Ranked gap list (where Forge is worse or missing)

Ranking = user impact first, then breadth of drop-in breakage, then cost to fix. Effort: S under a day, M a few days, L a week or more. Paths are relative to the Forge repo (`/home/floris/Documents/Repositories/Personal/AI/forge`).

| # | Gap | Impact | Effort |
| --- | --- | --- | --- |
| G1 | `forge run -p` text output is not scriptable | high | S |
| G2 | stream-json is only CC-shaped on the surface | high | M |
| G3 | Instruction files: first match wins, no hierarchy, no imports | high | M |
| G4 | No `!` shell mode in the composer | high | S |
| G5 | Unbound Ctrl chords type the letter; readline set incomplete | high | S |
| G6 | Entry-point ergonomics differ from `claude` | high | S |
| G7 | `claude -p` drop-in flags missing; `chat` has almost none | high | M |
| G8 | Hook coverage: 11 of 33 CC events, 3 of 5 handler types | high | M |
| G9 | Skill and command frontmatter: CC fields are ignored | high | M |
| G10 | Missing everyday slash commands | med | M |
| G11 | `/clear` and `Ctrl+L` mean something else | med | S |
| G12 | Permission prompt: no feedback and no session-scoped allow | med | S |
| G13 | Keybinding collisions with CC muscle memory | med | S |
| G14 | Rewind has one action | med | M |
| G15 | Input editing: vim mode, history search, queue take-back | med | M |
| G16 | Subagent definitions and built-in types are thin | med | M |
| G17 | Plugins are skill packs only; no enable/disable | med | L |
| G18 | Teams: no managed policy, no local config layer, weak sandbox | med | L |
| G19 | Project resources not found from a subdirectory | med | S |
| G20 | Themes, output styles and accessibility | low | M |
| G21 | Transcript search and dump | low | M |
| G22 | PDF input | low | S |
| G23 | IDE, SDK and CI integrations | med | L |
| G24 | MCP polish | low | M |
| G25 | Session ergonomics | low | S |

### G1. `forge run -p` text output is not scriptable

- **Impact:** high. **Effort:** S.
- **Problem and evidence:** Text mode writes the session header, the `mesh ->` routing line, every tool trace line, the `recap` line and a `$ session total` footer to stdout; stderr stays empty. `claude -p` prints only the final answer, so `result=$(forge run -p ...)`, pipes and benchmark scrapers get noise. Evidence: evidence/forge-run-p-stdout.txt (8 lines on stdout, the answer is one of them).
- **Suggested fix:** In crates/forge-tui/src/headless.rs (header at :93, footer at :222, tool/routing traces in the same presenter) print only the final assistant text to stdout in `text` mode and send the trace to stderr, or gate it behind `--verbose`. Keep the current layout for `forge run --tui/--plain` interactive use.

### G2. stream-json is only CC-shaped on the surface

- **Impact:** high. **Effort:** M.
- **Problem and evidence:** (a) one `assistant` event per streamed chunk (word-sized) instead of a whole message, so a CC consumer that reads `.message.content[0].text` per event sees fragments; (b) `system/init` has only `session_id` (no model, cwd, tools, permissionMode, mcp_servers); (c) the final `result` event has no `usage`, `total_cost_usd`, `duration_ms`, `num_turns` (they only exist in `--output-format json` and in a separate `system/usage` event); (d) no `stream_event` partials, no `--include-partial-messages`, no `--input-format stream-json`, no `--json-schema`, no `parent_tool_use_id` for subagents. Evidence: evidence/forge-stream-json-sample.ndjson, stream_json.rs:57-66 and :144.
- **Suggested fix:** In crates/forge-tui/src/stream_json.rs: buffer `AssistantDelta` into one `assistant` message flushed on message end (emit deltas as `stream_event` only when `--include-partial-messages`); extend `system/init` and `result` with the fields `json_result.rs` already computes; add the flags in crates/forge-cli/src/cli/args/claude_compat.rs and apply them in `run/claude_flags.rs`; add a stdin stream-json reader in `run.rs`.

### G3. Instruction files: first match wins, no hierarchy, no imports

- **Impact:** high. **Effort:** M.
- **Problem and evidence:** `read_project_agents_md` returns the first non-empty of .forge/AGENTS.md, AGENTS.md, CLAUDE.md, .claude/CLAUDE.md, config AGENTS.md, ~/.forge/AGENTS.md, ~/.claude/CLAUDE.md (lib.rs:1957-1990). Consequences vs CC: a project file hides the user's global CLAUDE.md; parent-directory files are never read; `CLAUDE.local.md`, `@path` imports, `.claude/rules/*.md` with `paths:` and lazily loaded subdirectory CLAUDE.md do not exist. Anyone with global preferences plus a repo AGENTS.md silently loses the global ones (this machine's own ~/.claude/CLAUDE.md is an example).
- **Suggested fix:** New `instructions.rs` in crates/forge-core/src replacing `agents_md_candidates`: concatenate [global files] + [ancestor dirs root-down: .forge/AGENTS.md, AGENTS.md, CLAUDE.md, .claude/CLAUDE.md, CLAUDE.local.md]; expand `@path` (depth 5, workspace-bounded, strip HTML comments); load `.claude/rules/*.md` and match `paths:` globs against files touched; load subdirectory files lazily from `tool_dispatch.rs` after `read_file/edit_file`. Keep `lean_context::claude_md_covers` so the claude bridge does not double-inject. Update the `project_agents_md_stat` fingerprint to cover all files.

### G4. No `!` shell mode in the composer

- **Impact:** high. **Effort:** S.
- **Problem and evidence:** `!echo hello` is sent to the model as a prompt (evidence/tui-bang-shell-mode.txt). In CC `! cmd` runs directly, shows live output, adds it to context and has Claude respond. A frequent daily action.
- **Suggested fix:** Intercept a leading `!` in the chat-loop submit handler (crates/forge-cli/src/cli/commands/run.rs, where `InputOutcome::Submit` is handled) and run it through `ShellTool` with the same safety floor but no approval; render it as a tool card, append it to the transcript as a user message, then start a turn (config `respond_to_bash`). Add `!` prefix hint in the composer placeholder.

### G5. Unbound Ctrl chords type the letter; readline set incomplete

- **Impact:** high. **Effort:** S.
- **Problem and evidence:** driver.rs:499 maps every unmatched `KeyCode::Char(c)` to a plain insert, with an exemption only for Alt (:498). Live: Ctrl+X, Ctrl+Y, Ctrl+P, Ctrl+Q, Ctrl+G, Ctrl+D, Ctrl+B produced `xypqgdb` in the draft (evidence/tui-unhandled-ctrl-chords.txt). So `Ctrl+D` neither exits nor deletes, `Ctrl+G` (editor) and `Ctrl+B` (background) corrupt the prompt.
- **Suggested fix:** In crates/forge-tui/src/driver.rs add `KeyCode::Char(_) if k.modifiers.contains(KeyModifiers::CONTROL) => return Ok(None)` before :499, then implement `Ctrl+D` (exit on empty, delete-forward otherwise), `Ctrl+G` (spawn $VISUAL/$EDITOR on a temp file, reload), `Ctrl+Y`/`Alt+Y` yank ring (needs a kill buffer fed by Ctrl+K/U/W), `Ctrl+P/N` history, `Ctrl+_` input undo.

### G6. Entry-point ergonomics differ from `claude`

- **Impact:** high. **Effort:** S.
- **Problem and evidence:** Bare `forge` prints a summary and exits (main.rs:249-255; first_run.rs:38). `forge chat "prompt"` is rejected (usage `forge chat [OPTIONS]`). `forge mcp add --transport http name URL` and `forge mcp list` are rejected although `forge mcp add --help` says it is compatible with `claude mcp add`.
- **Suggested fix:** main.rs: when stdout is a TTY and a provider is configured, dispatch bare `forge` to `chat`; add a trailing `[PROMPT]...` to the chat args (crates/forge-cli/src/cli/args) feeding the first turn; in crates/forge-cli/src/cli/commands/mcp.rs accept a positional URL for http/sse transports, `-H/--header 'K: V'`, and a `list` alias.

### G7. `claude -p` drop-in flags missing; `chat` has almost none

- **Impact:** high. **Effort:** M.
- **Problem and evidence:** Rejected on `forge run`: --add-dir, --system-prompt, --max-budget-usd, --mcp-config, --strict-mcp-config, --settings, --setting-sources, --session-id, --fork-session, --fallback-model, --json-schema, --input-format stream-json, --effort, --name, --tools, --agents, --agent, --worktree, --bare, --include-partial-messages, --no-session-persistence (live). `forge chat` additionally lacks --allowedTools, --disallowedTools, --system, --output-format. claude-code-parity.md promises scripts 'run unchanged'; that only holds for the dozen or so mapped flags.
- **Suggested fix:** Extend crates/forge-cli/src/cli/args/claude_compat.rs: --add-dir -> `[tools] extra_roots` (`resolve_extra_tool_roots`, setup.rs:76); --max-budget-usd -> run_budget.rs/spend_guard.rs; --effort -> exact_effort.rs; --mcp-config/--strict-mcp-config -> a per-run McpConfig overlay; --settings -> overlay TOML/JSON; --worktree -> worktree_ops.rs create+enter; --tools/--agents via existing agent and tool allow lists; --session-id/--fork-session/--name via the store. Hoist the shared flag set into a common struct used by both `run` and `chat`.

### G8. Hook coverage: 11 of 33 CC events, 3 of 5 handler types

- **Impact:** high. **Effort:** M.
- **Problem and evidence:** HookEvent (forge-config lib.rs:661-696) has PreToolUse, PostToolUse, UserPromptSubmit, SessionStart, SessionEnd, Notification, PreCompact, PostCompact, Stop, SubagentStop, PermissionRequest. Missing 22: PostToolUseFailure, PostToolBatch, PermissionDenied, SubagentStart, TaskCreated/Completed, StopFailure, InstructionsLoaded, ConfigChange, CwdChanged, FileChanged, DirectoryAdded, WorktreeCreate/Remove, PreModelSwitch/PostModelSwitch, Elicitation(+Result), Setup, UserPromptExpansion, MessageDisplay, TeammateIdle. No `http`/`mcp_tool` handlers, no `if`, `async`, `once`; `forge run` does not fire SessionStart/UserPromptSubmit; SessionStart `source` is always `startup`. Setups like this machine's (rtk rewrite hook, graph injectors) are hook-driven, so this is where drop-in claims break first.
- **Suggested fix:** Add variants to `HookEvent` and fire them at the existing seams: crates/forge-core/src/tool_dispatch.rs (failure, batch), subagent.rs (start), model switches in session_controls.rs, worktree_ops.rs, a file watcher reuse for FileChanged. Implement `type:"http"` in hooks_llm.rs next to prompt/agent, `if` via the permission-rule matcher already used by `hooks_cc.rs`, and run the matched hooks of one event in parallel. Fire SessionStart/UserPromptSubmit from `forge run` and set `source` from the resume/clear/compact path.

### G9. Skill and command frontmatter: CC fields are ignored

- **Impact:** high. **Effort:** M.
- **Problem and evidence:** forge-skills parses name, description, args, tier, resources (command-skill-system.md). `allowed-tools`, `disallowed-tools`, `disable-model-invocation`, `user-invocable`, `argument-hint`, `context: fork`+`agent`, `hooks`, `paths`, `shell`, `effort` are dropped (tests.rs:539-544 only checks that a CC command with `allowed-tools` parses without error; the key is never consumed). Templates lack `$ARGUMENTS[N]`, 0-based `$N` (`$0` stays literal), `${CLAUDE_SKILL_DIR}`, `${CLAUDE_SESSION_ID}` and ``!`cmd` `` injection (template.rs). Imported CC skills therefore prompt for permissions they pre-approve, cannot be hidden from the model, and lose dynamic context.
- **Suggested fix:** Extend `Command`/`SkillMeta` (crates/forge-skills/src/lib.rs:69-120) and `frontmatter.rs` consumers; in crates/forge-skills/src/template.rs add the missing substitutions; in forge-core `run_turn_with` apply `allowed-tools` as turn-scoped Allow rules and `disable-model-invocation` in `skill_listing`; execute ``!`cmd` `` through the shell tool only when the project is trusted.

### G10. Missing everyday slash commands

- **Impact:** med. **Effort:** M.
- **Problem and evidence:** No `/context`, `/diff`, `/permissions`, `/hooks`, `/add-dir`, `/cd`, `/rename`, `/branch`, `/fork`, `/doctor`, `/review`, `/security-review`, `/simplify`, `/theme`, `/skills`, `/tasks`, `/recap`, `/mcp reconnect|enable|disable|auth`. All answer `unknown command: /x` (live for /context, parse_command for the rest).
- **Suggested fix:** Add entries to crates/forge-tui/src/commands/registry.rs and arms in commands.rs `parse_command`, handled in the run loop. Cheap first five: `/context` (data already in the status-line gauge and compaction_budget.rs), `/diff` (git diff rendered with render/diff.rs), `/permissions` (list/edit rules with list_view.rs), `/mcp reconnect` (forge-mcp pool), `/review` and `/security-review` as builtin skills in forge-skills/src/builtin.rs.

### G11. `/clear` and `Ctrl+L` mean something else

- **Impact:** med. **Effort:** S.
- **Problem and evidence:** `/clear` only clears the screen and keeps the session (registry.rs:221); CC's `/clear` starts a new conversation. `Ctrl+L` (CC: redraw) triggers compaction, a model call (keybinds.md `compact`). `Ctrl+N` starts a new session (CC: history down), `Ctrl+Z` undoes a file write (CC: suspend).
- **Suggested fix:** Make `/clear` an alias of `/new` and keep `/cls` for the screen (commands.rs:401); rebind `compact` away from Ctrl+L and make Ctrl+L redraw (forge-config KeybindsConfig defaults). Ship a `claude-code` keybind preset (G13).

### G12. Permission prompt: no feedback and no session-scoped allow

- **Impact:** med. **Effort:** S.
- **Problem and evidence:** Prompt returns only Allow, AlwaysAllow, Deny (interaction_ui.rs:75-110). CC lets you attach a comment to Yes/No and choose 'allow for this session'. Forge's `a` always persists a scoped rule to `.forge/config.toml` (tool_dispatch.rs:553-575); only MCP tools get session-only rules.
- **Suggested fix:** Add `ConfirmOutcome::DenyWith(String)` and `AllowSession` in forge-types, a `Tab` comment field in crates/forge-tui/src/app/render/permission.rs, and handle both in tool_dispatch.rs (feed the note to the model as the tool result).

### G13. Keybinding collisions with CC muscle memory

- **Impact:** med. **Effort:** S.
- **Problem and evidence:** Same chord, different action: Ctrl+R (history search vs reasoning toggle), Ctrl+S (stash vs checkpoint), Ctrl+L, Ctrl+N, Ctrl+Z, Ctrl+K (delete-to-EOL vs command centre), Ctrl+O, Ctrl+T, Ctrl+V, Alt+T (thinking vs temper), Alt+P/Alt+M. Several are destructive when pressed by habit.
- **Suggested fix:** Add `[keybinds] preset = "claude-code"` in crates/forge-config (KeybindsConfig) that remaps Forge actions off CC's chords (command centre to Ctrl+Shift+P or F2, reasoning to Alt+R, checkpoint to Alt+S, new session to Alt+N) and offer it in `forge setup`.

### G14. Rewind has one action

- **Impact:** med. **Effort:** M.
- **Problem and evidence:** Picker footer: `Enter rewind to this point` (chat, context and files together). CC offers restore code and conversation, conversation only, code only, and summarize from here.
- **Suggested fix:** Second step in the rewind picker (crates/forge-tui/src/picker.rs, run loop) that selects among `rewind_to` variants in forge-core/src/session_history.rs and file_restore.rs (files-only, history-only) and a compaction-from-seq variant reusing compaction_run.rs.

### G15. Input editing: vim mode, history search, queue take-back

- **Impact:** med. **Effort:** M.
- **Problem and evidence:** No vim mode, no `Ctrl+R` history search, no `Ctrl+Enter` send-now, no `Up` to take back queued prompts, no emoji shortcodes, no spellcheck.
- **Suggested fix:** Prompt editing lives in crates/forge-tui/src/app.rs and prompt_history.rs: add a modal editor state machine behind `[tui] editor_mode = "vim"`, a fuzzy history overlay using list_view.rs, and queue introspection (the steer inbox in forge-core/src/steer.rs).

### G16. Subagent definitions and built-in types are thin

- **Impact:** med. **Effort:** M.
- **Problem and evidence:** AgentDef has name, description, tools, tier, model (agents.rs:32-42; parse_agent:158-180). CC adds disallowedTools, permissionMode, maxTurns, skills, mcpServers, hooks, memory, background, effort, isolation, color and ships Explore/Plan/general-purpose. No `--agents`, `/agents`, `/subtask` or user-driven fork with context.
- **Suggested fix:** Extend `AgentDef` and `parse_agent` in crates/forge-config/src/agents.rs and resolve the new fields in forge-core/src/subagent/requests.rs; ship builtin `explore` (read-only, trivial tier) and `plan` agents.

### G17. Plugins are skill packs only; no enable/disable

- **Impact:** med. **Effort:** L.
- **Problem and evidence:** `forge plugin install` copies skills and commands; hooks, agents and `.mcp.json` inside packs are ignored (marketplace.rs:309). No `--plugin-dir`, no per-plugin enable/disable, no bundled LSP/themes/styles.
- **Suggested fix:** Plugin manifest parser in crates/forge-cli/src/cli/commands/marketplace.rs mapping to forge-skills Sources, hooks config, agents dirs and mcp.toml, with an `enabled` flag in the lockfile and `--plugin-dir`.

### G18. Teams: no managed policy, no local config layer, weak sandbox

- **Impact:** med. **Effort:** L.
- **Problem and evidence:** No managed/enterprise settings, no gitignored local layer, Landlock sandbox is opt-in, Linux-only, write-only (no network, no PTY, nothing on macOS/Windows) vs CC's Seatbelt/bubblewrap with filesystem and network allowlists.
- **Suggested fix:** Add `/etc/forge/policy.toml` (or platform equivalent) and `.forge/config.local.toml` layers in crates/forge-config load order with a deny-only merge for policy; add a Seatbelt backend and Landlock network rules in crates/forge-tools/src/sandbox.rs.

### G19. Project resources not found from a subdirectory

- **Impact:** med. **Effort:** S.
- **Problem and evidence:** Running in `repo/pkg/` does not see `repo/.forge/commands` (live: a command listed from `proj` was absent from `proj/sub/deep`). Project config is cwd-only by design (lib.rs test `parent_project_config_is_not_inherited`), and the same cwd-rooted loaders very likely cover `.forge/skills`, `.forge/agents` and the instruction files; those were not exercised separately. CC walks up to the repo root for skills and reads CLAUDE.md in every parent. Hits monorepos.
- **Suggested fix:** Keep `config.toml` cwd-only (security) but resolve `Sources` for skills, commands and agents (forge-config `command_sources_in`) and the instruction files (G3) up to the git root, honouring trust.

### G20. Themes, output styles and accessibility

- **Impact:** low. **Effort:** M.
- **Problem and evidence:** Only auto/dark/light plus colour/glyph tiers; no named, daltonized or custom themes, no `/theme`, no output styles, no screen-reader mode.
- **Suggested fix:** Presets and `~/.config/forge/themes/*.toml` in crates/forge-tui/src/theme/; output styles as named system-prompt appendices (`system_prompt_overrides` already exists) with `/output-style`; a `--ax` flat renderer that disables rails, spinners and borders.

### G21. Transcript search and dump

- **Impact:** low. **Effort:** M.
- **Problem and evidence:** Fullscreen has no conversation search (only the `/output` tool viewer has `/`), no dump-to-scrollback or open-in-editor, no `/focus`.
- **Suggested fix:** Reuse the search state in crates/forge-tui/src/app/output_view.rs for the main transcript; add `[` (print to native scrollback) and `v` ($EDITOR) to the transcript mode.

### G22. PDF input

- **Impact:** low. **Effort:** S.
- **Problem and evidence:** PDF paths are recognised but not attachable (image-input.md).
- **Suggested fix:** Extract text with a PDF crate or `pdftotext` in crates/forge-cli/src/image_input.rs and expose a `pages` argument on `read_file`.

### G23. IDE, SDK and CI integrations

- **Impact:** med. **Effort:** L.
- **Problem and evidence:** No VS Code/JetBrains extension, ACP server, TS/Python SDK, GitHub Action or Slack app. Alternatives exist (desktop and mobile apps, `forge api`, `forge serve` control API, `forge mcp agent`).
- **Suggested fix:** A thin `forge acp` stdio server over the daemon API, a published GitHub Action wrapping `forge run --output-format stream-json`, and an SDK generated from the serve protocol (crates/forge-anywhere-protocol/, protocol/).

### G24. MCP polish

- **Impact:** low. **Effort:** M.
- **Problem and evidence:** MCP prompts are not slash commands, no `@server:resource`, no WebSocket transport, `.mcp.json` is import-only (no approval-gated auto-load), `/mcp` is read-only.
- **Suggested fix:** Register prompts as `/mcp__server__name` in the palette feed (crates/forge-tui/src/commands/palette.rs), resource completion in `AtPathPicker`, ws in forge-mcp/src/transport.rs.

### G25. Session ergonomics

- **Impact:** low. **Effort:** S.
- **Problem and evidence:** No `--name`/`/rename`, `--from-pr`, `-w`, `/background` of the live session, PR/MR badge, OSC-8 links, `claude purge`, auto-update, OpenTelemetry export.
- **Suggested fix:** Store title setter exists for daemon/app (`set_session_title`); expose it as `/rename` and `--name`; add `gh pr view` badge to the header (tui-header-statusline); `forge update --auto` timer via `forge service`.

### Quick wins (effort S, do first)

- G1 `forge run -p` text output is not scriptable
- G4 No `!` shell mode in the composer
- G5 Unbound Ctrl chords type the letter; readline set incomplete
- G6 Entry-point ergonomics differ from `claude`
- G11 `/clear` and `Ctrl+L` mean something else
- G12 Permission prompt: no feedback and no session-scoped allow
- G13 Keybinding collisions with CC muscle memory
- G19 Project resources not found from a subdirectory
- G22 PDF input
- G25 Session ergonomics

## Which gaps move the ccbench axes

- Headless ergonomics (dimension 6): G1, G2, G7. Any script or harness that treats `forge run -p` like `claude -p` is affected first.
- Usability and UX (dimension 4): G4, G5, G10, G11, G12, G13, G14, G15, G21. The scripted tmux scenarios (permission prompt, interrupt, queued follow-up, resume, slash discovery, multi-line/paste) will hit G5 and G12 directly; G4 and G10 show up as soon as a tester reaches for `!` or `/context`.
- Robustness (dimension 5): G5 (stray characters in the draft), G12 (cannot explain a denial to the model), G14 (coarse rewind).
- Quality (dimension 1): G3 and G9 matter when a task relies on project guidance or imported CC skills. The `claude-cli` bridge arms run Claude's own loop, which loads CLAUDE.md itself (Forge dedupes via `claude_md_covers`), so G3 mainly affects the native-loop arm (`inferhub::...`) and daily use.
- Speed and efficiency (dimensions 2 and 3): nothing in this static audit; G1 only affects how output is parsed, not latency.

## Where Forge is clearly better

- Model mesh: benchmark-ranked, cost-tiered routing, quota spreading and automatic cross-provider failover down the whole catalog (CC is single-vendor with `--fallback-model`). Evidence: forge models/mesh/benchmarks --help; mesh-routing.md; `forge mesh --smoke` checks a viable pick per tier without a model call.
- Subscription bridging: Forge runs Claude Code's own loop (and Codex, Antigravity) with Forge tools via MCP under Forge's permission gate (`claude-cli::haiku|sonnet|opus`). Evidence: persistent-bridge-transport.md; `forge doctor` prints a Bridge liveness section (live).
- Daemon-hosted multi-session fleet: `forge serve`, `forge board`, `forge attach`, `forge dispatch`, `forge send`, phone/desktop apps and Anywhere relay; sessions survive client disconnects. Evidence: forge --help; remote-control.md; project-board.md.
- Overnight autopilot and scheduling without a live session: `forge queue` (isolated worktree + branch + budget + digest) and `forge schedule` on native OS timers. Evidence: forge queue/schedule --help.
- Counterfactual and provenance tooling: `/duel`, `forge fork --turn N --model`, `forge replay --rerun/--html`, `forge blame`, `forge scoreboard`, `forge bench`. Evidence: forge --help.
- Rewind is one picker that restores chat, model context and files and also repairs bridge epochs, compaction and injected-context latches; `/uncompact` undoes compaction. Evidence: rewind.md; live picker.
- Safety floor that no mode or rule can lift (catastrophic deletes, sudo/su/pkexec refusal that protects pam_faillock) and a granular untrusted-project sanitizer. Evidence: temper-modes.md; project-trust.md.
- Transcripts as data: `forge replay` (per-turn model, tokens, cost, time; `--html`, `--json`, `--rerun`) with secrets redacted by default, plus `forge blame` line provenance. Evidence: forge replay --help; forge-blame.md.
- Local-first voice (whisper.cpp), clipboard/drag-drop image input across Wayland, X11, macOS and Windows. Evidence: voice.md; image-input.md.
- Code intelligence graph (Lattice, tree-sitter, 18 grammars) auto-injected into turns plus a `lattice` tool; post-edit LSP diagnostics as deltas. Evidence: lattice-token-savings.md; lsp-diagnostics.md; forge-index/Cargo.toml.
- Migration and interop: `forge switch` (continue one conversation in either tool), `forge import` from Claude/Codex/Cursor/Aider, MCP import that scans six tools without copying secrets, CC hook and statusline compatibility. Evidence: forge switch --help; mcp-client.md 5.3; hooks.md.
- Embeddable endpoint: `forge api` (OpenAI-compatible with mesh routing) and `forge mcp agent`. Evidence: embed-ai-backend.md.
- Goal and loop runners with quality gates and token/time budgets (`/goal --gate`, `/loop --gate`), `/dispatch` plan-and-approve fan-out into worktrees. Evidence: autonomous-gates.md; plan-and-dispatch.md.

## Forge docs and help text that overstate reality

- `forge mcp add --help` says 'compatible with `claude mcp add`': CC's `add <name> <commandOrUrl>` and `list` are rejected (G6).
- claude-code-parity.md: `forge run` accepts the `claude -p` flags 'so scripts written for Claude Code run unchanged': Only about a dozen flags map; stdout shape differs in text mode (G1) and stream-json (G2).
- `forge import claude --help` says it copies commands and skills: import.rs also transfers permission rules, hooks (`--hooks`) and `statusLine`; help understates it.
- hooks.md lists `UserPromptSubmit` and `SessionStart` as shipped: True for TUI/chat/daemon; the one-shot `forge run` does not fire them (hooks.md line 125 itself says so).
- mcp-client.md checklist items still unchecked in section 6: Implementation is ahead of the checklist (elicitation, sampling, streamable HTTP, OAuth are in code); treat the doc's 'Won't/Could' lines as stale.

## Caveats

- Verification depth: 166 of 280 rows cite a live run, a help dump or a source file; the remainder rest on the feature docs under `docs/features/` (read, not executed) and on the CC docs.
- CC rows come from documentation plus `claude --help`; a few CC behaviours (for example the exact set of rewind options or which keys exist in a given terminal) could differ in 2.1.308 from the live docs, which cover up to roughly v2.1.307.
- Key behaviour was checked through tmux, which does not report the kitty keyboard protocol, so `Shift+Enter` and `Ctrl+Enter` were not exercised; those rows rely on code and docs.
- The mock provider cannot show instruction-file injection (the replay of a mock turn contains no AGENTS.md text), so G3 rests on code reading at lib.rs:1957-1990 plus the doc comments, not on a captured prompt. A single `inferhub::cb/deepseek-v4.1-flash` turn with a distinctive AGENTS.md and global CLAUDE.md would confirm it end to end.
- Items outside the terminal client (desktop, mobile and web apps, Anywhere relay) were taken from docs only and were not exercised.

## Evidence index

- `evidence/claude-help.txt`, `evidence/forge-help.txt`, `evidence/forge-subcommand-help.txt`: full help dumps.
- `evidence/forge-run-p-stdout.txt`: what `forge run --mock -p hi` prints on stdout (G1).
- `evidence/forge-stream-json-sample.ndjson`, `evidence/forge-json-result-sample.json`: stream-json and json outputs (G2).
- `evidence/tui-bang-shell-mode.txt` (G4), `evidence/tui-unhandled-ctrl-chords.txt` (G5), `evidence/tui-unknown-slash-context.txt` (G10), `evidence/tui-help-overlay.txt` (Help and Keys overlay, command list).
- `ccdocs/*.md`: the Claude Code documentation pages used. `sandbox/`: throw-away HOME and project used for live checks.
