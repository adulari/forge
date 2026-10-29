// Non-iOS OS surface for notices. The browser and Tauri desktop shell share `notify()`, which
// feature-detects `Notification` vs the Tauri plugin. iOS and Android have their own files.
import { notify } from "../notify";
import { openSessionFromAlert } from "../openSessionFromAlert";
import type { AndroidChannel } from "../notifyPrefsCore";

export interface LocalNotice {
  title: string;
  body: string;
  serverId: string;
  sessionId: string | null;
  seq: number;
  at: number;
  /** Android notification channel; ignored elsewhere. */
  channel?: AndroidChannel;
  /** Play the alert sound (iOS); Android takes its sound from the channel. */
  sound?: boolean;
}

export interface NoticeTapData {
  serverId: string;
  sessionId: string | null;
  seq: number;
  at: number;
}

export interface NotificationObserver {
  /** A hosted (generic) push arrived or was tapped: the real text is only on the host. */
  onGeneric: (kind: "received" | "tap") => void;
  /** One of our own local notifications was tapped. */
  onNoticeTap: (data: NoticeTapData) => void;
}

export async function presentLocal(notice: LocalNotice): Promise<boolean> {
  await notify(notice.title, notice.body, {
    onClick: () => {
      if (notice.sessionId) openSessionFromAlert(notice.sessionId);
    },
  });
  return true;
}

/** Create the OS notification channels up front. Only Android has any. */
export async function prepareNotificationChannels(): Promise<void> {
  // nothing to prepare
}

export async function dismissGenericPushes(_replaced: number): Promise<void> {
  // No notification centre API outside iOS.
}

export function observeNotifications(_observer: NotificationObserver): () => void {
  return () => undefined;
}
