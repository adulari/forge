// The daemon's static slash-command set and skill-invocation detection: pure data and string
// logic, kept apart from commands.ts so it loads without the query layer.

/**
 * The complete list of built-in slash commands the daemon accepts, each prefixed with `/`.
 * Derived from the daemon's `COMMANDS` registry (`crates/forge-tui/src/commands.rs`) plus the
 * two always-injected built-in skill-commands (`/orchestrate`, `/rust`). De-duplicated, with
 * the most frequently used ones surfaced first (mirrors the registry's `/help` display order).
 */
export const BUILTIN_COMMANDS: string[] = [
  // Session / model basics (most frequent first)
  "/help",
  "/keys",
  "/mode",
  "/model",
  "/models",
  "/new",
  "/resume",
  "/sessions",
  "/config",
  "/compact",
  "/uncompact",
  "/commit",
  "/clear",
  "/undo",
  "/rewind",
  "/checkpoint",
  "/checkpoints",
  "/usage",
  // Authoring / planning
  "/plan",
  "/execute",
  "/goal",
  "/pr",
  "/loop",
  "/effort",
  "/subagents",
  "/delegation",
  "/crew",
  "/remember",
  "/memories",
  // Inspection / tooling
  "/assay",
  "/lattice",
  "/mcp",
  "/mesh",
  "/remote",
  "/self-mcp",
  "/thinking",
  "/image",
  "/init",
  "/copy",
  "/replay",
  "/btw",
  "/side",
  "/duel",
  "/workflow",
  "/voice",
  "/statusline",
  "/quit",
  // Built-in skill-commands injected by the forge-skills catalog (always present unless a
  // user/project command of the same name overrides them — the bare invocation still resolves
  // to the builtin, so they belong in the known set).
  "/orchestrate",
  "/rust",
];

/**
 * The skill a message invoked. The daemon stores a skill's prompt expanded, not the `/name` the
 * person typed, so besides a leading `/name` of a loaded skill (what a just-sent message still
 * says) this recognises the expansion the built-in skill-commands produce
 * ("Apply the **rust-best-practices** skill to this task"). A built-in command of the same name
 * as a skill wins, so only the two built-in skill-commands count by their slash form.
 */
export function skillInvoked(text: string, skills: readonly string[]): string | null {
  const expanded = text.match(/^Apply the \*\*([\w.-]+)\*\* skill to this task\b/)?.[1];
  if (expanded) return `/${expanded}`;
  const token = text.match(/^\/[^\s]+/)?.[0].toLowerCase();
  if (!token || !skills.includes(token)) return null;
  const builtinNonSkill = BUILTIN_COMMANDS.includes(token) && token !== "/orchestrate" && token !== "/rust";
  return builtinNonSkill ? null : token;
}
