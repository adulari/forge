import React from "react";
import { Platform } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { router } from "expo-router";

import { useAuth } from "../lib/auth";
import {
  canReclaimFrom,
  dismissKey,
  isPressured,
  offersReclaim,
  pressureMessage,
  RELAY_RECLAIM_HINT,
} from "../lib/hostPressure";
import { dismissHostPressure, requestHostReclaim, useDismissedHostPressure } from "../lib/hostReclaimRequest";
import { useHostPressure, useReclaimHostSpace } from "../lib/queries";
import { Banner } from "./ds/Banner";

/**
 * The host is low on disk or memory. Shown above every screen except Diagnostics and a live
 * session, with a Reclaim action (after a confirmation) when a disk is the cause and this device
 * talks to the host directly: the relay does not carry file deletion, so a relayed phone gets the
 * command to run instead. Memory-only pressure is a plain line, since Reclaim frees disk.
 */
export function HostPressureBanner() {
  const { baseUrl } = useAuth();
  const pressure = useHostPressure();
  const reclaim = useReclaimHostSpace();
  const dismissed = useDismissedHostPressure();
  const direct = canReclaimFrom(baseUrl);

  const current = pressure.data;
  if (!isPressured(current) || dismissed === dismissKey(current)) return null;
  const reclaimable = offersReclaim(current);
  const message = direct || !reclaimable ? pressureMessage(current) : `${pressureMessage(current)} ${RELAY_RECLAIM_HINT}`;
  const actionLabel = reclaim.isPending || !reclaimable ? undefined : direct ? "Reclaim" : "Details";
  return (
    <SafeAreaView edges={Platform.OS === "ios" ? ["top"] : []}>
      <Banner
        compact
        tone={current.level === "floor" ? "danger" : "warn"}
        message={reclaim.isPending ? "Reclaiming worktree space..." : message}
        actionLabel={actionLabel}
        onAction={direct ? requestHostReclaim : () => router.push("/diagnostics")}
        onDismiss={reclaim.isPending ? undefined : () => dismissHostPressure(dismissKey(current))}
      />
    </SafeAreaView>
  );
}
