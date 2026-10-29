// Publishes which sessions are on screen (the routed one plus every split pane) to the alert
// watchers, which sit above the workbench and cannot read it themselves. Renders nothing.
import { useEffect } from "react";

import { setVisibleSessionIds } from "../../lib/alertTargets";
import { visibleSessionIds } from "../../lib/notifyPrefsCore";
import { useWorkbench } from "../workbench/WorkbenchProvider";

export function VisibleSessionsPublisher() {
  const { paneIds, layout, primaryId } = useWorkbench();
  const splitActive = primaryId != null && layout.secondary.length > 0;
  const key = visibleSessionIds(paneIds, layout.maximized, splitActive).join("|");
  useEffect(() => {
    setVisibleSessionIds(key ? key.split("|") : []);
    return () => setVisibleSessionIds([]);
  }, [key]);
  return null;
}
