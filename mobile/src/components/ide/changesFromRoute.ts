// `GET /api/projects/changes` → the queue's `SessionChanges`. Pure so the mapping (real content
// hashes, the synthetic `unattributed` group) is tested without a daemon.
import type { ProjectChangesResponse, SessionRow } from "../../lib/api";
import type { IdeChangedFile, SessionChanges } from "./changesAdapter";

type RouteFile = ProjectChangesResponse["sessions"][number]["files"][number];

const STATUS: Record<RouteFile["kind"], string> = { created: "A", modified: "M", deleted: "D", renamed: "R" };

export function fileFromRoute(file: RouteFile, worktree: boolean): IdeChangedFile {
  return {
    path: file.path,
    status: STATUS[file.kind] ?? "M",
    adds: file.adds,
    dels: file.dels,
    bucket: worktree ? "worktree" : "unstaged",
    // The daemon hashes the file's current content: any edit, including one that keeps the +/-
    // totals, changes it.
    hash: file.content_hash,
  };
}

/**
 * Per session, its changed files. `unattributed` files (the tree changed, no recorded agent
 * edit) go to the shared-tree session that was active most recently, since a diff tab needs a
 * real session to ask; a path an agent already owns is not listed twice.
 */
export function sessionChangesFromRoute(
  response: ProjectChangesResponse,
  sessions: readonly SessionRow[],
): Map<string, SessionChanges> {
  const out = new Map<string, SessionChanges>();
  for (const row of sessions) {
    out.set(row.id, { sessionId: row.id, source: row.worktree ? "worktree" : "tree", files: [], sharedWith: null });
  }
  const orphans: RouteFile[] = [];
  for (const group of response.sessions) {
    const target = out.get(group.session_id);
    if (!target) {
      if (group.session_id === "unattributed") orphans.push(...group.files);
      continue;
    }
    target.files = group.files.map((file) => fileFromRoute(file, target.source === "worktree"));
  }
  if (orphans.length > 0) {
    const holder = [...sessions].filter((row) => !row.worktree).sort((a, b) => b.last_activity - a.last_activity)[0];
    const target = holder ? out.get(holder.id) : undefined;
    if (target) {
      const owned = new Set([...out.values()].flatMap((changes) => changes.files.map((file) => file.path)));
      target.files = [...target.files, ...orphans.filter((file) => !owned.has(file.path)).map((file) => fileFromRoute(file, false))];
    }
  }
  return out;
}

/** A daemon that predates the route answers 404 (or 405); anything else is a real failure. */
export function routeMissing(error: unknown): boolean {
  const status = (error as { status?: unknown } | null)?.status;
  return status === 404 || status === 405;
}

/** The directory to ask the route about: a shared-tree session's cwd is the project root. */
export function changesCwd(sessions: readonly SessionRow[]): string | null {
  const shared = sessions.find((row) => !row.worktree);
  return (shared ?? sessions[0])?.cwd ?? null;
}
