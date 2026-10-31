// Publishes which sessions are on screen (the routed one plus every split pane) to the alert
// watchers, which sit above the workbench and cannot read it themselves. Renders nothing.
import { useEffect, useSyncExternalStore } from "react";

import { getRenderedPanes, setVisibleSessionIds, subscribeRenderedPanes } from "../../lib/alertTargets";
import { visibleSessionIds } from "../../lib/notifyPrefsCore";
import { useWorkbench } from "../workbench/WorkbenchProvider";

export function VisibleSessionsPublisher() {
  const { paneIds } = useWorkbench();
  const rendered = useSyncExternalStore(subscribeRenderedPanes, getRenderedPanes, () => null);
  const key = visibleSessionIds(paneIds, rendered?.maximized ?? null, rendered?.count ?? null).join("|");
  useEffect(() => {
    setVisibleSessionIds(key ? key.split("|") : []);
    return () => setVisibleSessionIds([]);
  }, [key]);
  return null;
}
