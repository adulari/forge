// Android OS surface for notices. Android has no remote push in this app (the APNs and Anywhere
// registrations are iOS-only), so the only way the real text reaches the notification shade is
// the app fetching the notice from the daemon's inbox and raising a local notification, which
// NoticesWatcher does when the process is alive but not in front. `expo-notifications` and its
// `forge-sessions` channel are already in the build.
import * as Notifications from "expo-notifications";

import { observeNotifications, scheduleNotice } from "./expoNotices";
import type { LocalNotice } from "./localNotify";

export type { LocalNotice, NoticeTapData, NotificationObserver } from "./localNotify";
export { observeNotifications };

const CHANNEL_ID = "forge-sessions";

async function canNotify(): Promise<boolean> {
  const permission = await Notifications.getPermissionsAsync();
  if (permission.granted) return true;
  // Android 13+ asks at runtime. Only ask when the OS will still show the dialog.
  if (!permission.canAskAgain) return false;
  return (await Notifications.requestPermissionsAsync()).granted;
}

export async function presentLocal(notice: LocalNotice): Promise<boolean> {
  if (!(await canNotify())) return false;
  await Notifications.setNotificationChannelAsync(CHANNEL_ID, {
    name: "Forge sessions",
    importance: Notifications.AndroidImportance.DEFAULT,
  });
  await scheduleNotice(notice);
  return true;
}

export async function dismissGenericPushes(_replaced: number): Promise<void> {
  // No generic push exists on Android.
}
