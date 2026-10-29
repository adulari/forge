// Terminal tabs the person closed a moment ago. The daemon forgets a terminal when it is told to
// close, but the next list refresh can still arrive first; without this the tab would be adopted
// straight back.
const RECENT_MS = 15_000;
const closedAt = new Map<string, number>();

function keyOf(sessionId: string, terminalId: string): string {
  return `${sessionId}\u0000${terminalId}`;
}

export function markTerminalClosed(sessionId: string, terminalId: string, now = Date.now()): void {
  closedAt.set(keyOf(sessionId, terminalId), now);
}

export function wasTerminalClosedRecently(sessionId: string, terminalId: string, now = Date.now()): boolean {
  const at = closedAt.get(keyOf(sessionId, terminalId));
  if (at == null) return false;
  if (now - at > RECENT_MS) {
    closedAt.delete(keyOf(sessionId, terminalId));
    return false;
  }
  return true;
}

/** The terminal a tab stands for: the session-less default tab is the session's first one. */
export function tabTerminalId(resourceId: string | null): string {
  return resourceId ?? "term-1";
}

interface TabLike {
  kind: string;
  sessionId: string | null;
  resourceId: string | null;
}

/** Terminals the daemon lists for `sessionId` that have no tab yet and were not just closed. */
export function terminalsToAdopt(
  tabs: readonly TabLike[],
  sessionId: string,
  daemon: readonly { terminal_id: string; status: "running" | "exited"; exit_code?: number | null }[],
  now = Date.now(),
): string[] {
  const shown = new Set(
    tabs
      .filter((tab) => tab.kind === "terminal" && (tab.sessionId == null || tab.sessionId === sessionId))
      .map((tab) => tabTerminalId(tab.resourceId)),
  );
  return daemon
    .filter((terminal) => !(terminal.status === "exited" && terminal.exit_code === 0))
    .map((terminal) => terminal.terminal_id)
    .filter((id) => !shown.has(id) && !wasTerminalClosedRecently(sessionId, id, now));
}

/** Tabs to show for the session in focus: its own terminals plus the session-less default tab. */
export function tabsForSession<T extends TabLike>(tabs: readonly T[], sessionId: string | null): T[] {
  return tabs.filter((tab) => tab.kind !== "terminal" || tab.sessionId == null || tab.sessionId === sessionId);
}
