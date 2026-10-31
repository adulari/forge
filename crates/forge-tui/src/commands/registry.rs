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
        name: "context",
        desc: "what fills the context window: prompt, tools, instructions, conversation, free space",
        usage: "/context",
    },
    Command {
        name: "diff",
        desc: "the working tree's uncommitted changes (git diff, untracked files listed)",
        usage: "/diff [path]",
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
            "show a session transcript inline, or diff two sessions (/replay <a> <b>)",
        usage: "/replay <id> [<id2>]",
    },
    Command {
        name: "btw",
        desc: "ask a side question that never joins this session's transcript (alias /side)",
        usage: "/btw <question>",
    },
    Command {
        name: "export",
        desc: "export this session's transcript as one self-contained HTML file (--json for JSON)",
        usage: "/export [--json] [path]",
    },
    Command {
        name: "star",
        desc: "star or unstar this session; starred sessions float and skip auto-archive",
        usage: "/star",
    },
    Command {
        name: "done",
        desc: "mark this session done, or reopen it",
        usage: "/done",
    },
    Command {
        name: "delete",
        desc: "delete this session permanently (type its id to confirm)",
        usage: "/delete",
    },
    Command {
        name: "archived",
        desc: "list archived sessions, or restore one without resuming it",
        usage: "/archived [restore <id>]",
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
        desc: "configure Forge — provider and search keys, bridge plans, permission mode",
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
        desc: "list connected MCP servers, one server's tools (/mcp <server>), or reconnect them",
        usage: "/mcp [server] | /mcp reconnect [server]",
    },
    Command {
        name: "permissions",
        desc: "list the permission rules in force; add a session rule with allow/ask/deny",
        usage: "/permissions [allow|ask|deny <Tool(pattern)>]",
    },
    Command {
        name: "add-dir",
        desc: "let file tools work in another directory this session, like the workspace",
        usage: "/add-dir <path>",
    },
    Command {
        name: "doctor",
        desc: "check config, providers, store and daemon without leaving the session",
        usage: "/doctor",
    },
    Command {
        name: "bug",
        desc: "report a problem to the maintainers: shows exactly what is sent, then asks",
        usage: "/bug <what went wrong>",
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
        desc: "start a fresh session (also /clear, /reset)",
        usage: "/new",
    },
    Command {
        name: "rename",
        desc: "name this session (shown in /sessions and the resume picker)",
        usage: "/rename <title>",
    },
    Command {
        name: "rewind",
        desc: "pick a past message to rewind to: chat, context and file edits (alias /undo)",
        usage: "/rewind",
    },
    Command {
        name: "fork",
        desc: "branch this session into a child at its latest message and continue there",
        usage: "/fork [--model <id>] [message]",
    },
    Command {
        name: "timeline",
        desc: "pick a past message to jump back to (same picker as /rewind)",
        usage: "/timeline",
    },
    Command {
        name: "stash",
        desc: "park the draft (Ctrl+X) and bring it back later: pop, list, clear",
        usage: "/stash [push|pop|list|clear]",
    },
    Command {
        name: "skills",
        desc: "list the skills the model can load, with scope and description",
        usage: "/skills",
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
        desc: "review the trajectory and keep what it learned (notes, skills, subagent specs)",
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
        desc: "set a goal and track it as a plan, or steer it: pause, resume, edit, clear",
        usage: "/goal <objective> [--gate \"<cmd>\"]... [--max-tokens N] [--max-minutes N] | pause | resume | clear | status | edit <objective>",
    },
    Command {
        name: "pr",
        desc: "turn this session's work into a branch, a commit and a pull request",
        usage: "/pr [title]",
    },
    Command {
        name: "loop",
        desc: "re-run a task each turn until done, optionally gated on checks and a budget",
        usage: "/loop <task> [--gate \"<cmd>\"]... [--max-tokens N] [--max-minutes N]",
    },
    Command {
        name: "effort",
        desc: "pin the reasoning effort (a ceiling for mesh), or `exact` to force the rung",
        usage: "/effort [low|medium|high|xhigh|whitehot] | /effort exact <level> | /effort exact off",
    },
    Command {
        name: "theme",
        desc: "switch the accent palette (forge, ocean, grove, rose) and save it",
        usage: "/theme [forge|ocean|grove|rose]",
    },
    Command {
        name: "subagents",
        desc: "let subagents route off the model pin (free), or follow the pin (pinned)",
        usage: "/subagents [free|pinned]",
    },
    Command {
        name: "agents",
        desc: "list the subagent types spawn_agents can use (built-in plus your agent files)",
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
        desc: "start a new conversation with an empty context, as in Claude Code (same as /new)",
        usage: "/clear",
    },
    Command {
        name: "cls",
        desc: "clear the screen and keep the session",
        usage: "/cls",
    },
    Command {
        name: "stats",
        desc: "lifetime activity poster: 52-week heatmap, streaks, busiest providers",
        usage: "/stats [days]",
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
        name: "control",
        desc: "control tools: what agents may do to this app, and what they did",
        usage: "/control",
    },
    Command {
        name: "remote",
        desc: "toggle remote control — drive this session from a phone/desktop browser",
        usage: "/remote [--lan | --local | --anywhere]",
    },
    Command {
        name: "anywhere",
        desc: "Forge Anywhere menu: account, sync across machines, devices, recovery",
        usage: "/anywhere",
    },
    Command {
        name: "self-mcp",
        desc: "toggle self-MCP mode: a sub-Forge MCP agent this session can call as tools",
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
        desc: "author and run a mesh-routed workflow script, run a saved one, or list them",
        usage: "/workflow <goal> | /workflow run <name> [args] | /workflow list",
    },
    Command {
        name: "duel",
        desc: "race 2-3 mesh models on one task in separate worktrees, then pick the winner",
        usage: "/duel <task>",
    },
    Command {
        name: "voice",
        desc: "transcribe the microphone into the prompt (also Ctrl+V); local, never auto-sends",
        usage: "/voice",
    },
];
