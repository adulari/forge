/// Product-facing categories for the stable command registry.
pub fn command_category(name: &str) -> &'static str {
    match name {
        "new" | "plan" | "execute" | "goal" | "loop" | "workflow" | "duel" => "Start work",
        "sessions" | "resume" | "replay" | "rewind" | "undo" | "checkpoint" | "checkpoints"
        | "compact" | "uncompact" | "status" | "reground" | "refine" | "clear" | "btw"
        | "export" | "context" | "rename" | "cls" | "fork" | "stash" | "timeline" => "Session",
        "model" | "models" | "mode" | "effort" | "subagents" | "agents" | "thinking" | "mesh"
        | "usage" => "Model & usage",
        "assay" | "lattice" | "pr" | "commit" | "diff" => "Review & ship",
        "mcp" | "remote" | "anywhere" | "self-mcp" | "voice" | "image" | "claude" => "Integrations",
        "config" | "statusline" | "keys" | "help" | "init" | "remember" | "memories"
        | "permissions" | "add-dir" | "doctor" | "skills" => "Settings & help",
        "copy" | "quit" => "Utilities",
        _ => "More",
    }
}
