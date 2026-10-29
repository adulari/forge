import AsyncStorage from "@react-native-async-storage/async-storage";
import { useEffect, useState } from "react";

import type { Attachment } from "../components/chat/attach";

const draftKey = (sessionId: string) => `forge.draft.${sessionId}`;
const attachmentsKey = (sessionId: string) => `forge.draftAttachments.${sessionId}`;

const listeners = new Set<(sessionId: string, present: boolean) => void>();

export function getDraft(sessionId: string): Promise<string | null> {
  return AsyncStorage.getItem(draftKey(sessionId));
}

export async function setDraft(sessionId: string, text: string): Promise<void> {
  await AsyncStorage.setItem(draftKey(sessionId), text);
  for (const listener of listeners) listener(sessionId, text.trim().length > 0);
}

export async function clearDraft(sessionId: string): Promise<void> {
  await AsyncStorage.removeItem(draftKey(sessionId));
  for (const listener of listeners) listener(sessionId, false);
}

/** Whether `sessionId` has unsent text saved, kept current as drafts are written or cleared. */
export function useHasDraft(sessionId: string): boolean {
  const [present, setPresent] = useState(false);
  useEffect(() => {
    let live = true;
    getDraft(sessionId)
      .then((text) => live && setPresent((text ?? "").trim().length > 0))
      .catch(() => undefined);
    const listener = (id: string, next: boolean) => {
      if (id === sessionId) setPresent(next);
    };
    listeners.add(listener);
    return () => {
      live = false;
      listeners.delete(listener);
    };
  }, [sessionId]);
  return present;
}

/** Only an upload that finished can outlive the page: its bytes are on the host. A `blob:` URI
 * dies with the page, so it is dropped and the preview is rebuilt from the host's copy. */
export function persistableAttachments(attachments: readonly Attachment[]): Attachment[] {
  return attachments
    .filter((a) => a.state === "done" && a.path)
    .map((a) => ({
      id: a.id,
      name: a.name,
      image: a.image,
      state: "done" as const,
      path: a.path,
      ...(a.uri && !a.uri.startsWith("blob:") ? { uri: a.uri } : {}),
    }));
}

export function parseDraftAttachments(raw: string | null): Attachment[] {
  if (!raw) return [];
  try {
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed.flatMap((entry): Attachment[] => {
      const value = entry as Partial<Attachment> | null;
      if (!value || typeof value.id !== "string" || typeof value.name !== "string" || typeof value.path !== "string") return [];
      return [
        {
          id: value.id,
          name: value.name,
          image: value.image === true,
          state: "done",
          path: value.path,
          ...(typeof value.uri === "string" ? { uri: value.uri } : {}),
        },
      ];
    });
  } catch {
    return [];
  }
}

export async function getDraftAttachments(sessionId: string): Promise<Attachment[]> {
  return parseDraftAttachments(await AsyncStorage.getItem(attachmentsKey(sessionId)));
}

export function setDraftAttachments(sessionId: string, attachments: readonly Attachment[]): Promise<void> {
  const persist = persistableAttachments(attachments);
  if (persist.length === 0) return AsyncStorage.removeItem(attachmentsKey(sessionId));
  return AsyncStorage.setItem(attachmentsKey(sessionId), JSON.stringify(persist));
}

export function clearDraftAttachments(sessionId: string): Promise<void> {
  return AsyncStorage.removeItem(attachmentsKey(sessionId));
}
