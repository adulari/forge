// The review keys. They are registered only while the IDE is on screen (a hidden <Activity> runs
// no effects), and their `ideReview` context keeps them out of the composer and terminal.
import { useEffect } from "react";

import { useAppShortcut } from "../../lib/shortcuts";
import { publishFocusTag } from "../../lib/shortcuts/focusContext";
import { diffController } from "./diffController";
import { useIde } from "./IdeContext";

export function useIdeShortcuts(): void {
  const { stepFile, markActiveReviewed, focusComposer } = useIde();
  useEffect(() => {
    publishFocusTag("ideReview", true);
    return () => publishFocusTag("ideReview", false);
  }, []);
  useAppShortcut("ide.nextFile", () => stepFile(1));
  useAppShortcut("ide.prevFile", () => stepFile(-1));
  useAppShortcut("ide.nextHunk", () => diffController()?.nextHunk());
  useAppShortcut("ide.prevHunk", () => diffController()?.prevHunk());
  useAppShortcut("ide.markReviewed", markActiveReviewed);
  useAppShortcut("ide.messageAgent", focusComposer);
}
