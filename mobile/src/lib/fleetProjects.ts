// The Fleet's project filter: which projects the visible sessions belong to, and the filter
// itself. Pure, so the chip row and the list agree on what a project is.
import type { SessionRow } from "./api";
import { projectName } from "./sidebarModel";

export interface ProjectChip {
  name: string;
  count: number;
  waiting: number;
}

export function projectChips(rows: readonly Pick<SessionRow, "cwd" | "waiting">[]): ProjectChip[] {
  const byName = new Map<string, ProjectChip>();
  for (const row of rows) {
    const name = projectName(row.cwd);
    const chip = byName.get(name) ?? { name, count: 0, waiting: 0 };
    chip.count += 1;
    if (row.waiting) chip.waiting += 1;
    byName.set(name, chip);
  }
  return [...byName.values()].sort((a, b) => b.count - a.count || a.name.localeCompare(b.name));
}

export function filterByProject<R extends Pick<SessionRow, "cwd">>(rows: readonly R[], project: string | null): R[] {
  return project ? rows.filter((row) => projectName(row.cwd) === project) : [...rows];
}
