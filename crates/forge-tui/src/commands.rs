//! Slash-command registry, parsing, and palette state (RFC session-management-and-commands,
//! PR1). Pure + UI-only: forge-tui defines *what* commands exist and how the palette filters/
//! navigates; the render loop in the binary interprets the resulting [`CommandAction`] (it owns
//! the `Session`, which this crate must not depend on).

mod btw_args;
mod effort_args;
mod subagent_args;
mod workflow_args;

mod registry;
pub use registry::{Command, COMMANDS};

/// Subcommand for `/statusline`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatuslineAction {
    /// Show the current left/center/right layout.
    Layout,
    /// Add widget if absent, remove if present (in the right segment).
    Toggle(String),
    /// Reset to the built-in default.
    Reset,
    /// Open `~/.config/forge/config.toml` in $EDITOR.
    Edit,
}

/// `/goal` verbs that steer a goal instead of setting one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoalControl {
    /// Stop re-driving after the turn in flight; the goal can be resumed.
    Pause,
    /// Continue a paused goal with its counters and budget intact.
    Resume,
    /// Drop the goal. A running one stops after the turn in flight.
    Clear,
    /// Print the goal's state and budget.
    Status,
    /// Reword the goal; a running one takes the new wording at its next turn.
    Edit(String),
}

/// `/workflow` sub-actions (docs/rfcs/forge-workflow.md).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkflowAction {
    /// `/workflow <goal>` — author a new script for this goal, mesh-routed at Complex tier, then
    /// run it.
    Run(String),
    /// `/workflow run <name> [args]` — skip authoring; execute a saved
    /// `.forge/workflows/<name>.js` directly.
    RunSaved { name: String, args: String },
    /// `/workflow list` — list saved scripts in `.forge/workflows/`.
    List,
}

pub use crate::heartbeat_args::HeartbeatAction;
pub use crate::refine_args::RefineAction;
pub use crate::stash_args::StashOp;

/// What the render loop must do when a command is accepted. forge-tui produces it; the binary
/// (which owns the `Session`) executes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandAction {
    Help,
    /// Open the `/status` inspector: every fact the header and status line can drop.
    Status,
    /// Open the keybind help overlay (`/keys`) — the read-only keybind list (same as F1/ShowHelp).
    Keys,
    ListSessions,
    Board,
    /// Start a plan-and-dispatch on the daemon for this workspace (`/dispatch <request>`).
    Dispatch(String),
    Resume(String),
    /// Pin a specific model for all subsequent turns in this session. `None` opens the picker.
    PinModel(Option<String>),
    /// Open the interactive model browser (`/models`).
    ListModels,
    /// Open the interactive config wizard (`/config`) — set provider/search keys + plans.
    Config,
    /// List connected MCP servers (`/mcp`), or one server's full tool list (`/mcp <server>`).
    Mcp(Option<String>),
    New,
    ClearScreen,
    /// Open the operating-mode (temper) picker.
    Mode,
    /// Enter Assay mode — pick analysis-only vs full cleanup, then run the critic crew.
    /// `only`/`skip` are lens name lists; `scope` is `""` (repo), a path, `"--diff"`,
    /// `"--branch <b>"`, or `"--since <ref>"`.
    Assay {
        only: Vec<String>,
        skip: Vec<String>,
        scope: String,
    },
    /// Rewind the last turn (conversation + file edits).
    Undo,
    /// Save a checkpoint at the current point; `None` = an auto/unnamed checkpoint.
    Checkpoint(Option<String>),
    /// Open the checkpoint picker.
    ListCheckpoints,
    /// Summarize older transcript messages to free up context (`/compact [focus instructions]`).
    Compact(Option<String>),
    /// Ask the model to commit the session's uncommitted work (`/commit [hint]`).
    Commit(String),
    /// Restore the full pre-compaction transcript after a `/compact` (`/uncompact`).
    Uncompact,
    Reground,
    /// Continual Harness: review the trajectory and persist learned harness state (prompt notes,
    /// skills, subagent specs) — see docs/features/continual-harness.md.
    Refine(RefineAction),
    /// Show the code-intelligence subgraph for a symbol (`/lattice <symbol>`).
    Lattice(String),
    /// Set a session goal and decompose it into a tracked task plan (`/goal <objective>
    /// [--gate "<cmd>"]... [--max-tokens N] [--max-minutes N]`). `gates`/`max_tokens`/
    /// `max_minutes` are the shared autonomous-quality-gate/budget options
    /// (docs/features/autonomous-gates.md) — empty/`None` for the bare form.
    Goal {
        objective: String,
        gates: Vec<String>,
        max_tokens: Option<u64>,
        max_minutes: Option<u64>,
    },
    /// Steer the goal that is running or paused (`/goal pause|resume|clear|status|edit <text>`).
    GoalControl(GoalControl),
    /// Turn this session's work into a branch + commit + pull request with a provenance-rich
    /// body (`/pr [title]`).
    Pr(String),
    /// Re-run a task each turn until the model signals completion (`/loop <task> [--gate "<cmd>"]...
    /// [--max-tokens N] [--max-minutes N]`). See [`CommandAction::Goal`] for the shared fields.
    Loop {
        prompt: String,
        gates: Vec<String>,
        max_tokens: Option<u64>,
        max_minutes: Option<u64>,
    },
    /// Show a session transcript inline, or diff two sessions (`/replay <id> [<id2>]`).
    Replay(String, Option<String>),
    /// A side question that never joins this session's transcript or history — answered as a
    /// distinct side-note card, not a turn (`/btw <question>`, alias `/side`). Each call is
    /// independent (docs/features/side-questions.md).
    Btw(String),
    /// Export this session's transcript as HTML, or JSON with `--json` (`/export [--json] [path]`).
    /// `None` uses the default `forge-session-<id8>.html` in the current directory.
    Export(Option<String>),
    /// Open the usage overlay showing API spend + token breakdown (`/usage`).
    Usage,
    /// Print the lifetime activity poster (`/stats [days]`).
    Stats(Option<u32>),
    /// Exit the TUI and resume this conversation in Claude Code (`/claude`).
    Claude,
    /// Open the mesh routing inspector; optional prompt to trace (`/mesh [task]`).
    Mesh(Option<String>),
    /// Toggle remote control on/off. When turning on, `mode` selects how the server is exposed.
    /// The render loop prints the connect URL + QR code and lights the statusline indicator.
    Remote {
        mode: RemoteMode,
    },
    /// Show local Anywhere health and open the trusted controller surface for setup/approvals.
    Anywhere,
    /// Toggle display of model reasoning/thinking blocks (`/thinking`).
    Thinking,
    /// Open the latest tool call's complete output in the full-output viewer (`/output`).
    ToolOutput,
    /// Attach an image file to the next prompt (vision input) — `/image <path>`.
    Image(String),
    /// Scan the repository and write `.forge/AGENTS.md` project memory (`/init`).
    Init,
    /// Pin (or clear) the reasoning-effort level for this session (`/effort <level>` / `/effort`).
    /// The level string is the raw user-supplied value (e.g. "high"); the binary parses + applies
    /// it. `None` clears the pin and returns to the provider default.
    SetEffort(Option<String>),
    /// An EXACT rung (`/effort exact <level>`), sent as-is rather than capped — so it reaches rungs
    /// the benchmark ladder cannot. `None` clears it. Raw text; the binary parses + validates it.
    SetExactEffort(Option<String>),
    /// Release (`Some(true)`) or re-bind (`Some(false)`) this session's subagents from the active
    /// model pin — `/subagents free` / `/subagents pinned`. `None` = bare `/subagents`, which
    /// toggles and reports the resulting state.
    Subagents(Option<bool>),
    /// `/theme [name]`: list the accent palettes, or switch to one and save it.
    Theme(Option<String>),
    /// List the subagent types `spawn_agents` can use (`/agents`): built-ins plus agent files.
    Agents,
    /// Enter planning mode (`/plan <task>`): read-only investigation that proposes a plan.
    Plan(String),
    /// Approve the proposed plan and execute it (`/execute`): switches to Auto-edit and builds it.
    Execute,
    /// Copy an assistant response to the clipboard (`/copy [N]`). `nth` is 1-based and counts back
    /// from the most recent assistant message: 1 = the last response, 2 = the one before it.
    Copy {
        nth: usize,
    },
    /// Save a durable memory for this project (`/remember <text>`).
    Remember(String),
    /// List this project's memories (`/memories`).
    Memories,
    /// Toggle self-MCP mode (`/self-mcp [enable|disable]`). `None` = toggle current state.
    SelfMcp(Option<bool>),
    Quit,
    /// Statusline customization (`/statusline`).
    Statusline(StatuslineAction),
    /// Mesh-routed multi-agent workflow scripts (`/workflow`, docs/rfcs/forge-workflow.md).
    Workflow(WorkflowAction),
    /// The user's own recurring re-entry prompt for this session (`/heartbeat`,
    /// docs/features/session-heartbeats.md). Distinct from — and never modifies — any
    /// agent-created heartbeats (the `manage_heartbeats` virtual tool).
    Heartbeat(HeartbeatAction),
    /// Model arena (`/duel <task>`, docs/features/duel.md): race 2-3 mesh models on the same task
    /// concurrently, each in its own worktree, then let the user pick a winner.
    Duel(String),
    /// Open the recording overlay: record from the microphone, transcribe locally (whisper), and
    /// insert the transcript into the prompt (`/voice`, also bound to Ctrl+V — voice.md). Never
    /// auto-sends.
    Voice,
    /// What fills the context window, by category (`/context`).
    Context,
    /// The working tree's uncommitted changes, optionally under one path (`/diff [path]`).
    Diff(String),
    /// The permission rules in force, or `allow|ask|deny <Tool(pattern)>` to add a session rule
    /// (`/permissions`, alias `/allowed-tools`). Raw arg; the binary parses it.
    Permissions(String),
    /// Reconnect every MCP server, or the named one (`/mcp reconnect [server]`).
    McpReconnect(Option<String>),
    /// Let the file tools work in another directory for this session (`/add-dir <path>`).
    AddDir(String),
    /// In-session diagnostics: config, providers, store, daemon (`/doctor`).
    Doctor,
    /// Name this session (`/rename <title>`); empty shows the current name.
    Rename(String),
    /// Star or unstar this session (`/star`).
    Star,
    /// Mark this session done, or reopen it (`/done`).
    Done,
    /// List archived sessions, or `restore <id>` one (`/archived [restore <id>]`).
    Archived(String),
    /// Branch this session at its latest message into a child and continue there (`/fork [message]`).
    Fork(Option<String>),
    /// Park or restore a draft (`/stash [push|pop|list|clear]`).
    Stash(StashOp),
    /// List the skills the model can load (`/skills`).
    Skills,
    /// Report a problem to the maintainers (`/bug <what went wrong>`), after a preview.
    Bug(String),
    /// Not a known command — the binary shows `unknown command: X`.
    Unknown(String),
}

/// How `/remote` exposes the control server to a browser.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteMode {
    /// Bind `0.0.0.0` — reachable from the LAN (the default).
    Lan,
    /// Bind `127.0.0.1` only — control from this machine.
    Local,
    /// Bind loopback and pipe it through a public tunnel (cloudflared/ngrok) so any browser,
    /// anywhere, can reach it — no manual router port-forwarding. The token gate is then the only
    /// thing standing between the public internet and the session.
    Anywhere,
}

/// A `/command` token detected somewhere in the input line, used to drive highlighting and
/// autocomplete. Slash commands are recognized at ANY whitespace-delimited position (not just
/// the first word), so `please run /orchestrate scan` highlights+completes `/orchestrate`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlashToken {
    /// Byte offset of the leading `/` in the input.
    pub start: usize,
    /// Byte offset just past the command name (where args/whitespace begin).
    pub end: usize,
    /// The command name without the leading slash (the autocomplete/highlight query).
    pub name: String,
}

/// Find the `/command` token to drive the palette for, given the cursor position. We scan every
/// whitespace-delimited word; a word qualifies when it begins with exactly one `/` (a `//literal`
/// escape is skipped). The token *under or just before* the cursor wins, so the palette tracks
/// the word being edited; otherwise the last slash-token on the line is used. Returns `None` when
/// no slash-command token is present, and for a `!command` shell line, whose `/paths` are not
/// commands.
pub fn slash_token_at(input: &str, cursor: usize) -> Option<SlashToken> {
    if crate::bang_command(input).is_some() {
        return None;
    }
    let mut best: Option<SlashToken> = None;
    let mut last: Option<SlashToken> = None;
    let mut chars = input.char_indices().peekable();
    while let Some(&(i, ch)) = chars.peek() {
        // Advance to the start of a word (skip whitespace).
        if ch.is_whitespace() {
            chars.next();
            continue;
        }
        let word_start = i;
        // Consume to the next whitespace (word end).
        let mut word_end = input.len();
        while let Some(&(j, ch)) = chars.peek() {
            if ch.is_whitespace() {
                word_end = j;
                break;
            }
            chars.next();
        }
        let word = &input[word_start..word_end];
        // A slash command is a single leading `/` followed by a name; `//x` is a literal escape.
        if let Some(rest) = word.strip_prefix('/') {
            if rest.starts_with('/') {
                continue; // `//literal` escape — not a command token.
            }
            // The command name runs until the first whitespace (already the word) — but stop the
            // highlight at the name so `/cmd` in `/cmd arg` only spans the command word itself.
            let name = rest.to_string();
            let tok = SlashToken {
                start: word_start,
                end: word_end,
                name,
            };
            // Prefer the token the cursor is editing (cursor within [start, end]).
            if cursor >= tok.start && cursor <= tok.end {
                best = Some(tok.clone());
            }
            last = Some(tok);
        }
    }
    best.or(last)
}

/// An `@path` token found in the input line — drives the file-path autocomplete popup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AtToken {
    pub start: usize,
    pub end: usize,
    pub query: String,
}

/// Find the `@path` token to drive the file-path picker for, given the cursor position.
/// Scans every whitespace-delimited word; qualifies when it begins with `@` AND the cursor
/// is inside the token (cursor within [start, end]). Unlike the slash palette, there is no
/// last-token fallback — the picker closes as soon as the cursor moves off the `@` word.
pub fn at_token_at(input: &str, cursor: usize) -> Option<AtToken> {
    // A `!command` goes to the shell verbatim; `@path` expansion only applies to prompts.
    if crate::bang_command(input).is_some() {
        return None;
    }
    let mut chars = input.char_indices().peekable();
    while let Some(&(i, ch)) = chars.peek() {
        if ch.is_whitespace() {
            chars.next();
            continue;
        }
        let word_start = i;
        let mut word_end = input.len();
        while let Some(&(j, ch)) = chars.peek() {
            if ch.is_whitespace() {
                word_end = j;
                break;
            }
            chars.next();
        }
        let word = &input[word_start..word_end];
        if let Some(rest) = word.strip_prefix('@') {
            if cursor >= word_start && cursor <= word_end {
                return Some(AtToken {
                    start: word_start,
                    end: word_end,
                    query: rest.to_string(),
                });
            }
        }
    }
    None
}

#[path = "commands/category.rs"]
mod category;
pub use category::command_category;

pub use crate::picker::{Picker, PickerKind, PickerRow};
#[path = "commands/palette.rs"]
mod palette;
pub use palette::{
    arg_values, command_action, filter_commands, AtPathPicker, CommandCenter, Palette,
    PaletteEntry, COMMAND_GROUPS,
};

use crate::arg_parse::{assay_action, goal_action, has_flag, loop_action, mcp_action};

/// Parse a submitted command line (`"/resume ab12"`). The leading `/` is required; a `//`
/// prefix is NOT a command (it escapes to a literal prompt — handled by the caller).
pub fn parse_command(line: &str) -> CommandAction {
    let line = line.trim();
    let body = line.strip_prefix('/').unwrap_or(line).trim();
    let mut parts = body.splitn(2, char::is_whitespace);
    let name = parts.next().unwrap_or("").to_lowercase();
    let arg = parts.next().unwrap_or("").trim().to_string();
    match name.as_str() {
        "help" | "h" | "?" => CommandAction::Help,
        "keys" | "keybinds" | "shortcuts" => CommandAction::Keys,
        "sessions" | "ls" => CommandAction::ListSessions,
        "board" => CommandAction::Board,
        "dispatch" => CommandAction::Dispatch(arg),
        "resume" | "r" => {
            if arg.is_empty() {
                CommandAction::ListSessions // /resume with no id → open the picker
            } else {
                CommandAction::Resume(arg)
            }
        }
        "model" => CommandAction::PinModel((!arg.is_empty()).then_some(arg)),
        "models" | "mc" => CommandAction::ListModels,
        "config" | "cfg" | "settings" => CommandAction::Config,
        "mcp" => mcp_action(arg),
        "new" | "n" | "clear" | "reset" => CommandAction::New,
        "mode" | "m" | "temper" => CommandAction::Mode,
        "assay" | "analyze" | "analyse" => assay_action(&arg),
        "context" | "ctx" => CommandAction::Context,
        "diff" => CommandAction::Diff(arg),
        "permissions" | "allowed-tools" | "perms" => CommandAction::Permissions(arg),
        "add-dir" => CommandAction::AddDir(arg),
        "doctor" => CommandAction::Doctor,
        "rename" => CommandAction::Rename(arg),
        "star" => CommandAction::Star,
        "done" => CommandAction::Done,
        "archived" => CommandAction::Archived(arg),
        "bug" | "report" => CommandAction::Bug(arg),
        "rewind" | "undo" | "u" | "timeline" => CommandAction::Undo,
        "fork" | "branch" => CommandAction::Fork((!arg.is_empty()).then_some(arg)),
        "stash" => CommandAction::Stash(StashOp::parse(&arg)),
        "skills" => CommandAction::Skills,
        "checkpoint" | "cp" => CommandAction::Checkpoint((!arg.is_empty()).then_some(arg)),
        "checkpoints" => CommandAction::ListCheckpoints,
        "compact" => CommandAction::Compact((!arg.is_empty()).then_some(arg)),
        "commit" => CommandAction::Commit(arg.to_string()),
        "uncompact" => CommandAction::Uncompact,
        "reground" => CommandAction::Reground,
        "refine" => crate::refine_args::refine_action(&arg),
        "lattice" | "lat" => CommandAction::Lattice(arg),
        "goal" | "objective" => goal_action(&arg),
        "pr" | "pullrequest" => CommandAction::Pr(arg),
        "loop" => loop_action(&arg),
        "duel" => CommandAction::Duel(arg),
        "voice" | "record" => CommandAction::Voice,
        "replay" => {
            // `/replay <id>` or `/replay <a> <b>`
            let mut ids = arg.splitn(2, char::is_whitespace);
            let id_a = ids.next().unwrap_or("").trim().to_string();
            let id_b = ids
                .next()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            CommandAction::Replay(id_a, id_b)
        }
        "btw" | "side" => CommandAction::Btw(btw_args::parse_btw_arg(&arg)),
        "export" => CommandAction::Export((!arg.is_empty()).then(|| arg.trim().to_string())),
        "effort" => effort_args::parse_effort_arg(&arg),
        "subagents" | "subagent" => {
            CommandAction::Subagents(subagent_args::parse_subagents_arg(&arg))
        }
        "theme" | "palette" => {
            CommandAction::Theme((!arg.is_empty()).then(|| arg.trim().to_lowercase()))
        }
        "agents" => CommandAction::Agents,
        "remember" => CommandAction::Remember(arg.to_string()),
        "memories" => CommandAction::Memories,
        "self-mcp" | "self" => {
            let enable = match arg.to_lowercase().as_str() {
                "enable" | "on" | "1" => Some(true),
                "disable" | "off" | "0" => Some(false),
                _ => None, // bare /self-mcp → toggle
            };
            CommandAction::SelfMcp(enable)
        }
        "copy" | "yank" => {
            // `/copy` → last response; `/copy N` → Nth-latest. Non-numeric / <1 args fall back to 1.
            let nth = arg
                .split_whitespace()
                .next()
                .and_then(|s| s.parse::<usize>().ok())
                .filter(|n| *n >= 1)
                .unwrap_or(1);
            CommandAction::Copy { nth }
        }
        "cls" => CommandAction::ClearScreen,
        "usage" => CommandAction::Usage,
        "stats" => CommandAction::Stats(arg.split_whitespace().next().and_then(|d| d.parse().ok())),
        "status" => CommandAction::Status,
        "mesh" => CommandAction::Mesh((!arg.is_empty()).then_some(arg)),
        "remote" | "rc" => {
            // `/remote` toggles. `--anywhere`/`-a` pipes through a public tunnel (reachable from
            // any network, no port-forward); `--local` binds loopback only (this machine);
            // default `--lan` binds 0.0.0.0 so a phone on the same network can connect.
            let mode = if has_flag(&arg, "--anywhere") || has_flag(&arg, "-a") {
                RemoteMode::Anywhere
            } else if has_flag(&arg, "--local") {
                RemoteMode::Local
            } else if has_flag(&arg, "--lan") || !arg.split_whitespace().any(|t| t.starts_with('-'))
            {
                RemoteMode::Lan
            } else {
                // A mistyped flag must not silently bind 0.0.0.0.
                return CommandAction::Unknown(format!("remote {arg}"));
            };
            CommandAction::Remote { mode }
        }
        "anywhere" | "aw" => CommandAction::Anywhere,
        "thinking" | "think" => CommandAction::Thinking,
        "output" => CommandAction::ToolOutput,
        "image" | "img" => CommandAction::Image(arg),
        "init" => CommandAction::Init,
        "plan" => CommandAction::Plan(arg),
        "execute" | "approve" | "go" => CommandAction::Execute,
        "quit" | "exit" | "q" => CommandAction::Quit,
        "claude" => CommandAction::Claude,
        "statusline" | "sl" => {
            let action = match arg.to_lowercase().trim() {
                "layout" | "show" | "" => StatuslineAction::Layout,
                "reset" | "default" => StatuslineAction::Reset,
                "edit" => StatuslineAction::Edit,
                other => {
                    let widget = other
                        .strip_prefix("toggle ")
                        .unwrap_or(other)
                        .trim()
                        .to_string();
                    StatuslineAction::Toggle(widget)
                }
            };
            CommandAction::Statusline(action)
        }
        "workflow" | "wf" => CommandAction::Workflow(workflow_args::workflow_action(&arg)),
        "heartbeat" | "hb" => crate::heartbeat_args::heartbeat_action(&arg),
        other => CommandAction::Unknown(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_carries_usage_and_exposes_selected_hint() {
        let mut p = Palette::default();
        p.open_with("effort");
        // The /effort builtin carries a usage string with a fixed enum.
        let m = p.matches();
        let eff = m
            .iter()
            .find(|e| e.name == "effort")
            .expect("effort present");
        assert!(
            eff.usage.contains("low"),
            "usage hint retained, got {:?}",
            eff.usage
        );
        // selected_usage surfaces the highlighted row's hint.
        p.selected = m.iter().position(|e| e.name == "effort").unwrap();
        assert_eq!(p.selected_usage().as_deref(), Some(eff.usage.as_str()));
    }

    #[test]
    fn arg_values_parses_fixed_enums_only() {
        // Clean `a|b|c` enum → parsed.
        assert_eq!(
            arg_values("effort"),
            vec!["low", "medium", "high", "xhigh", "whitehot"]
        );
        // Freeform args (`/model [<id>]`, `/replay <id> …`) → no fixed set.
        assert!(arg_values("model").is_empty());
        assert!(arg_values("replay").is_empty());
        // `[--flag <v>]` style isn't a clean enum.
        assert!(arg_values("remote").is_empty());
        // Unknown command.
        assert!(arg_values("nope").is_empty());
    }

    fn rows() -> Vec<PickerRow> {
        vec![
            PickerRow {
                id: "aaa".into(),
                title: "aaa  $0.01  2 msgs".into(),
                subtitle: "fix the auth bug".into(),
            },
            PickerRow {
                id: "bbb".into(),
                title: "bbb  $0.02  5 msgs".into(),
                subtitle: "refactor the mesh".into(),
            },
        ]
    }

    #[test]
    fn parses_new_commands() {
        assert_eq!(
            parse_command("/dispatch  split the parser work "),
            CommandAction::Dispatch("split the parser work".into())
        );
        assert_eq!(
            parse_command("/dispatch"),
            CommandAction::Dispatch(String::new())
        );
        assert!(COMMANDS
            .iter()
            .any(|c| c.name == "dispatch" && c.usage == "/dispatch <request>"));
        assert_eq!(parse_command("/mode"), CommandAction::Mode);
        assert_eq!(parse_command("/m"), CommandAction::Mode);
        assert_eq!(parse_command("/undo"), CommandAction::Undo);
        assert_eq!(parse_command("/rewind"), CommandAction::Undo);
        assert_eq!(parse_command("/compact"), CommandAction::Compact(None));
        assert_eq!(
            parse_command("/compact  keep the migration plan "),
            CommandAction::Compact(Some("keep the migration plan".into()))
        );
        assert_eq!(parse_command("/uncompact"), CommandAction::Uncompact);
        assert_eq!(parse_command("/reground"), CommandAction::Reground);
        assert_eq!(
            parse_command("/checkpoints"),
            CommandAction::ListCheckpoints
        );
        assert_eq!(
            parse_command("/checkpoint"),
            CommandAction::Checkpoint(None)
        );
        assert_eq!(
            parse_command("/checkpoint before refactor"),
            CommandAction::Checkpoint(Some("before refactor".into()))
        );
        assert_eq!(parse_command("/config"), CommandAction::Config);
        assert_eq!(parse_command("/cfg"), CommandAction::Config);
        assert_eq!(parse_command("/settings"), CommandAction::Config);
        // /copy: bare → last response; numeric arg → that depth; junk/zero → clamp to 1.
        assert_eq!(parse_command("/copy"), CommandAction::Copy { nth: 1 });
        assert_eq!(parse_command("/copy 3"), CommandAction::Copy { nth: 3 });
        assert_eq!(parse_command("/copy 0"), CommandAction::Copy { nth: 1 });
        assert_eq!(parse_command("/copy abc"), CommandAction::Copy { nth: 1 });
        assert_eq!(parse_command("/yank 2"), CommandAction::Copy { nth: 2 });
        assert_eq!(parse_command("/mcp"), CommandAction::Mcp(None));
        assert_eq!(
            parse_command("/mcp gitlab"),
            CommandAction::Mcp(Some("gitlab".into()))
        );
        assert_eq!(
            parse_command("/goal ship the parser"),
            CommandAction::Goal {
                objective: "ship the parser".into(),
                gates: Vec::new(),
                max_tokens: None,
                max_minutes: None,
            }
        );
        assert_eq!(
            parse_command("/goal"),
            CommandAction::Goal {
                objective: String::new(),
                gates: Vec::new(),
                max_tokens: None,
                max_minutes: None,
            }
        );
        assert_eq!(
            parse_command("/pr feat: add rate limiter"),
            CommandAction::Pr("feat: add rate limiter".into())
        );
        assert_eq!(parse_command("/pr"), CommandAction::Pr(String::new()));
        assert_eq!(
            parse_command("/loop fix all warnings"),
            CommandAction::Loop {
                prompt: "fix all warnings".into(),
                gates: Vec::new(),
                max_tokens: None,
                max_minutes: None,
            }
        );
        assert_eq!(
            parse_command("/duel add pagination to the /users endpoint"),
            CommandAction::Duel("add pagination to the /users endpoint".into())
        );
        assert_eq!(parse_command("/duel"), CommandAction::Duel(String::new()));
        assert_eq!(parse_command("/init"), CommandAction::Init);
        assert_eq!(
            parse_command("/plan add a cache layer"),
            CommandAction::Plan("add a cache layer".into())
        );
        assert_eq!(parse_command("/plan"), CommandAction::Plan(String::new()));
        assert_eq!(parse_command("/execute"), CommandAction::Execute);
        assert_eq!(parse_command("/approve"), CommandAction::Execute);
        assert_eq!(parse_command("/go"), CommandAction::Execute);
        assert_eq!(parse_command("/voice"), CommandAction::Voice);
        assert_eq!(
            parse_command("/btw is this bug worth a ticket?"),
            CommandAction::Btw("is this bug worth a ticket?".into())
        );
        assert_eq!(parse_command("/btw"), CommandAction::Btw(String::new()));
        assert_eq!(parse_command("/btw   "), CommandAction::Btw(String::new()));
        // /side is an alias for /btw — same action, same parsing.
        assert_eq!(
            parse_command("/side what does EAGAIN mean?"),
            CommandAction::Btw("what does EAGAIN mean?".into())
        );
        assert_eq!(parse_command("/export"), CommandAction::Export(None));
        assert_eq!(
            parse_command("/export out/session.html"),
            CommandAction::Export(Some("out/session.html".into()))
        );
        assert_eq!(parse_command("/record"), CommandAction::Voice);
    }

    #[test]
    fn goal_steering_verbs_are_whole_words_and_everything_else_is_still_a_goal() {
        let goal = |objective: &str| CommandAction::Goal {
            objective: objective.into(),
            gates: Vec::new(),
            max_tokens: None,
            max_minutes: None,
        };
        assert_eq!(
            parse_command("/goal pause"),
            CommandAction::GoalControl(GoalControl::Pause)
        );
        assert_eq!(
            parse_command("/goal  RESUME "),
            CommandAction::GoalControl(GoalControl::Resume)
        );
        assert_eq!(
            parse_command("/goal clear"),
            CommandAction::GoalControl(GoalControl::Clear)
        );
        assert_eq!(
            parse_command("/goal status"),
            CommandAction::GoalControl(GoalControl::Status)
        );
        assert_eq!(
            parse_command("/goal edit make it faster"),
            CommandAction::GoalControl(GoalControl::Edit("make it faster".into()))
        );
        assert_eq!(
            parse_command("/goal pause the nightly job"),
            goal("pause the nightly job")
        );
        assert_eq!(
            parse_command("/goal clear the cache"),
            goal("clear the cache")
        );
        assert_eq!(parse_command("/goal edit"), goal("edit"));
    }

    #[test]
    fn parses_loop_and_goal_gate_and_budget_options() {
        // A single quoted gate, before the prompt.
        assert_eq!(
            parse_command("/loop --gate \"cargo test\" fix the bug"),
            CommandAction::Loop {
                prompt: "fix the bug".into(),
                gates: vec!["cargo test".into()],
                max_tokens: None,
                max_minutes: None,
            }
        );
        // Repeated gates, budgets, order-independent, prompt reassembled from remaining tokens.
        assert_eq!(
            parse_command(
                "/loop --gate \"cargo test\" --max-tokens 50000 --gate \"cargo clippy\" --max-minutes 30 ship it"
            ),
            CommandAction::Loop {
                prompt: "ship it".into(),
                gates: vec!["cargo test".into(), "cargo clippy".into()],
                max_tokens: Some(50_000),
                max_minutes: Some(30),
            }
        );
        assert_eq!(
            parse_command("/goal --gate \"npm run lint\" ship the parser"),
            CommandAction::Goal {
                objective: "ship the parser".into(),
                gates: vec!["npm run lint".into()],
                max_tokens: None,
                max_minutes: None,
            }
        );
        // Bare form (no options) must be untouched — exact prior behavior, whitespace and all.
        assert_eq!(
            parse_command("/loop   fix   all   warnings"),
            CommandAction::Loop {
                prompt: "fix   all   warnings".into(),
                gates: Vec::new(),
                max_tokens: None,
                max_minutes: None,
            }
        );
        // A non-numeric budget value is refused, not silently run uncapped.
        assert!(matches!(
            parse_command("/loop --max-tokens oops go"),
            CommandAction::Unknown(_)
        ));
        assert!(matches!(
            parse_command("/goal ship --max-minutes soon"),
            CommandAction::Unknown(_)
        ));
    }

    #[test]
    fn parses_refine_command() {
        assert_eq!(
            parse_command("/refine"),
            CommandAction::Refine(RefineAction::Run {
                instructions: None,
                global: false,
            })
        );
        assert_eq!(
            parse_command("/refine be stricter about tool-call retries"),
            CommandAction::Refine(RefineAction::Run {
                instructions: Some("be stricter about tool-call retries".into()),
                global: false,
            })
        );
        assert_eq!(
            parse_command("/refine --global"),
            CommandAction::Refine(RefineAction::Run {
                instructions: None,
                global: true,
            })
        );
        assert_eq!(
            parse_command("/refine --global prefer smaller diffs"),
            CommandAction::Refine(RefineAction::Run {
                instructions: Some("prefer smaller diffs".into()),
                global: true,
            })
        );
        assert_eq!(
            parse_command("/refine status"),
            CommandAction::Refine(RefineAction::Status)
        );
        assert_eq!(
            parse_command("/refine STATUS"),
            CommandAction::Refine(RefineAction::Status)
        );
        assert_eq!(
            parse_command("/refine rollback abc123"),
            CommandAction::Refine(RefineAction::Rollback("abc123".into()))
        );
        assert_eq!(
            parse_command("/refine rollback"),
            CommandAction::Refine(RefineAction::Rollback(String::new()))
        );
    }

    #[test]
    fn parses_remember_and_memories_commands() {
        assert_eq!(
            parse_command("/remember tabs not spaces"),
            CommandAction::Remember("tabs not spaces".into())
        );
        assert_eq!(parse_command("/memories"), CommandAction::Memories);
    }

    #[test]
    fn parses_heartbeat_command_and_subactions() {
        assert_eq!(
            parse_command("/heartbeat every 5m check the CI status"),
            CommandAction::Heartbeat(HeartbeatAction::Every {
                interval: "5m".into(),
                prompt: "check the CI status".into(),
            })
        );
        assert_eq!(
            parse_command("/heartbeat"),
            CommandAction::Heartbeat(HeartbeatAction::Status)
        );
        assert_eq!(
            parse_command("/heartbeat status"),
            CommandAction::Heartbeat(HeartbeatAction::Status)
        );
        assert_eq!(
            parse_command("/heartbeat pause"),
            CommandAction::Heartbeat(HeartbeatAction::Pause)
        );
        assert_eq!(
            parse_command("/heartbeat resume"),
            CommandAction::Heartbeat(HeartbeatAction::Resume)
        );
        assert_eq!(
            parse_command("/heartbeat clear"),
            CommandAction::Heartbeat(HeartbeatAction::Clear)
        );
        assert_eq!(
            parse_command("/hb every 30s ping"),
            CommandAction::Heartbeat(HeartbeatAction::Every {
                interval: "30s".into(),
                prompt: "ping".into(),
            })
        );
    }

    #[test]
    fn parses_workflow_command_and_subactions() {
        assert_eq!(
            parse_command("/workflow ship the release"),
            CommandAction::Workflow(WorkflowAction::Run("ship the release".into()))
        );
        assert_eq!(
            parse_command("/workflow"),
            CommandAction::Workflow(WorkflowAction::List)
        );
        assert_eq!(
            parse_command("/workflow list"),
            CommandAction::Workflow(WorkflowAction::List)
        );
        assert_eq!(
            parse_command("/workflow run audit"),
            CommandAction::Workflow(WorkflowAction::RunSaved {
                name: "audit".into(),
                args: String::new(),
            })
        );
        assert_eq!(
            parse_command("/workflow run audit --deep"),
            CommandAction::Workflow(WorkflowAction::RunSaved {
                name: "audit".into(),
                args: "--deep".into(),
            })
        );
        // The `wf` alias behaves identically.
        assert_eq!(
            parse_command("/wf run audit"),
            CommandAction::Workflow(WorkflowAction::RunSaved {
                name: "audit".into(),
                args: String::new(),
            })
        );
    }

    #[test]
    fn parses_agents_command() {
        assert_eq!(parse_command("/agents"), CommandAction::Agents);
        assert!(COMMANDS.iter().any(|c| c.name == "agents"));
    }

    #[test]
    fn parses_subagents_command() {
        assert_eq!(
            parse_command("/subagents free"),
            CommandAction::Subagents(Some(true))
        );
        assert_eq!(
            parse_command("/subagents pinned"),
            CommandAction::Subagents(Some(false))
        );
        // Bare `/subagents` toggles; an unrecognised argument must NOT silently mean "free" —
        // it falls through to the same toggle rather than guessing a direction.
        assert_eq!(parse_command("/subagents"), CommandAction::Subagents(None));
        assert_eq!(
            parse_command("/subagents whatever"),
            CommandAction::Subagents(None)
        );
        // Singular alias + case-insensitivity, mirroring /self-mcp.
        assert_eq!(
            parse_command("/subagent FREE"),
            CommandAction::Subagents(Some(true))
        );
    }

    #[test]
    fn parses_exact_effort_without_disturbing_the_ceiling_pin() {
        // `exact` is the only word that switches meaning; everything else stays the ceiling pin.
        assert_eq!(
            parse_command("/effort exact high"),
            CommandAction::SetExactEffort(Some("high".into()))
        );
        assert_eq!(
            parse_command("/effort exact max"),
            CommandAction::SetExactEffort(Some("max".into()))
        );
        // Three ways to clear it.
        assert_eq!(
            parse_command("/effort exact"),
            CommandAction::SetExactEffort(None)
        );
        assert_eq!(
            parse_command("/effort exact off"),
            CommandAction::SetExactEffort(None)
        );
        assert_eq!(
            parse_command("/effort exact none"),
            CommandAction::SetExactEffort(None)
        );
        // The existing command is untouched — a bare level is still the ceiling pin, and a model
        // whose name merely starts with "exact" is not swallowed as the subcommand.
        assert_eq!(
            parse_command("/effort high"),
            CommandAction::SetEffort(Some("high".into()))
        );
        assert_eq!(parse_command("/effort"), CommandAction::SetEffort(None));
    }

    #[test]
    fn parses_effort_command() {
        assert_eq!(
            parse_command("/effort high"),
            CommandAction::SetEffort(Some("high".into()))
        );
        assert_eq!(
            parse_command("/effort xhigh"),
            CommandAction::SetEffort(Some("xhigh".into()))
        );
        assert_eq!(
            parse_command("/effort low"),
            CommandAction::SetEffort(Some("low".into()))
        );
        assert_eq!(
            parse_command("/effort medium"),
            CommandAction::SetEffort(Some("medium".into()))
        );
        // bare /effort clears the pin
        assert_eq!(parse_command("/effort"), CommandAction::SetEffort(None));
    }

    #[test]
    fn parses_remote_command_and_alias() {
        // `/remote` (and alias `/rc`) toggle on with LAN binding by default.
        assert_eq!(
            parse_command("/remote"),
            CommandAction::Remote {
                mode: RemoteMode::Lan
            }
        );
        assert_eq!(
            parse_command("/rc"),
            CommandAction::Remote {
                mode: RemoteMode::Lan
            }
        );
        // `--local` binds loopback only; `--lan` is the explicit default.
        assert_eq!(
            parse_command("/remote --local"),
            CommandAction::Remote {
                mode: RemoteMode::Local
            }
        );
        assert_eq!(
            parse_command("/rc --lan"),
            CommandAction::Remote {
                mode: RemoteMode::Lan
            }
        );
        assert!(matches!(
            parse_command("/remote --lcoal"),
            CommandAction::Unknown(_)
        ));
        // `--anywhere` (and `-a`) pipe through a public tunnel.
        assert_eq!(
            parse_command("/remote --anywhere"),
            CommandAction::Remote {
                mode: RemoteMode::Anywhere
            }
        );
        assert_eq!(
            parse_command("/rc -a"),
            CommandAction::Remote {
                mode: RemoteMode::Anywhere
            }
        );
    }

    #[test]
    fn parses_anywhere_management_command() {
        assert_eq!(parse_command("/anywhere"), CommandAction::Anywhere);
        assert_eq!(parse_command("/aw"), CommandAction::Anywhere);
        assert_eq!(command_category("anywhere"), "Integrations");
    }

    #[test]
    fn picker_filters_navigates_and_resolves_a_row() {
        let mut p = Picker::default();
        p.open_with(PickerKind::Sessions, "resume", rows());
        assert_eq!(p.matches().len(), 2);
        assert_eq!(p.selected_row().unwrap().id, "aaa");

        p.move_down();
        assert_eq!(p.selected_row().unwrap().id, "bbb");
        p.move_up();
        assert_eq!(p.selected_row().unwrap().id, "aaa");

        // Filter narrows the list; selection re-clamps.
        p.query = "mesh".into();
        p.clamp();
        let m = p.matches();
        assert_eq!(m.len(), 1, "only the mesh session matches");
        assert_eq!(p.selected_row().unwrap().id, "bbb");
    }

    #[test]
    fn closing_a_picker_clears_it() {
        let mut p = Picker::default();
        p.open_with(PickerKind::Checkpoints, "restore", rows());
        p.close();
        assert!(!p.open);
        assert!(p.kind.is_none());
        assert!(p.matches().is_empty());
    }

    #[test]
    fn parses_commands_and_args() {
        assert_eq!(parse_command("/help"), CommandAction::Help);
        assert_eq!(parse_command("/h"), CommandAction::Help);
        assert_eq!(
            parse_command("/resume ab12"),
            CommandAction::Resume("ab12".into())
        );
        assert_eq!(parse_command("/resume"), CommandAction::ListSessions);
        assert_eq!(parse_command("/new"), CommandAction::New);
        assert_eq!(parse_command("/clear"), CommandAction::New);
        assert_eq!(parse_command("/reset"), CommandAction::New);
        assert_eq!(parse_command("/cls"), CommandAction::ClearScreen);
        assert_eq!(parse_command("/quit"), CommandAction::Quit);
        assert_eq!(
            parse_command("/lattice Session"),
            CommandAction::Lattice("Session".into())
        );
        assert_eq!(
            parse_command("/lat foo"),
            CommandAction::Lattice("foo".into())
        );
        assert_eq!(
            parse_command("/bogus"),
            CommandAction::Unknown("bogus".into())
        );
    }

    #[test]
    fn parses_claude_code_everyday_commands() {
        assert_eq!(parse_command("/context"), CommandAction::Context);
        assert_eq!(parse_command("/diff"), CommandAction::Diff(String::new()));
        assert_eq!(
            parse_command("/diff src/lib.rs"),
            CommandAction::Diff("src/lib.rs".into())
        );
        assert_eq!(
            parse_command("/permissions"),
            CommandAction::Permissions(String::new())
        );
        assert_eq!(
            parse_command("/allowed-tools allow Bash(git log:*)"),
            CommandAction::Permissions("allow Bash(git log:*)".into())
        );
        assert_eq!(parse_command("/mcp"), CommandAction::Mcp(None));
        assert_eq!(
            parse_command("/mcp github"),
            CommandAction::Mcp(Some("github".into()))
        );
        assert_eq!(
            parse_command("/mcp reconnect"),
            CommandAction::McpReconnect(None)
        );
        assert_eq!(
            parse_command("/mcp reconnect github"),
            CommandAction::McpReconnect(Some("github".into()))
        );
        assert_eq!(
            parse_command("/add-dir ../lib"),
            CommandAction::AddDir("../lib".into())
        );
        assert_eq!(parse_command("/doctor"), CommandAction::Doctor);
        assert_eq!(
            parse_command("/rename auth refactor"),
            CommandAction::Rename("auth refactor".into())
        );
        assert_eq!(parse_command("/star"), CommandAction::Star);
        assert_eq!(parse_command("/done"), CommandAction::Done);
        assert_eq!(
            parse_command("/archived restore ab12"),
            CommandAction::Archived("restore ab12".into())
        );
        for name in [
            "context",
            "diff",
            "permissions",
            "add-dir",
            "doctor",
            "rename",
            "clear",
            "cls",
        ] {
            assert!(
                COMMANDS.iter().any(|c| c.name == name),
                "/{name} is in the registry"
            );
        }
    }

    #[test]
    fn resume_keeps_only_the_id_arg() {
        assert_eq!(
            parse_command("/resume   7f3a-9 "),
            CommandAction::Resume("7f3a-9".into())
        );
    }

    #[test]
    fn empty_query_lists_all_in_registry_order() {
        let all = filter_commands("");
        assert_eq!(all.len(), COMMANDS.len());
        assert_eq!(all[0].name, "help");
    }

    #[test]
    fn prefix_matches_rank_above_fuzzy() {
        // "s" is a prefix of "sessions"; it's a subsequence of "clear"? no. of "sessions" prefix.
        let m = filter_commands("se");
        assert_eq!(m[0].name, "sessions", "prefix hit first");
    }

    #[test]
    fn fuzzy_subsequence_matches() {
        // "clr" is a subsequence of "clear" but not a prefix.
        let m = filter_commands("clr");
        assert!(m.iter().any(|c| c.name == "clear"), "fuzzy matched clear");
    }

    #[test]
    fn no_match_returns_empty() {
        assert!(filter_commands("zzzzz").is_empty());
    }

    #[test]
    fn palette_navigation_clamps() {
        let mut p = Palette::default();
        p.open_with("");
        assert_eq!(p.selected, 0);
        p.move_up(); // can't go below 0
        assert_eq!(p.selected, 0);
        for _ in 0..100 {
            p.move_down();
        }
        assert_eq!(p.selected, COMMANDS.len() - 1, "clamped to last");
        // Narrow the query so the list shrinks; selection must re-clamp.
        p.query = "help".into();
        p.clamp();
        assert_eq!(p.selected, 0);
        assert_eq!(p.selected_name().as_deref(), Some("help"));
    }

    #[test]
    fn palette_merges_file_entries_and_builtin_wins_name_clash() {
        let mut p = Palette {
            extra: vec![
                PaletteEntry {
                    name: "review".into(),
                    desc: "file command".into(),
                    usage: String::new(),
                },
                // Same name as a builtin: the builtin must win (no duplicate row).
                PaletteEntry {
                    name: "clear".into(),
                    desc: "shadowing attempt".into(),
                    usage: String::new(),
                },
            ],
            ..Default::default()
        };
        p.open_with("");
        let names: Vec<String> = p.matches().into_iter().map(|e| e.name).collect();
        assert!(names.iter().any(|n| n == "review"), "file command shown");
        assert_eq!(
            names.iter().filter(|n| *n == "clear").count(),
            1,
            "no duplicate 'clear'"
        );
        let clear = p.matches().into_iter().find(|e| e.name == "clear").unwrap();
        assert_ne!(clear.desc, "shadowing attempt", "builtin clear wins");

        // Filtering still works over the merged set.
        p.query = "review".into();
        p.clamp();
        assert_eq!(p.selected_name().as_deref(), Some("review"));
    }

    #[test]
    fn command_center_groups_builtins_and_finds_custom_entries() {
        let center = CommandCenter::default();
        let entries = center.matches(&[PaletteEntry {
            name: "shipit".into(),
            desc: "run the project's release workflow".into(),
            usage: "/shipit".into(),
        }]);
        assert!(
            entries
                .iter()
                .any(|entry| entry.name == "plan" && entry.category == "Start work"),
            "builtins have stable, user-facing categories"
        );
        assert!(
            entries
                .iter()
                .any(|entry| { entry.name == "shipit" && entry.category == "Skills & custom" }),
            "custom commands remain discoverable"
        );

        let mut search = CommandCenter::default();
        search.open();
        search.query = "release".into();
        assert_eq!(
            search
                .selected_entry(&[PaletteEntry {
                    name: "shipit".into(),
                    desc: "run the project's release workflow".into(),
                    usage: "/shipit".into(),
                }])
                .as_ref()
                .map(|entry| entry.name.as_str()),
            Some("shipit"),
            "description search finds commands when users do not know their name"
        );
    }

    #[test]
    fn slash_token_detected_at_leading_position() {
        let t = slash_token_at("/orchestrate scan repo", 0).unwrap();
        assert_eq!(t.name, "orchestrate");
        assert_eq!(t.start, 0);
        assert_eq!(t.end, "/orchestrate".len());
    }

    #[test]
    fn slash_token_detected_mid_line() {
        let input = "please run /orchestrate scan repo";
        let off = input.find("/orchestrate").unwrap();
        // Cursor anywhere on the line still finds the only slash-token.
        let t = slash_token_at(input, input.len()).unwrap();
        assert_eq!(t.name, "orchestrate");
        assert_eq!(t.start, off);
    }

    #[test]
    fn slash_token_under_cursor_wins_when_multiple() {
        // Two slash tokens; the one the cursor is editing is selected.
        let input = "/help and /clear";
        let clear_off = input.find("/clear").unwrap();
        // Cursor at the end of `/clear` → that token.
        let t = slash_token_at(input, input.len()).unwrap();
        assert_eq!(t.name, "clear");
        assert_eq!(t.start, clear_off);
        // Cursor on `/help` (offset 3, inside the first token) → first token.
        let t = slash_token_at(input, 3).unwrap();
        assert_eq!(t.name, "help");
        assert_eq!(t.start, 0);
    }

    #[test]
    fn double_slash_is_literal_not_a_command() {
        assert!(slash_token_at("//literal", 0).is_none());
        assert!(slash_token_at("run //escaped here", 18.min("run //escaped here".len())).is_none());
        // A real command alongside an escape is still detected.
        let t = slash_token_at("//lit and /help", 15).unwrap();
        assert_eq!(t.name, "help");
    }

    #[test]
    fn no_slash_token_returns_none() {
        assert!(slash_token_at("just plain text", 5).is_none());
        assert!(slash_token_at("", 0).is_none());
    }

    #[test]
    fn slash_token_at_does_not_panic_on_multibyte_utf8() {
        // Regression: scanning by raw byte value used to misdetect char boundaries inside
        // multi-byte UTF-8 sequences (e.g. CJK), panicking on the subsequent string slice.
        let input = "hello 你 world";
        assert!(slash_token_at(input, input.len()).is_none());
        assert!(slash_token_at("你", 0).is_none());
        assert!(at_token_at(input, input.len()).is_none());
        assert!(at_token_at("你", 0).is_none());

        let with_command = "/orchestrate 你好 world";
        let t = slash_token_at(with_command, 0).unwrap();
        assert_eq!(t.name, "orchestrate");

        let with_at = "@你好/path";
        let t = at_token_at(with_at, with_at.len()).unwrap();
        assert_eq!(t.query, "你好/path");
    }

    #[test]
    fn anim_eases_to_one() {
        let mut p = Palette::default();
        p.open_with("");
        for _ in 0..10 {
            p.tick_anim();
        }
        assert_eq!(p.anim, 1.0);
    }

    fn numbered_rows(n: usize) -> Vec<PickerRow> {
        (0..n)
            .map(|i| PickerRow {
                id: i.to_string(),
                title: format!("prompt number {i}"),
                subtitle: format!("#{i} · {}m ago", i % 60),
            })
            .collect()
    }

    #[test]
    fn picker_jump_keys_move_the_selection() {
        let mut p = Picker::default();
        p.open_with(PickerKind::Checkpoints, "rewind", numbered_rows(100));
        p.end();
        assert_eq!(p.selected, 99);
        p.page_up(10);
        assert_eq!(p.selected, 89);
        p.home();
        assert_eq!(p.selected, 0);
        p.page_down(10);
        assert_eq!(p.selected, 10);
        p.page_down(1000);
        assert_eq!(p.selected, 99, "a page past the end stops on the last row");
        p.query = "number 7".into();
        p.clamp();
        p.end();
        assert_eq!(p.selected_row().unwrap().id, "79");
    }

    #[test]
    fn picker_stays_fast_with_thousands_of_rows() {
        let mut p = Picker::default();
        p.open_with(PickerKind::Checkpoints, "rewind", numbered_rows(20_000));
        let start = std::time::Instant::now();
        for _ in 0..200 {
            p.move_down();
            let _ = p.selected_row();
            let _ = p.matches().len();
        }
        p.query = "number 19999".into();
        for _ in 0..50 {
            p.clamp();
            assert_eq!(p.matches().len(), 1);
        }
        let elapsed = start.elapsed();
        assert!(
            elapsed < std::time::Duration::from_secs(2),
            "20k-row picker took {elapsed:?} for 250 interactions"
        );
    }

    #[test]
    fn extended_rows_are_filterable_and_resettable() {
        let mut p = Picker::default();
        p.open_with(PickerKind::Sessions, "resume", numbered_rows(3));
        p.extend_rows(vec![PickerRow {
            id: "hit".into(),
            title: "other".into(),
            subtitle: "match: alpha body".into(),
        }]);
        p.query = "alpha".into();
        assert_eq!(p.matches().len(), 1);
        p.reset_to_base();
        assert_eq!(p.matches().len(), 0);
        assert_eq!(p.base_len(), 3);
    }

    #[test]
    fn parses_fork_stash_skills_and_timeline() {
        assert_eq!(parse_command("/fork"), CommandAction::Fork(None));
        assert_eq!(
            parse_command("/fork try the other approach"),
            CommandAction::Fork(Some("try the other approach".into()))
        );
        assert_eq!(parse_command("/stash"), CommandAction::Stash(StashOp::Pop));
        assert_eq!(
            parse_command("/stash list"),
            CommandAction::Stash(StashOp::List)
        );
        assert_eq!(parse_command("/skills"), CommandAction::Skills);
        assert_eq!(parse_command("/timeline"), CommandAction::Undo);
        assert_eq!(parse_command("/q"), CommandAction::Quit);
    }

    #[test]
    fn at_picker_ranks_edited_files_first_among_equal_matches() {
        let mut picker = AtPathPicker::default();
        picker.open_with(
            "lib",
            vec!["a/lib.rs".into(), "b/lib.rs".into(), "c/lib.rs".into()],
        );
        picker.set_modified(["c/lib.rs".to_string()]);
        let paths = picker.matches();
        assert_eq!(paths[0], "c/lib.rs");
    }
}
