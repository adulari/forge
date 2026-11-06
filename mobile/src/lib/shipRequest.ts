// Opens the Ship sheet from anywhere (session menu, Review dock, command palette). The sheet's
// host registers once for as long as the app is mounted, like the Claude Code picker's.
let listener: ((sessionId: string) => void) | null = null;

export function onShipOpen(next: (sessionId: string) => void): () => void {
  listener = next;
  return () => {
    if (listener === next) listener = null;
  };
}

/** True when a host was there to show the sheet. */
export function openShip(sessionId: string): boolean {
  if (!listener) return false;
  listener(sessionId);
  return true;
}
