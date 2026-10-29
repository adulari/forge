import { Linking, Platform } from "react-native";

export const MIC_DENIED_MESSAGE = "Microphone access is off";

export class MicDeniedError extends Error {
  constructor() {
    super(MIC_DENIED_MESSAGE);
    this.name = "MicDeniedError";
  }
}

const DENIED_DOM_ERRORS = new Set(["NotAllowedError", "PermissionDeniedError", "SecurityError"]);

export function isMicDenied(err: unknown): boolean {
  if (err instanceof MicDeniedError) return true;
  const name = (err as { name?: unknown } | null)?.name;
  return typeof name === "string" && DENIED_DOM_ERRORS.has(name);
}

/** A browser or webview has no API to reopen its permission prompt, so the web path explains
 * where the switch lives instead of offering a button that cannot work. */
export const MIC_WEB_INSTRUCTIONS = "Allow the microphone for this site in your browser or system settings, then try again.";

export function canOpenMicSettings(): boolean {
  return Platform.OS !== "web";
}

export async function openMicSettings(): Promise<void> {
  if (!canOpenMicSettings()) return;
  try {
    await Linking.openSettings();
  } catch {
    // the OS refused to open settings; the message above already says what to do
  }
}
