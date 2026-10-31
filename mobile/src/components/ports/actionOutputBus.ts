// Callers that start an action (sidebar menu, palette, Ports dock) live in different trees, and
// only the shell inside the workbench can open a terminal tab. They publish here; the shell's
// workbench commands subscribe.
export interface OpenActionOutput {
  sessionId: string;
  terminalId: string;
  title: string;
}

type Listener = (request: OpenActionOutput) => void;

const listeners = new Set<Listener>();

export function subscribeOpenActionOutput(listener: Listener): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** Returns false when nothing is listening (a phone has no workbench), so the caller can navigate instead. */
export function requestOpenActionOutput(request: OpenActionOutput): boolean {
  for (const listener of listeners) listener(request);
  return listeners.size > 0;
}
