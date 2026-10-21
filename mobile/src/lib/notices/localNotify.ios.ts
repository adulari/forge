// iOS OS surface for notices. `expo-notifications` is already in the native build for the
// Anywhere push registration, so scheduling an immediate local notification and listening for push
// receipt/taps need no new native code.
//
// What this alone cannot do: JS only runs while the app process runs. A hosted push that lands
// while the app is suspended or killed is drawn by iOS from the APNs payload. For a device that
// registered a notification key, the Notification Service Extension (targets/notification-service)
// decrypts the sealed blob and puts the real text there; for one that did not, the lock screen
// keeps the generic text until the app next runs, raises the real text and `dismissGenericPushes`
// clears the generic alert it replaced.
import * as Notifications from "expo-notifications";

import { genericAlertsToDismiss } from "./core";
import { isOwnNoticeData, observeNotifications, scheduleNotice } from "./expoNotices";
import type { LocalNotice } from "./localNotify";

export type { LocalNotice, NoticeTapData, NotificationObserver } from "./localNotify";
export { observeNotifications };

export async function presentLocal(notice: LocalNotice): Promise<boolean> {
  const permission = await Notifications.getPermissionsAsync();
  if (!permission.granted) return false;
  await scheduleNotice(notice);
  return true;
}

/** Once `replaced` real-text notifications are on screen, the generic alerts they stood in for are
 * clutter. With nothing replacing them they are the only trace of the update, so they stay. */
export async function dismissGenericPushes(replaced: number): Promise<void> {
  if (replaced <= 0) return;
  const presented = await Notifications.getPresentedNotificationsAsync();
  const ids = genericAlertsToDismiss(
    presented.map((n) => ({
      id: n.request.identifier,
      body: n.request.content.body,
      own: isOwnNoticeData(n.request.content.data),
    })),
    replaced,
  );
  await Promise.all(ids.map((id) => Notifications.dismissNotificationAsync(id)));
}
