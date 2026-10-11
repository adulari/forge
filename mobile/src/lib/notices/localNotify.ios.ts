// iOS OS surface for notices. `expo-notifications` is already in the native build for the
// Anywhere push registration, so scheduling an immediate local notification and listening for push
// receipt/taps need no new native code.
//
// What this alone cannot do: JS only runs while the app process runs. A hosted push that lands
// while the app is suspended or killed is drawn by iOS from the APNs payload. For a device that
// registered a notification key, the Notification Service Extension (targets/notification-service)
// decrypts the sealed blob and puts the real text there; for one that did not, the lock screen
// keeps the generic text until the app next runs and `dismissGenericPushes` replaces it.
import * as Notifications from "expo-notifications";

import { GENERIC_PUSH_BODY } from "./core";
import { observeNotifications, scheduleNotice } from "./expoNotices";
import type { LocalNotice } from "./localNotify";

export type { LocalNotice, NoticeTapData, NotificationObserver } from "./localNotify";
export { observeNotifications };

export async function presentLocal(notice: LocalNotice): Promise<void> {
  const permission = await Notifications.getPermissionsAsync();
  if (!permission.granted) return;
  await scheduleNotice(notice);
}

/** Once the real text is in hand, the generic alert it stood in for is clutter. */
export async function dismissGenericPushes(): Promise<void> {
  const presented = await Notifications.getPresentedNotificationsAsync();
  await Promise.all(
    presented
      .filter((n) => n.request.content.body === GENERIC_PUSH_BODY)
      .map((n) => Notifications.dismissNotificationAsync(n.request.identifier)),
  );
}
