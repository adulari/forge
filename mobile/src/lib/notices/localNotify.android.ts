// Android OS surface for notices. Android has no remote push in this app (the APNs and Anywhere
// registrations are iOS-only), so the only way the real text reaches the notification shade is
// the app fetching the notice from the daemon's inbox and raising a local notification, which
// NoticesWatcher does when the process is alive but not in front.
//
// Two channels: `attention` (HIGH: heads-up, sound) for a session waiting on you or a failure, and
// `status` (LOW: silent, shade only) for everything else. The OS owns their sound and visibility
// from then on, which is why Settings links to the system page instead of mirroring it.
import * as Notifications from "expo-notifications";

import { ANDROID_CHANNELS, type AndroidChannel } from "../notifyPrefsCore";
import { observeNotifications, scheduleNotice } from "./expoNotices";
import type { LocalNotice } from "./localNotify";

export type { LocalNotice, NoticeTapData, NotificationObserver } from "./localNotify";
export { observeNotifications };

const LEGACY_CHANNEL_ID = "forge-sessions";

const IMPORTANCE: Record<"high" | "low", Notifications.AndroidImportance> = {
  high: Notifications.AndroidImportance.HIGH,
  low: Notifications.AndroidImportance.LOW,
};

let prepared: Promise<void> | null = null;

export function prepareNotificationChannels(): Promise<void> {
  prepared ??= (async () => {
    await Notifications.deleteNotificationChannelAsync(LEGACY_CHANNEL_ID).catch(() => undefined);
    await Promise.all(
      (Object.keys(ANDROID_CHANNELS) as AndroidChannel[]).map((id) => {
        const channel = ANDROID_CHANNELS[id];
        return Notifications.setNotificationChannelAsync(id, {
          name: channel.name,
          description: channel.description,
          importance: IMPORTANCE[channel.importance],
        });
      }),
    );
  })().catch((error) => {
    prepared = null;
    throw error;
  });
  return prepared;
}

async function canNotify(): Promise<boolean> {
  const permission = await Notifications.getPermissionsAsync();
  if (permission.granted) return true;
  // Android 13+ asks at runtime. Only ask when the OS will still show the dialog.
  if (!permission.canAskAgain) return false;
  return (await Notifications.requestPermissionsAsync()).granted;
}

export async function presentLocal(notice: LocalNotice): Promise<boolean> {
  if (!(await canNotify())) return false;
  await prepareNotificationChannels();
  await scheduleNotice({ ...notice, channel: notice.channel ?? "status" });
  return true;
}

export async function dismissGenericPushes(_replaced: number): Promise<void> {
  // No generic push exists on Android.
}
