// Opens the Local CI sheet from anywhere (session menu, command palette, the blocked banner).
// The sheet's host registers once for as long as the app is mounted, like the Ship sheet's.
let listener: ((sessionId: string) => void) | null = null;

export function onCiOpen(next: (sessionId: string) => void): () => void {
  listener = next;
  return () => {
    if (listener === next) listener = null;
  };
}

/** True when a host was there to show the sheet. */
export function openCi(sessionId: string): boolean {
  if (!listener) return false;
  listener(sessionId);
  return true;
}
