import { router } from "expo-router";

import { requestComposerFocus } from "./alertTargets";

/** Where a toast action or a clicked notification lands: the session, with the composer ready. */
export function openSessionFromAlert(sessionId: string): void {
  requestComposerFocus(sessionId);
  router.push(`/session/${sessionId}`);
}
