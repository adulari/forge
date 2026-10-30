// The host refuses a WebSocket input this connection's scopes do not cover and says so in a
// `scope_denied:<scope>` note. The socket layer reports it here; ScopeDeniedToast shows it.
type Listener = (scope: string) => void;

const listeners = new Set<Listener>();

export function onScopeDenied(listener: Listener): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function reportInputError(note: unknown): void {
  if (typeof note !== "string" || !note.startsWith("scope_denied:")) return;
  const scope = note.slice("scope_denied:".length);
  for (const listener of listeners) listener(scope);
}

const ACTIONS: Record<string, string> = {
  prompt: "send prompts",
  approve: "approve or deny requests",
  terminal: "use the terminal",
  "workspace-write": "change files",
  "manage-providers": "change providers or configuration",
};

export function scopeDeniedMessage(scope: string): string {
  return `This connection can view but not ${ACTIONS[scope] ?? "do that"}.`;
}
