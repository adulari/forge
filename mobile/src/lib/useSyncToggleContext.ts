import { useMemo } from "react";
import { Platform } from "react-native";

import { useAuth } from "./auth";
import type { KeysToggleContext } from "./accountSync";
import { supportsDirectDaemonEndpoints } from "./transport";

/** Whether this client may switch on the keys category: only on a computer, connected directly. */
export function useSyncToggleContext(): KeysToggleContext {
  const { baseUrl } = useAuth();
  return useMemo(
    () => ({
      viaAnywhere: baseUrl != null && !supportsDirectDaemonEndpoints(baseUrl),
      isPhone: Platform.OS === "ios" || Platform.OS === "android",
    }),
    [baseUrl],
  );
}
