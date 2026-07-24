// Decision-waiting notifications for the desktop shell (docs/design/machined
// Desktop.dc.html L976-980, "native notification — generic when locked (Anywhere pushes reveal
// nothing)").
//
// The copy — and with it the privacy gate — lives in `desktopNotifyCore.ts`; this module is only
// the send path. `notify()` (lib/notify.ts) stays the single place that feature-detects Tauri vs
// browser Notification, and it never throws.
import { decisionCopy, type DecisionNotice } from "./desktopNotifyCore";
import { notify } from "./notify";

export {
  decisionCopy,
  isContentLocked,
  redactTrayTitle,
  type DecisionNotice,
  type NotificationCopy,
} from "./desktopNotifyCore";

/** Best-effort system notification for a session that has stopped and is waiting on the user. */
export async function notifyDecisionWaiting(notice: DecisionNotice): Promise<void> {
  const { title, body } = decisionCopy(notice);
  await notify(title, body);
}
