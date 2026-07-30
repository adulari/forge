import { type GitBranchRow } from "../../lib/api";

export function gitWorktreePathTail(path: string): string {
  const normalized = path.replaceAll("\\", "/");
  const parts = normalized.split("/").filter(Boolean);
  return parts.slice(-3).join("/");
}

export function gitBranchSubtitle(row: GitBranchRow): string {
  const labels: string[] = [];
  if (row.current) labels.push("current");
  if (row.default) labels.push("default");
  if (row.remote) labels.push("remote");
  if (row.worktree) labels.push(`worktree · ${gitWorktreePathTail(row.worktree)}`);
  if (row.upstream && !row.remote) labels.push(`tracks ${row.upstream}`);
  return labels.length > 0 ? labels.join(" · ") : "local branch";
}

export function filterGitBranches(rows: GitBranchRow[], query: string): GitBranchRow[] {
  const normalized = query.trim().toLowerCase();
  if (!normalized) return rows;
  return rows.filter((row) => row.name.toLowerCase().includes(normalized));
}

export function canSelectGitBranch(
  row: GitBranchRow,
  blockedReason: string | null,
  actionBusy: boolean,
): boolean {
  return !row.current && row.worktree == null && blockedReason == null && !actionBusy;
}
