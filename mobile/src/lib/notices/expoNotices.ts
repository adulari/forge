// Shared expo-notifications half of the notices OS surface (iOS and Android): scheduling an
// immediate local notification that carries the notice text, and listening for taps and for a
// hosted push arriving. The platform files add what differs (iOS dismisses the generic alert it
// replaces; Android must create a channel and may need to ask for permission first).
import * as Notifications from "expo-notifications";

import type { LocalNotice, NoticeTapData, NotificationObserver } from "./localNotify";

const MARKER = "forgeNotice";

export async function scheduleNotice(notice: LocalNotice): Promise<void> {
  await Notifications.scheduleNotificationAsync({
    content: {
      title: notice.title,
      body: notice.body,
      data: { [MARKER]: true, serverId: notice.serverId, sessionId: notice.sessionId, seq: notice.seq, at: notice.at },
    },
    trigger: null,
  });
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
