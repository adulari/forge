// What the desktop sidebar shows, as plain data: sessions grouped by project, forks nested under
// the session they came from, ordered by the chosen sort, with the per-project change totals.
// Pure so the grouping rules are tested without rendering anything.
import type { SessionRow } from "./api";
import type { ScopedRow } from "./fleetScope";
import type { SidebarSort } from "./shellPrefs";

export type ScopedSession = ScopedRow<SessionRow>;

export interface SessionNode {
  entry: ScopedSession;
  children: SessionNode[];
}

export interface ProjectGroup {
  /** `serverId\0name`: same-named projects on two servers are two checkouts. */
  key: string;
  /** Lowercase project name; what a workspace stores. */
  name: string;
  /** Directory a new thread in this project starts from. */
  root: string;
  serverId: string;
  nodes: SessionNode[];
  sessionCount: number;
  waitingCount: number;
  busyCount: number;
  failedCount: number;
  /** Lines changed across the project's distinct working directories (two sessions sharing a
   * checkout would otherwise count its diff twice). */
  added: number;
  removed: number;
}

export function projectName(cwd: string): string {
  const root = projectRoot(cwd).replace(/\/+$/, "");
  return (root.split("/").pop() || root || "~").toLowerCase();
}

/** The checkout a session's project lives in; a worktree session reports its repo, not the
 * throwaway worktree. */
export function projectRoot(cwd: string): string {
  const worktree = cwd.match(/^(.+?)\/\.forge\/worktrees\//i);
  return worktree ? worktree[1] : cwd;
}

function sortKey(row: SessionRow, sort: SidebarSort): number {
  return sort === "created" ? row.created_at : row.last_activity;
}

export function sortSessions(entries: readonly ScopedSession[], sort: SidebarSort): ScopedSession[] {
  return [...entries].sort((a, b) => {
    if (a.row.waiting !== b.row.waiting) return a.row.waiting ? -1 : 1;
    if (!!a.row.starred !== !!b.row.starred) return a.row.starred ? -1 : 1;
    return sortKey(b.row, sort) - sortKey(a.row, sort) || a.row.id.localeCompare(b.row.id);
  });
}

/** Nest each session under its parent when the parent is in the same list; a parent that is
 * missing, filtered out, or part of a cycle leaves the child at the top level. */
function nest(sorted: readonly ScopedSession[]): SessionNode[] {
  const byId = new Map(sorted.map((entry) => [entry.row.id, entry]));
  const nodes = new Map<string, SessionNode>(sorted.map((entry) => [entry.row.id, { entry, children: [] }]));
  const reaches = (start: string, target: string): boolean => {
    const seen = new Set<string>();
    let current: string | null | undefined = start;
    while (current && !seen.has(current)) {
      if (current === target) return true;
      seen.add(current);
      current = byId.get(current)?.row.parent_id;
    }
    return false;
  };
  const roots: SessionNode[] = [];
  for (const entry of sorted) {
    const node = nodes.get(entry.row.id) as SessionNode;
    const parentId = entry.row.parent_id;
    const parent = parentId && parentId !== entry.row.id ? nodes.get(parentId) : undefined;
    const sameServer = parent != null && parent.entry.serverId === entry.serverId;
    const cyclic = parentId != null && reaches(parentId, entry.row.id);
    if (parent && sameServer && !cyclic) parent.children.push(node);
    else roots.push(node);
  }
  return roots;
}

export function* walkNodes(nodes: readonly SessionNode[], depth = 0): Generator<{ node: SessionNode; depth: number }> {
  for (const node of nodes) {
    yield { node, depth };
    yield* walkNodes(node.children, depth + 1);
  }
}

export function buildProjectGroups(entries: readonly ScopedSession[], sort: SidebarSort): ProjectGroup[] {
  const sorted = sortSessions(entries, sort);
  const roots = nest(sorted);
  const groups = new Map<string, ProjectGroup>();
  const diffByCwd = new Map<string, { added: number; removed: number }>();
  const cwdsOfGroup = new Map<string, Set<string>>();
  for (const node of roots) {
    const name = projectName(node.entry.row.cwd);
    const key = `${node.entry.serverId}\u0000${name}`;
    let group = groups.get(key);
    if (!group) {
      group = { key, name, root: projectRoot(node.entry.row.cwd), serverId: node.entry.serverId, nodes: [], sessionCount: 0, waitingCount: 0, busyCount: 0, failedCount: 0, added: 0, removed: 0 };
      groups.set(key, group);
      cwdsOfGroup.set(key, new Set());
    }
    group.nodes.push(node);
    for (const { node: member } of walkNodes([node])) {
      group.sessionCount += 1;
      if (member.entry.row.waiting) group.waitingCount += 1;
      else if (member.entry.row.busy) group.busyCount += 1;
      else if (failedTurn(member.entry.row)) group.failedCount += 1;
      const { cwd, diff_added: added, diff_removed: removed } = member.entry.row;
      if (added != null || removed != null) {
        const cwdKey = `${member.entry.serverId}\u0000${cwd}`;
        diffByCwd.set(cwdKey, { added: added ?? 0, removed: removed ?? 0 });
        cwdsOfGroup.get(key)?.add(cwdKey);
      }
    }
  }
  for (const [key, group] of groups) {
    for (const cwdKey of cwdsOfGroup.get(key) ?? []) {
      const diff = diffByCwd.get(cwdKey);
      group.added += diff?.added ?? 0;
      group.removed += diff?.removed ?? 0;
    }
  }
  return [...groups.values()];
}

export function isDone(row: Pick<SessionRow, "done_at">): boolean {
  return row.done_at != null;
}

/** Done sessions leave the project groups for their own collapsed section. */
export function splitDone(entries: readonly ScopedSession[]): { active: ScopedSession[]; done: ScopedSession[] } {
  const active: ScopedSession[] = [];
  const done: ScopedSession[] = [];
  for (const entry of entries) (isDone(entry.row) ? done : active).push(entry);
  return { active, done };
}

export function filterByWorkspace(entries: readonly ScopedSession[], projects: ReadonlySet<string> | null): ScopedSession[] {
  if (!projects) return [...entries];
  return entries.filter((entry) => projects.has(projectName(entry.row.cwd)));
}

/** A stable swatch per project name, in lightness steps that read on either theme. */
export function projectAccent(name: string, scheme: "light" | "dark"): string {
  let hash = 0;
  for (let i = 0; i < name.length; i += 1) hash = (hash * 31 + name.charCodeAt(i)) >>> 0;
  return `hsl(${hash % 360}, 42%, ${scheme === "light" ? 42 : 62}%)`;
}

export function diffLabel(added: number | null | undefined, removed: number | null | undefined): string | null {
  const a = added ?? 0;
  const r = removed ?? 0;
  if (a === 0 && r === 0) return null;
  const parts = [`+${a}`];
  if (r > 0) parts.push(`-${r}`);
  return parts.join(" ");
}

/** A turn that ended badly on a session that is not busy or blocked, which nothing else on the row
 * would show. */
export function failedTurn(row: Pick<SessionRow, "busy" | "waiting" | "last_turn_outcome">): boolean {
  return !row.busy && !row.waiting && row.last_turn_outcome === "failed";
}

export function sessionStateWord(row: Pick<SessionRow, "busy" | "waiting" | "last_turn_outcome">): string {
  if (row.waiting) return "needs you";
  if (row.busy) return "working";
  return failedTurn(row) ? "last turn failed" : "idle";
}
