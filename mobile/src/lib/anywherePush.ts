import type { AnywherePushApi, AnywherePushStatus } from "./anywherePushCore";

export type { AnywherePushStatus } from "./anywherePushCore";

export async function getAnywherePushStatus(): Promise<AnywherePushStatus> {
  return "unsupported";
}

export async function enableAnywherePush(_api: AnywherePushApi): Promise<AnywherePushStatus> {
  return "unsupported";
}

export async function disableAnywherePush(_api: AnywherePushApi): Promise<AnywherePushStatus> {
  return "unsupported";
}

export function observeAnywherePushRefresh(_onRefresh: () => void): () => void {
  return () => undefined;
}

export async function clearAnywherePushState(): Promise<void> {
  // Native-only SecureStore state does not exist on this platform.
}
