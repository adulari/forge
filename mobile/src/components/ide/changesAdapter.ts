// Turns what the daemon already serves into the queue's `ChangedFile`s. This is the seam for
// `GET /api/projects/changes?cwd=` (one call, per-session files with a real content hash): when
// that route lands, only `useProjectChanges` changes and the rest of the shell keeps reading
// `SessionChanges`.
import type { GitDiffFile, GitStatusResponse, SessionRow } from "../../lib/api";
import { projectName } from "../../lib/sidebarModel";
import { fingerprint, type ChangedFile } from "./changeQueueModel";

export type ChangeBucket = "staged" | "unstaged" | "untracked" | "worktree";

export interface IdeChangedFile extends ChangedFile {
  bucket: ChangeBucket;
}

export interface SessionChanges {
  sessionId: string;
  /** How the files were found: a worktree session's own diff, or the working tree it shares. */
  source: "worktree" | "tree";
  files: IdeChangedFile[];
  /** Another session is the one asked about this working tree. */
  sharedWith: string | null;
}

const STATUS_LETTER: Record<GitDiffFile["kind"], string> = { created: "A", modified: "M", deleted: "D", renamed: "R" };

/** A worktree session's diff carries the hunks, so the hash is the real content. */
export function filesFromSessionDiff(files: readonly GitDiffFile[]): IdeChangedFile[] {
  return files.map((file) => ({
    path: file.path,
    status: STATUS_LETTER[file.kind] ?? "M",
    adds: file.adds,
    dels: file.dels,
    bucket: "worktree",
    hash: fingerprint([
      file.kind,
      ...file.hunks.flatMap((hunk) => [hunk.header, ...hunk.lines]),
      String(file.skipped_lines),
    ]),
  }));
}

/**
 * A shared working tree only reports per-file counts, so its hash is `status + counts`: an edit
 * that keeps the same +/- totals would not clear a reviewed mark. The real content hash arrives
 * with the project-changes route; until then the diff tab is the safety net.
 */
export function filesFromGitStatus(status: GitStatusResponse): IdeChangedFile[] {
  const conflicted = new Set((status.conflicted ?? []).map((row) => row.path));
  const out: IdeChangedFile[] = [];
  const seen = new Set<string>();
  const add = (rows: GitStatusResponse["staged"], bucket: ChangeBucket) => {
    for (const row of rows) {
      // A path edited after staging sits in two buckets; the queue lists it once.
      if (seen.has(row.path)) continue;
      seen.add(row.path);
      out.push({
        path: row.path,
        status: row.status,
        adds: row.adds,
        dels: row.dels,
        bucket,
        conflicted: conflicted.has(row.path),
        hash: fingerprint([row.status, String(row.adds), String(row.dels), bucket]),
      });
    }
  };
  add(status.conflicted ?? [], "unstaged");
  add(status.unstaged, "unstaged");
  add(status.staged, "staged");
  add(status.untracked, "untracked");
  return out;
}

/** The sessions of one project, needs-you first, then busy, then the most recent. */
export function projectSessions(rows: readonly SessionRow[] | undefined, project: string | null): SessionRow[] {
  if (!rows || project == null) return [];
  return rows
    .filter((row) => projectName(row.cwd) === project)
    .sort(
      (a, b) =>
        Number(b.waiting) - Number(a.waiting) ||
        Number(b.busy) - Number(a.busy) ||
        Number(a.done_at != null) - Number(b.done_at != null) ||
        b.last_activity - a.last_activity,
    );
}

/**
 * Which project IDE mode opens on: the one the routed session belongs to, else the project with
 * someone waiting, else the one with the most recent activity.
 */
export function defaultProject(rows: readonly SessionRow[] | undefined, routedSessionId: string | null): string | null {
  if (!rows || rows.length === 0) return null;
  const routed = routedSessionId ? rows.find((row) => row.id === routedSessionId) : undefined;
  if (routed) return projectName(routed.cwd);
  const waiting = rows.filter((row) => row.waiting).sort((a, b) => b.last_activity - a.last_activity)[0];
  if (waiting) return projectName(waiting.cwd);
  const latest = [...rows].sort((a, b) => b.last_activity - a.last_activity)[0];
  return projectName(latest.cwd);
}

/**
 * Sessions that share a checkout ask git about it once. The representative is the most recently
 * active one; the rest are told who holds the shared tree so the queue never lists one file under
 * four agents.
 */
export function planChangeQueries(sessions: readonly SessionRow[]): { row: SessionRow; sharedWith: string | null }[] {
  const holderByDir = new Map<string, SessionRow>();
  for (const row of sessions) {
    if (row.worktree) continue;
    const held = holderByDir.get(row.cwd);
    if (!held || row.last_activity > held.last_activity) holderByDir.set(row.cwd, row);
  }
  return sessions.map((row) => {
    if (row.worktree) return { row, sharedWith: null };
    const holder = holderByDir.get(row.cwd);
    return { row, sharedWith: holder && holder.id !== row.id ? holder.id : null };
  });
}
