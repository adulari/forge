// One-time tip: the workbench is invisible until you know a shortcut, so the first time a
// session is open the shell says it exists and how to reach it. Dismissed (or acted on) once,
// remembered in AsyncStorage, never shown again.
import AsyncStorage from "@react-native-async-storage/async-storage";
import React, { useCallback, useEffect, useState } from "react";

import { runDesktopMenuAction } from "../../lib/desktopMenu";
import { useShortcutLabel } from "../../lib/shortcuts";
import { Banner } from "../ds/Banner";
import { useWorkbench } from "../workbench/WorkbenchProvider";

const TIP_SEEN_KEY = "forge.workbench.tipSeen";
const SHOW_DELAY_MS = 1500;

export function LayoutTip() {
  const workbench = useWorkbench();
  const splitHint = useShortcutLabel("workbench.split");
  const [seen, setSeen] = useState<boolean | null>(null);
  const [ready, setReady] = useState(false);
  const inSession = workbench.primaryId != null;
  // Someone who already split or opened a dock does not need telling.
  const alreadyUsing = workbench.layout.secondary.length > 0 || workbench.state.right.visible || workbench.state.bottom.visible;

  useEffect(() => {
    let cancelled = false;
    void AsyncStorage.getItem(TIP_SEEN_KEY)
      .then((raw) => {
        if (!cancelled) setSeen(raw === "1");
      })
      .catch(() => {
        if (!cancelled) setSeen(true);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    if (seen !== false || !inSession) return;
    const timer = setTimeout(() => setReady(true), SHOW_DELAY_MS);
    return () => clearTimeout(timer);
  }, [seen, inSession]);

  const dismiss = useCallback(() => {
    setSeen(true);
    void AsyncStorage.setItem(TIP_SEEN_KEY, "1").catch(() => undefined);
  }, []);

  useEffect(() => {
    if (seen === false && alreadyUsing) dismiss();
  }, [seen, alreadyUsing, dismiss]);

  if (seen !== false || !ready || !inSession || alreadyUsing) return null;

  // In flow above the panes (not floating): an overlay covered the composer's send button.
  return (
    <Banner
      tone="neutral"
      message={`Work on several sessions at once: split panes with ${splitHint}, drag a session in from the sidebar, or open Layout in the title bar for Tasks, Agents, Plan and Git review.`}
      actionLabel="Split now"
      onAction={() => {
        dismiss();
        runDesktopMenuAction("view:split-pane");
      }}
      onDismiss={dismiss}
    />
  );
}
