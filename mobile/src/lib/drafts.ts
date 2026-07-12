import AsyncStorage from "@react-native-async-storage/async-storage";

const draftKey = (sessionId: string) => `forge.draft.${sessionId}`;

export function getDraft(sessionId: string): Promise<string | null> {
  return AsyncStorage.getItem(draftKey(sessionId));
}

export function setDraft(sessionId: string, text: string): Promise<void> {
  return AsyncStorage.setItem(draftKey(sessionId), text);
}

export function clearDraft(sessionId: string): Promise<void> {
  return AsyncStorage.removeItem(draftKey(sessionId));
}
