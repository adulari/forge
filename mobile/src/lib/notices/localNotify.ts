// Non-iOS OS surface for notices. The browser and Tauri desktop shell share `notify()`, which
// feature-detects `Notification` vs the Tauri plugin. iOS and Android have their own files.
import { notify } from "../notify";

export interface LocalNotice {
  title: string;
  body: string;
  serverId: string;
  sessionId: string | null;
  seq: number;
  at: number;
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

export async function presentLocal(notice: LocalNotice): Promise<void> {
  await notify(notice.title, notice.body);
}

export async function dismissGenericPushes(): Promise<void> {
  // No notification centre API outside iOS.
}

export function observeNotifications(_observer: NotificationObserver): () => void {
  return () => undefined;
}
