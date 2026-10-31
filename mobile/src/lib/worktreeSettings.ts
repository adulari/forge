// Pure helpers for the `[worktree]` settings editor. The config API carries every value as a
// string, so lists travel as JSON text.

export type WorktreeRootSetting = "project" | "global";

export const COPY_IGNORED_SUGGESTIONS = [".env*", ".vscode"] as const;

export function parseGlobList(value: string): string[] {
  try {
    const parsed: unknown = JSON.parse(value);
    return Array.isArray(parsed) ? parsed.filter((item): item is string => typeof item === "string") : [];
  } catch {
    return [];
  }
}

export function serializeGlobList(globs: readonly string[]): string {
  return JSON.stringify(globs);
}

/** Adds a pattern if it is non-empty and new; returns the same array otherwise. */
export function withGlob(globs: readonly string[], raw: string): string[] {
  const glob = raw.trim();
  if (!glob || globs.includes(glob)) return [...globs];
  return [...globs, glob];
}

export function withoutGlob(globs: readonly string[], index: number): string[] {
  return globs.filter((_, i) => i !== index);
}

/** Patterns are repo-relative: an absolute path or a parent escape would never match. */
export function globProblem(raw: string): string | null {
  const glob = raw.trim();
  if (!glob) return null;
  if (glob.startsWith("/") || /^[a-z]:[\\/]/i.test(glob)) return "Use a path relative to the repository, like .env*.";
  if (glob.split(/[\\/]/).includes("..")) return "Patterns cannot leave the repository.";
  return null;
}

export function normalizeRoot(value: string): WorktreeRootSetting {
  return value.trim().toLowerCase() === "global" ? "global" : "project";
}

/** A blank command clears the setting so the daemon falls back to "no command". */
export function commandToSave(draft: string): string | undefined {
  const command = draft.trim();
  return command ? command : undefined;
}
