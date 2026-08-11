//! What the bridge is allowed to spend.
//!
//! On the CLI-bridge path the bridged model (claude, codex) runs ITS own loop, and every tool
//! schema, description, and result is re-ingested on every turn of that loop. A cost that is paid
//! once on the direct path is therefore paid per turn here. This module owns the resulting
//! budget policy — result caps that mirror what the native CLIs apply to their own tools, the lean
//! tool surface, the hard cap on the advertised skill list, and the gate for connecting external
//! MCP servers. All of it is bridge-only; the direct path is untouched.

use forge_config::Config;

/// Bridge-side byte cap on a `read_file` result. Mirrors the native caps claude/codex apply to
/// their own read tools (they page files in ~small chunks); forge-tools' direct-path cap is
/// 256 KiB, which a bridged CLI re-ingests on every subsequent turn of ITS loop — a large read
/// through the bridge costs 2-4x the tokens of the same read in the native CLI. mcp-serve path
/// only; the direct-API path is untouched.
pub(super) const BRIDGE_READ_CAP_BYTES: usize = 32 * 1024;

/// Bridge-side byte cap on a `shell` result. Mirrors the ~10-16 KiB head+tail clamp claude/codex
/// apply to their own shell tools. mcp-serve path only.
pub(super) const BRIDGE_SHELL_CAP_BYTES: usize = 16 * 1024;

/// The cap + re-request advice for a tool's result on the bridge path, or `None` for uncapped
/// tools.
pub(super) fn bridge_cap_for(tool: &str) -> Option<(usize, &'static str)> {
    match tool {
        "read_file" => Some((
            BRIDGE_READ_CAP_BYTES,
            "re-request just the lines you need with start_line/end_line",
        )),
        "shell" => Some((
            BRIDGE_SHELL_CAP_BYTES,
            "re-run with a narrower command or filters (grep/head/tail)",
        )),
        _ => None,
    }
}

/// Clamp an oversized bridge tool result to its cap, keeping the head (headers/signatures) and
/// tail (totals/trailing errors) around an explicit marker so the model knows the middle is
/// missing and how to get it back.
pub(super) fn cap_bridge_result(tool: &str, text: String) -> String {
    let Some((cap, advice)) = bridge_cap_for(tool) else {
        return text;
    };
    if text.len() <= cap {
        return text;
    }
    let head_end = floor_char_boundary(&text, cap * 3 / 4);
    let tail_start = ceil_char_boundary(&text, text.len() - cap / 4);
    format!(
        "{}\n[… {} of {} bytes omitted by the Forge bridge ({} KiB cap) — {advice} …]\n{}",
        &text[..head_end],
        tail_start - head_end,
        text.len(),
        cap / 1024,
        &text[tail_start..]
    )
}

pub(super) fn floor_char_boundary(s: &str, mut i: usize) -> usize {
    i = i.min(s.len());
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

pub(super) fn ceil_char_boundary(s: &str, mut i: usize) -> usize {
    i = i.min(s.len());
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

/// Env var enabling the lean bridge tool surface (same effect as `mesh.bridge_lean = true`).
pub(super) const BRIDGE_LEAN_ENV: &str = "FORGE_BRIDGE_LEAN";

/// Env var overriding bridge external MCP loading. `1` forces on, `0` forces off, unset defers to
/// `mesh.bridge_mcp_external` (on by default).
pub(super) const BRIDGE_MCP_EXTERNAL_ENV: &str = "FORGE_BRIDGE_MCP_EXTERNAL";

/// The `FORGE_BRIDGE_MCP_EXTERNAL` override: `Some(true/false)` when set to `1`/`0`, `None` when
/// unset (config decides). Split out so the pure gate is unit-testable without touching env.
pub(super) fn env_bridge_external_mcp() -> Option<bool> {
    match std::env::var(BRIDGE_MCP_EXTERNAL_ENV) {
        Ok(v) if v == "1" => Some(true),
        Ok(v) if v == "0" => Some(false),
        _ => None,
    }
}

/// Pure gate: the env override wins when present, otherwise the config flag decides. Kept separate
/// from [`env_bridge_external_mcp`] so it can be exercised without process-global env in tests.
pub(super) fn bridge_external_mcp_enabled_with(
    config_flag: bool,
    env_override: Option<bool>,
) -> bool {
    env_override.unwrap_or(config_flag)
}

/// Whether the CLI bridge should connect external project MCP servers (dual-graph/token-counter/
/// helm/…). ON by default: each server connects concurrently in the background under its bounded
/// connect/discovery timeout, so slow/auth-gated servers are skipped without wedging the turn.
/// Disable via `mesh.bridge_mcp_external = false` or `FORGE_BRIDGE_MCP_EXTERNAL=0` (env wins).
/// The bridged model keeps every Forge CORE tool either way.
pub(super) fn bridge_external_mcp_enabled(config: &Config) -> bool {
    bridge_external_mcp_enabled_with(config.mesh.bridge_mcp_external, env_bridge_external_mcp())
}

/// Tools dropped from the advertised list in lean mode: every tool schema/description is
/// re-ingested by the bridged CLI on every turn of ITS loop, so rarely-used surface is a
/// per-instance token tax. The core coding surface (read/write/edit/shell/search/…) stays.
pub(super) const LEAN_DROPPED_TOOLS: &[&str] = &[
    "web_fetch",
    "web_search",
    "spawn_agents",
    "send_to_agent",
    "remember",
    "present_plan",
    "manage_heartbeats",
];

/// Hard cap (bytes) on the `use_skill` description advertised to a bridged CLI. The full skill
/// catalog (name + description each) reaches several KiB on a skill-heavy machine and is resent
/// every bridge-loop turn.
pub(super) const BRIDGE_USE_SKILL_DESC_CAP: usize = 1536;

/// Names-only `use_skill` description for the bridge, truncated to [`BRIDGE_USE_SKILL_DESC_CAP`].
pub(super) fn bridge_use_skill_description(skills: &forge_skills::Catalog) -> String {
    let names: Vec<String> = skills.skill_listing().into_iter().map(|(n, _)| n).collect();
    names_only_skill_description(&names)
}

pub(super) fn names_only_skill_description(names: &[String]) -> String {
    let base = "Load a Forge skill's methodology into this turn, then follow it. These are \
                Forge's OWN skills — do NOT search the filesystem (~/.claude, ~/.codex) for \
                skills; call this tool with the exact skill name. Available: ";
    let mut out = base.to_string();
    for (i, name) in names.iter().enumerate() {
        let sep = if i == 0 { "" } else { ", " };
        let more = format!("… (+{} more)", names.len() - i);
        if out.len() + sep.len() + name.len() + more.len() > BRIDGE_USE_SKILL_DESC_CAP {
            out.push_str(&more);
            return out;
        }
        out.push_str(sep);
        out.push_str(name);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cap_bridge_result_passes_small_and_uncapped_tools_through() {
        let small = "x".repeat(1000);
        assert_eq!(cap_bridge_result("read_file", small.clone()), small);
        let big = "y".repeat(BRIDGE_READ_CAP_BYTES + 1000);
        assert_eq!(
            cap_bridge_result("write_file", big.clone()),
            big,
            "only read_file/shell are capped"
        );
    }

    #[test]
    fn cap_bridge_result_clamps_read_with_head_tail_and_marker() {
        let text = format!("HEAD{}TAIL", "m".repeat(BRIDGE_READ_CAP_BYTES * 2));
        let capped = cap_bridge_result("read_file", text.clone());
        assert!(capped.len() < text.len());
        assert!(
            capped.len() <= BRIDGE_READ_CAP_BYTES + 256,
            "cap + marker only"
        );
        assert!(capped.starts_with("HEAD"));
        assert!(capped.ends_with("TAIL"));
        assert!(capped.contains("omitted by the Forge bridge"));
        assert!(capped.contains("start_line/end_line"));
    }

    #[test]
    fn cap_bridge_result_clamps_shell_with_shell_advice() {
        let text = "z".repeat(BRIDGE_SHELL_CAP_BYTES * 3);
        let capped = cap_bridge_result("shell", text);
        assert!(capped.len() <= BRIDGE_SHELL_CAP_BYTES + 256);
        assert!(capped.contains("narrower command"));
    }

    #[test]
    fn cap_bridge_result_is_multibyte_safe() {
        let text = "é".repeat(BRIDGE_READ_CAP_BYTES); // 2 bytes each → over cap, odd boundaries
        let capped = cap_bridge_result("read_file", text);
        assert!(capped.contains("omitted by the Forge bridge"));
    }

    #[test]
    fn bridge_external_mcp_gate_defaults_on_env_overrides_both_ways() {
        // Default (config true, env unset) → ON.
        assert!(bridge_external_mcp_enabled_with(true, None));
        // Config opt-out with no env → OFF.
        assert!(!bridge_external_mcp_enabled_with(false, None));
        // Env wins in both directions, over either config value.
        assert!(!bridge_external_mcp_enabled_with(true, Some(false)));
        assert!(bridge_external_mcp_enabled_with(false, Some(true)));
    }

    #[test]
    fn names_only_skill_description_is_hard_capped() {
        let names: Vec<String> = (0..200)
            .map(|i| format!("some-long-skill-name-{i}"))
            .collect();
        let desc = names_only_skill_description(&names);
        assert!(
            desc.len() <= BRIDGE_USE_SKILL_DESC_CAP,
            "len={}",
            desc.len()
        );
        assert!(desc.contains("more)"), "truncation marker present");
        assert!(desc.contains("some-long-skill-name-0"));
        let few = vec!["rust".to_string(), "tdd".to_string()];
        let short = names_only_skill_description(&few);
        assert!(short.contains("rust, tdd"));
        assert!(!short.contains("more)"));
    }
}
