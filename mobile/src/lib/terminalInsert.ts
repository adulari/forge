import { listTerminalSessions, openTerminalSocket } from "./api";

export const MAX_INSERT_CHARS = 240;

/**
 * A preview page chooses its elements' selectors, so the text is untrusted. Every control
 * character (C0 including CR, LF, ESC and Ctrl-C, DEL, C1) is dropped, the length is capped, and
 * the rest is wrapped as a single-quoted shell literal: even if Enter is pressed the shell sees
 * one inert word, never a command.
 */
export function sanitizeInsertText(raw: string): string {
  const clean = raw.replace(/[\u0000-\u001f\u007f-\u009f\u2028\u2029]/g, " ").replace(/\s+/g, " ").trim();
  const capped = clean.length > MAX_INSERT_CHARS ? clean.slice(0, MAX_INSERT_CHARS) : clean;
  return `'${capped.replace(/'/g, `'\\''`)}'`;
}

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
  const safe = sanitizeInsertText(text);
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
        onOpen: () => finish(socket.send(safe)),
        onError: () => finish(false),
        onClose: () => finish(false),
      },
      { terminalId },
    );
  });
}
