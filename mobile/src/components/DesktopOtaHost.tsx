import React, { useCallback, useEffect } from "react";

import {
  applyDesktopOta,
  dismissDesktopOtaBanner,
  markDesktopOtaHealthy,
  pendingBundleToOffer,
  refreshDesktopOtaStatus,
  startDesktopOtaListener,
  useDesktopOtaStatus,
} from "../lib/desktopOta";
import { isTauri } from "../lib/platform";
import { Banner } from "./ds/Banner";

/**
 * Desktop over-the-air UI updates (docs/features/desktop-ota.md). Reports that the app shell
 * rendered, which is what keeps a downloaded bundle from being rolled back, and offers
 * "Reload to update" when a newer bundle has been downloaded and verified.
 *
 * Mounted above the lock screen on purpose: a locked app has still rendered, and health must not
 * depend on the user unlocking it.
 */
export function DesktopOtaHost() {
  const status = useDesktopOtaStatus();

  useEffect(() => {
    if (!isTauri) return;
    void startDesktopOtaListener();
    void refreshDesktopOtaStatus();
    // Two frames: the first paints the shell, the second proves the page is still alive after it.
    let second = 0;
    const first = requestAnimationFrame(() => {
      second = requestAnimationFrame(() => void markDesktopOtaHealthy());
    });
    return () => {
      cancelAnimationFrame(first);
      cancelAnimationFrame(second);
    };
  }, []);

  const reload = useCallback(() => void applyDesktopOta().catch(() => undefined), []);

  if (!isTauri || !pendingBundleToOffer(status)) return null;
  return (
    <Banner
      compact
      tone="neutral"
      message="A Forge interface update is ready."
      actionLabel="Reload to update"
      onAction={reload}
      onDismiss={dismissDesktopOtaBanner}
    />
  );
}
