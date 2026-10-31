// Where a session worktree lives decides how it groups under its repository. Two shapes:
//   project root  <repo>/.forge/worktrees/<name>
//   global root   <data dir>/worktrees/<repo>-<8 hex>/<name>      (`[worktree] root = "global"`)
// A global checkout lives outside the repository, so only the repo's folder NAME survives in the
// path; its real root is unknowable from the path alone.

export interface WorktreeLocation {
  repoName: string;
  /** Absolute repo root; null for a global checkout, where the path does not carry it. */
  repoRoot: string | null;
  name: string;
}

const PROJECT_SHAPE = /^(.+?)\/\.forge\/worktrees\/([^/]+)/i;
const GLOBAL_SHAPE = /^(?:.*)\/worktrees\/([^/]+)-[0-9a-f]{8}\/([^/]+)/i;

export function parseWorktreePath(cwd: string): WorktreeLocation | null {
  const normalized = cwd.replace(/\\/g, "/");
  const project = normalized.match(PROJECT_SHAPE);
  if (project) {
    const root = project[1].replace(/\/+$/, "");
    return { repoName: root.split("/").pop() || root, repoRoot: project[1], name: project[2] };
  }
  const global = normalized.match(GLOBAL_SHAPE);
  if (global) return { repoName: global[1], repoRoot: null, name: global[2] };
  return null;
}

/** The repository a session path belongs to, or the path itself when it is no worktree (or a
 * global one whose repo root cannot be recovered). */
export function repoRootOf(cwd: string): string {
  const location = parseWorktreePath(cwd);
  return location?.repoRoot ?? cwd;
}

/** `repo · wt <name>`; machine ids are shortened to 8 characters, friendly names are kept. */
export function worktreeLabel(location: WorktreeLocation): string {
  const short = /^[a-f0-9-]{9,}$/i.test(location.name) ? location.name.slice(0, 8) : location.name;
  return `${location.repoName} · wt ${short}`;
}

/** The project to remember after a session starts. A global-root worktree hides its repo's
 * path, so the folder the user asked for wins over the session's own cwd there. */
export function rememberableProject(sessionCwd: string, requestedCwd: string): string {
  const location = parseWorktreePath(sessionCwd);
  if (location && location.repoRoot === null && requestedCwd.trim()) return repoRootOf(requestedCwd.trim());
  return repoRootOf(sessionCwd);
}
