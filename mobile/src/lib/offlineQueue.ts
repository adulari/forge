import type { RemoteInput } from "./ws";

export const OFFLINE_QUEUE_CAP = 20;

export interface QueuedPrompt {
  text: string;
  attachments: { path: string; image: boolean }[];
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
