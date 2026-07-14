// Single source of truth for the slash-command set surfaced across the app — the Composer
// chip/autocomplete row + highlight, and the CommandPalette's searchable command list.
//
// `BUILTIN_COMMANDS` mirrors the daemon's static command registry
// (`crates/forge-tui/src/commands.rs` `COMMANDS`) plus the two built-in skill-commands the
// `forge-skills` catalog always injects (`/orchestrate`, `/rust` — see
// `crates/forge-skills/src/lib.rs::insert_builtin_commands`). Skills served by the daemon at
// `GET /api/skills` are merged in dynamically via `useSkillCommands()` so external/project
// skills autocomplete and highlight too.
import { useSkills } from "./queries";

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
  "/clear",
  "/undo",
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

export interface SkillCommand {
  /** The slash-invocation form, e.g. `/agent-creator`. */
  name: string;
  /** The skill's human-readable description (from `GET /api/skills`). */
  description: string;
}

/**
 * React hook returning the daemon-served skill catalog as `/<name>` command entries. Reuses
 * the existing `useSkills()` query over `GET /api/skills` (see `lib/queries.ts`), which already
 * handles the baseUrl'd fetch, caching, and offline/error states. Returns `[]` while loading
 * or when the endpoint is unreachable (offline → no skill commands, no crash).
 */
export function useSkillCommands(): SkillCommand[] {
  const { data } = useSkills();
  return (data ?? []).map((s: { name: string; description: string }) => ({ name: `/${s.name}`, description: s.description }));
}

/**
 * Does `token` (e.g. `/orchestrate` or `/agent-creator`) name a known command — either a
 * built-in or one of the currently-loaded skills? `skills` is the list of skill invocation
 * names (with leading `/`), typically derived from `useSkillCommands()`.
 */
export function isKnownCommand(token: string, skills: string[]): boolean {
  return BUILTIN_COMMANDS.includes(token) || skills.includes(token);
}
