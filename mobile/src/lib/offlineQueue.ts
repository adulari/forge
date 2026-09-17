import type { RemoteInput } from "./ws";

export const OFFLINE_QUEUE_CAP = 20;

const OFFLINE_QUEUE_PREFIX = "forge.offlineQueue";

export interface QueuedPrompt {
  text: string;
  attachments: { path: string; image: boolean }[];
}

/** Shared AsyncStorage key so a prompt queued by one screen (e.g. new-session.tsx seeding a
 * brand-new session's first prompt) is found by the chat screen that later mounts for that
 * same `baseUrl`+`sessionId` pair and flushes it on the next socket-open edge. */
export function offlineQueueKey(baseUrl: string | null, sessionId: string): string {
  return `${OFFLINE_QUEUE_PREFIX}:${baseUrl ?? "unknown"}:${sessionId}`;
}

/** The task-composer's free text becomes the new session's first turn — queued the same way an
 * offline prompt is, so it survives the gap between "session created" and "its socket opened".
 * Empty/whitespace-only text queues nothing (a session with no prompt is a normal blank chat). */
export function buildInitialPromptQueue(text: string): QueuedPrompt[] {
  const trimmed = text.trim();
  return trimmed ? [{ text: trimmed, attachments: [] }] : [];
}

/** Parse both current queue records and the legacy string-only representation. */
export function parseOfflineQueue(raw: string | null): QueuedPrompt[] {
  if (!raw) return [];
  try {
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed.map((entry) => {
      if (typeof entry === "string") return { text: entry, attachments: [] };
      const value = entry as Partial<QueuedPrompt>;
      return {
        text: typeof value.text === "string" ? value.text : "",
        attachments: Array.isArray(value.attachments) ? value.attachments : [],
      };
    });
  } catch {
    return [];
  }
}

/** Convert stored prompts to wire inputs without changing FIFO ordering. */
export function queuedPromptInputs(queue: readonly QueuedPrompt[]): RemoteInput[] {
  return queue.map(({ text, attachments }) => ({ kind: "prompt", text, attachments }));
}
