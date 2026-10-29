// Single source of truth for the slash-command set surfaced across the app — the Composer
// chip/autocomplete row + highlight, and the CommandPalette's searchable command list.
//
// `BUILTIN_COMMANDS` mirrors the daemon's static command registry
// (`crates/forge-tui/src/commands.rs` `COMMANDS`) plus the two built-in skill-commands the
// `forge-skills` catalog always injects (`/orchestrate`, `/rust` — see
// `crates/forge-skills/src/lib.rs::insert_builtin_commands`). Skills served by the daemon at
// `GET /api/skills` are merged in dynamically via `useSkillCommands()` so external/project
// skills autocomplete and highlight too.
import { useMemo } from "react";

import { BUILTIN_COMMANDS } from "./builtinCommands";
import { useSkills } from "./queries";

export { BUILTIN_COMMANDS, skillInvoked } from "./builtinCommands";

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
  return useMemo(
    () => (data ?? []).map((s: { name: string; description: string }) => ({ name: `/${s.name}`, description: s.description })),
    [data],
  );
}

/**
 * Does `token` (e.g. `/orchestrate` or `/agent-creator`) name a known command — either a
 * built-in or one of the currently-loaded skills? `skills` is the list of skill invocation
 * names (with leading `/`), typically derived from `useSkillCommands()`.
 */
export function isKnownCommand(token: string, skills: string[]): boolean {
  return BUILTIN_COMMANDS.includes(token) || skills.includes(token);
}
