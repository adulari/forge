// iOS OS surface for notices, JS-only: `expo-notifications` is already in the native build for
// the Anywhere push registration, so scheduling an immediate local notification and listening for
// push receipt/taps need no new native code.
//
// What this can and cannot do: JS only runs while the app process runs. A generic hosted push that
// lands while the app is suspended or killed is drawn by iOS from the APNs payload alone, so the
// lock screen keeps the generic text until the app next runs. Replacing it in the background would
// need a `remote-notification` background mode or a Notification Service Extension (native).
import * as Notifications from "expo-notifications";

import { GENERIC_PUSH_BODY } from "./core";
import type { LocalNotice, NoticeTapData, NotificationObserver } from "./localNotify";

export type { LocalNotice, NoticeTapData, NotificationObserver } from "./localNotify";

const MARKER = "forgeNotice";

export async function presentLocal(notice: LocalNotice): Promise<void> {
  const permission = await Notifications.getPermissionsAsync();
  if (!permission.granted) return;
  await Notifications.scheduleNotificationAsync({
    content: {
      title: notice.title,
      body: notice.body,
      data: { [MARKER]: true, serverId: notice.serverId, sessionId: notice.sessionId, seq: notice.seq, at: notice.at },
    },
    trigger: null,
  });
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

function tapData(data: Record<string, unknown> | undefined): NoticeTapData | null {
  if (!data || data[MARKER] !== true || typeof data.serverId !== "string") return null;
  return {
    serverId: data.serverId,
    sessionId: typeof data.sessionId === "string" ? data.sessionId : null,
    seq: typeof data.seq === "number" ? data.seq : 0,
    at: typeof data.at === "number" ? data.at : 0,
  };
}

export function observeNotifications(observer: NotificationObserver): () => void {
  const handleResponse = (response: Notifications.NotificationResponse) => {
    const own = tapData(response.notification.request.content.data);
    if (own) observer.onNoticeTap(own);
    else observer.onGeneric("tap");
  };
  const received = Notifications.addNotificationReceivedListener((notification) => {
    if (!tapData(notification.request.content.data)) observer.onGeneric("received");
  });
  const opened = Notifications.addNotificationResponseReceivedListener(handleResponse);
  void Notifications.getLastNotificationResponseAsync()
    .then((response) => {
      if (response) handleResponse(response);
    })
    .catch(() => undefined);
  return () => {
    received.remove();
    opened.remove();
  };
}
