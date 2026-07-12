// expo-haptics wrapper — DESIGN_SYSTEM.md §5.3 named events, exactly these, nowhere else.
// No-op on web (no native haptics API).
import AsyncStorage from "@react-native-async-storage/async-storage";
import * as Haptics from "expo-haptics";
import { Platform } from "react-native";

const HAPTICS_KEY = "forge.haptics";
let enabled = true;
let initialized = false;
let initPromise: Promise<boolean> | null = null;

export function isHapticsEnabled(): boolean {
  return enabled;
}

export function setHapticsEnabled(value: boolean): void {
  enabled = value;
  initialized = true;
  void AsyncStorage.setItem(HAPTICS_KEY, String(value)).catch(() => {
    // persistence is best-effort; the in-memory preference still applies
  });
}

export function initHaptics(): Promise<boolean> {
  if (initPromise) return initPromise;
  initPromise = AsyncStorage.getItem(HAPTICS_KEY)
    .then((raw) => {
      if (!initialized) enabled = raw !== "false";
      initialized = true;
      return enabled;
    })
    .catch(() => {
      initialized = true;
      return enabled;
    });
  return initPromise;
}

function fire(fn: () => Promise<void>): void {
  if (!enabled || Platform.OS === "web") return;
  fn().catch(() => {
    // haptics are best-effort; never surface a failure to the UI
  });
}

export const haptics = {
  /** send prompt / palette execute */
  sendPrompt: (): void =>
    fire(() => Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light)),
  /** allow / plan approve */
  allow: (): void =>
    fire(() => Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Medium)),
  /** deny / destructive confirm */
  deny: (): void =>
    fire(() => Haptics.notificationAsync(Haptics.NotificationFeedbackType.Warning)),
  /** pairing success / merge clean */
  pairSuccess: (): void =>
    fire(() => Haptics.notificationAsync(Haptics.NotificationFeedbackType.Success)),
  /** merge conflict / error toast */
  mergeConflict: (): void =>
    fire(() => Haptics.notificationAsync(Haptics.NotificationFeedbackType.Error)),
  /** palette & overlay row navigation (keyboard/drag) */
  select: (): void => fire(() => Haptics.selectionAsync()),
  /** pull-to-refresh settle */
  refreshSettle: (): void =>
    fire(() => Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light)),
};
