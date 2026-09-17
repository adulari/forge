import type { ProjectRow } from "./api";

export function lastProjectStorageKey(serverId: string): string {
  return `forge.lastProject.${serverId}`;
}

export function projectName(path: string): string {
  const trimmed = path.replace(/[\\/]+$/, "");
  return trimmed.split(/[\\/]/).pop() || path;
}

export function isLoopbackServer(baseUrl: string | null): boolean {
  if (!baseUrl) return false;
  try {
    const host = new URL(baseUrl).hostname;
    return host === "localhost" || host === "127.0.0.1" || host === "::1";
  } catch {
    return false;
  }
}

/** `/api/projects` never includes `default_cwd` in `recent` (the daemon dedupes it out — see
 * serve_projects.rs), but `roots` always carries it as its first entry (the daemon resolves its
 * own cwd into the root list before any configured ones) — so that's where its real
 * `is_git_repo` lives. Falls back to `false`, not `true`: a daemon's launch directory is
 * frequently a plain home/workspace folder, not a git repo, and defaulting to "yes" is exactly
 * the wrong direction (it made "Isolated git worktree" default ON for a non-git project). */
export function projectChoices(
  defaultCwd: string,
  recent: readonly ProjectRow[],
  roots: readonly ProjectRow[] = [],
): ProjectRow[] {
  const defaultRow = roots.find((root) => root.path === defaultCwd)
    ?? { path: defaultCwd, name: projectName(defaultCwd), is_git_repo: false, last_activity: null };
  const seen = new Set<string>();
  return [defaultRow, ...recent].filter((project) => {
    if (seen.has(project.path)) return false;
    seen.add(project.path);
    return true;
  });
}

/** Whether `cwd` is a known git repository per the project catalog — `null` when it isn't one of
 * the rows the daemon told us about (a manually-typed path, or a path from a segment of `browse`
 * results the caller didn't thread through) and therefore genuinely unknown. Callers that need a
 * safe default for "should worktree be offered" should treat `null` and `true` the same way and
 * only act on an explicit `false`. */
export function isKnownGitRepo(
  cwd: string,
  catalog: { recent: readonly ProjectRow[]; roots: readonly ProjectRow[] } | null | undefined,
): boolean | null {
  if (!catalog || !cwd) return null;
  const match = [...catalog.roots, ...catalog.recent].find((row) => row.path === cwd);
  return match ? match.is_git_repo : null;
}

/** Remembers, at most, the one project the user explicitly turned "Isolated git worktree" off
 * for while composing a task (new-session.tsx) — `null` once cleared by turning it back on, or
 * if it never happened. Needed to tell that apart from the toggle auto-disabling itself because
 * the currently selected project isn't a git repo, which must NOT stick to a project the user
 * never touched the switch for. */
export interface WorktreePreference {
  disabledForCwd: string | null;
}

export const DEFAULT_WORKTREE_PREFERENCE: WorktreePreference = { disabledForCwd: null };

/** The toggle's value for `cwd` right now: off with no git repo underneath (full stop, matching
 * `isKnownGitRepo`'s "unknown treated as yes" contract — `null` and `true` both recommend on),
 * else on UNLESS the user explicitly turned it off for this exact project. Without tracking the
 * project the override belongs to, switching from a non-git project (auto-off) to a git one left
 * the toggle off — the auto-disable was mistaken for a standing user choice. */
export function recommendedWorktree(
  cwd: string,
  cwdIsGitRepo: boolean | null,
  pref: WorktreePreference,
): boolean {
  if (cwdIsGitRepo === false) return false;
  return pref.disabledForCwd !== cwd;
}

/** Records (or clears) the user's explicit choice after they flip the switch by hand. */
export function withWorktreeToggled(pref: WorktreePreference, cwd: string, value: boolean): WorktreePreference {
  return { disabledForCwd: value ? null : cwd };
}
