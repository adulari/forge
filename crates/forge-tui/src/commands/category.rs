/// Product-facing categories for the stable command registry.
pub fn command_category(name: &str) -> &'static str {
    match name {
        "new" | "plan" | "execute" | "goal" | "loop" | "workflow" | "duel" => "Start work",
        "sessions" | "resume" | "replay" | "rewind" | "undo" | "checkpoint" | "checkpoints"
        | "compact" | "uncompact" | "refine" | "clear" | "btw" | "export" => "Session",
        "model" | "models" | "mode" | "effort" | "subagents" | "thinking" | "mesh" | "usage" => {
            "Model & usage"
        }
        "assay" | "lattice" | "pr" | "commit" => "Review & ship",
        "mcp" | "remote" | "anywhere" | "self-mcp" | "voice" | "image" => "Integrations",
        "config" | "statusline" | "keys" | "help" | "init" | "remember" | "memories" => {
            "Settings & help"
        }
        "copy" | "quit" => "Utilities",
        _ => "More",
    }
}
