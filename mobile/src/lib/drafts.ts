import AsyncStorage from "@react-native-async-storage/async-storage";
import { useEffect, useState } from "react";

const draftKey = (sessionId: string) => `forge.draft.${sessionId}`;

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
