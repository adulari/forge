import { listTerminalSessions, openTerminalSocket } from "./api";

/** The terminal that should receive typed-in text: the first one still running. */
export function pickTerminalId(
  terminals: readonly { terminal_id: string; status: "running" | "exited" }[],
): string | null {
  return terminals.find((terminal) => terminal.status === "running")?.terminal_id ?? null;
}

/**
 * Type `text` into the session's running terminal without pressing Enter, so the person reviews
 * it first. Resolves false when no terminal is running.
 */
export async function insertIntoTerminal(baseUrl: string, sessionId: string, text: string): Promise<boolean> {
  const terminalId = pickTerminalId(await listTerminalSessions(baseUrl, sessionId));
  if (!terminalId) return false;
  return new Promise<boolean>((resolve) => {
    let settled = false;
    const finish = (value: boolean) => {
      if (settled) return;
      settled = true;
      setTimeout(() => socket.close(), 150);
      resolve(value);
    };
    const socket = openTerminalSocket(
      baseUrl,
      sessionId,
      {
        onOutput: () => undefined,
        onOpen: () => finish(socket.send(text)),
        onError: () => finish(false),
        onClose: () => finish(false),
      },
      { terminalId },
    );
  });
}
