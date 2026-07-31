export const MAX_INCOMING_SHARE_CHARS = 65_536;

export interface IncomingShareDraft {
  id: string;
  text: string;
  createdAt: number;
}

export interface IncomingSharePayload {
  shareType: "text" | "url" | "audio" | "image" | "video" | "file";
  value: string;
  mimeType?: string;
}

export function decodeIncomingShare(raw: string | null): IncomingShareDraft | null {
  if (!raw) return null;
  try {
    const value = JSON.parse(raw) as Partial<IncomingShareDraft>;
    if (
      typeof value.id !== "string"
      || typeof value.text !== "string"
      || !value.text.trim()
      || value.text.length > MAX_INCOMING_SHARE_CHARS
      || typeof value.createdAt !== "number"
      || !Number.isFinite(value.createdAt)
    ) {
      return null;
    }
    return { id: value.id, text: value.text, createdAt: value.createdAt };
  } catch {
    return null;
  }
}

export function textFromSharedPayloads(
  payloads: readonly IncomingSharePayload[],
): string | null {
  const parts = payloads
    .filter((payload) => payload.shareType === "text" || payload.shareType === "url")
    .map((payload) => payload.value.trim())
    .filter(Boolean);
  if (parts.length === 0) return null;
  const text = parts.join("\n\n");
  return text.length <= MAX_INCOMING_SHARE_CHARS ? text : null;
}

export function appendIncomingShareText(current: string, shared: string): string {
  const existing = current.trimEnd();
  return existing.trim().length > 0 ? `${existing}\n\n${shared}` : shared;
}
