import React, { useCallback, useState } from "react";
import { Platform } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { router } from "expo-router";

import { useAuth } from "../lib/auth";
import {
  canReclaimFrom,
  isPressured,
  pressureMessage,
  reclaimResultMessage,
  RELAY_RECLAIM_HINT,
} from "../lib/hostPressure";
import { useHostPressure, useReclaimHostSpace } from "../lib/queries";
import { Banner } from "./ds/Banner";
import { useToast } from "./ds/ToastHost";

/**
 * The host is low on disk or memory. Shown above every screen except Diagnostics and a live
 * session, with a Reclaim action when this device talks to the host directly: the relay does not
 * carry file deletion, so a relayed phone gets the command to run instead.
 */
export function HostPressureBanner() {
  const { baseUrl } = useAuth();
  const pressure = useHostPressure();
  const reclaim = useReclaimHostSpace();
  const toast = useToast();
  const [dismissedLevel, setDismissedLevel] = useState<string | null>(null);
  const direct = canReclaimFrom(baseUrl);

  const act = useCallback(async () => {
    if (!direct) {
      router.push("/diagnostics");
      return;
    }
    try {
      toast.show(reclaimResultMessage(await reclaim.mutateAsync(undefined)), { tone: "neutral" });
    } catch {
      toast.show("Couldn't reclaim space. Try `forge worktree reclaim` on the host.", { tone: "danger" });
    }
  }, [direct, reclaim, toast]);

  const current = pressure.data;
  if (!isPressured(current) || dismissedLevel === current.level) return null;
  const message = direct ? pressureMessage(current) : `${pressureMessage(current)} ${RELAY_RECLAIM_HINT}`;
  return (
    <SafeAreaView edges={Platform.OS === "ios" ? ["top"] : []}>
      <Banner
        compact
        tone={current.level === "floor" ? "danger" : "warn"}
        message={reclaim.isPending ? "Reclaiming worktree space..." : message}
        actionLabel={reclaim.isPending ? undefined : direct ? "Reclaim" : "Details"}
        onAction={act}
        onDismiss={reclaim.isPending ? undefined : () => setDismissedLevel(current.level)}
      />
    </SafeAreaView>
  );
}
