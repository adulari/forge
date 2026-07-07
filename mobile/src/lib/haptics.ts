// expo-haptics wrapper — DESIGN_SYSTEM.md §5.3 named events, exactly these, nowhere else.
// No-op on web (no native haptics API).
import * as Haptics from "expo-haptics";
import { Platform } from "react-native";

function fire(fn: () => Promise<void>): void {
  if (Platform.OS === "web") return;
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
