import { useEffect } from "react";

import {
  markAnonymousTelemetryActivated,
  shouldShowAnonymousTelemetryNotice,
  startAnonymousTelemetry,
} from "../lib/anonymousTelemetry";
import { useAuth } from "../lib/auth";
import { useToast } from "./ds/ToastHost";

/** Starts anonymous counters and discloses them once without blocking first launch. */
export function AnonymousTelemetry() {
  const toast = useToast();
  const { isLoading, isPaired } = useAuth();

  useEffect(() => {
    const stop = startAnonymousTelemetry();
    void shouldShowAnonymousTelemetryNotice().then((show) => {
      if (show) {
        toast.show(
          "Forge shares anonymous usage counts—never code, prompts, paths, or device IDs. Opt out in Settings.",
          { duration: 8_000 },
        );
      }
    });
    return stop;
  }, [toast]);

  useEffect(() => {
    if (!isLoading && isPaired) markAnonymousTelemetryActivated();
  }, [isLoading, isPaired]);

  return null;
}
