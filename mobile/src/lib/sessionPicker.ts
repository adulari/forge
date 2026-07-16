import type { SessionRow } from "./api";

export type SessionPickerState = "loading" | "error" | "empty" | "ready";

export function filterSessions(
  sessions: readonly SessionRow[],
  search: string,
  needsYouOnly = false,
): SessionRow[] {
  const needle = search.trim().toLocaleLowerCase();
  return sessions.filter((row) => {
    if (needsYouOnly && !row.waiting) return false;
    if (!needle) return true;
    const status = row.waiting ? "waiting" : row.busy ? "busy" : "idle";
    return [row.title, row.cwd, status].some((value) => value.toLocaleLowerCase().includes(needle));
  });
}

export function sessionPickerState({
  isLoading,
  isError,
  visibleCount,
}: {
  isLoading: boolean;
  isError: boolean;
  visibleCount: number;
}): SessionPickerState {
  if (isLoading) return "loading";
  if (isError) return "error";
  return visibleCount === 0 ? "empty" : "ready";
}

export function isOfflineError(error: unknown): boolean {
  return error instanceof TypeError || (error instanceof Error && /network|fetch|offline|connect/i.test(error.message));
}
