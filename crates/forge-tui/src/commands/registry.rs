//! The static slash-command registry: what exists, in display order.

/// One command's metadata, shown in the palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Command {
    pub name: &'static str,
    pub desc: &'static str,
    pub usage: &'static str,
}

/// The static command registry. Order = display order in `/help`.
pub const COMMANDS: &[Command] = &[
    Command {
        name: "help",
        desc: "show available commands",
        usage: "/help",
    },
    Command {
        name: "keys",
        desc: "show keyboard shortcuts (also F1)",
        usage: "/keys",
    },
    Command {
        name: "status",
        desc: "everything the header and status line can drop: model, mode, context, quotas",
        usage: "/status",
    },
    Command {
        name: "sessions",
        desc: "browse & resume past sessions",
        usage: "/sessions",
    },
    Command {
        name: "board",
        desc: "the live project board — runs in its own terminal (forge board)",
        usage: "/board",
    },
    Command {
        name: "dispatch",
        desc: "split a request into parallel sessions on the daemon (see forge board)",
        usage: "/dispatch <request>",
    },
    Command {
        name: "replay",
        desc:
            "show a session transcript inline (/replay <id>) or diff two sessions (/replay <a> <b>)",
        usage: "/replay <id> [<id2>]",
    },
    Command {
        name: "btw",
        desc: "ask a side question that never joins this session's transcript (alias /side)",
        usage: "/btw <question>",
    },
    Command {
        name: "export",
        desc: "export this session's transcript as one self-contained HTML file",
        usage: "/export [path]",
    },
    Command {
        name: "resume",
        desc: "resume a session by id prefix",
        usage: "/resume <id>",
    },
    Command {
        name: "mode",
        desc: "switch the operating mode (temper)",
        usage: "/mode",
    },
    Command {
        name: "assay",
        desc: "analyze code quality (AI-slop, dead/unsafe/untested) — or full cleanup",
        usage:
            "/assay [--diff|--branch <b>|--since <ref>|<path>] [--only <lens,…>] [--skip <lens,…>]",
    },
    Command {
        name: "model",
        desc: "pick or pin a model for this session",
        usage: "/model [<id>]",
    },
    Command {
        name: "models",
        desc: "browse available models by provider (counts + frontier/free)",
        usage: "/models",
    },
    Command {
        name: "config",
        desc: "configure Forge — provider & search API keys, bridge plans, permission mode, credit conservation",
        usage: "/config",
    },
    Command {
        name: "copy",
        desc: "copy the last assistant response to the clipboard (/copy N for the Nth-latest)",
        usage: "/copy [N]",
    },
    Command {
        name: "thinking",
        desc: "toggle model reasoning/thinking display on or off",
        usage: "/thinking",
    },
    Command {
        name: "output",
        desc: "open the latest tool call's full output (search, copy, $PAGER)",
        usage: "/output",
    },
    Command {
        name: "image",
        desc: "attach an image file to your next message (vision input)",
        usage: "/image <path>",
    },
    Command {
        name: "mcp",
        desc: "list connected MCP servers (or one server's tools: /mcp <server>)",
        usage: "/mcp [server]",
    },
    Command {
        name: "init",
        desc: "scan the repo and generate .forge/AGENTS.md project memory",
        usage: "/init",
    },
    Command {
        name: "heartbeat",
        desc: "a recurring prompt that re-enters this session (status/pause/resume/clear)",
        usage: "/heartbeat every <interval> <prompt> | status | pause | resume | clear",
    },
    Command {
        name: "new",
        desc: "start a fresh session",
        usage: "/new",
    },
    Command {
        name: "rewind",
        desc: "pick a past message to rewind to — chat, context AND file edits (alias /undo, Esc Esc)",
        usage: "/rewind",
    },
    Command {
        name: "checkpoint",
        desc: "save a named checkpoint here",
        usage: "/checkpoint [name]",
    },
    Command {
        name: "checkpoints",
        desc: "browse & restore checkpoints",
        usage: "/checkpoints",
    },
    Command {
        name: "compact",
        desc: "summarize older messages to free up context",
        usage: "/compact [what to keep]",
    },
    Command {
        name: "commit",
        desc: "commit this session's verified work in focused conventional commits (no push)",
        usage: "/commit [message hint]",
    },
    Command {
        name: "uncompact",
        desc: "restore full transcript after a /compact",
        usage: "/uncompact",
    },
    Command {
        name: "reground",
        desc: "re-detect the host layout and re-summarize history without stale host paths",
        usage: "/reground",
    },
    Command {
        name: "refine",
        desc: "review the trajectory and persist learned harness state (prompt notes, skills, subagent specs)",
        usage: "/refine [instructions] | /refine --global [instructions] | /refine rollback <id> | /refine status",
    },
    Command {
        name: "lattice",
        desc: "show a symbol's code-intelligence subgraph (callers + provenance)",
        usage: "/lattice <symbol>",
    },
    Command {
        name: "plan",
        desc: "planning mode: investigate read-only and propose a plan (no edits) — then /execute",
        usage: "/plan <task>",
    },
    Command {
        name: "execute",
        desc: "approve the proposed plan and carry it out (switches to Auto-edit)",
        usage: "/execute",
    },
    Command {
        name: "goal",
        desc: "set a session goal and break it into a tracked task plan — optionally gated on quality checks and a token/time budget",
        usage: "/goal <objective> [--gate \"<cmd>\"]... [--max-tokens N] [--max-minutes N]",
    },
    Command {
        name: "pr",
        desc: "turn this session's work into a branch + commit + pull request (provenance-rich body)",
        usage: "/pr [title]",
    },
    Command {
        name: "loop",
        desc: "re-run a task each turn until the model signals it's complete — optionally gated on quality checks and a token/time budget",
        usage: "/loop <task> [--gate \"<cmd>\"]... [--max-tokens N] [--max-minutes N]",
    },
    Command {
        name: "effort",
        desc: "pin the reasoning-effort level (a ceiling the mesh optimises under), or with `exact` force the rung the provider actually runs at",
        usage: "/effort [low|medium|high|xhigh|whitehot] | /effort exact <level> | /effort exact off",
    },
    Command {
        name: "subagents",
        desc: "let this session's subagents route off the active model pin (free), or bind them back to it (pinned)",
        usage: "/subagents [free|pinned]",
    },
    Command {
        name: "agents",
        desc: "list the subagent types spawn_agents can use — built-in explore/plan plus your agent files",
        usage: "/agents",
    },
    Command {
        name: "remember",
        desc: "save a durable memory for this project",
        usage: "/remember <text>",
    },
    Command {
        name: "memories",
        desc: "list this project's memories",
        usage: "/memories",
    },
    Command {
        name: "clear",
        desc: "clear the screen (keep the session)",
        usage: "/clear",
    },
    Command {
        name: "usage",
        desc: "show API spend + token usage across providers",
        usage: "/usage",
    },
    Command {
        name: "mesh",
        desc: "inspect mesh routing — classification, scores, quota, conservation",
        usage: "/mesh [task prompt]",
    },
    Command {
        name: "remote",
        desc: "toggle remote control — drive this session from a phone/desktop browser",
        usage: "/remote [--lan | --local | --anywhere]",
    },
    Command {
        name: "anywhere",
        desc: "set up and manage Forge Anywhere hosts, recovery, and device approvals",
        usage: "/anywhere",
    },
    Command {
        name: "self-mcp",
        desc: "toggle self-MCP mode: start/stop a sub-Forge MCP agent so the session can call forge_chat / forge_assay as tools",
        usage: "/self-mcp [enable|disable]",
    },
    Command {
        name: "quit",
        desc: "exit Forge",
        usage: "/quit",
    },
    Command {
        name: "claude",
        desc: "leave Forge and continue this conversation in Claude Code",
        usage: "/claude",
    },
    Command {
        name: "statusline",
        desc: "customize the statusline layout — toggle widgets, show layout, reset to default",
        usage: "/statusline [layout|toggle <widget>|reset|edit]",
    },
    Command {
        name: "workflow",
        desc: "author and run a mesh-routed multi-agent workflow script for a goal, run a saved one, or list saved scripts",
        usage: "/workflow <goal> | /workflow run <name> [args] | /workflow list",
    },
    Command {
        name: "duel",
        desc: "race 2-3 mesh models on the same task, each in its own worktree, then pick the winner",
        usage: "/duel <task>",
    },
    Command {
        name: "voice",
        desc: "record from the microphone and transcribe into the prompt (also Ctrl+V) — local whisper, never auto-sends",
        usage: "/voice",
    },
];
